use std::collections::HashSet;
use std::convert::Infallible;
use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Multipart, Query, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use axum::Router;
use futures::SinkExt;
use serde_json::{json, Value};

use boxrun_types::error::{BoxRunError, ErrorCode};
use boxrun_types::models::*;

use crate::app::AppState;
use crate::store::BoxRow;

/// Build the v1 API router.
///
/// NOTE: axum does not support colon-style actions in path parameters
/// (e.g. `/v1/boxes/{box_id}:stop`). We handle this by registering a
/// POST handler on `/v1/boxes/{box_id_action}` that parses "id:action"
/// from the raw path segment. Similarly for exec cancel.
pub fn v1_router() -> Router<Arc<AppState>> {
    Router::new()
        // Info
        .route("/v1/info", get(server_info))
        // Boxes
        .route("/v1/boxes", post(create_box).get(list_boxes))
        .route(
            "/v1/boxes/{box_id_action}",
            get(get_box).delete(remove_box).post(box_action),
        )
        // Exec
        .route("/v1/boxes/{box_id}/exec", post(exec_in_box))
        .route("/v1/boxes/{box_id}/execs", get(list_execs))
        .route(
            "/v1/boxes/{box_id}/exec/{exec_id}",
            get(get_exec).post(exec_action),
        )
        .route(
            "/v1/boxes/{box_id}/exec/{exec_id}/events",
            get(exec_events_sse),
        )
        // Files
        .route("/v1/boxes/{box_id}/files/upload", post(upload_file))
        .route("/v1/boxes/{box_id}/files/download", post(download_file))
        // WebSocket attach
        .route("/v1/boxes/{box_id}/attach", get(attach_box_ws))
        // Run (ephemeral)
        .route("/v1/run", post(run_ephemeral))
        // GC
        .route("/v1/gc", post(gc))
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn error_response(e: &BoxRunError) -> Response {
    let status = StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::BAD_REQUEST);
    let body = json!({
        "code": e.code.as_str(),
        "message": e.message,
    });
    (status, Json(body)).into_response()
}

fn box_to_response(row: &BoxRow) -> Value {
    let mut resp = json!({
        "id": row.id,
        "name": row.name,
        "status": row.status,
        "image": row.image,
        "cpu": row.cpu,
        "memory_mb": row.memory_mb,
        "disk_size_gb": row.disk_size_gb,
        "network": row.network,
        "workdir": row.workdir,
        "created_at": row.created_at,
    });
    let obj = resp.as_object_mut().unwrap();
    if let Some(ref env) = row.env {
        obj.insert("env".into(), serde_json::to_value(env).unwrap());
    } else {
        obj.insert("env".into(), Value::Null);
    }
    if let Some(ref vols) = row.volumes {
        obj.insert("volumes".into(), serde_json::to_value(vols).unwrap());
    } else {
        obj.insert("volumes".into(), Value::Null);
    }
    if let Some(ref v) = row.boxlite_id {
        obj.insert("boxlite_id".into(), json!(v));
    } else {
        obj.insert("boxlite_id".into(), Value::Null);
    }
    if let Some(ref v) = row.error_code {
        obj.insert("error_code".into(), json!(v));
    } else {
        obj.insert("error_code".into(), Value::Null);
    }
    if let Some(ref v) = row.error_message {
        obj.insert("error_message".into(), json!(v));
    } else {
        obj.insert("error_message".into(), Value::Null);
    }
    if let Some(ref v) = row.started_at {
        obj.insert("started_at".into(), json!(v));
    } else {
        obj.insert("started_at".into(), Value::Null);
    }
    if let Some(ref v) = row.stopped_at {
        obj.insert("stopped_at".into(), json!(v));
    } else {
        obj.insert("stopped_at".into(), Value::Null);
    }
    resp
}

