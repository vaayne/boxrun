use std::collections::HashMap;
use std::path::Path;
use std::pin::Pin;

use boxrun_types::config::resolve_image;
use boxrun_types::error::{BoxRunError, ErrorCode};
use eventsource_stream::Eventsource;
use futures::{Stream, StreamExt};
use serde_json::{json, Value};

/// A stream of exec events from an SSE connection.
pub type ExecEventStream = Pin<Box<dyn Stream<Item = Result<ExecEvent, BoxRunError>> + Send>>;

/// Information about a box.
#[derive(Debug, Clone)]
pub struct BoxInfo {
    pub id: String,
    pub name: Option<String>,
    pub status: String,
    pub image: String,
    pub cpu: i64,
    pub memory_mb: i64,
    pub disk_size_gb: i64,
    pub network: bool,
    pub workdir: String,
    pub env: Option<HashMap<String, String>>,
    pub volumes: Option<Vec<Value>>,
    pub boxlite_id: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub stopped_at: Option<String>,
}

impl BoxInfo {
    fn from_json(data: &Value) -> Self {
        Self {
            id: data["id"].as_str().unwrap_or("").into(),
            name: data["name"].as_str().map(|s| s.into()),
            status: data["status"].as_str().unwrap_or("").into(),
            image: data["image"].as_str().unwrap_or("").into(),
            cpu: data["cpu"].as_i64().unwrap_or(0),
            memory_mb: data["memory_mb"].as_i64().unwrap_or(0),
            disk_size_gb: data["disk_size_gb"].as_i64().unwrap_or(0),
            network: data["network"].as_bool().unwrap_or(false),
            workdir: data["workdir"].as_str().unwrap_or("/root").into(),
            env: data["env"].as_object().map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").into()))
                    .collect()
            }),
            volumes: data["volumes"].as_array().cloned(),
            boxlite_id: data["boxlite_id"].as_str().map(|s| s.into()),
            error_code: data["error_code"].as_str().map(|s| s.into()),
            error_message: data["error_message"].as_str().map(|s| s.into()),
            created_at: data["created_at"].as_str().unwrap_or("").into(),
            started_at: data["started_at"].as_str().map(|s| s.into()),
            stopped_at: data["stopped_at"].as_str().map(|s| s.into()),
        }
    }
}

/// Information about an execution.
#[derive(Debug, Clone)]
pub struct ExecInfo {
    pub id: String,
    pub box_id: String,
    pub status: String,
    pub cmd: Vec<String>,
    pub env: Option<HashMap<String, String>>,
    pub workdir: Option<String>,
    pub timeout_ms: Option<i64>,
    pub exit_code: Option<i64>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub finished_at: Option<String>,
}

impl ExecInfo {
    fn from_json(data: &Value) -> Self {
        Self {
            id: data["id"].as_str().unwrap_or("").into(),
            box_id: data["box_id"].as_str().unwrap_or("").into(),
            status: data["status"].as_str().unwrap_or("").into(),
            cmd: data["cmd"]
                .as_array()
                .map(|a| a.iter().map(|v| v.as_str().unwrap_or("").into()).collect())
                .unwrap_or_default(),
            env: data["env"].as_object().map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").into()))
                    .collect()
            }),
            workdir: data["workdir"].as_str().map(|s| s.into()),
            timeout_ms: data["timeout_ms"].as_i64(),
            exit_code: data["exit_code"].as_i64(),
            error_message: data["error_message"].as_str().map(|s| s.into()),
            created_at: data["created_at"].as_str().unwrap_or("").into(),
            finished_at: data["finished_at"].as_str().map(|s| s.into()),
        }
    }
}

/// An SSE event from an execution.
#[derive(Debug, Clone)]
pub struct ExecEvent {
    pub event_type: String,
    pub data: String,
    pub stream: Option<String>,
    pub seq: i64,
}

/// Result from an ephemeral /run call.
#[derive(Debug, Clone)]
pub struct RunResult {
    pub exit_code: Option<i64>,
    pub stdout: String,
    pub stderr: String,
    pub error_message: Option<String>,
}

/// Async Rust SDK client for the BoxRun server.
pub struct BoxRunClient {
    http: reqwest::Client,
    base_url: String,
}

