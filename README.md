# BoxRun [![Discord](https://img.shields.io/badge/Discord-Join-5865F2?logo=discord&logoColor=white)](https://discord.gg/bCmaK4Ce)

Ultra-lightweight local VM platform. Spin up isolated Linux VMs in milliseconds — no Docker, no Vagrant, no heavyweight hypervisor.

## Why BoxRun?

- **Millisecond boot times** — VMs start in <500ms, not minutes
- **Real Linux VMs** — full kernel isolation via microVM technology, not containers
- **Volume mounts** — share host directories with Docker-style `-v /host:/guest[:ro]`
- **Web dashboard** — real-time browser UI at `http://localhost:9090/ui`
- **Dead simple** — one binary, one socket, one SQLite file
- **Auto-start** — server launches automatically on first command
- **Shell completions** — bash, zsh, fish, powershell

## Install

### From source (requires [BoxLite](https://github.com/boxlite-ai/boxlite))

```bash
git clone --recurse-submodules https://github.com/boxlite-ai/boxlite.git ../boxlite
./scripts/install-local.sh
```

### Pre-built binary (macOS Apple Silicon)

```bash
curl -fsSL https://raw.githubusercontent.com/boxlite-ai/boxrun/main/install.sh | sh
```

## Quick Start

```bash
# One command: create a VM and drop into a shell
boxrun shell ubuntu

# Or step by step
boxrun create ubuntu:24.04 --name dev
boxrun attach dev

# Run commands non-interactively
boxrun exec dev -- uname -a

# Mount a host directory into the VM
boxrun create ubuntu:24.04 --name work -v /path/to/project:/root/project

# Copy files in and out
boxrun cp ./data.csv dev:/root/data.csv
boxrun cp dev:/root/results.csv ./results.csv

# Lifecycle: stop (preserves disk), restart, destroy
boxrun stop dev
boxrun start dev
boxrun rm dev --force
```

> **Note:** The server starts automatically — no need to run `boxrun serve` manually.

## Shell Completions

```bash
# Zsh (add to ~/.zshrc)
eval "$(boxrun completion zsh)"

# Bash (add to ~/.bashrc)
eval "$(boxrun completion bash)"

# Fish
boxrun completion fish | source
```

## Python SDK

Install the Python SDK separately:

```bash
pip install boxrun
```

```python
import asyncio
from boxrun import BoxRunClient

async def main():
    async with BoxRunClient() as client:
        box = await client.create("ubuntu:24.04", name="dev")

        result = await box.exec(["echo", "hello"])
        print(f"Exit code: {result.exit_code}")

        async for event in box.exec_stream(["apt-get", "update"]):
            if event.type == "log":
                print(event.data, end="")

        await box.remove()

asyncio.run(main())
```

See [docs/sdk.md](docs/sdk.md) for the full API reference and [examples/](examples/) for runnable scripts.

## Rust SDK

Add to your `Cargo.toml`:

```toml
[dependencies]
boxrun-sdk = { git = "https://github.com/boxlite-ai/boxrun" }
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

## Documentation

| Document | Description |
|----------|-------------|
| [CLI Reference](docs/cli.md) | All commands, flags, interactive terminal, volume mounts, web dashboard |
| [Python SDK](docs/sdk.md) | Full SDK API, data types, error handling, AI agent patterns |
| [REST API](docs/api.md) | HTTP endpoints, WebSocket attach, SSE streaming |
| [Architecture](docs/architecture.md) | System overview, component descriptions, box lifecycle |
| [Configuration](docs/configuration.md) | Environment variables, resource limits, per-box defaults |

## Development

```sh
cargo build              # Build all crates
cargo test --workspace   # Run tests
cargo clippy --workspace # Lint
cargo fmt --all          # Format
cargo run -- serve       # Run server
```

## Project Structure

```
crates/
├── boxrun-types/     # Shared types (errors, models, config)
├── boxrun-server/    # Server + CLI binary
└── boxrun-sdk/       # Rust SDK crate
```

## License

Apache-2.0
