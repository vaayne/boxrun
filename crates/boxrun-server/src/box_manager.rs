use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use dashmap::DashMap;
use tokio::sync::Mutex;

use boxrun_types::config::{MAX_TOTAL_CPU, MAX_TOTAL_MEMORY_MB};
use boxrun_types::error::{BoxRunError, ErrorCode};

use crate::events::{Event, EventBus};
use crate::store::{BoxRow, BoxValue, ExecRow, Store};

fn gen_id(prefix: &str) -> String {
    format!("{}_{}", prefix, nanoid::nanoid!(12))
}

fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

/// Tracks total resource usage across all running boxes.
#[derive(Debug, Default)]
pub struct ResourceUsage {
    pub total_cpu: i64,
    pub total_memory_mb: i64,
    pub box_count: i64,
}

/// Core business logic for managing boxes and executions.
/// Note: BoxLite integration is stubbed out — the real implementation would
/// call boxlite::Boxlite::default() etc. For now, the manager handles all
/// state management, resource tracking, and event streaming logic.
pub struct BoxManager {
    store: Store,
    event_bus: EventBus,
    resources: Arc<Mutex<ResourceUsage>>,
    // exec_id -> JoinHandle for streaming tasks
    exec_tasks: DashMap<String, tokio::task::JoinHandle<()>>,
    // box_id -> true (tracks which boxes we "own" a handle for)
    box_handles: DashMap<String, bool>,
}

impl BoxManager {
    pub fn new(store: Store, event_bus: EventBus) -> Self {
        Self {
            store,
            event_bus,
            resources: Arc::new(Mutex::new(ResourceUsage::default())),
            exec_tasks: DashMap::new(),
            box_handles: DashMap::new(),
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn event_bus(&self) -> &EventBus {
        &self.event_bus
    }

    /// Initialize the manager: recover running boxes from DB.
    pub async fn init(&self) -> Result<(), String> {
        let boxes = self.store.list_boxes(None).await?;
        let mut res = self.resources.lock().await;
        for box_data in boxes {
            if matches!(box_data.status.as_str(), "running" | "creating")
                && box_data.boxlite_id.is_some()
            {
                // In real implementation: try to get BoxLite handle
                // For now, mark as running and track resources
                self.box_handles.insert(box_data.id.clone(), true);
                res.total_cpu += box_data.cpu;
                res.total_memory_mb += box_data.memory_mb;
                res.box_count += 1;
            }
        }
        Ok(())
    }

    /// Shutdown: cancel all exec tasks.
    pub async fn shutdown(&self) {
        for entry in self.exec_tasks.iter() {
            entry.value().abort();
        }
        self.exec_tasks.clear();
    }

    // ── Box lifecycle ────────────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    pub async fn create_box(
        &self,
        image: &str,
        name: Option<&str>,
        cpu: i64,
        memory_mb: i64,
        disk_size_gb: i64,
        network: bool,
        workdir: &str,
        env: &Option<HashMap<String, String>>,
        volumes: &Option<Vec<serde_json::Value>>,
    ) -> Result<BoxRow, BoxRunError> {
        // Check resource limits
        {
            let res = self.resources.lock().await;
            if res.total_cpu + cpu > *MAX_TOTAL_CPU {
                return Err(BoxRunError::new(
                    ErrorCode::ResourceExceeded,
                    format!("Total CPU limit ({}) would be exceeded", *MAX_TOTAL_CPU),
                ));
            }
            if res.total_memory_mb + memory_mb > *MAX_TOTAL_MEMORY_MB {
                return Err(BoxRunError::new(
                    ErrorCode::ResourceExceeded,
                    format!(
                        "Total memory limit ({}MB) would be exceeded",
                        *MAX_TOTAL_MEMORY_MB
                    ),
                ));
            }
        }

        // Check for duplicate name
        if let Some(n) = name {
            if let Ok(Some(_)) = self.store.get_box(n).await {
                return Err(BoxRunError::new(
                    ErrorCode::NameAlreadyExists,
                    format!("A box with name '{n}' already exists"),
                ));
            }
        }

        // Validate volume host paths
        if let Some(vols) = volumes {
            for vol in vols {
                if let Some(hp) = vol.get("host_path").and_then(|v| v.as_str()) {
                    if !std::path::Path::new(hp).exists() {
                        return Err(BoxRunError::new(
                            ErrorCode::RuntimeError,
                            format!("Volume host path does not exist: {hp}"),
                        ));
                    }
                }
            }
        }

        let box_id = gen_id("box");

        // Record in DB
        let _box_data = self
            .store
            .create_box(
                &box_id,
                name,
                image,
                cpu,
                memory_mb,
                disk_size_gb,
                network,
                workdir,
                env,
                volumes,
            )
            .await
            .map_err(|e| {
                if e.contains("already exists") {
                    BoxRunError::new(ErrorCode::NameAlreadyExists, e)
                } else {
                    BoxRunError::new(ErrorCode::RuntimeError, e)
                }
            })?;

        // In real implementation: create BoxLite VM, start it
        // For now, simulate successful creation
        self.box_handles.insert(box_id.clone(), true);
        {
            let mut res = self.resources.lock().await;
            res.total_cpu += cpu;
            res.total_memory_mb += memory_mb;
            res.box_count += 1;
        }

        let box_data = self
            .store
            .update_box(
                &box_id,
                &[
                    ("status", BoxValue::Text("running".into())),
                    ("boxlite_id", BoxValue::Text(format!("bl_{box_id}"))),
                    ("started_at", BoxValue::Text(now_iso())),
                ],
            )
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e))?;