impl BoxRunClient {
    /// Create a new client. If base_url is None, auto-detects Unix socket or TCP.
    pub fn new(base_url: Option<&str>) -> Self {
        let http = reqwest::Client::new();
        let url = if let Some(u) = base_url {
            u.to_string()
        } else {
            // reqwest doesn't support Unix sockets natively, use TCP
            let host = std::env::var("BOXRUN_HOST").unwrap_or_else(|_| "127.0.0.1".into());
            let port = std::env::var("BOXRUN_PORT").unwrap_or_else(|_| "9090".into());
            format!("http://{host}:{port}")
        };
        Self {
            http,
            base_url: url,
        }
    }

    fn check_error(status: u16, body: &str) -> Result<Value, BoxRunError> {
        if status >= 400 {
            if let Ok(data) = serde_json::from_str::<Value>(body) {
                if let Some(obj) = data.as_object() {
                    let code = obj
                        .get("code")
                        .and_then(|v| v.as_str())
                        .unwrap_or("UNKNOWN");
                    let message = obj.get("message").and_then(|v| v.as_str()).unwrap_or(body);
                    return Err(BoxRunError::new(
                        ErrorCode::RuntimeError,
                        format!("[{code}] {message}"),
                    ));
                }
            }
            return Err(BoxRunError::new(
                ErrorCode::RuntimeError,
                format!("HTTP {status}: {body}"),
            ));
        }
        serde_json::from_str(body)
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, format!("Invalid JSON: {e}")))
    }

    // ── Box operations ───────────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        &self,
        image: &str,
        name: Option<&str>,
        cpu: i64,
        memory_mb: i64,
        disk_size_gb: i64,
        network: bool,
        workdir: &str,
        env: Option<&HashMap<String, String>>,
        volumes: Option<&Vec<Value>>,
    ) -> Result<BoxHandle<'_>, BoxRunError> {
        let image = resolve_image(image);
        let mut body = json!({
            "image": image,
            "name": name,
            "cpu": cpu,
            "memory_mb": memory_mb,
            "disk_size_gb": disk_size_gb,
            "network": network,
            "workdir": workdir,
            "env": env,
        });
        if let Some(vols) = volumes {
            body.as_object_mut()
                .unwrap()
                .insert("volumes".into(), json!(vols));
        }

        let resp = self
            .http
            .post(format!("{}/v1/boxes", self.base_url))
            .json(&body)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e.to_string()))?;

        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let data = Self::check_error(status, &text)?;
        Ok(BoxHandle::new(self, BoxInfo::from_json(&data)))
    }

    pub async fn get_box(&self, id_or_name: &str) -> Result<BoxInfo, BoxRunError> {
        let resp = self
            .http
            .get(format!("{}/v1/boxes/{}", self.base_url, id_or_name))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e.to_string()))?;

        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let data = Self::check_error(status, &text)?;
        Ok(BoxInfo::from_json(&data))
    }

    pub async fn list_boxes(
        &self,
        status_filter: Option<&str>,
    ) -> Result<Vec<BoxInfo>, BoxRunError> {
        let mut url = format!("{}/v1/boxes", self.base_url);
        if let Some(s) = status_filter {
            url.push_str(&format!("?status={s}"));
        }

        let resp = self
            .http
            .get(&url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e.to_string()))?;

        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let data = Self::check_error(status, &text)?;

        Ok(data
            .as_array()
            .map(|a| a.iter().map(BoxInfo::from_json).collect())
            .unwrap_or_default())
    }

    pub async fn stop_box(&self, box_id: &str) -> Result<BoxInfo, BoxRunError> {
        let resp = self
            .http
            .post(format!("{}/v1/boxes/{}:stop", self.base_url, box_id))
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e.to_string()))?;

        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let data = Self::check_error(status, &text)?;
        Ok(BoxInfo::from_json(&data))
    }

    pub async fn start_box(&self, box_id: &str) -> Result<BoxInfo, BoxRunError> {
        let resp = self
            .http
            .post(format!("{}/v1/boxes/{}:start", self.base_url, box_id))
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e.to_string()))?;

        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let data = Self::check_error(status, &text)?;
        Ok(BoxInfo::from_json(&data))
    }

    pub async fn remove_box(&self, box_id: &str, force: bool) -> Result<(), BoxRunError> {
        let resp = self
            .http
            .delete(format!(
                "{}/v1/boxes/{}?force={}",
                self.base_url, box_id, force
            ))
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e.to_string()))?;

        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        Self::check_error(status, &text)?;
        Ok(())
    }

    // ── Exec operations ──────────────────────────────────────────────────

    pub async fn start_exec(
        &self,
        box_id: &str,
        cmd: &[String],
        env: Option<&HashMap<String, String>>,
        timeout_ms: Option<i64>,
    ) -> Result<ExecInfo, BoxRunError> {
        let body = json!({
            "cmd": cmd,
            "env": env,
            "timeout_ms": timeout_ms,
        });

        let resp = self
            .http
            .post(format!("{}/v1/boxes/{}/exec", self.base_url, box_id))
            .json(&body)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e.to_string()))?;

        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let data = Self::check_error(status, &text)?;
        Ok(ExecInfo::from_json(&data))
    }

    pub async fn get_exec(&self, box_id: &str, exec_id: &str) -> Result<ExecInfo, BoxRunError> {
        let resp = self
            .http
            .get(format!(
                "{}/v1/boxes/{}/exec/{}",
                self.base_url, box_id, exec_id
            ))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e.to_string()))?;

        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let data = Self::check_error(status, &text)?;
        Ok(ExecInfo::from_json(&data))
    }

    // ── File operations ──────────────────────────────────────────────────

    pub async fn upload_file(
        &self,
        box_id: &str,
        local_path: &str,
        dest_path: &str,
    ) -> Result<(), BoxRunError> {
        let file_bytes = std::fs::read(local_path).map_err(|e| {
            BoxRunError::new(ErrorCode::RuntimeError, format!("Failed to read file: {e}"))
        })?;
        let file_name = Path::new(local_path)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("file")
            .to_string();

        let part = reqwest::multipart::Part::bytes(file_bytes).file_name(file_name);
        let form = reqwest::multipart::Form::new()
            .part("file", part)
            .text("dest", dest_path.to_string());

        let resp = self
            .http
            .post(format!(
                "{}/v1/boxes/{}/files/upload",
                self.base_url, box_id
            ))
            .multipart(form)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e.to_string()))?;

        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        Self::check_error(status, &text)?;
        Ok(())
    }

    pub async fn download_file(
        &self,
        box_id: &str,
        remote_path: &str,
        local_path: &str,
    ) -> Result<(), BoxRunError> {
        let resp = self
            .http
            .post(format!(
                "{}/v1/boxes/{}/files/download",
                self.base_url, box_id
            ))
            .json(&json!({"path": remote_path}))
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e.to_string()))?;

        let status = resp.status().as_u16();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e.to_string()))?;

        if status >= 400 {
            let text = String::from_utf8_lossy(&bytes).to_string();
            Self::check_error(status, &text)?;
        }

        let local = Path::new(local_path);
        if let Some(parent) = local.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(local, &bytes).map_err(|e| {
            BoxRunError::new(
                ErrorCode::RuntimeError,
                format!("Failed to write file: {e}"),
            )
        })?;
        Ok(())
    }

    // ── Convenience ──────────────────────────────────────────────────────

    pub async fn run(
        &self,
        image: &str,
        cmd: &[String],
        env: Option<&HashMap<String, String>>,
        timeout_ms: Option<i64>,
        disk_size_gb: i64,
        volumes: Option<&Vec<Value>>,
    ) -> Result<RunResult, BoxRunError> {
        let image = resolve_image(image);
        let mut body = json!({
            "image": image,
            "cmd": cmd,
            "env": env,
            "timeout_ms": timeout_ms,
            "disk_size_gb": disk_size_gb,
        });
        if let Some(vols) = volumes {
            body.as_object_mut()
                .unwrap()
                .insert("volumes".into(), json!(vols));
        }

        let resp = self
            .http
            .post(format!("{}/v1/run", self.base_url))
            .json(&body)
            .timeout(std::time::Duration::from_secs(300))
            .send()
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e.to_string()))?;

        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let data = Self::check_error(status, &text)?;

        Ok(RunResult {
            exit_code: data["exit_code"].as_i64(),
            stdout: data["stdout"].as_str().unwrap_or("").into(),
            stderr: data["stderr"].as_str().unwrap_or("").into(),
            error_message: data["error_message"].as_str().map(|s| s.into()),
        })
    }

    pub async fn gc(&self, older_than: i64) -> Result<i64, BoxRunError> {
        let resp = self
            .http
            .post(format!("{}/v1/gc", self.base_url))
            .json(&json!({"older_than": older_than}))
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e.to_string()))?;

        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let data = Self::check_error(status, &text)?;
        Ok(data["removed"].as_i64().unwrap_or(0))
    }

    // ── SSE streaming ───────────────────────────────────────────────────

    /// Stream exec events via SSE for an already-started execution.
    pub async fn exec_events_stream(
        &self,
        box_id: &str,
        exec_id: &str,
    ) -> Result<ExecEventStream, BoxRunError> {
        let resp = self
            .http
            .get(format!(
                "{}/v1/boxes/{}/exec/{}/events",
                self.base_url, box_id, exec_id
            ))
            .send()
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e.to_string()))?;

        let status = resp.status().as_u16();
        if status >= 400 {
            let text = resp.text().await.unwrap_or_default();
            return Err(Self::check_error(status, &text).unwrap_err());
        }

        let stream = resp
            .bytes_stream()
            .eventsource()
            .filter_map(|result| async move {
                match result {
                    Ok(event) => parse_sse_event(event),
                    Err(e) => Some(Err(BoxRunError::new(
                        ErrorCode::RuntimeError,
                        format!("SSE stream error: {e}"),
                    ))),
                }
            })
            .scan(false, |done, item| {
                if *done {
                    return std::future::ready(None);
                }
                if let Ok(ref event) = item {
                    if event.event_type == "exit" {
                        *done = true;
                    }
                }
                std::future::ready(Some(item))
            });

        Ok(Box::pin(stream))
    }

    /// Start an execution and stream its events via SSE.
    pub async fn exec_stream(
        &self,
        box_id: &str,
        cmd: &[String],
        env: Option<&HashMap<String, String>>,
        timeout_ms: Option<i64>,
    ) -> Result<ExecEventStream, BoxRunError> {
        let exec_info = self.start_exec(box_id, cmd, env, timeout_ms).await?;
        self.exec_events_stream(box_id, &exec_info.id).await
    }
}

