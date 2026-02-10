use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;

use boxrun_types::config::{resolve_image, socket_path, IMAGE_CATALOG};
use serde_json::{json, Value};

/// ANSI color helpers (only when stderr/stdout is a terminal).
fn is_tty() -> bool {
    unsafe { libc::isatty(1) != 0 }
}

fn green(s: &str) -> String {
    if is_tty() {
        format!("\x1b[32m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

fn red(s: &str) -> String {
    if is_tty() {
        format!("\x1b[31m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

fn yellow(s: &str) -> String {
    if is_tty() {
        format!("\x1b[33m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

fn dim(s: &str) -> String {
    if is_tty() {
        format!("\x1b[2m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

fn bold(s: &str) -> String {
    if is_tty() {
        format!("\x1b[1m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// Create an HTTP client.
fn client() -> reqwest::Client {
    reqwest::Client::new()
}

/// Determine the base URL for API requests.
fn base_url() -> String {
    let sock = socket_path();
    if Path::new(&sock).exists() {
        // reqwest doesn't natively support Unix sockets, so we'll use TCP
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

            // Friendly hints based on error code
            eprintln!("{} [{code}] {msg}", red("Error:"));
            match code {
                "BOX_NOT_RUNNING" => {
                    eprintln!(
                        "{}",
                        dim("  Hint: Start the box first with: boxrun start <box>")
                    );
                }
                "BOX_ALREADY_RUNNING" => {
                    eprintln!("{}", dim("  Hint: Stop it first with: boxrun stop <box>"));
                }
                "NAME_ALREADY_EXISTS" => {
                    eprintln!(
                        "{}",
                        dim("  Hint: Use a different name, or remove the existing box: boxrun rm <name> --force")
                    );
                }
                "BOX_NOT_FOUND" => {
                    eprintln!("{}", dim("  Hint: List available boxes with: boxrun ls"));
                }
                _ => {}
            }
        } else {
            eprintln!("{} {status} {body}", red("Error:"));
        }
        process::exit(1);
    }
    serde_json::from_str(body).unwrap_or(json!({}))
}

/// Try to auto-start the server in the background if it's not running.
/// Returns true if server became available.
async fn ensure_server() -> bool {
    // Quick check if server is already running
    let url = format!("{}/v1/info", base_url());
    if client()
        .get(&url)
        .timeout(std::time::Duration::from_millis(500))
        .send()
        .await
        .is_ok()
    {
        return true;
    }

    // Server not running — try to start it
    eprintln!(
        "{}",
        dim("Server not running. Starting boxrun serve in background...")
    );

    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(_) => return false,
    };

    // Spawn server process in background
    let child = std::process::Command::new(&exe)
        .arg("serve")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        .spawn();

    if child.is_err() {
        return false;
    }

    // Wait for server to become ready (up to 30s)
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if client()
            .get(&url)
            .timeout(std::time::Duration::from_millis(500))
            .send()
            .await
            .is_ok()
        {
            eprintln!("{}", dim("Server started."));
            return true;
        }
    }
    false
}

fn handle_connection_error(_e: &reqwest::Error) {
    eprintln!("{} Cannot connect to BoxRun server.", red("Error:"));
    eprintln!("{}", dim("  Start it with: boxrun serve"));
    process::exit(1);
}

/// Wrapper: ensure server is running, then execute the async operation.
/// If connection fails, auto-start server and retry once.
async fn with_server<F, Fut>(f: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    // Try to connect first; if it fails, auto-start
    let url = format!("{}/v1/info", base_url());
    if client()
        .get(&url)
        .timeout(std::time::Duration::from_millis(500))
        .send()
        .await
        .is_err()
        && !ensure_server().await
    {
        eprintln!("{} Could not start BoxRun server.", red("Error:"));
        eprintln!("{}", dim("  Try starting manually: boxrun serve"));
        process::exit(1);
    }
    f().await;
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
    with_server(|| async {
        create_inner(image, name, cpu, memory, disk, network, volume).await;
    })
    .await;
}

async fn create_inner(
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
                        eprintln!("{} {e}", red("Error:"));
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

/// Create a box and return its ID (for use by shell command).
async fn create_and_get_id(
    image: &str,
    name: Option<&str>,
    cpu: i64,
    memory: i64,
    disk: i64,
    volume: &[String],
) -> String {
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
                        eprintln!("{} {e}", red("Error:"));
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
            let id = box_data
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let display_name = box_data.get("name").and_then(|v| v.as_str()).unwrap_or(&id);
            eprintln!("Created box {}", bold(display_name));
            id
        }
        Err(e) => {
            handle_connection_error(&e);
            unreachable!()
        }
    }
}

// ── ls ───────────────────────────────────────────────────────────────────

pub async fn ls(status: Option<&str>) {
    with_server(|| async {
        ls_inner(status).await;
    })
    .await;
}

async fn ls_inner(status: Option<&str>) {
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

            // Table header
            let header = format!(
                "{:<20} {:<15} {:<12} {:<25} {:>4} {:>8} {:>6} {:<20}",
                "ID", "NAME", "STATUS", "IMAGE", "CPU", "MEM", "DISK", "CREATED"
            );
            println!("{}", bold(&header));
            println!("{}", dim(&"-".repeat(header.len())));

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

                let status_colored = match status {
                    "running" => green(status),
                    "stopped" => red(status),
                    "creating" => yellow(status),
                    _ => status.to_string(),
                };

                println!(
                    "{:<20} {:<15} {:<22} {:<25} {:>4} {:>7}MB {:>5}G {:<20}",
                    id, name, status_colored, image, cpu, mem, disk, created_short
                );
            }
        }
        Err(e) => handle_connection_error(&e),
    }
}

// ── stop ─────────────────────────────────────────────────────────────────

pub async fn stop(box_id: &str) {
    with_server(|| async {
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
    })
    .await;
}

// ── start ────────────────────────────────────────────────────────────────

pub async fn start(box_id: &str) {
    with_server(|| async {
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
    })
    .await;
}

// ── rm ───────────────────────────────────────────────────────────────────

pub async fn rm(box_id: &str, force: bool) {
    with_server(|| async {
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
    })
    .await;
}

// ── exec ─────────────────────────────────────────────────────────────────

pub async fn exec_cmd(box_id: &str, cmd: &[String], detach: bool, timeout: Option<i64>) {
    with_server(|| async {
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

                stream_exec_events(exec_box_id, exec_id).await;
            }
            Err(e) => handle_connection_error(&e),
        }
    })
    .await;
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

    eprintln!("{} Connection to server lost", red("Error:"));
    process::exit(1);
}

// ── attach ───────────────────────────────────────────────────────────────

pub async fn attach(box_id: &str, shell: &str) {
    with_server(|| async {
        attach_inner(box_id, shell).await;
    })
    .await;
}

async fn attach_inner(box_id: &str, shell: &str) {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite;

    let (cols, rows) = terminal_size();
    let term = std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".into());

    let host = std::env::var("BOXRUN_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = std::env::var("BOXRUN_PORT").unwrap_or_else(|_| "9090".into());
    let url = format!(
        "ws://{host}:{port}/v1/boxes/{box_id}/attach?shell={shell}&cols={cols}&rows={rows}&term={term}"
    );

    let (ws_stream, _) = match tokio_tungstenite::connect_async(&url).await {
        Ok(conn) => conn,
        Err(e) => {
            let err_msg = e.to_string();
            if err_msg.contains("404") || err_msg.contains("Not Found") {
                eprintln!("{} Box '{}' not found.", red("Error:"), box_id);
                eprintln!("{}", dim("  Hint: List available boxes with: boxrun ls"));
            } else if err_msg.contains("409") || err_msg.contains("not running") {
                eprintln!("{} Box '{}' is not running.", red("Error:"), box_id);
                eprintln!(
                    "{}",
                    dim(&format!(
                        "  Hint: Start it first with: boxrun start {box_id}"
                    ))
                );
            } else {
                eprintln!("{} Cannot attach to box: {e}", red("Error:"));
                eprintln!(
                    "{}",
                    dim("  Hint: Is the server running? Try: boxrun serve")
                );
            }
            process::exit(1);
        }
    };

    let (mut ws_sink, mut ws_stream) = ws_stream.split();

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

    let stdin_fd = 0;
    let old_termios = unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(stdin_fd, &mut t) != 0 {
            eprintln!("{} Failed to get terminal attributes", red("Error:"));
            process::exit(1);
        }
        t
    };

    let _terminal_guard = TerminalGuard {
        fd: stdin_fd,
        original: old_termios,
    };
    unsafe {
        let mut raw = old_termios;
        libc::cfmakeraw(&mut raw);
        libc::tcsetattr(stdin_fd, libc::TCSADRAIN, &raw);
    }

    let (stdin_tx, mut stdin_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);

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

    let mut sigwinch =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())
            .expect("Failed to register SIGWINCH handler");

    let mut exit_code: i32 = 0;
    loop {
        tokio::select! {
            Some(data) = stdin_rx.recv() => {
                let msg = tungstenite::Message::Binary(data.into());
                if ws_sink.send(msg).await.is_err() {
                    exit_code = 1;
                    break;
                }
            }
            msg = ws_stream.next() => {
                match msg {
                    Some(Ok(tungstenite::Message::Binary(data))) => {
                        let stdout_fd = 1;
                        unsafe {
                            libc::write(stdout_fd, data.as_ptr() as *const libc::c_void, data.len());
                        }
                    }
                    Some(Ok(tungstenite::Message::Text(text))) => {
                        if let Ok(ctrl) = serde_json::from_str::<serde_json::Value>(&text) {
                            match ctrl.get("type").and_then(|v| v.as_str()) {
                                Some("exit") => {
                                    exit_code = ctrl.get("code")
                                        .and_then(|v| v.as_i64())
                                        .unwrap_or(0) as i32;
                                    break;
                                }
                                Some("error") => {
                                    unsafe {
                                        libc::tcsetattr(stdin_fd, libc::TCSADRAIN, &old_termios);
                                    }
                                    let msg = ctrl.get("message")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("Unknown error");
                                    eprintln!("\r\n{} {msg}", red("Error:"));
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

    stdin_task.abort();
    let _ = ws_sink.close().await;

    if exit_code != 0 {
        process::exit(exit_code);
    }
}

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

// ── shell (create + attach) ─────────────────────────────────────────────

pub async fn shell(
    image: &str,
    name: Option<&str>,
    cpu: i64,
    memory: i64,
    disk: i64,
    shell_cmd: &str,
    volume: &[String],
) {
    with_server(|| async {
        let box_id = create_and_get_id(image, name, cpu, memory, disk, volume).await;
        attach_inner(&box_id, shell_cmd).await;
    })
    .await;
}

// ── cp ───────────────────────────────────────────────────────────────────

fn is_remote_path(s: &str) -> bool {
    if let Some((before_colon, _)) = s.split_once(':') {
        !before_colon.starts_with('/') && !before_colon.starts_with('.')
    } else {
        false
    }
}

pub async fn cp(src: &str, dst: &str) {
    with_server(|| async {
        if is_remote_path(src) {
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
                        eprintln!("{} Writing file: {e}", red("Error:"));
                        process::exit(1);
                    }
                    println!("Downloaded to {dst}");
                }
                Err(e) => handle_connection_error(&e),
            }
        } else if is_remote_path(dst) {
            let (box_part, remote_path) = dst.split_once(':').unwrap();
            let src_path = Path::new(src);
            if !src_path.exists() {
                eprintln!("{} File not found: {src}", red("Error:"));
                process::exit(1);
            }

            let file_bytes = match std::fs::read(src_path) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("{} Reading file: {e}", red("Error:"));
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
            eprintln!(
                "{} One of src/dst must be in BOX:PATH format",
                red("Error:")
            );
            process::exit(1);
        }
    })
    .await;
}

// ── run ──────────────────────────────────────────────────────────────────

pub async fn run_ephemeral(image: &str, cmd: &[String], timeout: Option<i64>, disk: i64) {
    with_server(|| async {
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
    })
    .await;
}

// ── gc ───────────────────────────────────────────────────────────────────

pub async fn gc(older_than: i64) {
    with_server(|| async {
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
    })
    .await;
}

// ── images ───────────────────────────────────────────────────────────────

pub fn images() {
    let header = format!("{:<12} {:<25} {}", "ALIAS", "IMAGE", "DESCRIPTION");
    println!("{}", bold(&header));
    println!("{}", dim(&"-".repeat(header.len())));
    for (alias, (image, description)) in IMAGE_CATALOG.iter() {
        println!("{:<12} {:<25} {}", green(alias), image, description);
    }
}

// ── upgrade ──────────────────────────────────────────────────────────────

const GITHUB_REPO: &str = "boxlite-ai/boxrun";

fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn install_dir() -> PathBuf {
    std::env::current_exe()
        .expect("cannot determine executable path")
        .parent()
        .expect("executable has no parent directory")
        .to_path_buf()
}

fn detect_archive_name() -> String {
    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else {
        "unknown"
    };
    let os = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    };
    format!("boxrun-{arch}-{os}.tar.gz")
}

pub async fn upgrade() {
    let current = current_version();
    eprintln!("Current version: v{current}");
    eprintln!("{}", dim("Checking for updates..."));

    // Fetch latest release info from GitHub API
    let api_url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
    let resp = match client()
        .get(&api_url)
        .header("User-Agent", "boxrun-upgrade")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{} Failed to check for updates: {e}", red("Error:"));
            process::exit(1);
        }
    };

    let body: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{} Failed to parse release info: {e}", red("Error:"));
            process::exit(1);
        }
    };

    let tag = body.get("tag_name").and_then(|v| v.as_str()).unwrap_or("");
    if tag.is_empty() {
        eprintln!("{} Could not determine latest release tag", red("Error:"));
        process::exit(1);
    }

    let latest = tag.strip_prefix('v').unwrap_or(tag);
    if latest == current {
        println!("{}", green("Already up to date."));
        return;
    }

    println!("New version available: v{current} -> {tag}");

    let archive_name = detect_archive_name();
    let download_url =
        format!("https://github.com/{GITHUB_REPO}/releases/download/{tag}/{archive_name}");
    let checksums_url =
        format!("https://github.com/{GITHUB_REPO}/releases/download/{tag}/checksums.txt");

    // Download archive
    eprintln!("{}", dim(&format!("Downloading {archive_name}...")));
    let archive_bytes = match client()
        .get(&download_url)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
    {
        Ok(r) => {
            if !r.status().is_success() {
                eprintln!("{} Download failed: HTTP {}", red("Error:"), r.status());
                process::exit(1);
            }
            match r.bytes().await {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("{} Download failed: {e}", red("Error:"));
                    process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("{} Download failed: {e}", red("Error:"));
            process::exit(1);
        }
    };

    // Verify checksum if available
    if let Ok(resp) = client()
        .get(&checksums_url)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
    {
        if resp.status().is_success() {
            if let Ok(checksums_text) = resp.text().await {
                verify_checksum(&archive_bytes, &archive_name, &checksums_text);
            }
        } else {
            eprintln!(
                "{}",
                yellow("Warning: checksums.txt not found, skipping verification")
            );
        }
    }

    // Extract archive to a staging directory, then atomically swap into place.
    let dir = install_dir();
    eprintln!("{}", dim(&format!("Installing to {}...", dir.display())));

    let decoder = flate2::read::GzDecoder::new(&archive_bytes[..]);
    let mut archive = tar::Archive::new(decoder);

    let tmp_dir = dir.join(".upgrade-tmp");
    if tmp_dir.exists() {
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
    std::fs::create_dir_all(&tmp_dir).unwrap_or_else(|e| {
        eprintln!("{} Failed to create temp dir: {e}", red("Error:"));
        process::exit(1);
    });

    if let Err(e) = archive.unpack(&tmp_dir) {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        eprintln!("{} Failed to extract archive: {e}", red("Error:"));
        process::exit(1);
    }

    // The archive contains a boxrun/ directory
    let extracted = tmp_dir.join("boxrun");

    // Stage new binary next to the real one, then atomic-rename into place.
    let new_binary = extracted.join("boxrun");
    if new_binary.exists() {
        let dest_binary = dir.join("boxrun");
        let staged_binary = dir.join(".boxrun-new");

        // Copy to staging location in same directory (same filesystem → rename is atomic)
        if let Err(e) = std::fs::copy(&new_binary, &staged_binary) {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            let _ = std::fs::remove_file(&staged_binary);
            eprintln!("{} Failed to stage new binary: {e}", red("Error:"));
            process::exit(1);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ =
                std::fs::set_permissions(&staged_binary, std::fs::Permissions::from_mode(0o755));
        }

        // Atomic rename
        if let Err(e) = std::fs::rename(&staged_binary, &dest_binary) {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            let _ = std::fs::remove_file(&staged_binary);
            eprintln!("{} Failed to replace binary: {e}", red("Error:"));
            process::exit(1);
        }
    }

    // Replace runtime: stage as .runtime-new, remove old, rename into place.
    let new_runtime = extracted.join("runtime");
    if new_runtime.is_dir() {
        let dest_runtime = dir.join("runtime");
        let staged_runtime = dir.join(".runtime-new");

        if staged_runtime.exists() {
            let _ = std::fs::remove_dir_all(&staged_runtime);
        }
        if let Err(e) = copy_dir_all(&new_runtime, &staged_runtime) {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            let _ = std::fs::remove_dir_all(&staged_runtime);
            eprintln!("{} Failed to stage runtime: {e}", red("Error:"));
            process::exit(1);
        }

        // Swap: remove old, rename new into place
        let old_runtime = dir.join(".runtime-old");
        if dest_runtime.exists() {
            // Move current runtime aside (not delete yet — rollback safety)
            let _ = std::fs::rename(&dest_runtime, &old_runtime);
        }
        if let Err(e) = std::fs::rename(&staged_runtime, &dest_runtime) {
            // Rollback: restore old runtime
            if old_runtime.exists() {
                let _ = std::fs::rename(&old_runtime, &dest_runtime);
            }
            let _ = std::fs::remove_dir_all(&tmp_dir);
            eprintln!("{} Failed to replace runtime: {e}", red("Error:"));
            process::exit(1);
        }
        let _ = std::fs::remove_dir_all(&old_runtime);
    }

    // Clean up
    let _ = std::fs::remove_dir_all(&tmp_dir);

    println!("{}", green(&format!("Upgraded to {tag}")));
}

fn verify_checksum(data: &[u8], filename: &str, checksums_text: &str) {
    use sha2::{Digest, Sha256};

    // Find the line for our file in checksums.txt
    let expected = checksums_text
        .lines()
        .find(|line| line.ends_with(filename))
        .and_then(|line| line.split_whitespace().next());

    let expected = match expected {
        Some(h) => h,
        None => {
            eprintln!(
                "{}",
                yellow(&format!(
                    "Warning: no checksum found for {filename}, skipping verification"
                ))
            );
            return;
        }
    };

    // Compute SHA-256 in-process (no external tool dependency)
    let hash = Sha256::digest(data);
    let actual_hash = format!("{hash:x}");

    if actual_hash != expected {
        eprintln!("{} Checksum verification failed!", red("Error:"));
        eprintln!("  Expected: {expected}");
        eprintln!("  Got:      {actual_hash}");
        process::exit(1);
    }

    eprintln!("{}", dim("Checksum verified."));
}

fn copy_dir_all(src: &Path, dst: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

// ── uninstall ────────────────────────────────────────────────────────────

pub async fn uninstall() {
    let dir = install_dir();

    println!("This will remove:");
    println!("  {}", dir.display());

    // Check for symlink in common bin dirs
    let symlink_paths = ["/usr/local/bin/boxrun", "/opt/homebrew/bin/boxrun"];
    let mut found_symlink: Option<PathBuf> = None;
    for path in &symlink_paths {
        let p = Path::new(path);
        if p.exists() || p.symlink_metadata().is_ok() {
            // Check if it's a symlink pointing into our install dir
            if let Ok(target) = std::fs::read_link(p) {
                if target.starts_with(&dir) {
                    found_symlink = Some(p.to_path_buf());
                    println!("  {path} -> {}", target.display());
                }
            }
        }
    }

    // Confirm
    eprint!("\nContinue? [y/N] ");
    io::stderr().flush().ok();
    let mut answer = String::new();
    io::stdin().read_line(&mut answer).ok();
    if !answer.trim().eq_ignore_ascii_case("y") {
        println!("Aborted.");
        return;
    }

    // Remove symlink
    if let Some(link) = &found_symlink {
        if let Err(e) = std::fs::remove_file(link) {
            eprintln!(
                "{} Failed to remove symlink {}: {e}",
                yellow("Warning:"),
                link.display()
            );
            eprintln!("{}", dim(&format!("  Try: sudo rm {}", link.display())));
        } else {
            println!("Removed {}", link.display());
        }
    }

    // Remove install directory
    if dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            eprintln!("{} Failed to remove {}: {e}", red("Error:"), dir.display());
            process::exit(1);
        }
        println!("Removed {}", dir.display());
    }

    println!("{}", green("boxrun has been uninstalled."));
}
