use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use dashmap::DashMap;
use futures::StreamExt;
use tokio::sync::Mutex;

use boxlite::runtime::options::{BoxOptions as BlBoxOptions, NetworkSpec, RootfsSpec, VolumeSpec};
use boxlite::{BoxCommand, BoxliteRuntime, CopyOptions};

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
/// Integrates with BoxLite runtime for VM lifecycle and command execution.
pub struct BoxManager {
    store: Store,
    event_bus: EventBus,
    resources: Arc<Mutex<ResourceUsage>>,
    exec_tasks: DashMap<String, tokio::task::JoinHandle<()>>,
    runtime: BoxliteRuntime,
}

impl BoxManager {
    pub fn new(store: Store, event_bus: EventBus) -> Result<Self, String> {
        let runtime = BoxliteRuntime::with_defaults()
            .map_err(|e| format!("Failed to initialize BoxLite runtime: {e}"))?;
        Ok(Self {
            store,
            event_bus,
            resources: Arc::new(Mutex::new(ResourceUsage::default())),
            exec_tasks: DashMap::new(),
            runtime,
        })
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn event_bus(&self) -> &EventBus {
        &self.event_bus
    }

    #[allow(dead_code)]
    pub fn runtime(&self) -> &BoxliteRuntime {
        &self.runtime
    }

    /// Initialize the manager: recover running boxes from DB.
    pub async fn init(&self) -> Result<(), String> {
        let boxes = self.store.list_boxes(None).await?;
        let mut res = self.resources.lock().await;
        for box_data in boxes {
            if let Some(boxlite_id) = box_data.boxlite_id.as_ref() {
                if !matches!(box_data.status.as_str(), "running" | "creating") {
                    continue;
                }
                match self.runtime.get(boxlite_id).await {
                    Ok(Some(_)) => {
                        // Box still exists in BoxLite, track resources
                        res.total_cpu += box_data.cpu;
                        res.total_memory_mb += box_data.memory_mb;
                        res.box_count += 1;
                    }
                    _ => {
                        // Box no longer exists in BoxLite, mark as stopped
                        tracing::warn!(
                            "Box {} (boxlite_id={}) not found in BoxLite, marking as stopped",
                            box_data.id,
                            boxlite_id
                        );
                        let _ = self
                            .store
                            .update_box(
                                &box_data.id,
                                &[
                                    ("status", BoxValue::Text("stopped".into())),
                                    ("stopped_at", BoxValue::Text(now_iso())),
                                ],
                            )
                            .await;
                    }
                }
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

    /// Get a BoxLite handle for a box, looking up by boxlite_id.
    async fn get_litebox(&self, box_data: &BoxRow) -> Result<boxlite::LiteBox, BoxRunError> {
        let boxlite_id = box_data
            .boxlite_id
            .as_ref()
            .ok_or_else(|| BoxRunError::new(ErrorCode::RuntimeError, "Box has no BoxLite ID"))?;
        self.runtime
            .get(boxlite_id)
            .await
            .map_err(|e| {
                BoxRunError::new(
                    ErrorCode::RuntimeError,
                    format!("Failed to get BoxLite handle: {e}"),
                )
            })?
            .ok_or_else(|| BoxRunError::new(ErrorCode::RuntimeError, "BoxLite VM not found"))
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

        // Record in DB with "creating" status
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

        // Build BoxLite options
        let bl_env: Vec<(String, String)> = env
            .as_ref()
            .map(|e| e.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();

        let bl_volumes: Vec<VolumeSpec> = volumes
            .as_ref()
            .map(|vols| {
                vols.iter()
                    .filter_map(|v| {
                        let host_path = v.get("host_path")?.as_str()?.to_string();
                        let guest_path = v.get("guest_path")?.as_str()?.to_string();
                        let read_only =
                            v.get("readonly").and_then(|r| r.as_bool()).unwrap_or(false);
                        Some(VolumeSpec {
                            host_path,
                            guest_path,
                            read_only,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let bl_options = BlBoxOptions {
            cpus: Some(cpu as u8),
            memory_mib: Some(memory_mb as u32),
            disk_size_gb: if disk_size_gb > 0 {
                Some(disk_size_gb as u64)
            } else {
                None
            },
            working_dir: Some(workdir.to_string()),
            env: bl_env,
            rootfs: RootfsSpec::Image(image.to_string()),
            volumes: bl_volumes,
            network: if network {
                NetworkSpec::default()
            } else {
                NetworkSpec::Isolated
            },
            auto_remove: false, // Persistent boxes
            detach: true,       // Persist after parent exit
            ..Default::default()
        };

        // Reserve resources atomically (check + reserve in same lock)
        // Prevents TOCTOU race where concurrent creates both pass the check
        {
            let mut res = self.resources.lock().await;
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
            res.total_cpu += cpu;
            res.total_memory_mb += memory_mb;
            res.box_count += 1;
        }

        // Create the BoxLite VM
        let litebox = match self.runtime.create(bl_options, Some(box_id.clone())).await {
            Ok(lb) => lb,
            Err(e) => {
                // Release reserved resources
                let mut res = self.resources.lock().await;
                res.total_cpu -= cpu;
                res.total_memory_mb -= memory_mb;
                res.box_count -= 1;
                return Err(BoxRunError::new(
                    ErrorCode::RuntimeError,
                    format!("Failed to create BoxLite VM: {e}"),
                ));
            }
        };

        let boxlite_id = litebox.id().as_str().to_string();

        // Start the BoxLite VM
        if let Err(e) = litebox.start().await {
            // Release reserved resources and clean up created VM
            {
                let mut res = self.resources.lock().await;
                res.total_cpu -= cpu;
                res.total_memory_mb -= memory_mb;
                res.box_count -= 1;
            }
            let _ = self.runtime.remove(&boxlite_id, true).await;
            return Err(BoxRunError::new(
                ErrorCode::RuntimeError,
                format!("Failed to start BoxLite VM: {e}"),
            ));
        }

        let box_data = self
            .store
            .update_box(
                &box_id,
                &[
                    ("status", BoxValue::Text("running".into())),
                    ("boxlite_id", BoxValue::Text(boxlite_id)),
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

        // Stop via BoxLite
        if let Ok(litebox) = self.get_litebox(&box_data).await {
            if let Err(e) = litebox.stop().await {
                tracing::warn!("Failed to stop BoxLite VM: {}", e);
            }
        }

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

        // Get BoxLite handle first (cheap, before resource reservation)
        let litebox = self.get_litebox(&box_data).await?;

        // Reserve resources atomically (check + reserve in same lock)
        {
            let mut res = self.resources.lock().await;
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
            res.total_cpu += box_data.cpu;
            res.total_memory_mb += box_data.memory_mb;
            res.box_count += 1;
        }

        // Start via BoxLite
        if let Err(e) = litebox.start().await {
            // Release reserved resources
            let mut res = self.resources.lock().await;
            res.total_cpu -= box_data.cpu;
            res.total_memory_mb -= box_data.memory_mb;
            res.box_count -= 1;
            return Err(BoxRunError::new(
                ErrorCode::RuntimeError,
                format!("Failed to start BoxLite VM: {e}"),
            ));
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

        if box_data.status == "running" && !force {
            return Err(BoxRunError::new(
                ErrorCode::BoxAlreadyRunning,
                "Box is running. Use force=true to remove.",
            ));
        }

        // Remove from BoxLite first, then release resources
        if let Some(ref boxlite_id) = box_data.boxlite_id {
            if let Err(e) = self.runtime.remove(boxlite_id, force).await {
                tracing::warn!("Failed to remove BoxLite VM {}: {}", boxlite_id, e);
            }
        }

        // Release resources only for running boxes (stopped boxes already released)
        if box_data.status == "running" {
            let mut res = self.resources.lock().await;
            res.total_cpu -= box_data.cpu;
            res.total_memory_mb -= box_data.memory_mb;
            res.box_count -= 1;
        }

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

        let litebox = self.get_litebox(&box_data).await?;

        let exec_id = gen_id("exec");
        let exec_data = self
            .store
            .create_exec(&exec_id, &box_data.id, cmd, env, workdir, timeout_ms)
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e))?;

        // Build BoxCommand
        let effective_workdir = workdir.unwrap_or(&box_data.workdir);
        let mut box_cmd = BoxCommand::new(&cmd[0]);
        if cmd.len() > 1 {
            box_cmd = box_cmd.args(cmd[1..].iter().map(|s| s.as_str()));
        }
        box_cmd = box_cmd.working_dir(effective_workdir);

        // Add environment variables
        if let Some(env_map) = env {
            for (k, v) in env_map {
                box_cmd = box_cmd.env(k, v);
            }
        }

        // Add timeout
        if let Some(ms) = timeout_ms {
            if ms > 0 {
                box_cmd = box_cmd.timeout(Duration::from_millis(ms as u64));
            }
        }

        // Start execution
        let mut execution = litebox.exec(box_cmd).await.map_err(|e| {
            BoxRunError::new(
                ErrorCode::RuntimeError,
                format!("Failed to start execution: {e}"),
            )
        })?;

        // Take stdout and stderr streams (take-once)
        let stdout_stream = execution.stdout();
        let stderr_stream = execution.stderr();

        // Spawn streaming task
        let store = self.store.clone();
        let event_bus = self.event_bus.clone();
        let exec_id_clone = exec_id.clone();

        let task = tokio::spawn(async move {
            let seq = Arc::new(AtomicI64::new(0));

            // Spawn stdout reader
            let stdout_handle = {
                let seq = seq.clone();
                let store = store.clone();
                let event_bus = event_bus.clone();
                let exec_id = exec_id_clone.clone();
                tokio::spawn(async move {
                    if let Some(mut stream) = stdout_stream {
                        while let Some(chunk) = stream.next().await {
                            let s = seq.fetch_add(1, Ordering::SeqCst);
                            let event = Event {
                                seq: s,
                                event_type: "log".into(),
                                data: chunk.clone(),
                                stream: Some("stdout".into()),
                            };
                            let _ = store
                                .append_event(&exec_id, s, "log", &chunk, Some("stdout"))
                                .await;
                            event_bus.publish(&exec_id, event).await;
                        }
                    }
                })
            };

            // Spawn stderr reader
            let stderr_handle = {
                let seq = seq.clone();
                let store = store.clone();
                let event_bus = event_bus.clone();
                let exec_id = exec_id_clone.clone();
                tokio::spawn(async move {
                    if let Some(mut stream) = stderr_stream {
                        while let Some(chunk) = stream.next().await {
                            let s = seq.fetch_add(1, Ordering::SeqCst);
                            let event = Event {
                                seq: s,
                                event_type: "log".into(),
                                data: chunk.clone(),
                                stream: Some("stderr".into()),
                            };
                            let _ = store
                                .append_event(&exec_id, s, "log", &chunk, Some("stderr"))
                                .await;
                            event_bus.publish(&exec_id, event).await;
                        }
                    }
                })
            };

            // Wait for both streams to complete
            let _ = stdout_handle.await;
            let _ = stderr_handle.await;

            // Get execution result
            let result = execution.wait().await;
            let (exit_code, error_msg) = match result {
                Ok(r) => (r.exit_code as i64, r.error_message),
                Err(e) => (-1_i64, Some(format!("{e}"))),
            };

            // Publish exit event
            let s = seq.load(Ordering::SeqCst);
            let exit_data = if let Some(ref msg) = error_msg {
                serde_json::json!({"exit_code": exit_code, "error": msg}).to_string()
            } else {
                serde_json::json!({"exit_code": exit_code}).to_string()
            };

            let exit_event = Event {
                seq: s,
                event_type: "exit".into(),
                data: exit_data.clone(),
                stream: None,
            };
            let _ = store
                .append_event(&exec_id_clone, s, "exit", &exit_data, None)
                .await;
            event_bus.publish(&exec_id_clone, exit_event).await;
            let _ = store
                .finish_exec(&exec_id_clone, exit_code, error_msg.as_deref())
                .await;
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

        // Cancel the streaming task (this aborts the tokio task,
        // which drops the Execution handle and should clean up)
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
        host_path: &str,
        dest_path: &str,
    ) -> Result<(), BoxRunError> {
        let box_data = self.get_box(id_or_name).await?;
        if box_data.status != "running" {
            return Err(BoxRunError::new(
                ErrorCode::BoxNotRunning,
                "Box is not running",
            ));
        }

        let litebox = self.get_litebox(&box_data).await?;

        // BoxLite copy_into treats dest as a directory and uses the source filename.
        // Rename the host file to match the desired dest filename, then copy to parent dir.
        let dest = std::path::Path::new(dest_path);
        let dest_name = dest.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
            BoxRunError::new(
                ErrorCode::RuntimeError,
                format!("Invalid destination path (no filename): {dest_path}"),
            )
        })?;
        let dest_dir = dest
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());

        let host = std::path::Path::new(host_path);
        let renamed = host.with_file_name(dest_name);

        // Rename (or copy) the source file to match the desired dest name
        if host_path != renamed.to_string_lossy() {
            std::fs::copy(host_path, &renamed).map_err(|e| {
                BoxRunError::new(
                    ErrorCode::RuntimeError,
                    format!("Failed to rename upload file: {e}"),
                )
            })?;
        }

        let opts = CopyOptions {
            include_parent: false,
            ..Default::default()
        };

        let result = litebox
            .copy_into(renamed.to_str().unwrap_or(""), &dest_dir, opts)
            .await;

        // Clean up the renamed file (if we created a copy)
        if host_path != renamed.to_string_lossy() {
            let _ = std::fs::remove_file(&renamed);
        }

        result.map_err(|e| {
            BoxRunError::new(
                ErrorCode::RuntimeError,
                format!("Failed to upload file: {e}"),
            )
        })
    }

    pub async fn download_file(
        &self,
        id_or_name: &str,
        src_path: &str,
        host_dest: &str,
    ) -> Result<(), BoxRunError> {
        let box_data = self.get_box(id_or_name).await?;
        if box_data.status != "running" {
            return Err(BoxRunError::new(
                ErrorCode::BoxNotRunning,
                "Box is not running",
            ));
        }

        let litebox = self.get_litebox(&box_data).await?;

        // BoxLite copy_out treats host_dest as a directory, placing the file with
        // its container filename inside. Use a temp dir, then move the result.
        let src_name = std::path::Path::new(src_path)
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| {
                BoxRunError::new(
                    ErrorCode::RuntimeError,
                    format!("Invalid source path (no filename): {src_path}"),
                )
            })?;

        let tmp_dir = tempfile::tempdir().map_err(|e| {
            BoxRunError::new(
                ErrorCode::RuntimeError,
                format!("Failed to create temp dir: {e}"),
            )
        })?;

        let opts = CopyOptions {
            include_parent: false,
            ..Default::default()
        };

        litebox
            .copy_out(src_path, tmp_dir.path().to_str().unwrap_or(""), opts)
            .await
            .map_err(|e| {
                BoxRunError::new(
                    ErrorCode::RuntimeError,
                    format!("Failed to download file: {e}"),
                )
            })?;

        // Move the downloaded file from temp dir to the final destination
        let copied_file = tmp_dir.path().join(src_name);
        if !copied_file.exists() {
            return Err(BoxRunError::new(
                ErrorCode::RuntimeError,
                format!("File '{src_path}' not found in box"),
            ));
        }

        std::fs::copy(&copied_file, host_dest).map_err(|e| {
            BoxRunError::new(
                ErrorCode::RuntimeError,
                format!("Failed to move downloaded file: {e}"),
            )
        })?;

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
            // Try to remove from BoxLite (may already be gone)
            if let Ok(Some(box_data)) = self.store.get_box(box_id).await {
                if let Some(ref boxlite_id) = box_data.boxlite_id {
                    let _ = self.runtime.remove(boxlite_id, true).await;
                }
            }
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