fn exec_to_response(row: &crate::store::ExecRow) -> Value {
    json!({
        "id": row.id,
        "box_id": row.box_id,
        "status": row.status,
        "cmd": row.cmd,
        "env": row.env,
        "workdir": row.workdir,
        "timeout_ms": row.timeout_ms,
        "exit_code": row.exit_code,
        "error_message": row.error_message,
        "created_at": row.created_at,
        "finished_at": row.finished_at,
    })
}

// ── Info ─────────────────────────────────────────────────────────────────

async fn server_info() -> Json<Value> {
    use boxrun_types::config::{MAX_TOTAL_CPU, MAX_TOTAL_MEMORY_MB};
    Json(json!({
        "max_cpu": *MAX_TOTAL_CPU,
        "max_memory_mb": *MAX_TOTAL_MEMORY_MB,
    }))
}

// ── Boxes ────────────────────────────────────────────────────────────────

async fn create_box(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateBoxRequest>,
) -> Response {
    if let Err(msg) = req.validate() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"code": "VALIDATION_ERROR", "message": msg})),
        )
            .into_response();
    }

    let volumes = req.volumes.as_ref().map(|vols| {
        vols.iter()
            .map(|v| serde_json::to_value(v).unwrap())
            .collect()
    });

    match state
        .manager
        .create_box(
            &req.image,
            req.name.as_deref(),
            req.cpu,
            req.memory_mb,
            req.disk_size_gb,
            req.network,
            &req.workdir,
            &req.env,
            &volumes,
        )
        .await
    {
        Ok(row) => (StatusCode::CREATED, Json(box_to_response(&row))).into_response(),
        Err(e) => error_response(&e),
    }
}

#[derive(serde::Deserialize)]
struct ListBoxesParams {
    status: Option<String>,
}

async fn list_boxes(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListBoxesParams>,
) -> Response {
    match state.manager.list_boxes(params.status.as_deref()).await {
        Ok(boxes) => {
            let resp: Vec<Value> = boxes.iter().map(box_to_response).collect();
            Json(resp).into_response()
        }
        Err(e) => error_response(&e),
    }
}

async fn get_box(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(box_id_action): axum::extract::Path<String>,
) -> Response {
    // For GET, the path param should just be the box_id (no colon action)
    let box_id = box_id_action.split(':').next().unwrap_or(&box_id_action);
    match state.manager.get_box(box_id).await {
        Ok(row) => Json(box_to_response(&row)).into_response(),
        Err(e) => error_response(&e),
    }
}

/// Handle POST /v1/boxes/{box_id}:{action} (stop, start)
async fn box_action(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(box_id_action): axum::extract::Path<String>,
) -> Response {
    let (box_id, action) = match box_id_action.rsplit_once(':') {
        Some((id, act)) => (id, act),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"code": "VALIDATION_ERROR", "message": "Missing action (expected :stop or :start)"})),
            )
                .into_response();
        }
    };

    match action {
        "stop" => match state.manager.stop_box(box_id).await {
            Ok(row) => Json(box_to_response(&row)).into_response(),
            Err(e) => error_response(&e),
        },
        "start" => match state.manager.start_box(box_id).await {
            Ok(row) => Json(box_to_response(&row)).into_response(),
            Err(e) => error_response(&e),
        },
        _ => (
            StatusCode::BAD_REQUEST,
            Json(
                json!({"code": "VALIDATION_ERROR", "message": format!("Unknown action: {action}")}),
            ),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
struct RemoveBoxParams {
    #[serde(default)]
    force: Option<String>,
}

async fn remove_box(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(box_id_action): axum::extract::Path<String>,
    Query(params): Query<RemoveBoxParams>,
) -> Response {
    // For DELETE, strip any accidental action suffix
    let box_id = box_id_action.split(':').next().unwrap_or(&box_id_action);
    let force = params.force.as_deref() == Some("true");
    match state.manager.remove_box(box_id, force).await {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => error_response(&e),
    }
}

// ── Exec ─────────────────────────────────────────────────────────────────

async fn exec_in_box(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(box_id): axum::extract::Path<String>,
    Json(req): Json<ExecRequest>,
) -> Response {
    if let Err(msg) = req.validate() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"code": "VALIDATION_ERROR", "message": msg})),
        )
            .into_response();
    }

    match state
        .manager
        .exec_in_box(
            &box_id,
            &req.cmd,
            &req.env,
            req.workdir.as_deref(),
            req.timeout_ms,
        )
        .await
    {
        Ok(row) => (StatusCode::CREATED, Json(exec_to_response(&row))).into_response(),
        Err(e) => error_response(&e),
    }
}

