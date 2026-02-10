use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

// ── Resource defaults ────────────────────────────────────────────────────

pub const DEFAULT_CPU: i64 = 2;
pub const DEFAULT_MEMORY_MB: i64 = 1024;
pub const DEFAULT_DISK_SIZE_GB: i64 = 8;
pub const DEFAULT_WORKDIR: &str = "/root";

// ── Paths ────────────────────────────────────────────────────────────────

pub fn state_dir() -> PathBuf {
    let dir = if let Ok(d) = env::var("BOXRUN_STATE_DIR") {
        PathBuf::from(d)
    } else {
        dirs_home().join(".boxrun")
    };
    std::fs::create_dir_all(&dir).ok();
    dir
}

pub fn socket_path() -> String {
    env::var("BOXRUN_SOCKET").unwrap_or_else(|_| {
        state_dir()
            .join("boxrun.sock")
            .to_string_lossy()
            .into_owned()
    })
}

pub fn db_path() -> String {
    env::var("BOXRUN_DB")
        .unwrap_or_else(|_| state_dir().join("boxrun.db").to_string_lossy().into_owned())
}

pub fn files_dir() -> PathBuf {
    let dir = state_dir().join("files");
    std::fs::create_dir_all(&dir).ok();
    dir
}

pub fn server_url() -> String {
    let sock = socket_path();
    if Path::new(&sock).exists() {
        return format!("http+unix://{sock}");
    }
    let host = env::var("BOXRUN_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = env::var("BOXRUN_PORT").unwrap_or_else(|_| "9090".into());
    format!("http://{host}:{port}")
}

fn dirs_home() -> PathBuf {
    env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/root"))
}

// ── Resource limits ──────────────────────────────────────────────────────

fn detect_cpu() -> i64 {
    num_cpus::get() as i64
}

fn detect_memory_mb() -> i64 {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        if let Ok(output) = Command::new("sysctl").args(["-n", "hw.memsize"]).output() {
            if let Ok(s) = String::from_utf8(output.stdout) {
                if let Ok(bytes) = s.trim().parse::<u64>() {
                    return (bytes / (1024 * 1024)) as i64;
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        use std::fs;
        if let Ok(content) = fs::read_to_string("/proc/meminfo") {
            for line in content.lines() {
                if line.starts_with("MemTotal:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        if let Ok(kb) = parts[1].parse::<u64>() {
                            return (kb / 1024) as i64;
                        }
                    }
                }
            }
        }
    }

    // Fallback: 8 GB
    8192
}

fn parse_int_env(name: &str, default: i64) -> i64 {
    match env::var(name) {
        Ok(raw) => {
            match raw.parse::<i64>() {
                Ok(v) if v > 0 => v,
                Ok(v) => {
                    eprintln!("Warning: {name} must be positive (got {v}), using detected value {default}");
                    default
                }
                Err(_) => {
                    eprintln!("Warning: Invalid {name}={raw:?}, using detected value {default}");
                    default
                }
            }
        }
        Err(_) => default,
    }
}

pub static MAX_TOTAL_CPU: LazyLock<i64> =
    LazyLock::new(|| parse_int_env("BOXRUN_MAX_CPU", detect_cpu()));

pub static MAX_TOTAL_MEMORY_MB: LazyLock<i64> =
    LazyLock::new(|| parse_int_env("BOXRUN_MAX_MEMORY_MB", detect_memory_mb()));

// ── Image catalog ────────────────────────────────────────────────────────

/// Image catalog entry: (full_image_name, description).
pub static IMAGE_CATALOG: LazyLock<HashMap<&'static str, (&'static str, &'static str)>> =
    LazyLock::new(|| {
        let mut m = HashMap::new();
        m.insert(
            "default",
            (
                "ubuntu:24.04",
                "Ubuntu 24.04 (general purpose, recommended)",
            ),
        );
        m.insert("ubuntu", ("ubuntu:24.04", "Ubuntu 24.04 LTS"));
        m.insert("python", ("python:3.12-slim", "Python 3.12 pre-installed"));
        m.insert("node", ("node:22-slim", "Node.js 22 LTS pre-installed"));
        m.insert("golang", ("golang:1.22", "Go 1.22 pre-installed"));
        m.insert("rust", ("rust:1.77-slim", "Rust 1.77 pre-installed"));
        m.insert("alpine", ("alpine:3.20", "Lightweight Linux (5MB)"));
        m
    });

pub const DEFAULT_IMAGE: &str = "ubuntu:24.04";

/// Resolve an alias to a full image name, or return as-is.
pub fn resolve_image(name: &str) -> String {
    IMAGE_CATALOG
        .get(name)
        .map(|(img, _)| img.to_string())
        .unwrap_or_else(|| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_image_alias() {
        assert_eq!(resolve_image("ubuntu"), "ubuntu:24.04");
        assert_eq!(resolve_image("python"), "python:3.12-slim");
        assert_eq!(resolve_image("node"), "node:22-slim");
        assert_eq!(resolve_image("default"), "ubuntu:24.04");
    }

    #[test]
    fn test_resolve_image_passthrough() {
        assert_eq!(resolve_image("custom:latest"), "custom:latest");
        assert_eq!(resolve_image("nginx:1.25"), "nginx:1.25");
    }

    #[test]
    fn test_image_catalog_complete() {
        let catalog = &*IMAGE_CATALOG;
        assert_eq!(catalog.len(), 7);
        assert!(catalog.contains_key("default"));
        assert!(catalog.contains_key("ubuntu"));
        assert!(catalog.contains_key("python"));
        assert!(catalog.contains_key("node"));
        assert!(catalog.contains_key("golang"));
        assert!(catalog.contains_key("rust"));
        assert!(catalog.contains_key("alpine"));
    }

    #[test]
    fn test_constants() {
        assert_eq!(DEFAULT_CPU, 2);
        assert_eq!(DEFAULT_MEMORY_MB, 1024);
        assert_eq!(DEFAULT_DISK_SIZE_GB, 8);
        assert_eq!(DEFAULT_WORKDIR, "/root");
        assert_eq!(DEFAULT_IMAGE, "ubuntu:24.04");
    }

    #[test]
    fn test_resource_limits_positive() {
        assert!(*MAX_TOTAL_CPU > 0);
        assert!(*MAX_TOTAL_MEMORY_MB > 0);
    }
}
