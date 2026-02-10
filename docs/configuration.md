# Configuration

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `BOXRUN_STATE_DIR` | `~/.boxrun` | State directory (socket, database) |
| `BOXRUN_SOCKET` | `~/.boxrun/boxrun.sock` | Unix socket path |
| `BOXRUN_DB` | `~/.boxrun/boxrun.db` | SQLite database path |
| `BOXRUN_HOST` | `127.0.0.1` | Server bind host (TCP mode) |
| `BOXRUN_PORT` | `9090` | Server bind port (TCP mode) |
| `BOXRUN_MAX_CPU` | auto-detected | Total CPU limit override |
| `BOXRUN_MAX_MEMORY_MB` | auto-detected | Total memory limit override (MB) |

## Resource Limits

Total CPU and memory limits are **auto-detected from host hardware**. There is no hard limit on box count — you can run as many boxes as your CPU and memory allow.

| Resource | Default |
|----------|---------|
| Total CPU | auto-detected (host CPU count) |
| Total memory | auto-detected (host RAM) |
| Per-box CPU | 2 cores |
| Per-box memory | 512 MB |
| Per-box disk | 8 GB |
| Per-box workdir | `/root` |

### Overriding Limits

Override total resource limits via environment variables:

```bash
export BOXRUN_MAX_CPU=8
export BOXRUN_MAX_MEMORY_MB=8192
```

Configure per-box resources at creation time:

```bash
boxrun create ubuntu:24.04 --name beefy --cpu 4 --memory 2048
```

Or via the SDK:

```python
box = await client.create("ubuntu:24.04", cpu=4, memory_mb=2048)
```

### Checking Current Limits

Query the info endpoint:

```bash
curl http://localhost:9090/v1/info
# {"max_cpu": 8, "max_memory_mb": 16384}
```
