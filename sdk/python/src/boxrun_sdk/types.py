"""BoxRun data types."""

from __future__ import annotations

from dataclasses import dataclass, field


@dataclass
class BoxInfo:
    id: str
    name: str | None
    status: str
    image: str
    cpu: int
    memory_mb: int
    disk_size_gb: int
    network: bool
    workdir: str
    env: dict[str, str] | None = None
    volumes: list[dict] | None = None
    boxlite_id: str | None = None
    error_code: str | None = None
    error_message: str | None = None
    created_at: str = ""
    started_at: str | None = None
    stopped_at: str | None = None


@dataclass
class ExecInfo:
    id: str
    box_id: str
    status: str
    cmd: list[str]
    env: dict[str, str] | None = None
    workdir: str | None = None
    timeout_ms: int | None = None
    exit_code: int | None = None
    error_message: str | None = None
    created_at: str = ""
    finished_at: str | None = None


@dataclass
class ExecEvent:
    type: str
    data: str
    stream: str | None = None
    seq: int = 0


@dataclass
class RunResult:
    exit_code: int | None
    stdout: str = ""
    stderr: str = ""
    error_message: str | None = None
