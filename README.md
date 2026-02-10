# BoxRun (Rust)

A local VM execution platform built on BoxLite. Single binary for server + CLI, with a Rust SDK crate.

## Install

### From GitHub Releases

```sh
curl -fsSL https://raw.githubusercontent.com/aspect-build/boxrun/main/boxrun-rs/install.sh | sh
```

### From source

```sh
cargo install --path crates/boxrun-server
```

## Quick Start

```sh
# Start the server
boxrun serve

# Create a box
boxrun create ubuntu --name mybox

# Execute a command
boxrun exec mybox -- echo "Hello from BoxRun"

# List boxes
boxrun ls

# Stop and remove
boxrun stop mybox
boxrun rm mybox
```

## CLI Commands

| Command | Description |
|---------|-------------|
| `boxrun serve` | Start the API server |
| `boxrun create <image>` | Create and start a box |
| `boxrun ls` | List boxes |
| `boxrun stop <box>` | Stop a running box |
| `boxrun start <box>` | Start a stopped box |
| `boxrun rm <box>` | Remove a box |
| `boxrun exec <box> -- <cmd>` | Execute a command |
| `boxrun attach <box>` | Attach to a box (TTY) |
| `boxrun cp <src> <dst>` | Copy files (box:path or local) |
| `boxrun run <image> -- <cmd>` | Run command in ephemeral box |
| `boxrun gc` | Garbage collect stopped boxes |
| `boxrun images` | List available images |

## API

The server exposes a REST API at `http://127.0.0.1:9090` (or Unix socket `~/.boxrun/boxrun.sock`):

- `GET /v1/info` — Server info
- `POST /v1/boxes` — Create a box
- `GET /v1/boxes` — List boxes
- `GET /v1/boxes/{id}` — Get box details
- `POST /v1/boxes/{id}:stop` — Stop a box
- `POST /v1/boxes/{id}:start` — Start a box
- `DELETE /v1/boxes/{id}` — Remove a box
- `POST /v1/boxes/{id}/exec` — Start an execution
- `GET /v1/boxes/{id}/exec/{eid}/events` — SSE event stream
- `POST /v1/boxes/{id}/files/upload` — Upload file (multipart)
- `POST /v1/boxes/{id}/files/download` — Download file
- `GET /v1/boxes/{id}/attach` — WebSocket TTY attach
- `POST /v1/run` — Run command in ephemeral box
- `POST /v1/gc` — Garbage collect
- `GET /ui` — Web dashboard

## Rust SDK

Add to your `Cargo.toml`:

```toml
[dependencies]
boxrun-sdk = { path = "crates/boxrun-sdk" }
```

```rust
use boxrun_sdk::client::BoxRunClient;

#[tokio::main]
async fn main() {
    let client = BoxRunClient::new(None);
    let mut box_handle = client.create(
        "ubuntu", Some("mybox"), 2, 1024, 8, false, "/root", None, None,
    ).await.unwrap();

    let exec = box_handle.exec(
        &["echo".into(), "hello".into()], None, None,
    ).await.unwrap();

    println!("Exit code: {:?}", exec.exit_code);
    box_handle.remove(true).await.unwrap();
}
```

## Project Structure

```
crates/
├── boxrun-types/     # Shared types (errors, models, config)
├── boxrun-server/    # Server + CLI binary
└── boxrun-sdk/       # Rust SDK crate
```

## Development

```sh
cargo build              # Build all crates
cargo test --workspace   # Run tests
cargo clippy --workspace # Lint
cargo fmt --all          # Format
cargo run -- serve       # Run server
```

## License

MIT
