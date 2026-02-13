# CLI Reference

BoxRun provides a single `boxrun` command with subcommands for managing boxes, executing commands, transferring files, and more.

`BOX` arguments accept either a box ID (`box_abc123`) or a name (`dev`).

## Server

```bash
boxrun serve [--host 127.0.0.1] [--port 9090] [--socket PATH]
```

Start the BoxRun server. By default it listens on TCP `127.0.0.1:9090` (browser-accessible). Use `--socket` to bind to a Unix domain socket instead.

The web dashboard is available at `http://<host>:<port>/ui` when running in TCP mode.

## Box Lifecycle

### Create

```bash
boxrun create IMAGE [--name NAME] [--cpu 2] [--memory 1024] [--network] [-v /host:/guest[:ro]]
```

Create a new box from a container image.

| Flag | Default | Description |
|------|---------|-------------|
| `--name`, `-n` | (auto) | Human-readable name |
| `--cpu` | `2` | CPU cores |
| `--memory`, `-m` | `1024` | Memory in MB |
| `--network` | `false` | Enable networking |
| `-v`, `--volume` | (none) | Volume mount (repeatable) |

### List

```bash
boxrun ls [--status running|stopped|all]
```

List all boxes. Filter by status with `--status`.

### Stop

```bash
boxrun stop BOX
```

Stop a running box. Disk state is preserved — you can restart it later.

### Start

```bash
boxrun start BOX
```

Restart a previously stopped box with its disk intact.

### Remove

```bash
boxrun rm BOX [--force]
```

Remove a box permanently. Use `--force` to remove a running box without stopping it first.

## Interactive Terminal

```bash
boxrun attach BOX [--shell /bin/bash]
```

Open a full interactive terminal session inside a running box. This is BoxRun's killer feature for human users.

Features:
- Full PTY support — vim, htop, tmux, everything works
- Automatic terminal resize handling
- Raw terminal mode — same experience as SSH

The session runs over WebSocket. The box keeps running if you disconnect — reattach anytime. Exit with `Ctrl-D` or type `exit`.

```bash
# Use a different shell
boxrun attach dev --shell /bin/sh
```

## Command Execution

```bash
boxrun exec BOX -- CMD [ARGS...]
boxrun exec BOX --detach -- CMD [ARGS...]
boxrun exec BOX --timeout 30 -- CMD [ARGS...]
```

Execute a command in a box. Output streams to your terminal in real time via SSE.

| Flag | Description |
|------|-------------|
| `--detach`, `-d` | Return the exec ID immediately without waiting |
| `--timeout`, `-t` | Timeout in seconds |

## File Transfer

```bash
boxrun cp LOCAL_PATH BOX:REMOTE_PATH    # upload
boxrun cp BOX:REMOTE_PATH LOCAL_PATH    # download
```

Copy files between the host and a box. Direction is inferred from the `BOX:PATH` syntax.

## Ephemeral Run

```bash
boxrun run IMAGE -- CMD [ARGS...]
boxrun run IMAGE --timeout 30 -- CMD [ARGS...]
```

Create a box, run a command, print the output, and destroy the box — all in one step. Useful for one-off tasks.

## Garbage Collection

```bash
boxrun gc [--older-than 3600]
```

Remove boxes that have been stopped for longer than `--older-than` seconds (default: 3600 = 1 hour).

## Images

```bash
boxrun images
```

List available images with their aliases.

## Volume Mounts

Share host directories into VMs using Docker-style `-v` syntax on `boxrun create`:

```bash
# Read-write mount
boxrun create ubuntu:24.04 --name dev -v /home/user/project:/root/project

# Read-only mount
boxrun create ubuntu:24.04 --name dev -v /home/user/data:/mnt/data:ro

# Multiple mounts
boxrun create ubuntu:24.04 -v /src:/root/src -v /data:/mnt/data:ro
```

Volumes persist across stop/start cycles. Host paths are validated at creation time.

## Web Dashboard

A built-in web UI is available at `http://localhost:9090/ui` whenever the server is running in TCP mode.

Features:
- Real-time view of all boxes with status, image, CPU, and memory
- Start, stop, and delete boxes from the browser
- Expandable rows showing execution history per box
- Auto-refreshes every 3 seconds
