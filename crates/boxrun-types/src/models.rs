use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::config::{DEFAULT_CPU, DEFAULT_DISK_SIZE_GB, DEFAULT_MEMORY_MB, DEFAULT_WORKDIR};

// ── Volume ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Volume {
    pub host_path: String,
    pub guest_path: String,
    #[serde(default)]
    pub readonly: bool,
}

// ── Box ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateBoxRequest {
    pub image: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default = "default_cpu")]
    pub cpu: i64,
    #[serde(default = "default_memory_mb")]
    pub memory_mb: i64,
    #[serde(default = "default_disk_size_gb")]
    pub disk_size_gb: i64,
    #[serde(default)]
    pub network: bool,
    #[serde(default = "default_workdir")]
    pub workdir: String,
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    #[serde(default)]
    pub volumes: Option<Vec<Volume>>,
}

fn default_cpu() -> i64 {
    DEFAULT_CPU
}
fn default_memory_mb() -> i64 {
    DEFAULT_MEMORY_MB
}
fn default_disk_size_gb() -> i64 {
    DEFAULT_DISK_SIZE_GB
}
fn default_workdir() -> String {
    DEFAULT_WORKDIR.to_string()
}

impl CreateBoxRequest {
    /// Validate the request, returning an error message if invalid.
    pub fn validate(&self) -> Result<(), String> {
        if self.cpu <= 0 {
            return Err("cpu must be greater than 0".into());
        }
        if self.memory_mb <= 0 {
            return Err("memory_mb must be greater than 0".into());
        }
        if self.disk_size_gb <= 0 {
            return Err("disk_size_gb must be greater than 0".into());
        }
        if self.workdir.is_empty() || !self.workdir.starts_with('/') {
            return Err("workdir must be an absolute path (starting with /)".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxResponse {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub status: String,
    pub image: String,
    pub cpu: i64,
    pub memory_mb: i64,
    pub disk_size_gb: i64,
    pub network: bool,
    pub workdir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volumes: Option<Vec<Volume>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boxlite_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped_at: Option<String>,
}

// ── Exec ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecRequest {
    pub cmd: Vec<String>,
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    #[serde(default)]
    pub workdir: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<i64>,
}

impl ExecRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.cmd.is_empty() {
            return Err("cmd must be a non-empty list".into());
        }
        if let Some(ref w) = self.workdir {
            if w.is_empty() || !w.starts_with('/') {
                return Err("workdir must be an absolute path (starting with /)".into());
            }
        }
        if let Some(t) = self.timeout_ms {
            if t <= 0 {
                return Err("timeout_ms must be positive".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResponse {
    pub id: String,
    pub box_id: String,
    pub status: String,
    pub cmd: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
}

// ── Files ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRequest {
    pub path: String,
}

// ── Run (ephemeral) ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRequest {
    pub image: String,
    pub cmd: Vec<String>,
    #[serde(default = "default_cpu")]
    pub cpu: i64,
    #[serde(default = "default_memory_mb")]
    pub memory_mb: i64,
    #[serde(default = "default_disk_size_gb")]
    pub disk_size_gb: i64,
    #[serde(default)]
    pub network: bool,
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    #[serde(default)]
    pub volumes: Option<Vec<Volume>>,
    #[serde(default)]
    pub timeout_ms: Option<i64>,
}

impl RunRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.cmd.is_empty() {
            return Err("cmd must be a non-empty list".into());
        }
        if let Some(t) = self.timeout_ms {
            if t <= 0 {
                return Err("timeout_ms must be positive".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResponse {
    #[serde(default)]
    pub exit_code: Option<i64>,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

// ── GC ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GCRequest {
    #[serde(default = "default_gc_older_than")]
    pub older_than: i64,
}

fn default_gc_older_than() -> i64 {
    3600
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GCResponse {
    pub removed: i64,
}

// ── Error ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
}

// ── Server Info ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub max_cpu: i64,
    pub max_memory_mb: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_box_request_defaults() {
        let req: CreateBoxRequest = serde_json::from_str(r#"{"image": "ubuntu:24.04"}"#).unwrap();
        assert_eq!(req.image, "ubuntu:24.04");
        assert_eq!(req.cpu, 2);
        assert_eq!(req.memory_mb, 1024);
        assert_eq!(req.disk_size_gb, 8);
        assert!(!req.network);
        assert_eq!(req.workdir, "/root");
        assert!(req.name.is_none());
        assert!(req.env.is_none());
        assert!(req.volumes.is_none());
    }

    #[test]
    fn test_create_box_request_validation() {
        let req = CreateBoxRequest {
            image: "test".into(),
            name: None,
            cpu: 0,
            memory_mb: 1024,
            disk_size_gb: 8,
            network: false,
            workdir: "/root".into(),
            env: None,
            volumes: None,
        };
        assert!(req.validate().is_err());

        let req = CreateBoxRequest {
            image: "test".into(),
            name: None,
            cpu: 2,
            memory_mb: 1024,
            disk_size_gb: 8,
            network: false,
            workdir: "relative".into(),
            env: None,
            volumes: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_exec_request_validation() {
        let req = ExecRequest {
            cmd: vec![],
            env: None,
            workdir: None,
            timeout_ms: None,
        };
        assert!(req.validate().is_err());

        let req = ExecRequest {
            cmd: vec!["ls".into()],
            env: None,
            workdir: Some("relative".into()),
            timeout_ms: None,
        };
        assert!(req.validate().is_err());

        let req = ExecRequest {
            cmd: vec!["ls".into()],
            env: None,
            workdir: None,
            timeout_ms: Some(-1),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_box_response_roundtrip() {
        let resp = BoxResponse {
            id: "box_abc123".into(),
            name: Some("mybox".into()),
            status: "running".into(),
            image: "ubuntu:24.04".into(),
            cpu: 2,
            memory_mb: 1024,
            disk_size_gb: 8,
            network: false,
            workdir: "/root".into(),
            env: None,
            volumes: None,
            boxlite_id: None,
            error_code: None,
            error_message: None,
            created_at: "2024-01-01T00:00:00+00:00".into(),
            started_at: None,
            stopped_at: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: BoxResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "box_abc123");
        assert_eq!(parsed.status, "running");
    }

    #[test]
    fn test_volume_serde() {
        let vol = Volume {
            host_path: "/tmp/data".into(),
            guest_path: "/data".into(),
            readonly: true,
        };
        let json = serde_json::to_string(&vol).unwrap();
        let parsed: Volume = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.host_path, "/tmp/data");
        assert!(parsed.readonly);
    }

    #[test]
    fn test_gc_request_default() {
        let req: GCRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(req.older_than, 3600);
    }

    #[test]
    fn test_error_response_serde() {
        let resp = ErrorResponse {
            code: "BOX_NOT_FOUND".into(),
            message: "Box not found".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("BOX_NOT_FOUND"));
    }
}
