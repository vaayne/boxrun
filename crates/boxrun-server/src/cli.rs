use std::io::{self, Write};
use std::path::Path;
use std::process;

use boxrun_types::config::{resolve_image, socket_path, IMAGE_CATALOG};
use serde_json::{json, Value};

/// Create an HTTP client that connects via Unix socket or TCP.
fn client() -> reqwest::Client {
    reqwest::Client::new()
}

/// Determine the base URL for API requests.
fn base_url() -> String {
    let sock = socket_path();
    if Path::new(&sock).exists() {
        // reqwest doesn't natively support Unix sockets, so we'll use TCP
        // In production, use hyper-util with Unix socket connector
    }
    let host = std::env::var("BOXRUN_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = std::env::var("BOXRUN_PORT").unwrap_or_else(|_| "9090".into());
    format!("http://{host}:{port}")
}

fn check_response(status: u16, body: &str) -> Value {
    if status >= 400 {
        if let Ok(err) = serde_json::from_str::<Value>(body) {
            let code = err
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or("UNKNOWN");
            let msg = err.get("message").and_then(|v| v.as_str()).unwrap_or(body);
            eprintln!("Error: [{code}] {msg}");
        } else {
            eprintln!("Error: {status} {body}");
        }
        process::exit(1);
    }
    serde_json::from_str(body).unwrap_or(json!({}))
}

fn handle_connection_error(_e: &reqwest::Error) {
    eprintln!("Error: Cannot connect to BoxRun server. Is it running?");
    eprintln!("Start it with: boxrun serve");
    process::exit(1);
}

fn parse_volume(value: &str) -> Result<Value, String> {
    let parts: Vec<&str> = value.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return Err(format!(
            "Invalid volume format: {value:?} (expected /host:/guest[:ro])"
        ));
    }
    let host_path = parts[0];
    let guest_path = parts[1];
    if host_path.is_empty() {
        return Err(format!("Host path cannot be empty in volume: {value:?}"));
    }
    if guest_path.is_empty() {
        return Err(format!("Guest path cannot be empty in volume: {value:?}"));
    }
    if !host_path.starts_with('/') {
        return Err(format!("Host path must be absolute: {host_path:?}"));
    }
    if !guest_path.starts_with('/') {
        return Err(format!("Guest path must be absolute: {guest_path:?}"));
    }
    let readonly = if parts.len() == 3 {
        match parts[2] {
            "ro" => true,
            "rw" => false,
            other => {
                return Err(format!(
                    "Invalid volume mode: {other:?} (expected 'ro' or 'rw')"
                ))
            }
        }
    } else {
        false
    };
    Ok(json!({
        "host_path": host_path,
        "guest_path": guest_path,
        "readonly": readonly,
    }))
}

// ── serve ────────────────────────────────────────────────────────────────

pub async fn serve(host: &str, port: u16, socket: Option<&str>) {
    use crate::app;
    use boxrun_types::config::db_path;

    let db = db_path();
    let (router, state) = match app::build_app(&db).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to start server: {e}");
            process::exit(1);
        }
    };

    if let Some(sock_path) = socket {
        // Unix socket mode
        if Path::new(sock_path).exists() {
            let _ = std::fs::remove_file(sock_path);
        }
        println!("Starting BoxRun server on {sock_path}");

        let listener = match tokio::net::UnixListener::bind(sock_path) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Failed to bind Unix socket: {e}");
                process::exit(1);
            }
        };

        let state_clone = state.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            app::shutdown(state_clone).await;
            process::exit(0);
        });

        axum::serve(listener, router).await.unwrap_or_else(|e| {
            eprintln!("Server error: {e}");
            process::exit(1);
        });
    } else {
        // TCP mode — clean up stale socket
        let stale_sock = socket_path();
        if Path::new(&stale_sock).exists() {
            let _ = std::fs::remove_file(&stale_sock);
        }
        println!("Starting BoxRun server on http://{host}:{port}");
        println!("Dashboard: http://{host}:{port}/ui");

        let addr = format!("{host}:{port}");
        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Failed to bind {addr}: {e}");
                process::exit(1);
            }
        };

        let state_clone = state.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            app::shutdown(state_clone).await;
            process::exit(0);
        });

        axum::serve(listener, router).await.unwrap_or_else(|e| {
            eprintln!("Server error: {e}");
            process::exit(1);
        });
    }
}