/// Parse an SSE event from eventsource-stream into an ExecEvent.
/// Returns None for keep-alive events (empty data).
fn parse_sse_event(event: eventsource_stream::Event) -> Option<Result<ExecEvent, BoxRunError>> {
    let data_str = event.data;
    if data_str.is_empty() {
        return None;
    }
    let parsed: Value = match serde_json::from_str(&data_str) {
        Ok(v) => v,
        Err(e) => {
            return Some(Err(BoxRunError::new(
                ErrorCode::RuntimeError,
                format!("Failed to parse SSE event JSON: {e}"),
            )));
        }
    };
    Some(Ok(ExecEvent {
        event_type: event.event,
        data: parsed["data"].as_str().unwrap_or("").to_string(),
        stream: parsed["stream"].as_str().map(|s| s.to_string()),
        seq: parsed["seq"].as_i64().unwrap_or(0),
    }))
}

/// Handle to a specific box, obtained from BoxRunClient.
pub struct BoxHandle<'a> {
    client: &'a BoxRunClient,
    pub info: BoxInfo,
}

impl<'a> BoxHandle<'a> {
    fn new(client: &'a BoxRunClient, info: BoxInfo) -> Self {
        Self { client, info }
    }

    pub fn id(&self) -> &str {
        &self.info.id
    }