        Ok(box_data)
    }

    pub async fn get_box(&self, id_or_name: &str) -> Result<BoxRow, BoxRunError> {
        self.store
            .get_box(id_or_name)
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e))?
            .ok_or_else(|| {
                BoxRunError::new(
                    ErrorCode::BoxNotFound,
                    format!("Box '{id_or_name}' not found"),
                )
            })
    }

    pub async fn list_boxes(&self, status: Option<&str>) -> Result<Vec<BoxRow>, BoxRunError> {
        self.store
            .list_boxes(status)
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e))
    }

    pub async fn stop_box(&self, id_or_name: &str) -> Result<BoxRow, BoxRunError> {
        let box_data = self.get_box(id_or_name).await?;

        if box_data.status != "running" {
            return Err(BoxRunError::new(
                ErrorCode::BoxNotRunning,
                format!(
                    "Box '{}' is not running (status: {})",
                    id_or_name, box_data.status
                ),
            ));
        }

        // In real implementation: call handle.stop()
        self.box_handles.remove(&box_data.id);
        {
            let mut res = self.resources.lock().await;
            res.total_cpu -= box_data.cpu;
            res.total_memory_mb -= box_data.memory_mb;
            res.box_count -= 1;
        }

        self.store
            .update_box(
                &box_data.id,
                &[
                    ("status", BoxValue::Text("stopped".into())),
                    ("stopped_at", BoxValue::Text(now_iso())),
                ],
            )
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e))
    }

    pub async fn start_box(&self, id_or_name: &str) -> Result<BoxRow, BoxRunError> {
        let box_data = self.get_box(id_or_name).await?;

        if box_data.status == "running" {
            return Err(BoxRunError::new(
                ErrorCode::BoxAlreadyRunning,
                format!("Box '{id_or_name}' is already running"),
            ));
        }
        if box_data.status != "stopped" {
            return Err(BoxRunError::new(
                ErrorCode::RuntimeError,
                format!(
                    "Box '{id_or_name}' cannot be started (status: {})",
                    box_data.status
                ),
            ));
        }

        // Check resource limits
        {
            let res = self.resources.lock().await;
            if res.total_cpu + box_data.cpu > *MAX_TOTAL_CPU {
                return Err(BoxRunError::new(
                    ErrorCode::ResourceExceeded,
                    "Total CPU limit would be exceeded",
                ));
            }
            if res.total_memory_mb + box_data.memory_mb > *MAX_TOTAL_MEMORY_MB {
                return Err(BoxRunError::new(
                    ErrorCode::ResourceExceeded,
                    "Total memory limit would be exceeded",
                ));
            }
        }

        // In real implementation: get handle, call start()
        self.box_handles.insert(box_data.id.clone(), true);
        {
            let mut res = self.resources.lock().await;
            res.total_cpu += box_data.cpu;
            res.total_memory_mb += box_data.memory_mb;
            res.box_count += 1;
        }

        self.store
            .update_box(
                &box_data.id,
                &[
                    ("status", BoxValue::Text("running".into())),
                    ("started_at", BoxValue::Text(now_iso())),
                    ("stopped_at", BoxValue::Null),
                ],
            )
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e))
    }

    pub async fn remove_box(&self, id_or_name: &str, force: bool) -> Result<(), BoxRunError> {
        let box_data = self.get_box(id_or_name).await?;

        if box_data.status == "running" {
            if !force {
                return Err(BoxRunError::new(
                    ErrorCode::BoxAlreadyRunning,
                    "Box is running. Use force=true to remove.",
                ));
            }
            // Force stop
            self.box_handles.remove(&box_data.id);
            {
                let mut res = self.resources.lock().await;
                res.total_cpu -= box_data.cpu;
                res.total_memory_mb -= box_data.memory_mb;
                res.box_count -= 1;
            }
        }

        self.box_handles.remove(&box_data.id);
        self.store
            .delete_box(&box_data.id)
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e))
    }

    // ── Exec ─────────────────────────────────────────────────────────────

    pub async fn exec_in_box(
        &self,
        id_or_name: &str,
        cmd: &[String],
        env: &Option<HashMap<String, String>>,
        workdir: Option<&str>,
        timeout_ms: Option<i64>,
    ) -> Result<ExecRow, BoxRunError> {
        let box_data = self.get_box(id_or_name).await?;

        if box_data.status != "running" {
            return Err(BoxRunError::new(
                ErrorCode::BoxNotRunning,
                format!("Box '{}' is not running", id_or_name),
            ));
        }

        if !self.box_handles.contains_key(&box_data.id) {
            return Err(BoxRunError::new(
                ErrorCode::RuntimeError,
                "BoxLite handle not found",
            ));
        }

        let exec_id = gen_id("exec");
        let exec_data = self
            .store
            .create_exec(&exec_id, &box_data.id, cmd, env, workdir, timeout_ms)
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e))?;

        // In real implementation: start execution in BoxLite, spawn streaming task.
        // For now, spawn a task that simulates completion.
        let store = self.store.clone();
        let event_bus = self.event_bus.clone();
        let exec_id_clone = exec_id.clone();

        let task = tokio::spawn(async move {
            // In real implementation: read stdout/stderr streams, publish events
            // Simulate immediate completion with exit code 0
            let exit_data = serde_json::json!({"exit_code": 0}).to_string();
            let exit_event = Event {
                seq: 0,
                event_type: "exit".into(),
                data: exit_data.clone(),
                stream: None,
            };
            let _ = store
                .append_event(&exec_id_clone, 0, "exit", &exit_data, None)
                .await;
            event_bus.publish(&exec_id_clone, exit_event).await;
            let _ = store.finish_exec(&exec_id_clone, 0, None).await;
            event_bus.finish(&exec_id_clone).await;
        });

        self.exec_tasks.insert(exec_id, task);

        Ok(exec_data)
    }

    pub async fn get_exec(&self, exec_id: &str) -> Result<ExecRow, BoxRunError> {
        self.store
            .get_exec(exec_id)
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e))?
            .ok_or_else(|| {
                BoxRunError::new(
                    ErrorCode::ExecNotFound,
                    format!("Exec '{exec_id}' not found"),
                )
            })
    }

    pub async fn cancel_exec(&self, exec_id: &str) -> Result<ExecRow, BoxRunError> {
        let exec_data = self.get_exec(exec_id).await?;

        if exec_data.status != "running" {
            return Err(BoxRunError::new(
                ErrorCode::ExecAlreadyFinished,
                "Exec already finished",
            ));
        }

        // Cancel the streaming task
        if let Some((_, task)) = self.exec_tasks.remove(exec_id) {
            task.abort();
        }

        self.store
            .cancel_exec(exec_id)
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e))
    }

    // ── Files ────────────────────────────────────────────────────────────

    pub async fn upload_file(
        &self,
        id_or_name: &str,
        _host_path: &str,
        _dest_path: &str,
    ) -> Result<(), BoxRunError> {
        let box_data = self.get_box(id_or_name).await?;
        if box_data.status != "running" {
            return Err(BoxRunError::new(
                ErrorCode::BoxNotRunning,
                "Box is not running",
            ));
        }
        if !self.box_handles.contains_key(&box_data.id) {
            return Err(BoxRunError::new(
                ErrorCode::RuntimeError,
                "BoxLite handle not found",
            ));
        }
        // In real implementation: copy_in via BoxLite
        Ok(())
    }

    pub async fn download_file(
        &self,
        id_or_name: &str,
        _src_path: &str,
        _host_dest: &str,
    ) -> Result<(), BoxRunError> {
        let box_data = self.get_box(id_or_name).await?;
        if box_data.status != "running" {
            return Err(BoxRunError::new(
                ErrorCode::BoxNotRunning,
                "Box is not running",
            ));
        }
        if !self.box_handles.contains_key(&box_data.id) {
            return Err(BoxRunError::new(
                ErrorCode::RuntimeError,
                "BoxLite handle not found",
            ));
        }
        // In real implementation: copy_out via BoxLite
        Ok(())
    }

    // ── GC ───────────────────────────────────────────────────────────────

    pub async fn gc(&self, older_than: i64) -> Result<i64, BoxRunError> {
        let box_ids = self
            .store
            .gc_stopped_boxes(older_than)
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e))?;
        let count = box_ids.len() as i64;
        for box_id in &box_ids {
            // In real implementation: remove from BoxLite
            self.store
                .delete_box(box_id)
                .await
                .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e))?;
        }
        Ok(count)
    }

    /// Get current resource usage snapshot.
    #[allow(dead_code)]
    pub async fn resource_usage(&self) -> ResourceUsage {
        let res = self.resources.lock().await;
        ResourceUsage {
            total_cpu: res.total_cpu,
            total_memory_mb: res.total_memory_mb,
            box_count: res.box_count,
        }
    }

    /// Check if an exec task is still running (for /run endpoint).
    #[allow(dead_code)]
    pub fn get_exec_task(
        &self,
        exec_id: &str,
    ) -> Option<dashmap::mapref::one::Ref<'_, String, tokio::task::JoinHandle<()>>> {
        self.exec_tasks.get(exec_id)
    }
}