// ── create ───────────────────────────────────────────────────────────────

pub async fn create(
    image: &str,
    name: Option<&str>,
    cpu: i64,
    memory: i64,
    disk: i64,
    network: bool,
    volume: &[String],
) {
    let image = resolve_image(image);
    let volumes: Option<Vec<Value>> = if volume.is_empty() {
        None
    } else {
        Some(
            volume
                .iter()
                .map(|v| match parse_volume(v) {
                    Ok(val) => val,
                    Err(e) => {
                        eprintln!("Error: {e}");
                        process::exit(1);
                    }
                })
                .collect(),
        )
    };

    let mut body = json!({
        "image": image,
        "name": name,
        "cpu": cpu,
        "memory_mb": memory,
        "disk_size_gb": disk,
        "network": network,
    });
    if let Some(vols) = volumes {
        body.as_object_mut()
            .unwrap()
            .insert("volumes".into(), json!(vols));
    }

    let url = format!("{}/v1/boxes", base_url());
    match client()
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            let box_data = check_response(status, &text);
            if let Some(id) = box_data.get("id").and_then(|v| v.as_str()) {
                println!("{id}");
            }
            if let Some(name) = box_data.get("name").and_then(|v| v.as_str()) {
                println!("Name: {name}");
            }
            if let Some(status) = box_data.get("status").and_then(|v| v.as_str()) {
                println!("Status: {status}");
            }
        }
        Err(e) => handle_connection_error(&e),
    }
}

// ── ls ───────────────────────────────────────────────────────────────────

pub async fn ls(status: Option<&str>) {
    let mut url = format!("{}/v1/boxes", base_url());
    if let Some(s) = status {
        url.push_str(&format!("?status={s}"));
    }

    match client()
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) => {
            let status_code = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            let data = check_response(status_code, &text);

            let boxes = match data.as_array() {
                Some(b) => b,
                None => {
                    println!("No boxes found.");
                    return;
                }
            };

            if boxes.is_empty() {
                println!("No boxes found.");
                return;
            }

            let header = format!(
                "{:<20} {:<15} {:<12} {:<25} {:>4} {:>8} {:>6} {:<20}",
                "ID", "NAME", "STATUS", "IMAGE", "CPU", "MEM", "DISK", "CREATED"
            );
            println!("{header}");
            println!("{}", "-".repeat(header.len()));
            for b in boxes {
                let id = b.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let name = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let status = b.get("status").and_then(|v| v.as_str()).unwrap_or("");
                let image = b.get("image").and_then(|v| v.as_str()).unwrap_or("");
                let cpu = b.get("cpu").and_then(|v| v.as_i64()).unwrap_or(0);
                let mem = b.get("memory_mb").and_then(|v| v.as_i64()).unwrap_or(0);
                let disk = b.get("disk_size_gb").and_then(|v| v.as_i64()).unwrap_or(0);
                let created = b.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
                let created_short = if created.len() >= 19 {
                    &created[..19]
                } else {
                    created
                };
                println!(
                    "{:<20} {:<15} {:<12} {:<25} {:>4} {:>7}MB {:>5}G {:<20}",
                    id, name, status, image, cpu, mem, disk, created_short
                );
            }
        }
        Err(e) => handle_connection_error(&e),
    }
}

// ── stop ─────────────────────────────────────────────────────────────────

pub async fn stop(box_id: &str) {
    let url = format!("{}/v1/boxes/{box_id}:stop", base_url());
    match client()
        .post(&url)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            let data = check_response(status, &text);
            if let Some(id) = data.get("id").and_then(|v| v.as_str()) {
                println!("Stopped {id}");
            }
        }
        Err(e) => handle_connection_error(&e),
    }
}

// ── start ────────────────────────────────────────────────────────────────

pub async fn start(box_id: &str) {
    let url = format!("{}/v1/boxes/{box_id}:start", base_url());
    match client()
        .post(&url)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            let data = check_response(status, &text);
            if let Some(id) = data.get("id").and_then(|v| v.as_str()) {
                println!("Started {id}");
            }
        }
        Err(e) => handle_connection_error(&e),
    }
}

// ── rm ───────────────────────────────────────────────────────────────────

