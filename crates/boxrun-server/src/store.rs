use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use rusqlite::{params, Connection};
use tokio::sync::Mutex;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS boxes (
    id TEXT PRIMARY KEY,
    name TEXT UNIQUE,
    status TEXT NOT NULL DEFAULT 'creating',
    image TEXT NOT NULL,
    cpu INTEGER NOT NULL DEFAULT 2,
    memory_mb INTEGER NOT NULL DEFAULT 1024,
    disk_size_gb INTEGER NOT NULL DEFAULT 8,
    network INTEGER NOT NULL DEFAULT 0,
    workdir TEXT NOT NULL DEFAULT '/root',
    env TEXT,
    volumes TEXT,
    boxlite_id TEXT,
    error_code TEXT,
    error_message TEXT,
    created_at TEXT NOT NULL,
    started_at TEXT,
    stopped_at TEXT
);

CREATE TABLE IF NOT EXISTS execs (
    id TEXT PRIMARY KEY,
    box_id TEXT NOT NULL REFERENCES boxes(id),
    status TEXT NOT NULL DEFAULT 'running',
    cmd TEXT NOT NULL,
    env TEXT,
    workdir TEXT,
    timeout_ms INTEGER,
    exit_code INTEGER,
    error_message TEXT,
    created_at TEXT NOT NULL,
    finished_at TEXT
);

CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    exec_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    type TEXT NOT NULL,
    stream TEXT,
    data TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_events_exec_seq ON events(exec_id, seq);
"#;

/// Columns allowed in update_box to prevent accidental misuse.
const BOX_COLUMNS: &[&str] = &[
    "status",
    "image",
    "cpu",
    "memory_mb",
    "disk_size_gb",
    "network",
    "workdir",
    "env",
    "volumes",
    "boxlite_id",
    "error_code",
    "error_message",
    "started_at",
    "stopped_at",
];

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn encode_env(env: &Option<HashMap<String, String>>) -> Option<String> {
    env.as_ref().map(|e| serde_json::to_string(e).unwrap())
}

fn decode_env(raw: Option<String>) -> Option<HashMap<String, String>> {
    raw.and_then(|s| serde_json::from_str(&s).ok())
}

fn encode_volumes(volumes: &Option<Vec<serde_json::Value>>) -> Option<String> {
    volumes.as_ref().map(|v| serde_json::to_string(v).unwrap())
}

fn decode_volumes(raw: Option<String>) -> Option<Vec<serde_json::Value>> {
    raw.and_then(|s| serde_json::from_str(&s).ok())
}

fn decode_cmd(raw: Option<String>) -> Vec<String> {
    raw.and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// A row from the boxes table.
#[derive(Debug, Clone)]
pub struct BoxRow {
    pub id: String,
    pub name: Option<String>,
    pub status: String,
    pub image: String,
    pub cpu: i64,
    pub memory_mb: i64,
    pub disk_size_gb: i64,
    pub network: bool,
    pub workdir: String,
    pub env: Option<HashMap<String, String>>,
    pub volumes: Option<Vec<serde_json::Value>>,
    pub boxlite_id: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub stopped_at: Option<String>,
}

/// A row from the execs table.
#[derive(Debug, Clone)]
pub struct ExecRow {
    pub id: String,
    pub box_id: String,
    pub status: String,
    pub cmd: Vec<String>,
    pub env: Option<HashMap<String, String>>,
    pub workdir: Option<String>,
    pub timeout_ms: Option<i64>,
    pub exit_code: Option<i64>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub finished_at: Option<String>,
}

/// A row from the events table.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct EventRow {
    pub id: i64,
    pub exec_id: String,
    pub seq: i64,
    pub event_type: String,
    pub stream: Option<String>,
    pub data: String,
    pub created_at: String,
}

/// SQLite-backed store for BoxRun state.
#[derive(Clone)]
pub struct Store {
    db: Arc<Mutex<Connection>>,
}