async fn list_execs(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(box_id): axum::extract::Path<String>,
) -> Response {
    // Verify box exists
    match state.manager.get_box(&box_id).await {
        Ok(bx) => match state.manager.store().list_execs(&bx.id).await {
            Ok(execs) => {
                let resp: Vec<Value> = execs.iter().map(exec_to_response).collect();
                Json(resp).into_response()
            }
            Err(e) => error_response(&BoxRunError::new(ErrorCode::RuntimeError, e)),
        },
        Err(e) => error_response(&e),
    }
}

async fn get_exec(
    State(state): State<Arc<AppState>>,
    axum::extract::Path((_box_id, exec_id)): axum::extract::Path<(String, String)>,
) -> Response {
    match state.manager.get_exec(&exec_id).await {
        Ok(row) => Json(exec_to_response(&row)).into_response(),
        Err(e) => error_response(&e),
    }
}

async fn exec_events_sse(
    State(state): State<Arc<AppState>>,
    axum::extract::Path((_box_id, exec_id)): axum::extract::Path<(String, String)>,
) -> Response {
    // Verify exec exists
    if let Err(e) = state.manager.get_exec(&exec_id).await {
        return error_response(&e);
    }

    let event_bus = state.manager.event_bus().clone();
    let store = state.manager.store().clone();
    let exec_id_clone = exec_id.clone();

    let stream = async_stream::stream! {
        // Subscribe BEFORE reading past events to avoid missing events
        let mut sub = event_bus.subscribe(&exec_id_clone).await;
        let mut seen_seqs: HashSet<i64> = HashSet::new();

        // Replay past events from SQLite
        if let Ok(past_events) = store.get_events(&exec_id_clone, 0).await {
            for ev in &past_events {
                seen_seqs.insert(ev.seq);
                let data = json!({
                    "stream": ev.stream,
                    "data": ev.data,
                    "seq": ev.seq,
                });
                yield Ok::<_, Infallible>(
                    SseEvent::default()
                        .event(&ev.event_type)
                        .data(data.to_string())
                );
                if ev.event_type == "exit" {
                    return;
                }
            }
        }

        // Check if exec already finished (race window)
        if let Ok(Some(fresh)) = store.get_exec(&exec_id_clone).await {
            if fresh.status != "running" {
                // Re-read events
                if let Ok(late_events) = store.get_events(&exec_id_clone, 0).await {
                    for ev in &late_events {
                        if seen_seqs.contains(&ev.seq) {
                            continue;
                        }
                        seen_seqs.insert(ev.seq);
                        let data = json!({
                            "stream": ev.stream,
                            "data": ev.data,
                            "seq": ev.seq,
                        });
                        yield Ok::<_, Infallible>(
                            SseEvent::default()
                                .event(&ev.event_type)
                                .data(data.to_string())
                        );
                        if ev.event_type == "exit" {
                            return;
                        }
                    }
                }
                // Synthesize exit event
                let exit_code = fresh.exit_code;
                let next_seq = seen_seqs.iter().copied().max().unwrap_or(-1) + 1;
                let data = json!({
                    "stream": null,
                    "data": json!({"exit_code": exit_code}).to_string(),
                    "seq": next_seq,
                });
                yield Ok::<_, Infallible>(
                    SseEvent::default()
                        .event("exit")
                        .data(data.to_string())
                );
                return;
            }
        }

        // Stream live events, deduplicating with seen_seqs
        while let Some(event) = sub.recv().await {
            if seen_seqs.contains(&event.seq) {
                continue;
            }
            let data = json!({
                "stream": event.stream,
                "data": event.data,
                "seq": event.seq,
            });
            yield Ok::<_, Infallible>(
                SseEvent::default()
                    .event(&event.event_type)
                    .data(data.to_string())
            );
            if event.event_type == "exit" {
                return;
            }
        }
    };

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Handle POST /v1/boxes/{box_id}/exec/{exec_id}:{action} (cancel)
async fn exec_action(
    State(state): State<Arc<AppState>>,
    axum::extract::Path((_box_id, exec_id_action)): axum::extract::Path<(String, String)>,
) -> Response {
    let (exec_id, action) = match exec_id_action.rsplit_once(':') {
        Some((id, act)) => (id, act),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"code": "VALIDATION_ERROR", "message": "Missing action (expected :cancel)"})),
            )
                .into_response();
        }
    };

    match action {
        "cancel" => match state.manager.cancel_exec(exec_id).await {
            Ok(row) => Json(exec_to_response(&row)).into_response(),
            Err(e) => error_response(&e),
        },
        _ => (
            StatusCode::BAD_REQUEST,
            Json(
                json!({"code": "VALIDATION_ERROR", "message": format!("Unknown action: {action}")}),
            ),
        )
            .into_response(),
    }
}