pub async fn rm(box_id: &str, force: bool) {
    let url = format!("{}/v1/boxes/{box_id}?force={}", base_url(), force);
    match client()
        .delete(&url)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            check_response(status, &text);
            println!("Removed {box_id}");
        }
        Err(e) => handle_connection_error(&e),
    }
}

// ── exec ─────────────────────────────────────────────────────────────────

pub async fn exec_cmd(box_id: &str, cmd: &[String], detach: bool, timeout: Option<i64>) {
    let timeout_ms = timeout.map(|t| t * 1000);
    let url = format!("{}/v1/boxes/{box_id}/exec", base_url());
    let body = json!({
        "cmd": cmd,
        "timeout_ms": timeout_ms,
    });

    match client()
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            let exec_data = check_response(status, &text);

            let exec_id = exec_data.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let exec_box_id = exec_data
                .get("box_id")
                .and_then(|v| v.as_str())
                .unwrap_or(box_id);

            if detach {
                println!("{exec_id}");
                return;
            }

            // Stream SSE events
            stream_exec_events(exec_box_id, exec_id).await;
        }
        Err(e) => handle_connection_error(&e),
    }
}

async fn stream_exec_events(box_id: &str, exec_id: &str) {
    let url = format!("{}/v1/boxes/{box_id}/exec/{exec_id}/events", base_url());
    let resp = match client().get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            handle_connection_error(&e);
            return;
        }
    };

    let mut buffer = String::new();
    let mut stream = resp.bytes_stream();

    use futures::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(_) => break,
        };
        let text = String::from_utf8_lossy(&chunk);
        buffer.push_str(&text);
        buffer = buffer.replace("\r\n", "\n");

        while let Some(pos) = buffer.find("\n\n") {
            let event_str = buffer[..pos].to_string();
            buffer = buffer[pos + 2..].to_string();

            let mut event_type = None;
            let mut event_data_parts = Vec::new();

            for line in event_str.trim().lines() {
                if let Some(rest) = line.strip_prefix("event:") {
                    event_type = Some(rest.trim().to_string());
                } else if let Some(rest) = line.strip_prefix("data:") {
                    event_data_parts.push(rest.trim().to_string());
                }
            }

            let event_data = if event_data_parts.is_empty() {
                continue;
            } else {
                event_data_parts.join("\n")
            };

            let data: Value = match serde_json::from_str(&event_data) {
                Ok(d) => d,
                Err(_) => continue,
            };

            match event_type.as_deref() {
                Some("log") => {
                    let stream = data
                        .get("stream")
                        .and_then(|v| v.as_str())
                        .unwrap_or("stdout");
                    let text = data.get("data").and_then(|v| v.as_str()).unwrap_or("");
                    if stream == "stderr" {
                        eprint!("{text}");
                        io::stderr().flush().ok();
                    } else {
                        print!("{text}");
                        io::stdout().flush().ok();
                    }
                }
                Some("exit") => {
                    let inner = data.get("data").and_then(|v| v.as_str()).unwrap_or("{}");
                    let inner: Value = serde_json::from_str(inner).unwrap_or(json!({}));
                    let exit_code = inner.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(0);
                    if exit_code != 0 {
                        process::exit(exit_code as i32);
                    }
                    return;
                }
                _ => {}
            }
        }
    }

    // If we reach here, the stream ended without an exit event
    eprintln!("Error: Connection to server lost");
    process::exit(1);
}

// ── attach ───────────────────────────────────────────────────────────────