    pub fn name(&self) -> Option<&str> {
        self.info.name.as_deref()
    }

    pub async fn refresh(&mut self) -> Result<&BoxInfo, BoxRunError> {
        self.info = self.client.get_box(&self.info.id).await?;
        Ok(&self.info)
    }

    pub async fn exec(
        &self,
        cmd: &[String],
        env: Option<&HashMap<String, String>>,
        timeout_ms: Option<i64>,
    ) -> Result<ExecInfo, BoxRunError> {
        let exec_info = self
            .client
            .start_exec(&self.info.id, cmd, env, timeout_ms)
            .await?;
        // Poll until finished
        let poll_deadline = if let Some(t) = timeout_ms {
            (t as f64 / 1000.0) + 30.0
        } else {
            600.0
        };
        let mut elapsed = 0.0f64;
        let mut current = exec_info;
        while current.status == "running" {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            elapsed += 0.5;
            if elapsed > poll_deadline {
                return Err(BoxRunError::new(
                    ErrorCode::Timeout,
                    format!(
                        "Exec {} did not complete within {poll_deadline}s",
                        current.id
                    ),
                ));
            }
            current = self.client.get_exec(&self.info.id, &current.id).await?;
        }
        Ok(current)
    }

    pub async fn upload(&self, local_path: &str, dest_path: &str) -> Result<(), BoxRunError> {
        self.client
            .upload_file(&self.info.id, local_path, dest_path)
            .await
    }