// ── Files ────────────────────────────────────────────────────────────────

async fn upload_file(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(box_id): axum::extract::Path<String>,
    mut multipart: Multipart,
) -> Response {
    let mut file_data: Option<Vec<u8>> = None;
    let mut dest: Option<String> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "file" => {
                if let Ok(bytes) = field.bytes().await {
                    file_data = Some(bytes.to_vec());
                }
            }
            "dest" => {
                if let Ok(text) = field.text().await {
                    dest = Some(text);
                }
            }
            _ => {}
        }
    }

    let Some(_file_data) = file_data else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"code": "VALIDATION_ERROR", "message": "Missing file field"})),
        )
            .into_response();
    };
    let Some(dest) = dest else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"code": "VALIDATION_ERROR", "message": "Missing dest field"})),
        )
            .into_response();
    };

    // Write to temp file, then upload
    let tmp = match tempfile::NamedTempFile::new() {
        Ok(t) => t,
        Err(e) => return error_response(&BoxRunError::new(ErrorCode::RuntimeError, e.to_string())),
    };
    if let Err(e) = std::fs::write(tmp.path(), _file_data) {
        return error_response(&BoxRunError::new(ErrorCode::RuntimeError, e.to_string()));
    }

    match state
        .manager
        .upload_file(&box_id, tmp.path().to_str().unwrap_or(""), &dest)
        .await
    {
        Ok(()) => Json(json!({"ok": true, "dest": dest})).into_response(),
        Err(e) => error_response(&e),
    }
}

async fn download_file(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(box_id): axum::extract::Path<String>,
    Json(req): Json<DownloadRequest>,
) -> Response {
    let tmp = match tempfile::NamedTempFile::new() {
        Ok(t) => t,
        Err(e) => return error_response(&BoxRunError::new(ErrorCode::RuntimeError, e.to_string())),
    };

    match state
        .manager
        .download_file(&box_id, &req.path, tmp.path().to_str().unwrap_or(""))
        .await
    {
        Ok(()) => {
            let filename = Path::new(&req.path)
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("file");
            match tokio::fs::read(tmp.path()).await {
                Ok(data) => Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "application/octet-stream")
                    .header(
                        "content-disposition",
                        format!("attachment; filename=\"{filename}\""),
                    )
                    .body(Body::from(data))
                    .unwrap(),
                Err(e) => error_response(&BoxRunError::new(ErrorCode::RuntimeError, e.to_string())),
            }
        }
        Err(e) => error_response(&e),
    }
}

