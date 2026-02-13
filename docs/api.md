# REST API Reference

Base URL: `http://boxrun` (Unix socket) or `http://HOST:PORT` (TCP)

All endpoints return JSON unless otherwise noted. Error responses use the format:

```json
{"code": "ERROR_CODE", "message": "Human-readable message"}
```

## Boxes

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/boxes` | Create a box |
| `GET` | `/v1/boxes` | List boxes |
| `GET` | `/v1/boxes/{id}` | Get box by ID or name |
| `POST` | `/v1/boxes/{id}:stop` | Stop (preserves disk) |
| `POST` | `/v1/boxes/{id}:start` | Restart a stopped box |
| `DELETE` | `/v1/boxes/{id}` | Remove a box |

### POST /v1/boxes

Create a new box.

**Request body:**

```json
{
  "image": "ubuntu:24.04",
  "name": "dev",
  "cpu": 2,
  "memory_mb": 1024,
  "disk_size_gb": 8,
  "network": false,
  "workdir": "/root",
  "env": {"KEY": "value"},
  "volumes": [
    {"host_path": "/src", "guest_path": "/root/src", "readonly": false}
  ]
}
```

All fields except `image` are optional with the defaults shown above.

**Response** (201):

```json
{
  "id": "box_abc123",
  "name": "dev",
  "status": "running",
  "image": "ubuntu:24.04",
  "cpu": 2,
  "memory_mb": 1024,
  "disk_size_gb": 8,
  "network": false,
  "workdir": "/root",
  "env": null,
  "volumes": null,
  "created_at": "2025-01-01T00:00:00Z",
  "started_at": "2025-01-01T00:00:00Z",
  "stopped_at": null
}
```

### GET /v1/boxes

List all boxes. Optional query parameter: `?status=running` or `?status=stopped`.

### DELETE /v1/boxes/{id}

Remove a box. Optional query parameter: `?force=true` to remove a running box without stopping first.

## Exec

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/boxes/{id}/exec` | Start a command |
| `GET` | `/v1/boxes/{id}/execs` | List executions for a box |
| `GET` | `/v1/boxes/{id}/exec/{eid}` | Get exec status |
| `GET` | `/v1/boxes/{id}/exec/{eid}/events` | SSE output stream |
| `POST` | `/v1/boxes/{id}/exec/{eid}:cancel` | Cancel execution |

### POST /v1/boxes/{id}/exec

Start a command in a box.

**Request body:**

```json
{
  "cmd": ["echo", "hello"],
  "env": {"KEY": "value"},
  "workdir": "/root/project",
  "timeout_ms": 30000
}
```

Only `cmd` is required.

**Response** (201):

```json
{
  "id": "exec_xyz789",
  "box_id": "box_abc123",
  "status": "running",
  "cmd": ["echo", "hello"],
  "exit_code": null,
  "created_at": "2025-01-01T00:00:00Z",
  "finished_at": null
}
```

### GET /v1/boxes/{id}/exec/{eid}/events — SSE Stream

Stream execution output in real time using Server-Sent Events.

**Event types:**

`log` — output from the command:
```
event: log
data: {"stream": "stdout", "data": "hello\n", "seq": 1}
```

`exit` — command finished:
```
event: exit
data: {"stream": null, "data": "{\"exit_code\": 0}", "seq": 5}
```

The stream replays past events first, then streams live events, and closes after the `exit` event.

## Files

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/boxes/{id}/files/upload` | Upload a file |
| `POST` | `/v1/boxes/{id}/files/download` | Download a file |

### POST /v1/boxes/{id}/files/upload

Upload a file to the box. Uses multipart form data.

**Form fields:**
- `file` — the file content (multipart file)
- `dest` — destination path inside the box (string)

**Response:**

```json
{"ok": true, "dest": "/root/data.csv"}
```

### POST /v1/boxes/{id}/files/download

Download a file from the box.

**Request body:**

```json
{"path": "/root/results.csv"}
```

**Response:** raw file bytes with `Content-Type: application/octet-stream`.

## Interactive Terminal (WebSocket)

```
WS /v1/boxes/{id}/attach?shell=/bin/bash&cols=80&rows=24&term=xterm-256color
```

Opens an interactive terminal session inside the box.

**Query parameters:**

| Parameter | Default | Description |
|-----------|---------|-------------|
| `shell` | `/bin/bash` | Shell to run |
| `cols` | `80` | Terminal columns |
| `rows` | `24` | Terminal rows |
| `term` | `xterm-256color` | TERM environment variable |

**Protocol:**
- Binary frames = raw terminal I/O (stdin/stdout)
- Text frames = JSON control messages

**Client → Server:**
```json
{"type": "resize", "cols": 120, "rows": 40}
```

**Server → Client:**
```json
{"type": "exit", "code": 0}
```

## Convenience

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/info` | Server info |
| `POST` | `/v1/run` | Ephemeral run |
| `POST` | `/v1/gc` | Garbage collect |

### GET /v1/info

Returns current server resource limits.

```json
{"max_cpu": 8, "max_memory_mb": 16384}
```

### POST /v1/run

Ephemeral one-shot: create a box, run a command, return output, destroy the box.

**Request body:**

```json
{
  "image": "ubuntu:24.04",
  "cmd": ["echo", "hello"],
  "cpu": 2,
  "memory_mb": 1024,
  "network": false,
  "env": null,
  "volumes": null,
  "timeout_ms": null
}
```

Only `image` and `cmd` are required.

**Response:**

```json
{
  "exit_code": 0,
  "stdout": "hello\n",
  "stderr": "",
  "error_message": null
}
```

### POST /v1/gc

Garbage collect stopped boxes.

**Request body:**

```json
{"older_than": 3600}
```

**Response:**

```json
{"removed": 2}
```
