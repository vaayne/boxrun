# Architecture

BoxRun is intentionally minimal. The entire platform is one process, one socket, and one database.

## State Directory

```
~/.boxrun/
├── boxrun.sock  # Unix socket (server ↔ clients)
└── boxrun.db    # SQLite (boxes, execs, event log)
```

No etcd, no Redis, no container runtime, no image registry.

## System Overview

```
┌─────────────────────────────────────────┐
│            boxrun serve                 │
│                                         │
│  axum + tokio (Unix socket / TCP)       │
│  ├── BoxManager ── microVM management   │
│  ├── EventBus ──── SSE pub/sub          │
│  ├── Store ─────── SQLite (rusqlite)    │
│  └── Web UI ────── /ui dashboard        │
└─────────────────────────────────────────┘
         ↕ Unix socket / HTTP
┌──────────────┐  ┌──────────────┐  ┌──────────────┐
│  boxrun CLI  │  │  Python SDK  │  │   Browser    │
│  (clap)      │  │  (httpx)     │  │  (Web UI)    │
└──────────────┘  └──────────────┘  └──────────────┘
```

## Components

### Server (`crates/boxrun-server/src/`)

axum application served by tokio. Listens on either a Unix domain socket (for CLI/SDK) or TCP (for browser access). Routes are defined in `routes.rs`, request/response models in `boxrun-types`.

### BoxManager (`crates/boxrun-server/src/box_manager.rs`)

Core orchestration layer. Wraps the BoxLite Rust SDK to create, stop, start, and remove microVMs. Tracks resource usage (CPU, memory) and manages the full lifecycle of boxes and executions. Each exec runs as a background task with its output piped to the EventBus.

### Store (`crates/boxrun-server/src/store.rs`)

SQLite persistence via rusqlite (with `spawn_blocking`). Stores box metadata, exec records, and output events. Uses WAL journal mode and foreign keys for consistency.

### EventBus (`crates/boxrun-server/src/events.rs`)

In-memory pub/sub system for streaming exec output. Each execution gets a channel (tokio mpsc). The SSE endpoint subscribes to a channel and replays past events from the Store before streaming live events.

### CLI (`crates/boxrun-server/src/cli.rs`)

Command-line interface built with clap. Uses reqwest to communicate with the server over Unix socket or TCP. The `attach` command uses tokio-tungstenite for interactive terminal sessions with raw terminal mode.

### SDK (`crates/boxrun-sdk/`)

Async Rust client built with reqwest. Provides `BoxRunClient` (connection management, box creation, ephemeral runs) and `BoxHandle` (per-box operations like exec, file transfer, lifecycle).

### Web UI (`crates/boxrun-server/src/ui.rs`)

Single-page dashboard served at `/ui`. Shows all boxes with real-time status, supports start/stop/delete from the browser, and displays execution history.

## Box Lifecycle

```
creating ──→ running ←──→ stopped
                │              │
                └──→ error     └──→ (rm)
                       │
                       └──→ (rm)
```

- **creating** → Box is being provisioned by BoxLite
- **running** → Box is up and accepting commands
- **stopped** → Box is paused, disk state preserved
- **error** → Box creation or runtime failure

Disk state persists across stop/start cycles. Only `rm` (remove) destroys data.
