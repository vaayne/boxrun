use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use boxrun_server::app::AppState;
use boxrun_server::box_manager::BoxManager;
use boxrun_server::events::EventBus;
use boxrun_server::routes::v1_router;
use boxrun_server::store::Store;

// ── Helpers ──────────────────────────────────────────────────────────────

async fn test_app() -> (Router, Arc<AppState>) {
    let store = Store::new(":memory:").await.unwrap();
    let event_bus = EventBus::new();
    let manager = BoxManager::new_without_runtime(store, event_bus);
    let state = Arc::new(AppState { manager });
    let app = v1_router().with_state(state.clone());
    (app, state)
}

async fn json_request(
    app: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let req = match body {
        Some(b) => Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&b).unwrap()))
            .unwrap(),
        None => Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap(),
    };

    let response = app.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_json: Value = serde_json::from_slice(&body_bytes)
        .unwrap_or_else(|_| json!({"raw": String::from_utf8_lossy(&body_bytes).to_string()}));
    (status, body_json)
}

/// Seed a box directly in the Store (bypass BoxManager/BoxLite).
async fn seed_box(state: &Arc<AppState>, id: &str, name: Option<&str>, status: &str) {
    use boxrun_server::store::BoxValue;
    state
        .manager
        .store()
        .create_box(
            id,
            name,
            "ubuntu:24.04",
            2,
            1024,
            8,
            false,
            "/root",
            &None,
            &None,
        )
        .await
        .unwrap();
    state
        .manager
        .store()
        .update_box(id, &[("status", BoxValue::Text(status.into()))])
        .await
        .unwrap();
}

/// Seed an exec directly in the Store.
async fn seed_exec(
    state: &Arc<AppState>,
    exec_id: &str,
    box_id: &str,
    status: &str,
    exit_code: Option<i64>,
) {
    state
        .manager
        .store()
        .create_exec(
            exec_id,
            box_id,
            &["echo".into(), "hello".into()],
            &None,
            None,
            None,
        )
        .await
        .unwrap();

    if status != "running" {
        state
            .manager
            .store()
            .finish_exec(exec_id, exit_code.unwrap_or(0), None)
            .await
            .unwrap();
    }
}

// ── Tier 1: No BoxLite needed ────────────────────────────────────────────

// Info

#[tokio::test]
async fn test_server_info() {
    let (app, _state) = test_app().await;
    let (status, body) = json_request(&app, Method::GET, "/v1/info", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("max_cpu").is_some());
    assert!(body.get("max_memory_mb").is_some());
}

// Validation errors (422)

