use std::net::SocketAddr;

use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use futures::StreamExt;
use serde_json::json;

use boxrun_sdk::client::BoxRunClient;

// ── Helpers ──────────────────────────────────────────────────────────────

/// Start a mock axum server on a random port, returning the base URL.
async fn start_mock_server(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{}", addr.port())
}

/// Build an SSE response from a list of (event_type, data_json) pairs.
fn sse_events_response(events: Vec<(&'static str, serde_json::Value)>) -> impl IntoResponse {
    let stream = async_stream::stream! {
        for (event_type, data) in events {
            yield Ok::<_, std::convert::Infallible>(
                SseEvent::default()
                    .event(event_type)
                    .data(data.to_string())
            );
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ── Tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_exec_events_stream_basic() {
    let app = Router::new().route(
        "/v1/boxes/{box_id}/exec/{exec_id}/events",
        get(|| async {
            sse_events_response(vec![
                (
                    "log",
                    json!({"stream": "stdout", "data": "hello\n", "seq": 0}),
                ),
                (
                    "log",
                    json!({"stream": "stderr", "data": "warn\n", "seq": 1}),
                ),
                (
                    "exit",
                    json!({"stream": null, "data": "{\"exit_code\":0}", "seq": 2}),
                ),
            ])
        }),
    );

    let base_url = start_mock_server(app).await;
    let client = BoxRunClient::new(Some(&base_url));

    let mut stream = client.exec_events_stream("box_1", "exec_1").await.unwrap();

    let mut events = Vec::new();
    while let Some(result) = stream.next().await {
        events.push(result.unwrap());
    }

    assert_eq!(events.len(), 3);
    assert_eq!(events[0].event_type, "log");
    assert_eq!(events[0].stream, Some("stdout".to_string()));
    assert_eq!(events[0].data, "hello\n");
    assert_eq!(events[0].seq, 0);

    assert_eq!(events[1].event_type, "log");
    assert_eq!(events[1].stream, Some("stderr".to_string()));
    assert_eq!(events[1].seq, 1);

    assert_eq!(events[2].event_type, "exit");
    assert!(events[2].stream.is_none());
    assert_eq!(events[2].seq, 2);
}

#[tokio::test]
async fn test_exec_events_stream_error_response() {
    let app = Router::new().route(
        "/v1/boxes/{box_id}/exec/{exec_id}/events",
        get(|| async {
            (
                StatusCode::NOT_FOUND,
                axum::Json(json!({"code": "EXEC_NOT_FOUND", "message": "Exec not found"})),
            )
        }),
    );

    let base_url = start_mock_server(app).await;
    let client = BoxRunClient::new(Some(&base_url));

    let result = client.exec_events_stream("box_1", "nonexistent").await;
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("Expected error, got Ok"),
    };
    assert!(err.message.contains("EXEC_NOT_FOUND"));
}

#[tokio::test]
async fn test_exec_stream_starts_and_streams() {
    let app = Router::new()
        .route(
            "/v1/boxes/{box_id}/exec",
            post(|Path(box_id): Path<String>| async move {
                (
                    StatusCode::CREATED,
                    axum::Json(json!({
                        "id": "exec_mock",
                        "box_id": box_id,
                        "status": "running",
                        "cmd": ["echo", "hi"],
                        "env": null,
                        "workdir": null,
                        "timeout_ms": null,
                        "exit_code": null,
                        "error_message": null,
                        "created_at": "2024-01-01T00:00:00+00:00",
                        "finished_at": null,
                    })),
                )
            }),
        )
        .route(
            "/v1/boxes/{box_id}/exec/{exec_id}/events",
            get(|| async {
                sse_events_response(vec![
                    ("log", json!({"stream": "stdout", "data": "hi\n", "seq": 0})),
                    (
                        "exit",
                        json!({"stream": null, "data": "{\"exit_code\":0}", "seq": 1}),
                    ),
                ])
            }),
        );

    let base_url = start_mock_server(app).await;
    let client = BoxRunClient::new(Some(&base_url));

    let cmd: Vec<String> = vec!["echo".into(), "hi".into()];
    let mut stream = client.exec_stream("box_1", &cmd, None, None).await.unwrap();

    let mut events = Vec::new();
    while let Some(result) = stream.next().await {
        events.push(result.unwrap());
    }

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, "log");
    assert_eq!(events[0].data, "hi\n");
    assert_eq!(events[1].event_type, "exit");
}

#[tokio::test]
async fn test_exec_events_stream_exit_terminates() {
    // Send events after exit — they should not be yielded
    let app = Router::new().route(
        "/v1/boxes/{box_id}/exec/{exec_id}/events",
        get(|| async {
            sse_events_response(vec![
                (
                    "log",
                    json!({"stream": "stdout", "data": "before\n", "seq": 0}),
                ),
                (
                    "exit",
                    json!({"stream": null, "data": "{\"exit_code\":0}", "seq": 1}),
                ),
                (
                    "log",
                    json!({"stream": "stdout", "data": "after\n", "seq": 2}),
                ),
            ])
        }),
    );

    let base_url = start_mock_server(app).await;
    let client = BoxRunClient::new(Some(&base_url));

    let mut stream = client.exec_events_stream("box_1", "exec_1").await.unwrap();

    let mut events = Vec::new();
    while let Some(result) = stream.next().await {
        events.push(result.unwrap());
    }

    // Should only get 2 events (log + exit), not the one after exit
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, "log");
    assert_eq!(events[1].event_type, "exit");
}