    pub async fn download(&self, remote_path: &str, local_path: &str) -> Result<(), BoxRunError> {
        self.client
            .download_file(&self.info.id, remote_path, local_path)
            .await
    }

    pub async fn stop(&mut self) -> Result<&BoxInfo, BoxRunError> {
        self.info = self.client.stop_box(&self.info.id).await?;
        Ok(&self.info)
    }

    pub async fn start(&mut self) -> Result<&BoxInfo, BoxRunError> {
        self.info = self.client.start_box(&self.info.id).await?;
        Ok(&self.info)
    }

    pub async fn remove(&self, force: bool) -> Result<(), BoxRunError> {
        self.client.remove_box(&self.info.id, force).await
    }

    /// Stream exec events via SSE for an already-started execution.
    pub async fn exec_events_stream(&self, exec_id: &str) -> Result<ExecEventStream, BoxRunError> {
        self.client.exec_events_stream(&self.info.id, exec_id).await
    }

    /// Start an execution and stream its events via SSE.
    pub async fn exec_stream(
        &self,
        cmd: &[String],
        env: Option<&HashMap<String, String>>,
        timeout_ms: Option<i64>,
    ) -> Result<ExecEventStream, BoxRunError> {
        self.client
            .exec_stream(&self.info.id, cmd, env, timeout_ms)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(event: &str, data: &str) -> eventsource_stream::Event {
        eventsource_stream::Event {
            event: event.to_string(),
            data: data.to_string(),
            id: String::new(),
            retry: None,
        }
    }

    #[test]
    fn test_parse_log_event() {
        let event = make_event("log", r#"{"stream":"stdout","data":"hello\n","seq":0}"#);
        let result = parse_sse_event(event).unwrap().unwrap();
        assert_eq!(result.event_type, "log");
        assert_eq!(result.stream, Some("stdout".to_string()));
        assert_eq!(result.data, "hello\n");
        assert_eq!(result.seq, 0);
    }

    #[test]
    fn test_parse_exit_event() {
        let event = make_event(
            "exit",
            r#"{"stream":null,"data":"{\"exit_code\":0}","seq":2}"#,
        );
        let result = parse_sse_event(event).unwrap().unwrap();
        assert_eq!(result.event_type, "exit");
        assert!(result.stream.is_none());
        assert_eq!(result.seq, 2);
    }

    #[test]
    fn test_parse_stderr_event() {
        let event = make_event("log", r#"{"stream":"stderr","data":"error msg","seq":1}"#);
        let result = parse_sse_event(event).unwrap().unwrap();
        assert_eq!(result.event_type, "log");
        assert_eq!(result.stream, Some("stderr".to_string()));
        assert_eq!(result.data, "error msg");
        assert_eq!(result.seq, 1);
    }

    #[test]
    fn test_parse_empty_data_returns_none() {
        let event = make_event("", "");
        assert!(parse_sse_event(event).is_none());
    }

    #[test]
    fn test_parse_malformed_json_returns_error() {
        let event = make_event("log", "not valid json{");
        let result = parse_sse_event(event).unwrap();
        assert!(result.is_err());
    }
}