pub async fn attach(box_id: &str, shell: &str) {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite;

    // Get terminal size
    let (cols, rows) = terminal_size();
    let term = std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".into());

    // Build WebSocket URL
    let host = std::env::var("BOXRUN_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = std::env::var("BOXRUN_PORT").unwrap_or_else(|_| "9090".into());
    let url = format!(
        "ws://{host}:{port}/v1/boxes/{box_id}/attach?shell={shell}&cols={cols}&rows={rows}&term={term}"
    );

    // Connect WebSocket BEFORE entering raw mode
    let (ws_stream, _) = match tokio_tungstenite::connect_async(&url).await {
        Ok(conn) => conn,
        Err(e) => {
            eprintln!("Error: Cannot connect to BoxRun server: {e}");
            eprintln!("Is the server running? Start with: boxrun serve");
            process::exit(1);
        }
    };

    let (mut ws_sink, mut ws_stream) = ws_stream.split();

    // RAII guard that restores terminal on drop (including panics)
    struct TerminalGuard {
        fd: i32,
        original: libc::termios,
    }
    impl Drop for TerminalGuard {
        fn drop(&mut self) {
            unsafe {
                libc::tcsetattr(self.fd, libc::TCSANOW, &self.original);
            }
        }
    }

    // Save terminal state and enter raw mode
    let stdin_fd = 0; // STDIN_FILENO
    let old_termios = unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(stdin_fd, &mut t) != 0 {
            eprintln!("Error: Failed to get terminal attributes");
            process::exit(1);
        }
        t
    };

    // Set raw mode — the guard ensures restore on any exit path (including panic)
    let _terminal_guard = TerminalGuard {
        fd: stdin_fd,
        original: old_termios,
    };
    unsafe {
        let mut raw = old_termios;
        libc::cfmakeraw(&mut raw);
        libc::tcsetattr(stdin_fd, libc::TCSADRAIN, &raw);
    }

    // Channel for stdin data (read in a blocking thread)
    let (stdin_tx, mut stdin_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);

    // Spawn blocking stdin reader
    let stdin_task = tokio::task::spawn_blocking(move || {
        let mut buf = [0u8; 1024];
        loop {
            let n =
                unsafe { libc::read(stdin_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 {
                break;
            }
            let data = buf[..n as usize].to_vec();
            if stdin_tx.blocking_send(data).is_err() {
                break;
            }
        }
    });

    // Handle SIGWINCH (terminal resize)
    let mut sigwinch =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())
            .expect("Failed to register SIGWINCH handler");

    // Main event loop
    let mut exit_code: i32 = 0;
    loop {
        tokio::select! {
            // stdin → WebSocket
            Some(data) = stdin_rx.recv() => {
                let msg = tungstenite::Message::Binary(data.into());
                if ws_sink.send(msg).await.is_err() {
                    exit_code = 1;
                    break;
                }
            }
            // WebSocket → stdout
            msg = ws_stream.next() => {
                match msg {
                    Some(Ok(tungstenite::Message::Binary(data))) => {
                        let stdout_fd = 1; // STDOUT_FILENO
                        unsafe {
                            libc::write(stdout_fd, data.as_ptr() as *const libc::c_void, data.len());
                        }
                    }
                    Some(Ok(tungstenite::Message::Text(text))) => {
                        // Control messages from server
                        if let Ok(ctrl) = serde_json::from_str::<serde_json::Value>(&text) {
                            match ctrl.get("type").and_then(|v| v.as_str()) {
                                Some("exit") => {
                                    exit_code = ctrl.get("code")
                                        .and_then(|v| v.as_i64())
                                        .unwrap_or(0) as i32;
                                    break;
                                }
                                Some("error") => {
                                    // Restore terminal before printing error
                                    unsafe {
                                        libc::tcsetattr(stdin_fd, libc::TCSADRAIN, &old_termios);
                                    }
                                    let msg = ctrl.get("message")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("Unknown error");
                                    eprintln!("\r\nError: {msg}");
                                    process::exit(1);
                                }
                                _ => {}
                            }
                        }
                    }
                    Some(Ok(tungstenite::Message::Close(_))) | None => {
                        break;
                    }
                    _ => {}
                }
            }
            // Terminal resize
            _ = sigwinch.recv() => {
                let (new_cols, new_rows) = terminal_size();
                let resize_msg = serde_json::json!({
                    "type": "resize",
                    "cols": new_cols,
                    "rows": new_rows,
                });
                let msg = tungstenite::Message::Text(resize_msg.to_string().into());
                let _ = ws_sink.send(msg).await;
            }
        }
    }

    // Terminal is restored automatically by _terminal_guard Drop

    // Clean up
    stdin_task.abort();
    let _ = ws_sink.close().await;

    if exit_code != 0 {
        process::exit(exit_code);
    }
}

/// Get the current terminal size (cols, rows).
fn terminal_size() -> (u16, u16) {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 {
            (ws.ws_col, ws.ws_row)
        } else {
            (80, 24)
        }
    }
}

// ── cp ───────────────────────────────────────────────────────────────────