impl Store {
    /// Open (or create) the database at the given path and initialize schema.
    pub async fn new(db_path: &str) -> Result<Self, String> {
        let path = db_path.to_string();
        let conn = tokio::task::spawn_blocking(move || -> Result<Connection, String> {
            let conn = Connection::open(&path).map_err(|e| format!("Failed to open DB: {e}"))?;
            conn.execute_batch("PRAGMA journal_mode=WAL;")
                .map_err(|e| e.to_string())?;
            conn.execute_batch("PRAGMA foreign_keys=ON;")
                .map_err(|e| e.to_string())?;
            conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
            // Migrations
            migrate_add_column(&conn, "boxes", "volumes", "TEXT");
            migrate_add_column(&conn, "boxes", "disk_size_gb", "INTEGER NOT NULL DEFAULT 8");
            Ok(conn)
        })
        .await
        .map_err(|e| e.to_string())??;
        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    // ── Boxes ────────────────────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    pub async fn create_box(
        &self,
        box_id: &str,
        name: Option<&str>,
        image: &str,
        cpu: i64,
        memory_mb: i64,
        disk_size_gb: i64,
        network: bool,
        workdir: &str,
        env: &Option<HashMap<String, String>>,
        volumes: &Option<Vec<serde_json::Value>>,
    ) -> Result<BoxRow, String> {
        let db = self.db.clone();
        let box_id = box_id.to_string();
        let name = name.map(|n| n.to_string());
        let image = image.to_string();
        let workdir = workdir.to_string();
        let env_json = encode_env(env);
        let volumes_json = encode_volumes(volumes);
        let ts = now();

        tokio::task::spawn_blocking(move || {
            let db = db.blocking_lock();
            let result = db.execute(
                "INSERT INTO boxes (id, name, status, image, cpu, memory_mb, disk_size_gb, network, workdir, env, volumes, created_at)
                 VALUES (?1, ?2, 'creating', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![box_id, name, image, cpu, memory_mb, disk_size_gb, network as i64, workdir, env_json, volumes_json, ts],
            );
            match result {
                Ok(_) => {}
                Err(e) => {
                    let msg = e.to_string();
                    if msg.to_uppercase().contains("UNIQUE") {
                        if let Some(ref n) = name {
                            return Err(format!("A box with name '{n}' already exists"));
                        }
                    }
                    return Err(msg);
                }
            }
            get_box_inner(&db, &box_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn update_box(
        &self,
        box_id: &str,
        updates: &[(&str, BoxValue)],
    ) -> Result<BoxRow, String> {
        if updates.is_empty() {
            return self
                .get_box(box_id)
                .await?
                .ok_or_else(|| format!("Box '{box_id}' not found"));
        }

        let db = self.db.clone();
        let box_id = box_id.to_string();
        let updates: Vec<(String, BoxValue)> = updates
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();

        tokio::task::spawn_blocking(move || {
            let db = db.blocking_lock();
            let mut sets = Vec::new();
            let mut vals: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            for (col, val) in &updates {
                if !BOX_COLUMNS.contains(&col.as_str()) {
                    return Err(format!("Invalid column: {col}"));
                }
                sets.push(format!("{col} = ?"));
                match val {
                    BoxValue::Text(s) => vals.push(Box::new(s.clone())),
                    BoxValue::Int(i) => vals.push(Box::new(*i)),
                    BoxValue::Bool(b) => vals.push(Box::new(*b as i64)),
                    BoxValue::Null => vals.push(Box::new(rusqlite::types::Null)),
                    BoxValue::Env(env) => vals.push(Box::new(encode_env(env))),
                    BoxValue::Volumes(vols) => vals.push(Box::new(encode_volumes(vols))),
                }
            }

            vals.push(Box::new(box_id.clone()));
            let set_clause = sets.join(", ");
            let sql = format!("UPDATE boxes SET {set_clause} WHERE id = ?");

            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                vals.iter().map(|v| v.as_ref()).collect();
            db.execute(&sql, param_refs.as_slice())
                .map_err(|e| e.to_string())?;

            get_box_inner(&db, &box_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn get_box(&self, id_or_name: &str) -> Result<Option<BoxRow>, String> {
        let db = self.db.clone();
        let id_or_name = id_or_name.to_string();
        tokio::task::spawn_blocking(move || {
            let db = db.blocking_lock();
            match get_box_by_id(&db, &id_or_name) {
                Ok(Some(row)) => Ok(Some(row)),
                Ok(None) => get_box_by_name(&db, &id_or_name),
                Err(e) => Err(e),
            }
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn list_boxes(&self, status: Option<&str>) -> Result<Vec<BoxRow>, String> {
        let db = self.db.clone();
        let status = status.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let db = db.blocking_lock();

            if let Some(ref s) = status {
                if s == "all" {
                    query_boxes(&db, "SELECT * FROM boxes ORDER BY created_at DESC", &[])
                } else {
                    query_boxes(
                        &db,
                        "SELECT * FROM boxes WHERE status = ?1 ORDER BY created_at DESC",
                        &[s as &dyn rusqlite::types::ToSql],
                    )
                }
            } else {
                query_boxes(&db, "SELECT * FROM boxes ORDER BY created_at DESC", &[])
            }
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn delete_box(&self, box_id: &str) -> Result<(), String> {
        let db = self.db.clone();
        let box_id = box_id.to_string();
        tokio::task::spawn_blocking(move || {
            let db = db.blocking_lock();
            db.execute(
                "DELETE FROM events WHERE exec_id IN (SELECT id FROM execs WHERE box_id = ?1)",
                params![box_id],
            )
            .map_err(|e| e.to_string())?;
            db.execute("DELETE FROM execs WHERE box_id = ?1", params![box_id])
                .map_err(|e| e.to_string())?;
            db.execute("DELETE FROM boxes WHERE id = ?1", params![box_id])
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?
    }

    // ── Execs ────────────────────────────────────────────────────────────

    pub async fn create_exec(
        &self,
        exec_id: &str,
        box_id: &str,
        cmd: &[String],
        env: &Option<HashMap<String, String>>,
        workdir: Option<&str>,
        timeout_ms: Option<i64>,
    ) -> Result<ExecRow, String> {
        let db = self.db.clone();
        let exec_id = exec_id.to_string();
        let box_id = box_id.to_string();
        let cmd_json = serde_json::to_string(cmd).unwrap();
        let env_json = encode_env(env);
        let workdir = workdir.map(|w| w.to_string());
        let ts = now();

        tokio::task::spawn_blocking(move || {
            let db = db.blocking_lock();
            db.execute(
                "INSERT INTO execs (id, box_id, status, cmd, env, workdir, timeout_ms, created_at)
                 VALUES (?1, ?2, 'running', ?3, ?4, ?5, ?6, ?7)",
                params![exec_id, box_id, cmd_json, env_json, workdir, timeout_ms, ts],
            )
            .map_err(|e| e.to_string())?;
            get_exec_inner(&db, &exec_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn get_exec(&self, exec_id: &str) -> Result<Option<ExecRow>, String> {
        let db = self.db.clone();
        let exec_id = exec_id.to_string();
        tokio::task::spawn_blocking(move || {
            let db = db.blocking_lock();
            let mut stmt = db
                .prepare("SELECT * FROM execs WHERE id = ?1")
                .map_err(|e| e.to_string())?;
            let rows: Vec<ExecRow> = stmt
                .query_map(params![exec_id], row_to_exec)
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
            Ok(rows.into_iter().next())
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn list_execs(&self, box_id: &str) -> Result<Vec<ExecRow>, String> {
        let db = self.db.clone();
        let box_id = box_id.to_string();
        tokio::task::spawn_blocking(move || {
            let db = db.blocking_lock();
            let mut stmt = db
                .prepare("SELECT * FROM execs WHERE box_id = ?1 ORDER BY created_at DESC")
                .map_err(|e| e.to_string())?;
            let rows: Vec<ExecRow> = stmt
                .query_map(params![box_id], row_to_exec)
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
            Ok(rows)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn finish_exec(
        &self,
        exec_id: &str,
        exit_code: i64,
        error_message: Option<&str>,
    ) -> Result<ExecRow, String> {
        let db = self.db.clone();
        let exec_id = exec_id.to_string();
        let error_message = error_message.map(|s| s.to_string());
        let ts = now();

        let status = if error_message.is_some() {
            "failed"
        } else if exit_code == 0 {
            "succeeded"
        } else {
            "failed"
        };
        let status = status.to_string();

        tokio::task::spawn_blocking(move || {
            let db = db.blocking_lock();
            db.execute(
                "UPDATE execs SET status = ?1, exit_code = ?2, error_message = ?3, finished_at = ?4 WHERE id = ?5",
                params![status, exit_code, error_message, ts, exec_id],
            )
            .map_err(|e| e.to_string())?;
            get_exec_inner(&db, &exec_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn cancel_exec(&self, exec_id: &str) -> Result<ExecRow, String> {
        let db = self.db.clone();
        let exec_id = exec_id.to_string();
        let ts = now();

        tokio::task::spawn_blocking(move || {
            let db = db.blocking_lock();
            db.execute(
                "UPDATE execs SET status = 'canceled', finished_at = ?1 WHERE id = ?2",
                params![ts, exec_id],
            )
            .map_err(|e| e.to_string())?;
            get_exec_inner(&db, &exec_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    // ── Events ───────────────────────────────────────────────────────────

    pub async fn append_event(
        &self,
        exec_id: &str,
        seq: i64,
        event_type: &str,
        data: &str,
        stream: Option<&str>,
    ) -> Result<(), String> {
        let db = self.db.clone();
        let exec_id = exec_id.to_string();
        let event_type = event_type.to_string();
        let data = data.to_string();
        let stream = stream.map(|s| s.to_string());
        let ts = now();

        tokio::task::spawn_blocking(move || {
            let db = db.blocking_lock();
            db.execute(
                "INSERT INTO events (exec_id, seq, type, stream, data, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![exec_id, seq, event_type, stream, data, ts],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn get_events(&self, exec_id: &str, since_seq: i64) -> Result<Vec<EventRow>, String> {
        let db = self.db.clone();
        let exec_id = exec_id.to_string();
        tokio::task::spawn_blocking(move || {
            let db = db.blocking_lock();
            let mut stmt = db
                .prepare("SELECT * FROM events WHERE exec_id = ?1 AND seq >= ?2 ORDER BY seq")
                .map_err(|e| e.to_string())?;
            let rows: Vec<EventRow> = stmt
                .query_map(params![exec_id, since_seq], |row| {
                    Ok(EventRow {
                        id: row.get(0)?,
                        exec_id: row.get(1)?,
                        seq: row.get(2)?,
                        event_type: row.get(3)?,
                        stream: row.get(4)?,
                        data: row.get(5)?,
                        created_at: row.get(6)?,
                    })
                })
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
            Ok(rows)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    // ── GC ───────────────────────────────────────────────────────────────

    pub async fn gc_stopped_boxes(&self, older_than_seconds: i64) -> Result<Vec<String>, String> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let db = db.blocking_lock();
            let cutoff = Utc::now() - chrono::Duration::seconds(older_than_seconds);
            let cutoff_iso = cutoff.to_rfc3339();
            let mut stmt = db
                .prepare(
                    "SELECT id FROM boxes WHERE
                     (status = 'stopped' AND stopped_at < ?1) OR
                     (status = 'error' AND created_at < ?1)",
                )
                .map_err(|e| e.to_string())?;
            let rows: Vec<String> = stmt
                .query_map(params![cutoff_iso], |row| row.get(0))
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
            Ok(rows)
        })
        .await
        .map_err(|e| e.to_string())?
    }
}

// ── Value type for dynamic updates ───────────────────────────────────────

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum BoxValue {
    Text(String),
    Int(i64),
    Bool(bool),
    Null,
    Env(Option<HashMap<String, String>>),
    Volumes(Option<Vec<serde_json::Value>>),
}

// ── Helper functions (sync, called inside spawn_blocking) ────────────────

fn migrate_add_column(conn: &Connection, table: &str, column: &str, col_type: &str) {
    let sql = format!("ALTER TABLE {table} ADD COLUMN {column} {col_type}");
    if let Err(e) = conn.execute_batch(&sql) {
        let msg = e.to_string().to_lowercase();
        if !msg.contains("duplicate column") {
            tracing::warn!("Migration warning for {table}.{column}: {e}");
        }
    }
}

fn row_to_box(row: &rusqlite::Row<'_>) -> rusqlite::Result<BoxRow> {
    let network_int: i64 = row.get("network")?;
    let env_raw: Option<String> = row.get("env")?;
    let volumes_raw: Option<String> = row.get("volumes")?;
    Ok(BoxRow {
        id: row.get("id")?,
        name: row.get("name")?,
        status: row.get("status")?,
        image: row.get("image")?,
        cpu: row.get("cpu")?,
        memory_mb: row.get("memory_mb")?,
        disk_size_gb: row.get("disk_size_gb")?,
        network: network_int != 0,
        workdir: row.get("workdir")?,
        env: decode_env(env_raw),
        volumes: decode_volumes(volumes_raw),
        boxlite_id: row.get("boxlite_id")?,
        error_code: row.get("error_code")?,
        error_message: row.get("error_message")?,
        created_at: row.get("created_at")?,
        started_at: row.get("started_at")?,
        stopped_at: row.get("stopped_at")?,
    })
}

fn row_to_exec(row: &rusqlite::Row<'_>) -> rusqlite::Result<ExecRow> {
    let cmd_raw: Option<String> = row.get("cmd")?;
    let env_raw: Option<String> = row.get("env")?;
    Ok(ExecRow {
        id: row.get("id")?,
        box_id: row.get("box_id")?,
        status: row.get("status")?,
        cmd: decode_cmd(cmd_raw),
        env: decode_env(env_raw),
        workdir: row.get("workdir")?,
        timeout_ms: row.get("timeout_ms")?,
        exit_code: row.get("exit_code")?,
        error_message: row.get("error_message")?,
        created_at: row.get("created_at")?,
        finished_at: row.get("finished_at")?,
    })
}

fn get_box_inner(db: &Connection, box_id: &str) -> Result<BoxRow, String> {
    let mut stmt = db
        .prepare("SELECT * FROM boxes WHERE id = ?1")
        .map_err(|e| e.to_string())?;
    let rows: Vec<BoxRow> = stmt
        .query_map(params![box_id], row_to_box)
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    rows.into_iter()
        .next()
        .ok_or_else(|| format!("Box '{box_id}' not found"))
}

fn get_box_by_id(db: &Connection, id: &str) -> Result<Option<BoxRow>, String> {
    let mut stmt = db
        .prepare("SELECT * FROM boxes WHERE id = ?1")
        .map_err(|e| e.to_string())?;
    let rows: Vec<BoxRow> = stmt
        .query_map(params![id], row_to_box)
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows.into_iter().next())
}

fn get_box_by_name(db: &Connection, name: &str) -> Result<Option<BoxRow>, String> {
    let mut stmt = db
        .prepare("SELECT * FROM boxes WHERE name = ?1")
        .map_err(|e| e.to_string())?;
    let rows: Vec<BoxRow> = stmt
        .query_map(params![name], row_to_box)
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows.into_iter().next())
}

fn get_exec_inner(db: &Connection, exec_id: &str) -> Result<ExecRow, String> {
    let mut stmt = db
        .prepare("SELECT * FROM execs WHERE id = ?1")
        .map_err(|e| e.to_string())?;
    let rows: Vec<ExecRow> = stmt
        .query_map(params![exec_id], row_to_exec)
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    rows.into_iter()
        .next()
        .ok_or_else(|| format!("Exec '{exec_id}' not found"))
}

fn query_boxes(
    db: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::types::ToSql],
) -> Result<Vec<BoxRow>, String> {
    let mut stmt = db.prepare(sql).map_err(|e| e.to_string())?;
    let rows: Vec<BoxRow> = stmt
        .query_map(params, row_to_box)
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_store() -> Store {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        Store::new(tmp.path().to_str().unwrap()).await.unwrap()
    }

    #[tokio::test]
    async fn test_create_and_get_box() {
        let store = test_store().await;
        let row = store
            .create_box(
                "box_123",
                Some("mybox"),
                "ubuntu:24.04",
                2,
                1024,
                8,
                false,
                "/root",
                &None,
                &None,
            )
            .await
            .unwrap();
        assert_eq!(row.id, "box_123");
        assert_eq!(row.name, Some("mybox".into()));
        assert_eq!(row.status, "creating");
        assert_eq!(row.image, "ubuntu:24.04");

        let fetched = store.get_box("box_123").await.unwrap().unwrap();
        assert_eq!(fetched.id, "box_123");

        // Get by name
        let fetched = store.get_box("mybox").await.unwrap().unwrap();
        assert_eq!(fetched.id, "box_123");
    }

    #[tokio::test]
    async fn test_update_box() {
        let store = test_store().await;
        store
            .create_box(
                "box_1",
                None,
                "ubuntu:24.04",
                2,
                1024,
                8,
                false,
                "/root",
                &None,
                &None,
            )
            .await
            .unwrap();

        let updated = store
            .update_box("box_1", &[("status", BoxValue::Text("running".into()))])
            .await
            .unwrap();
        assert_eq!(updated.status, "running");
    }

    #[tokio::test]
    async fn test_duplicate_name() {
        let store = test_store().await;
        store
            .create_box(
                "box_1",
                Some("dup"),
                "ubuntu:24.04",
                2,
                1024,
                8,
                false,
                "/root",
                &None,
                &None,
            )
            .await
            .unwrap();
        let err = store
            .create_box(
                "box_2",
                Some("dup"),
                "ubuntu:24.04",
                2,
                1024,
                8,
                false,
                "/root",
                &None,
                &None,
            )
            .await;
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("already exists"));
    }

    #[tokio::test]
    async fn test_list_boxes() {
        let store = test_store().await;
        store
            .create_box(
                "box_1",
                None,
                "ubuntu:24.04",
                2,
                1024,
                8,
                false,
                "/root",
                &None,
                &None,
            )
            .await
            .unwrap();
        store
            .create_box(
                "box_2",
                None,
                "ubuntu:24.04",
                2,
                1024,
                8,
                false,
                "/root",
                &None,
                &None,
            )
            .await
            .unwrap();

        let all = store.list_boxes(None).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_delete_box() {
        let store = test_store().await;
        store
            .create_box(
                "box_del",
                None,
                "ubuntu:24.04",
                2,
                1024,
                8,
                false,
                "/root",
                &None,
                &None,
            )
            .await
            .unwrap();
        store.delete_box("box_del").await.unwrap();
        let fetched = store.get_box("box_del").await.unwrap();
        assert!(fetched.is_none());
    }

    #[tokio::test]
    async fn test_create_and_get_exec() {
        let store = test_store().await;
        store
            .create_box(
                "box_e",
                None,
                "ubuntu:24.04",
                2,
                1024,
                8,
                false,
                "/root",
                &None,
                &None,
            )
            .await
            .unwrap();

        let exec = store
            .create_exec(
                "exec_1",
                "box_e",
                &["ls".into(), "-la".into()],
                &None,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(exec.id, "exec_1");
        assert_eq!(exec.status, "running");
        assert_eq!(exec.cmd, vec!["ls", "-la"]);

        let fetched = store.get_exec("exec_1").await.unwrap().unwrap();
        assert_eq!(fetched.box_id, "box_e");
    }

    #[tokio::test]
    async fn test_finish_exec() {
        let store = test_store().await;
        store
            .create_box(
                "box_f",
                None,
                "ubuntu:24.04",
                2,
                1024,
                8,
                false,
                "/root",
                &None,
                &None,
            )
            .await
            .unwrap();
        store
            .create_exec("exec_f", "box_f", &["echo".into()], &None, None, None)
            .await
            .unwrap();

        let finished = store.finish_exec("exec_f", 0, None).await.unwrap();
        assert_eq!(finished.status, "succeeded");
        assert_eq!(finished.exit_code, Some(0));

        // Failed exec
        store
            .create_exec("exec_f2", "box_f", &["false".into()], &None, None, None)
            .await
            .unwrap();
        let failed = store.finish_exec("exec_f2", 1, None).await.unwrap();
        assert_eq!(failed.status, "failed");
    }

    #[tokio::test]
    async fn test_cancel_exec() {
        let store = test_store().await;
        store
            .create_box(
                "box_c",
                None,
                "ubuntu:24.04",
                2,
                1024,
                8,
                false,
                "/root",
                &None,
                &None,
            )
            .await
            .unwrap();
        store
            .create_exec("exec_c", "box_c", &["sleep".into()], &None, None, None)
            .await
            .unwrap();

        let canceled = store.cancel_exec("exec_c").await.unwrap();
        assert_eq!(canceled.status, "canceled");
    }

    #[tokio::test]
    async fn test_events() {
        let store = test_store().await;
        store
            .create_box(
                "box_ev",
                None,
                "ubuntu:24.04",
                2,
                1024,
                8,
                false,
                "/root",
                &None,
                &None,
            )
            .await
            .unwrap();
        store
            .create_exec("exec_ev", "box_ev", &["ls".into()], &None, None, None)
            .await
            .unwrap();

        store
            .append_event("exec_ev", 0, "log", "hello\n", Some("stdout"))
            .await
            .unwrap();
        store
            .append_event("exec_ev", 1, "log", "err\n", Some("stderr"))
            .await
            .unwrap();
        store
            .append_event("exec_ev", 2, "exit", r#"{"exit_code": 0}"#, None)
            .await
            .unwrap();

        let events = store.get_events("exec_ev", 0).await.unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event_type, "log");
        assert_eq!(events[0].stream, Some("stdout".into()));
        assert_eq!(events[2].event_type, "exit");

        // since_seq filter
        let events = store.get_events("exec_ev", 1).await.unwrap();
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn test_env_roundtrip() {
        let store = test_store().await;
        let mut env = HashMap::new();
        env.insert("FOO".into(), "bar".into());
        env.insert("BAZ".into(), "qux".into());

        store
            .create_box(
                "box_env",
                None,
                "ubuntu:24.04",
                2,
                1024,
                8,
                false,
                "/root",
                &Some(env.clone()),
                &None,
            )
            .await
            .unwrap();

        let fetched = store.get_box("box_env").await.unwrap().unwrap();
        assert_eq!(fetched.env, Some(env));
    }

    #[tokio::test]
    async fn test_gc_stopped_boxes() {
        let store = test_store().await;
        store
            .create_box(
                "box_gc",
                None,
                "ubuntu:24.04",
                2,
                1024,
                8,
                false,
                "/root",
                &None,
                &None,
            )
            .await
            .unwrap();
        // Set box as stopped with an old timestamp
        let old_time = "2020-01-01T00:00:00+00:00".to_string();
        store
            .update_box(
                "box_gc",
                &[
                    ("status", BoxValue::Text("stopped".into())),
                    ("stopped_at", BoxValue::Text(old_time)),
                ],
            )
            .await
            .unwrap();

        let ids = store.gc_stopped_boxes(3600).await.unwrap();
        assert_eq!(ids, vec!["box_gc"]);
    }
}