#[tokio::test]
async fn test_create_box_validation_zero_cpu() {
    let (app, _state) = test_app().await;
    let (status, body) = json_request(
        &app,
        Method::POST,
        "/v1/boxes",
        Some(json!({"image": "ubuntu:24.04", "cpu": 0})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["code"], "VALIDATION_ERROR");
}

#[tokio::test]
async fn test_create_box_validation_bad_memory() {
    let (app, _state) = test_app().await;
    let (status, body) = json_request(
        &app,
        Method::POST,
        "/v1/boxes",
        Some(json!({"image": "ubuntu:24.04", "memory_mb": -1})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["code"], "VALIDATION_ERROR");
}

#[tokio::test]
async fn test_create_box_validation_bad_workdir() {
    let (app, _state) = test_app().await;
    let (status, body) = json_request(
        &app,
        Method::POST,
        "/v1/boxes",
        Some(json!({"image": "ubuntu:24.04", "workdir": "relative"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["code"], "VALIDATION_ERROR");
}

#[tokio::test]
async fn test_exec_validation_empty_cmd() {
    let (app, state) = test_app().await;
    seed_box(&state, "box_1", None, "running").await;
    let (status, body) = json_request(
        &app,
        Method::POST,
        "/v1/boxes/box_1/exec",
        Some(json!({"cmd": []})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["code"], "VALIDATION_ERROR");
}

#[tokio::test]
async fn test_exec_validation_bad_workdir() {
    let (app, state) = test_app().await;
    seed_box(&state, "box_1", None, "running").await;
    let (status, body) = json_request(
        &app,
        Method::POST,
        "/v1/boxes/box_1/exec",
        Some(json!({"cmd": ["ls"], "workdir": "relative"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["code"], "VALIDATION_ERROR");
}

#[tokio::test]
async fn test_exec_validation_bad_timeout() {
    let (app, state) = test_app().await;
    seed_box(&state, "box_1", None, "running").await;
    let (status, body) = json_request(
        &app,
        Method::POST,
        "/v1/boxes/box_1/exec",
        Some(json!({"cmd": ["ls"], "timeout_ms": -1})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["code"], "VALIDATION_ERROR");
}

#[tokio::test]
async fn test_run_validation_empty_cmd() {
    let (app, _state) = test_app().await;
    let (status, body) = json_request(
        &app,
        Method::POST,
        "/v1/run",
        Some(json!({"image": "ubuntu:24.04", "cmd": []})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["code"], "VALIDATION_ERROR");
}

// Not found errors (404)

#[tokio::test]
async fn test_get_box_not_found() {
    let (app, _state) = test_app().await;
    let (status, body) = json_request(&app, Method::GET, "/v1/boxes/nonexistent", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "BOX_NOT_FOUND");
}

#[tokio::test]
async fn test_stop_box_not_found() {
    let (app, _state) = test_app().await;
    let (status, body) = json_request(&app, Method::POST, "/v1/boxes/nonexistent:stop", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "BOX_NOT_FOUND");
}

#[tokio::test]
async fn test_remove_box_not_found() {
    let (app, _state) = test_app().await;
    let (status, body) = json_request(&app, Method::DELETE, "/v1/boxes/nonexistent", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "BOX_NOT_FOUND");
}

#[tokio::test]
async fn test_get_exec_not_found() {
    let (app, state) = test_app().await;
    seed_box(&state, "box_1", None, "running").await;
    let (status, body) =
        json_request(&app, Method::GET, "/v1/boxes/box_1/exec/nonexistent", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "EXEC_NOT_FOUND");
}

#[tokio::test]
async fn test_cancel_exec_not_found() {
    let (app, state) = test_app().await;
    seed_box(&state, "box_1", None, "running").await;
    let (status, body) = json_request(
        &app,
        Method::POST,
        "/v1/boxes/box_1/exec/nonexistent:cancel",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "EXEC_NOT_FOUND");
}

// State errors (400/409)

#[tokio::test]
async fn test_stop_box_not_running() {
    let (app, state) = test_app().await;
    seed_box(&state, "box_1", None, "stopped").await;
    let (status, body) = json_request(&app, Method::POST, "/v1/boxes/box_1:stop", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "BOX_NOT_RUNNING");
}

#[tokio::test]
async fn test_start_box_already_running() {
    let (app, state) = test_app().await;
    seed_box(&state, "box_1", None, "running").await;
    let (status, body) = json_request(&app, Method::POST, "/v1/boxes/box_1:start", None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "BOX_ALREADY_RUNNING");
}

#[tokio::test]
async fn test_remove_running_box_no_force() {
    let (app, state) = test_app().await;
    seed_box(&state, "box_1", None, "running").await;
    let (status, body) = json_request(&app, Method::DELETE, "/v1/boxes/box_1", None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "BOX_ALREADY_RUNNING");
}

#[tokio::test]
async fn test_cancel_exec_already_finished() {
    let (app, state) = test_app().await;
    seed_box(&state, "box_1", None, "running").await;
    seed_exec(&state, "exec_1", "box_1", "succeeded", Some(0)).await;
    let (status, body) = json_request(
        &app,
        Method::POST,
        "/v1/boxes/box_1/exec/exec_1:cancel",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "EXEC_ALREADY_FINISHED");
}

#[tokio::test]
async fn test_exec_box_not_running() {
    let (app, state) = test_app().await;
    seed_box(&state, "box_1", None, "stopped").await;
    let (status, body) = json_request(
        &app,
        Method::POST,
        "/v1/boxes/box_1/exec",
        Some(json!({"cmd": ["ls"]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "BOX_NOT_RUNNING");
}

// Action parsing

#[tokio::test]
async fn test_unknown_box_action() {
    let (app, state) = test_app().await;
    seed_box(&state, "box_1", None, "running").await;
    let (status, body) = json_request(&app, Method::POST, "/v1/boxes/box_1:unknown", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["message"].as_str().unwrap().contains("Unknown action"));
}

#[tokio::test]
async fn test_missing_box_action() {
    let (app, state) = test_app().await;
    seed_box(&state, "box_1", None, "running").await;
    let (status, body) = json_request(&app, Method::POST, "/v1/boxes/box_1", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["message"].as_str().unwrap().contains("Missing action"));
}

// List/read operations

#[tokio::test]
async fn test_list_boxes_empty() {
    let (app, _state) = test_app().await;
    let (status, body) = json_request(&app, Method::GET, "/v1/boxes", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_list_boxes_with_data() {
    let (app, state) = test_app().await;
    seed_box(&state, "box_1", Some("alpha"), "running").await;
    seed_box(&state, "box_2", Some("beta"), "stopped").await;
    let (status, body) = json_request(&app, Method::GET, "/v1/boxes", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_list_boxes_status_filter() {
    let (app, state) = test_app().await;
    seed_box(&state, "box_1", None, "running").await;
    seed_box(&state, "box_2", None, "stopped").await;
    let (status, body) = json_request(&app, Method::GET, "/v1/boxes?status=running", None).await;
    assert_eq!(status, StatusCode::OK);
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["status"], "running");
}

#[tokio::test]
async fn test_get_box_by_id() {
    let (app, state) = test_app().await;
    seed_box(&state, "box_123", Some("mybox"), "running").await;
    let (status, body) = json_request(&app, Method::GET, "/v1/boxes/box_123", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], "box_123");
    assert_eq!(body["name"], "mybox");
    assert_eq!(body["status"], "running");
}

#[tokio::test]
async fn test_get_box_by_name() {
    let (app, state) = test_app().await;
    seed_box(&state, "box_456", Some("namedbox"), "running").await;
    let (status, body) = json_request(&app, Method::GET, "/v1/boxes/namedbox", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], "box_456");
    assert_eq!(body["name"], "namedbox");
}

#[tokio::test]
async fn test_list_execs_empty() {
    let (app, state) = test_app().await;
    seed_box(&state, "box_1", None, "running").await;
    let (status, body) = json_request(&app, Method::GET, "/v1/boxes/box_1/execs", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_list_execs_with_data() {
    let (app, state) = test_app().await;
    seed_box(&state, "box_1", None, "running").await;
    seed_exec(&state, "exec_1", "box_1", "succeeded", Some(0)).await;
    seed_exec(&state, "exec_2", "box_1", "failed", Some(1)).await;
    let (status, body) = json_request(&app, Method::GET, "/v1/boxes/box_1/execs", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_gc_no_boxes() {
    let (app, _state) = test_app().await;
    let (status, body) = json_request(
        &app,
        Method::POST,
        "/v1/gc",
        Some(json!({"older_than": 3600})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["removed"], 0);
}

// JSON structure

#[tokio::test]
async fn test_box_response_fields() {
    let (app, state) = test_app().await;
    seed_box(&state, "box_resp", Some("testbox"), "running").await;
    let (status, body) = json_request(&app, Method::GET, "/v1/boxes/box_resp", None).await;
    assert_eq!(status, StatusCode::OK);

    let expected_fields = [
        "id",
        "name",
        "status",
        "image",
        "cpu",
        "memory_mb",
        "disk_size_gb",
        "network",
        "workdir",
        "env",
        "volumes",
        "boxlite_id",
        "error_code",
        "error_message",
        "created_at",
        "started_at",
        "stopped_at",
    ];
    for field in &expected_fields {
        assert!(
            body.get(field).is_some(),
            "Missing field '{}' in box response",
            field
        );
    }
}

#[tokio::test]
async fn test_exec_response_fields() {
    let (app, state) = test_app().await;
    seed_box(&state, "box_1", None, "running").await;
    seed_exec(&state, "exec_resp", "box_1", "succeeded", Some(0)).await;
    let (status, body) =
        json_request(&app, Method::GET, "/v1/boxes/box_1/exec/exec_resp", None).await;
    assert_eq!(status, StatusCode::OK);

    let expected_fields = [
        "id",
        "box_id",
        "status",
        "cmd",
        "env",
        "workdir",
        "timeout_ms",
        "exit_code",
        "error_message",
        "created_at",
        "finished_at",
    ];
    for field in &expected_fields {
        assert!(
            body.get(field).is_some(),
            "Missing field '{}' in exec response",
            field
        );
    }
}

// ── Tier 2: Need BoxLite (conditional) ───────────────────────────────────

fn boxlite_available() -> bool {
    // Check if BoxLite runtime can be initialized.
    // We do a sync check here; the actual tests will use async runtime.
    std::env::var("BOXRUN_TEST_BOXLITE").is_ok()
}

#[tokio::test]
async fn test_full_box_lifecycle() {
    if !boxlite_available() {
        eprintln!(
            "Skipping test_full_box_lifecycle: BoxLite not available (set BOXRUN_TEST_BOXLITE=1)"
        );
        return;
    }

    let store = Store::new(":memory:").await.unwrap();
    let event_bus = EventBus::new();
    let manager = BoxManager::new(store, event_bus).unwrap();
    let state = Arc::new(AppState { manager });
    let app = v1_router().with_state(state.clone());

    // Create
    let (status, body) = json_request(
        &app,
        Method::POST,
        "/v1/boxes",
        Some(json!({"image": "ubuntu:24.04", "name": "lifecycle"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let box_id = body["id"].as_str().unwrap().to_string();

    // Get
    let (status, body) =
        json_request(&app, Method::GET, &format!("/v1/boxes/{box_id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "running");

    // Stop
    let (status, _body) = json_request(
        &app,
        Method::POST,
        &format!("/v1/boxes/{box_id}:stop"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Start
    let (status, _body) = json_request(
        &app,
        Method::POST,
        &format!("/v1/boxes/{box_id}:start"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Delete (force since running)
    let (status, _body) = json_request(
        &app,
        Method::DELETE,
        &format!("/v1/boxes/{box_id}?force=true"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Verify 404
    let (status, _body) =
        json_request(&app, Method::GET, &format!("/v1/boxes/{box_id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_create_box_duplicate_name() {
    if !boxlite_available() {
        eprintln!("Skipping test_create_box_duplicate_name: BoxLite not available");
        return;
    }

    let store = Store::new(":memory:").await.unwrap();
    let event_bus = EventBus::new();
    let manager = BoxManager::new(store, event_bus).unwrap();
    let state = Arc::new(AppState { manager });
    let app = v1_router().with_state(state.clone());

    let (status, _body) = json_request(
        &app,
        Method::POST,
        "/v1/boxes",
        Some(json!({"image": "ubuntu:24.04", "name": "dupname"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = json_request(
        &app,
        Method::POST,
        "/v1/boxes",
        Some(json!({"image": "ubuntu:24.04", "name": "dupname"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "NAME_ALREADY_EXISTS");

    // Cleanup
    let _ = state.manager.remove_box("dupname", true).await;
}

#[tokio::test]
async fn test_exec_lifecycle() {
    if !boxlite_available() {
        eprintln!("Skipping test_exec_lifecycle: BoxLite not available");
        return;
    }

    let store = Store::new(":memory:").await.unwrap();
    let event_bus = EventBus::new();
    let manager = BoxManager::new(store, event_bus).unwrap();
    let state = Arc::new(AppState { manager });
    let app = v1_router().with_state(state.clone());

    // Create box
    let (status, box_body) = json_request(
        &app,
        Method::POST,
        "/v1/boxes",
        Some(json!({"image": "ubuntu:24.04"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let box_id = box_body["id"].as_str().unwrap().to_string();

    // Exec echo
    let (status, exec_body) = json_request(
        &app,
        Method::POST,
        &format!("/v1/boxes/{box_id}/exec"),
        Some(json!({"cmd": ["echo", "hello"]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let exec_id = exec_body["id"].as_str().unwrap().to_string();

    // Poll until finished
    for _ in 0..50 {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        let (_, exec_data) = json_request(
            &app,
            Method::GET,
            &format!("/v1/boxes/{box_id}/exec/{exec_id}"),
            None,
        )
        .await;
        if exec_data["status"] != "running" {
            assert_eq!(exec_data["exit_code"], 0);
            break;
        }
    }

    // Cleanup
    let _ = state.manager.remove_box(&box_id, true).await;
}

#[tokio::test]
async fn test_exec_nonzero_exit() {
    if !boxlite_available() {
        eprintln!("Skipping test_exec_nonzero_exit: BoxLite not available");
        return;
    }

    let store = Store::new(":memory:").await.unwrap();
    let event_bus = EventBus::new();
    let manager = BoxManager::new(store, event_bus).unwrap();
    let state = Arc::new(AppState { manager });
    let app = v1_router().with_state(state.clone());

    let (_, box_body) = json_request(
        &app,
        Method::POST,
        "/v1/boxes",
        Some(json!({"image": "ubuntu:24.04"})),
    )
    .await;
    let box_id = box_body["id"].as_str().unwrap().to_string();

    let (_, exec_body) = json_request(
        &app,
        Method::POST,
        &format!("/v1/boxes/{box_id}/exec"),
        Some(json!({"cmd": ["false"]})),
    )
    .await;
    let exec_id = exec_body["id"].as_str().unwrap().to_string();

    for _ in 0..50 {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        let (_, exec_data) = json_request(
            &app,
            Method::GET,
            &format!("/v1/boxes/{box_id}/exec/{exec_id}"),
            None,
        )
        .await;
        if exec_data["status"] != "running" {
            assert_ne!(exec_data["exit_code"], 0);
            break;
        }
    }

    let _ = state.manager.remove_box(&box_id, true).await;
}

#[tokio::test]
async fn test_ephemeral_run() {
    if !boxlite_available() {
        eprintln!("Skipping test_ephemeral_run: BoxLite not available");
        return;
    }

    let store = Store::new(":memory:").await.unwrap();
    let event_bus = EventBus::new();
    let manager = BoxManager::new(store, event_bus).unwrap();
    let state = Arc::new(AppState { manager });
    let app = v1_router().with_state(state.clone());

    let (status, body) = json_request(
        &app,
        Method::POST,
        "/v1/run",
        Some(json!({"image": "ubuntu:24.04", "cmd": ["echo", "ephemeral"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["stdout"].as_str().unwrap().contains("ephemeral"));
}

#[tokio::test]
async fn test_file_upload_download() {
    if !boxlite_available() {
        eprintln!("Skipping test_file_upload_download: BoxLite not available");
        return;
    }

    let store = Store::new(":memory:").await.unwrap();
    let event_bus = EventBus::new();
    let manager = BoxManager::new(store, event_bus).unwrap();
    let state = Arc::new(AppState { manager });
    let app = v1_router().with_state(state.clone());

    let (_, box_body) = json_request(
        &app,
        Method::POST,
        "/v1/boxes",
        Some(json!({"image": "ubuntu:24.04"})),
    )
    .await;
    let box_id = box_body["id"].as_str().unwrap().to_string();

    // Upload via multipart — build a multipart request
    let boundary = "----testboundary";
    let body_str = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"dest\"\r\n\r\n/root/test.txt\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.txt\"\r\nContent-Type: application/octet-stream\r\n\r\nhello world\r\n--{boundary}--\r\n"
    );
    let req = Request::builder()
        .method(Method::POST)
        .uri(format!("/v1/boxes/{box_id}/files/upload"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body_str))
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Download
    let (status, _body) = json_request(
        &app,
        Method::POST,
        &format!("/v1/boxes/{box_id}/files/download"),
        Some(json!({"path": "/root/test.txt"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let _ = state.manager.remove_box(&box_id, true).await;
}

#[tokio::test]
async fn test_gc_removes_old_stopped() {
    if !boxlite_available() {
        eprintln!("Skipping test_gc_removes_old_stopped: BoxLite not available");
        return;
    }

    let store = Store::new(":memory:").await.unwrap();
    let event_bus = EventBus::new();
    let manager = BoxManager::new(store, event_bus).unwrap();
    let state = Arc::new(AppState { manager });
    let app = v1_router().with_state(state.clone());

    // Create and stop a box
    let (_, box_body) = json_request(
        &app,
        Method::POST,
        "/v1/boxes",
        Some(json!({"image": "ubuntu:24.04"})),
    )
    .await;
    let box_id = box_body["id"].as_str().unwrap().to_string();

    let _ = json_request(
        &app,
        Method::POST,
        &format!("/v1/boxes/{box_id}:stop"),
        None,
    )
    .await;

    // Backdate stopped_at
    use boxrun_server::store::BoxValue;
    state
        .manager
        .store()
        .update_box(
            &box_id,
            &[(
                "stopped_at",
                BoxValue::Text("2020-01-01T00:00:00+00:00".into()),
            )],
        )
        .await
        .unwrap();

    // GC
    let (status, body) =
        json_request(&app, Method::POST, "/v1/gc", Some(json!({"older_than": 1}))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["removed"].as_i64().unwrap() >= 1);
}