// ── Attach (WebSocket) ──────────────────────────────────────────────────

#[derive(serde::Deserialize)]
#[allow(dead_code)]
struct AttachParams {
    #[serde(default = "default_shell")]
    shell: String,
    #[serde(default = "default_cols")]
    cols: u16,
    #[serde(default = "default_rows")]
    rows: u16,
    #[serde(default = "default_term")]
    term: String,
}

fn default_shell() -> String {
    "/bin/bash".into()
}
fn default_cols() -> u16 {
    80
}
fn default_rows() -> u16 {
    24
}
fn default_term() -> String {
    "xterm-256color".into()
}

async fn attach_box_ws(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(box_id): axum::extract::Path<String>,
    Query(params): Query<AttachParams>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |socket| handle_attach(state, box_id, params, socket))
}

async fn handle_attach(
    state: Arc<AppState>,
    box_id: String,
    params: AttachParams,
    mut socket: WebSocket,
) {
    use futures::StreamExt;

    // Verify box exists and is running
    let box_data = match state.manager.get_box(&box_id).await {
        Ok(bd) => {
            if bd.status != "running" {
                let _ = socket
                    .send(Message::Text(json!({"type": "error", "code": "BOX_NOT_RUNNING", "message": "Box is not running"}).to_string().into()))
                    .await;
                let _ = socket.close().await;
                return;
            }
            bd
        }
        Err(e) => {
            let _ = socket
                .send(Message::Text(
                    json!({"type": "error", "code": e.code.as_str(), "message": e.message})
                        .to_string()
                        .into(),
                ))
                .await;
            let _ = socket.close().await;
            return;
        }
    };

    // Get BoxLite handle
    let boxlite_id = match box_data.boxlite_id.as_ref() {
        Some(id) => id,
        None => {
            let _ = socket
                .send(Message::Text(
                    json!({"type": "error", "code": "RUNTIME_ERROR", "message": "Box has no BoxLite ID"})
                        .to_string()
                        .into(),
                ))
                .await;
            let _ = socket.close().await;
            return;
        }
    };

    let litebox = match state.manager.runtime().get(boxlite_id).await {
        Ok(Some(lb)) => lb,
        _ => {
            let _ = socket
                .send(Message::Text(
                    json!({"type": "error", "code": "RUNTIME_ERROR", "message": "BoxLite VM not found"})
                        .to_string()
                        .into(),
                ))
                .await;
            let _ = socket.close().await;
            return;
        }
    };

    // Start TTY exec
    let cmd = boxlite::BoxCommand::new(&params.shell)
        .tty(true)
        .env("TERM", &params.term)
        .working_dir(&box_data.workdir);

    let mut execution = match litebox.exec(cmd).await {
        Ok(e) => e,
        Err(e) => {
            let _ = socket
                .send(Message::Text(
                    json!({"type": "error", "code": "RUNTIME_ERROR", "message": format!("Failed to start exec: {e}")})
                        .to_string()
                        .into(),
                ))
                .await;
            let _ = socket.close().await;
            return;
        }
    };

    // Set initial terminal size
    let _ = execution
        .resize_tty(params.rows as u32, params.cols as u32)
        .await;

    // Take stdin and stdout (take-once)
    let mut exec_stdin = execution.stdin();
    let exec_stdout = execution.stdout();

    // Channel for forwarding exec stdout to the main loop
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);

    // Spawn stdout reader: exec → channel
    let stdout_task = tokio::spawn(async move {
        if let Some(mut stdout) = exec_stdout {
            while let Some(chunk) = stdout.next().await {
                if out_tx.send(chunk.into_bytes()).await.is_err() {
                    break;
                }
            }
        }
    });

    // Main event loop: bridge channel ↔ websocket
    loop {
        tokio::select! {
            // exec stdout → websocket
            Some(data) = out_rx.recv() => {
                if socket.send(Message::Binary(data.into())).await.is_err() {
                    break;
                }
            }
            // websocket → exec stdin (+ control messages)
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        if let Some(ref mut stdin) = exec_stdin {
                            let _ = stdin.write(&data).await;
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(ctrl) = serde_json::from_str::<Value>(&text) {
                            if ctrl.get("type").and_then(|v| v.as_str()) == Some("resize") {
                                let cols = ctrl.get("cols").and_then(|v| v.as_u64()).unwrap_or(80) as u32;
                                let rows = ctrl.get("rows").and_then(|v| v.as_u64()).unwrap_or(24) as u32;
                                let _ = execution.resize_tty(rows, cols).await;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }

    // Clean up: kill execution, close socket
    let _ = execution.kill().await;
    stdout_task.abort();
    let _ = socket
        .send(Message::Text(
            json!({"type": "exit", "code": 0}).to_string().into(),
        ))
        .await;
    let _ = socket.close().await;
}

// ── Run (ephemeral) ─────────────────────────────────────────────────────

async fn run_ephemeral(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RunRequest>,
) -> Response {
    if let Err(msg) = req.validate() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"code": "VALIDATION_ERROR", "message": msg})),
        )
            .into_response();
    }

    let volumes = req.volumes.as_ref().map(|vols| {
        vols.iter()
            .map(|v| serde_json::to_value(v).unwrap())
            .collect()
    });

    let box_data = match state
        .manager
        .create_box(
            &req.image,
            None,
            req.cpu,
            req.memory_mb,
            req.disk_size_gb,
            req.network,
            "/root",
            &req.env,
            &volumes,
        )
        .await
    {
        Ok(b) => b,
        Err(e) => return error_response(&e),
    };

    let box_id = box_data.id.clone();

    let result = async {
        let exec_data = state
            .manager
            .exec_in_box(&box_id, &req.cmd, &req.env, None, req.timeout_ms)
            .await?;

        let exec_id = exec_data.id.clone();

        // Wait for exec to finish
        // Poll until complete
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            match state.manager.get_exec(&exec_id).await {
                Ok(exec) if exec.status != "running" => break,
                Err(_) => break,
                _ => continue,
            }
        }

        // Collect output
        let events = state
            .manager
            .store()
            .get_events(&exec_id, 0)
            .await
            .map_err(|e| BoxRunError::new(ErrorCode::RuntimeError, e))?;

        let mut stdout_parts = Vec::new();
        let mut stderr_parts = Vec::new();
        let mut exit_code = None;

        for ev in &events {
            if ev.event_type == "log" {
                match ev.stream.as_deref() {
                    Some("stdout") => stdout_parts.push(ev.data.clone()),
                    Some("stderr") => stderr_parts.push(ev.data.clone()),
                    _ => {}
                }
            } else if ev.event_type == "exit" {
                if let Ok(data) = serde_json::from_str::<Value>(&ev.data) {
                    exit_code = data.get("exit_code").and_then(|v| v.as_i64());
                }
            }
        }

        Ok::<_, BoxRunError>(json!({
            "exit_code": exit_code,
            "stdout": stdout_parts.join(""),
            "stderr": stderr_parts.join(""),
            "error_message": null,
        }))
    }
    .await;

    // Always clean up the box
    let _ = state.manager.remove_box(&box_id, true).await;

    match result {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => error_response(&e),
    }
}

// ── GC ───────────────────────────────────────────────────────────────────

async fn gc(State(state): State<Arc<AppState>>, Json(req): Json<GCRequest>) -> Response {
    match state.manager.gc(req.older_than).await {
        Ok(removed) => Json(json!({"removed": removed})).into_response(),
        Err(e) => error_response(&e),
    }
}
