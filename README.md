# BoxRun [![Discord](https://img.shields.io/badge/Discord-Join-5865F2?logo=discord&logoColor=white)](https://discord.gg/xe4QK8wycu)

Ultra-lightweight sandbox platform for AI agents.

BoxRun is the management layer for [BoxLite](https://github.com/boxlite-ai/boxlite) sandboxes — create, list, stop, and delete boxes through CLI, REST API, Python SDK, or web dashboard. All in a single binary, no Docker, no Kubernetes, no external dependencies.

## Why BoxRun?

- **Millisecond boot times** — VMs start in <500ms, not minutes
- **Real Linux VMs** — full kernel isolation via microVM technology, not containers
- **Volume mounts** — share host directories with Docker-style `-v /host:/guest[:ro]`
- **Web dashboard** — real-time browser UI at `http://localhost:9090/ui`
- **Dead simple** — one binary, one socket, one SQLite file
- **Auto-start** — server launches automatically on first command
- **Shell completions** — bash, zsh, fish, powershell

## Install

> **Requirements:** macOS Apple Silicon (M1+). Linux support coming soon.

```bash
# Homebrew
brew tap boxlite-ai/tap
brew install boxrun

# Or via install script
curl -fsSL https://boxlite.ai/boxrun/install | sh
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

## Python SDK

```bash
pip install boxrun-sdk
```

```python
import asyncio
from boxrun_sdk import BoxRunClient

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
    let mut bx = client.create(
        "ubuntu",           // image
        Some("mybox"),      // name
        2,                  // cpus
        1024,               // memory_mb
        8,                  // disk_size_gb
        false,              // network
        "/root",            // workdir
        None,               // env
        None,               // volumes
    ).await.unwrap();

    let exec = bx.exec(&["echo", "hello"], None, None).await.unwrap();
    println!("Exit code: {:?}", exec.exit_code);

    bx.remove(true).await.unwrap();
}
```

## Shell Completions

```bash
# Zsh (add to ~/.zshrc)
eval "$(boxrun completion zsh)"

# Bash (add to ~/.bashrc)
eval "$(boxrun completion bash)"

# Fish
boxrun completion fish | source
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

### Build from source

```bash
./scripts/install-local.sh
```

This will automatically fetch [BoxLite](https://github.com/boxlite-ai/boxlite) if needed, build the binary, and install it.

### Commands

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
sdk/
└── python/           # Python SDK (boxrun-sdk on PyPI)
```

## License

Apache-2.0