/// Check if a string looks like BOX:PATH (not an absolute or relative local path with a colon).
/// BOX:PATH has the form "name:/path" where name does not start with '/' or '.'.
fn is_remote_path(s: &str) -> bool {
    if let Some((before_colon, _)) = s.split_once(':') {
        // If the part before colon starts with '/' or '.', it's a local path
        !before_colon.starts_with('/') && !before_colon.starts_with('.')
    } else {
        false
    }
}

pub async fn cp(src: &str, dst: &str) {
    if is_remote_path(src) {
        // Download: BOX:PATH -> LOCAL
        let (box_part, remote_path) = src.split_once(':').unwrap();
        let url = format!("{}/v1/boxes/{box_part}/files/download", base_url());

        match client()
            .post(&url)
            .json(&json!({"path": remote_path}))
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status().as_u16();
                if status >= 400 {
                    let text = resp.text().await.unwrap_or_default();
                    check_response(status, &text);
                    return;
                }
                let bytes = resp.bytes().await.unwrap_or_default();
                let dst_path = Path::new(dst);
                if let Some(parent) = dst_path.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                if let Err(e) = std::fs::write(dst_path, &bytes) {
                    eprintln!("Error writing file: {e}");
                    process::exit(1);
                }
                println!("Downloaded to {dst}");
            }
            Err(e) => handle_connection_error(&e),
        }
    } else if is_remote_path(dst) {
        // Upload: LOCAL -> BOX:PATH
        let (box_part, remote_path) = dst.split_once(':').unwrap();
        let src_path = Path::new(src);
        if !src_path.exists() {
            eprintln!("Error: {src} not found");
            process::exit(1);
        }

        let file_bytes = match std::fs::read(src_path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("Error reading file: {e}");
                process::exit(1);
            }
        };

        let file_name = src_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("file")
            .to_string();

        let file_part = reqwest::multipart::Part::bytes(file_bytes).file_name(file_name);
        let form = reqwest::multipart::Form::new()
            .part("file", file_part)
            .text("dest", remote_path.to_string());

        let url = format!("{}/v1/boxes/{box_part}/files/upload", base_url());
        match client()
            .post(&url)
            .multipart(form)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let text = resp.text().await.unwrap_or_default();
                check_response(status, &text);
                println!("Uploaded to {box_part}:{remote_path}");
            }
            Err(e) => handle_connection_error(&e),
        }
    } else {
        eprintln!("Error: One of src/dst must be in BOX:PATH format");
        process::exit(1);
    }
}

// ── run ──────────────────────────────────────────────────────────────────

pub async fn run_ephemeral(image: &str, cmd: &[String], timeout: Option<i64>, disk: i64) {
    let image = resolve_image(image);
    let timeout_ms = timeout.map(|t| t * 1000);
    let url = format!("{}/v1/run", base_url());
    let body = json!({
        "image": image,
        "cmd": cmd,
        "timeout_ms": timeout_ms,
        "disk_size_gb": disk,
    });

    match client()
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(300))
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            let result = check_response(status, &text);

            if let Some(stdout) = result.get("stdout").and_then(|v| v.as_str()) {
                if !stdout.is_empty() {
                    print!("{stdout}");
                    io::stdout().flush().ok();
                }
            }
            if let Some(stderr) = result.get("stderr").and_then(|v| v.as_str()) {
                if !stderr.is_empty() {
                    eprint!("{stderr}");
                    io::stderr().flush().ok();
                }
            }

            let exit_code = result
                .get("exit_code")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            if exit_code != 0 {
                process::exit(exit_code as i32);
            }
        }
        Err(e) => handle_connection_error(&e),
    }
}

// ── gc ───────────────────────────────────────────────────────────────────

pub async fn gc(older_than: i64) {
    let url = format!("{}/v1/gc", base_url());
    let body = json!({"older_than": older_than});

    match client()
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            let result = check_response(status, &text);
            let removed = result.get("removed").and_then(|v| v.as_i64()).unwrap_or(0);
            println!("Removed {removed} box(es)");
        }
        Err(e) => handle_connection_error(&e),
    }
}

// ── images ───────────────────────────────────────────────────────────────

pub fn images() {
    let header = format!("{:<12} {:<25} {}", "ALIAS", "IMAGE", "DESCRIPTION");
    println!("{header}");
    println!("{}", "-".repeat(header.len()));
    for (alias, (image, description)) in IMAGE_CATALOG.iter() {
        println!("{:<12} {:<25} {}", alias, image, description);
    }
}
