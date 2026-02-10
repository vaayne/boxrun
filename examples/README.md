# BoxRun Examples

Runnable scripts demonstrating BoxRun SDK and CLI usage.

## Prerequisites

Start the BoxRun server before running any example:

```bash
boxrun serve &
```

## Python SDK Examples

| File | Description |
|------|-------------|
| [basic_usage.py](basic_usage.py) | Create a box, run a command, stream output, remove |
| [streaming_output.py](streaming_output.py) | Stream command output in real time with `exec_stream()` |
| [file_transfer.py](file_transfer.py) | Upload a file, process it in the VM, download the result |
| [ephemeral_run.py](ephemeral_run.py) | One-shot `client.run()` — shortest possible example |
| [volume_mounts.py](volume_mounts.py) | Create a box with a volume mount, demonstrate shared filesystem |
| [ai_agent.py](ai_agent.py) | Full agent workflow: upload code, stream execution, download results |
| [multi_claude_agents.py](multi_claude_agents.py) | Spin up multiple VMs running Claude Code in parallel |

```bash
python examples/basic_usage.py
python examples/streaming_output.py
# etc.
```

## CLI Examples

[cli_examples.sh](cli_examples.sh) demonstrates common CLI operations:

```bash
# One-shot ephemeral run
boxrun run ubuntu:24.04 echo "Hello from BoxRun!"

# Create a persistent box
boxrun create ubuntu:24.04 --name mybox

# Execute commands (use -- before commands with flags)
boxrun exec mybox -- uname -a
boxrun exec mybox -- sh -c "echo hello"

# File transfer
boxrun cp local_file.txt mybox:/root/input.txt    # upload
boxrun cp mybox:/root/output.txt local_result.txt  # download

# List, stop, start, remove
boxrun ls
boxrun stop mybox
boxrun start mybox
boxrun rm mybox --force

# Attach an interactive terminal
boxrun attach mybox
```

Run all CLI examples end-to-end:

```bash
bash examples/cli_examples.sh
```

## Supported Images

BoxRun supports **any OCI-compatible container image**. Use the same image names as Docker:

| Image | Pre-installed tools | Best for |
|-------|-------------------|----------|
| `ubuntu:24.04` | bash, apt-get, coreutils | General purpose, install packages via apt |
| `python:3.12-slim` | Python 3.12, pip | Python workloads (no apt-get needed) |
| `node:22-slim` | Node.js 22, npm | JavaScript/TypeScript workloads |
| `alpine:3.20` | sh, apk, wget | Lightweight, fast startup |
| `debian:bookworm-slim` | bash, apt-get | Minimal Debian base |

> **Note:** Minimal images like `ubuntu:24.04` do not include `curl`, `python3`, `git`, or `wget`.
> Either install them with `apt-get` (requires `--network` flag) or use a pre-built image
> like `python:3.12-slim` or `node:22-slim`.

### Using custom images

```bash
# Any public OCI image works
boxrun run golang:1.22 -- go version
boxrun run rust:1.77-slim -- rustc --version

# Create a box with networking to install additional packages
boxrun create ubuntu:24.04 --name dev --network
boxrun exec dev -- sh -c "apt-get update && apt-get install -y curl git"
```
