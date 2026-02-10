"""BoxRun async client and box handle."""

from __future__ import annotations

import asyncio
import json
import os
from pathlib import Path
from typing import Any, AsyncIterator

import httpx
from httpx_sse import aconnect_sse

from .errors import BoxRunError
from .types import BoxInfo, ExecEvent, ExecInfo, RunResult

_SOCKET_PATH = os.path.expanduser("~/.boxrun/boxrun.sock")
_POLL_INTERVAL = 0.5  # seconds


def _server_url() -> str:
    """Auto-detect the BoxRun server URL."""
    if os.path.exists(_SOCKET_PATH):
        return "http+unix://" + _SOCKET_PATH
    host = os.environ.get("BOXRUN_HOST", "127.0.0.1")
    port = os.environ.get("BOXRUN_PORT", "9090")
    return f"http://{host}:{port}"


def _make_client(base_url: str) -> httpx.AsyncClient:
    if base_url.startswith("http+unix://"):
        socket_path = base_url.removeprefix("http+unix://")
        transport = httpx.AsyncHTTPTransport(uds=socket_path)
        return httpx.AsyncClient(
            transport=transport,
            base_url="http://localhost",
        )
    return httpx.AsyncClient(base_url=base_url)


def _parse_box_info(data: dict[str, Any]) -> BoxInfo:
    return BoxInfo(
        id=data["id"],
        name=data.get("name"),
        status=data["status"],
        image=data["image"],
        cpu=data["cpu"],
        memory_mb=data["memory_mb"],
        disk_size_gb=data["disk_size_gb"],
        network=data.get("network", False),
        workdir=data.get("workdir", "/root"),
        env=data.get("env"),
        volumes=data.get("volumes"),
        boxlite_id=data.get("boxlite_id"),
        error_code=data.get("error_code"),
        error_message=data.get("error_message"),
        created_at=data.get("created_at", ""),
        started_at=data.get("started_at"),
        stopped_at=data.get("stopped_at"),
    )


def _parse_exec_info(data: dict[str, Any]) -> ExecInfo:
    return ExecInfo(
        id=data["id"],
        box_id=data["box_id"],
        status=data["status"],
        cmd=data["cmd"],
        env=data.get("env"),
        workdir=data.get("workdir"),
        timeout_ms=data.get("timeout_ms"),
        exit_code=data.get("exit_code"),
        error_message=data.get("error_message"),
        created_at=data.get("created_at", ""),
        finished_at=data.get("finished_at"),
    )


def _check_error(resp: httpx.Response) -> None:
    """Raise BoxRunError for non-2xx responses."""
    if resp.status_code < 400:
        return
    try:
        body = resp.json()
        code = body.get("code", "RUNTIME_ERROR")
        message = body.get("message", resp.text)
    except Exception:
        code = "RUNTIME_ERROR"
        message = resp.text
    raise BoxRunError(code, message)


class BoxRunClient:
    """Async client for the BoxRun REST API.

    Usage::

        async with BoxRunClient() as client:
            box = await client.create("ubuntu:24.04")
            ...
    """

    def __init__(self, base_url: str | None = None) -> None:
        self._base_url = base_url or _server_url()
        self._http = _make_client(self._base_url)

    async def __aenter__(self) -> BoxRunClient:
        return self

    async def __aexit__(self, *exc: Any) -> None:
        await self._http.aclose()

    async def close(self) -> None:
        await self._http.aclose()

    # ------------------------------------------------------------------
    # Box operations
    # ------------------------------------------------------------------

    async def create(
        self,
        image: str,
        *,
        name: str | None = None,
        cpu: int = 2,
        memory_mb: int = 512,
        disk_size_gb: int = 8,
        network: bool = False,
        workdir: str = "/root",
        env: dict[str, str] | None = None,
        volumes: list[dict] | None = None,
    ) -> BoxHandle:
        """Create a new box and return a handle to it."""
        body: dict[str, Any] = {
            "image": image,
            "cpu": cpu,
            "memory_mb": memory_mb,
            "disk_size_gb": disk_size_gb,
            "network": network,
            "workdir": workdir,
        }
        if name is not None:
            body["name"] = name
        if env is not None:
            body["env"] = env
        if volumes is not None:
            body["volumes"] = volumes
        resp = await self._http.post("/v1/boxes", json=body, timeout=60)
        _check_error(resp)
        info = _parse_box_info(resp.json())
        return BoxHandle(self, info)

    async def get_box(self, id_or_name: str) -> BoxInfo:
        """Get info about a box by ID or name."""
        resp = await self._http.get(f"/v1/boxes/{id_or_name}", timeout=10)
        _check_error(resp)
        return _parse_box_info(resp.json())

    async def list_boxes(self, status: str | None = None) -> list[BoxInfo]:
        """List all boxes, optionally filtered by status."""
        params: dict[str, str] = {}
        if status is not None:
            params["status"] = status
        resp = await self._http.get("/v1/boxes", params=params, timeout=10)
        _check_error(resp)
        return [_parse_box_info(b) for b in resp.json()]

    async def stop_box(self, box_id: str) -> BoxInfo:
        """Stop a running box."""
        resp = await self._http.post(f"/v1/boxes/{box_id}:stop", timeout=30)
        _check_error(resp)
        return _parse_box_info(resp.json())

    async def start_box(self, box_id: str) -> BoxInfo:
        """Start a stopped box."""
        resp = await self._http.post(f"/v1/boxes/{box_id}:start", timeout=30)
        _check_error(resp)
        return _parse_box_info(resp.json())

    async def remove_box(self, box_id: str, force: bool = False) -> None:
        """Remove a box permanently."""
        params: dict[str, str] = {}
        if force:
            params["force"] = "true"
        resp = await self._http.delete(
            f"/v1/boxes/{box_id}", params=params, timeout=30
        )
        _check_error(resp)

    # ------------------------------------------------------------------
    # Exec operations
    # ------------------------------------------------------------------

    async def start_exec(
        self,
        box_id: str,
        cmd: list[str],
        *,
        env: dict[str, str] | None = None,
        workdir: str | None = None,
        timeout_ms: int | None = None,
    ) -> ExecInfo:
        """Start an exec in a box (returns immediately)."""
        body: dict[str, Any] = {"cmd": cmd}
        if env is not None:
            body["env"] = env
        if workdir is not None:
            body["workdir"] = workdir
        if timeout_ms is not None:
            body["timeout_ms"] = timeout_ms
        resp = await self._http.post(
            f"/v1/boxes/{box_id}/exec", json=body, timeout=30
        )
        _check_error(resp)
        return _parse_exec_info(resp.json())

    async def get_exec(self, box_id: str, exec_id: str) -> ExecInfo:
        """Get info about an exec."""
        resp = await self._http.get(
            f"/v1/boxes/{box_id}/exec/{exec_id}", timeout=10
        )
        _check_error(resp)
        return _parse_exec_info(resp.json())

    async def exec_stream(
        self,
        box_id: str,
        exec_id: str,
    ) -> AsyncIterator[ExecEvent]:
        """Stream SSE events for an exec."""
        async with aconnect_sse(
            self._http,
            "GET",
            f"/v1/boxes/{box_id}/exec/{exec_id}/events",
            timeout=httpx.Timeout(None),
        ) as sse:
            async for event in sse.aiter_sse():
                data = json.loads(event.data)
                yield ExecEvent(
                    type=data.get("event_type", event.event),
                    data=data.get("data", ""),
                    stream=data.get("stream"),
                    seq=data.get("seq", 0),
                )

    # ------------------------------------------------------------------
    # File operations
    # ------------------------------------------------------------------

    async def upload_file(
        self, box_id: str, local_path: str, dest_path: str
    ) -> None:
        """Upload a file from the host to a box."""
        file_bytes = Path(local_path).read_bytes()
        resp = await self._http.post(
            f"/v1/boxes/{box_id}/files/upload",
            files={"file": (Path(local_path).name, file_bytes)},
            data={"path": dest_path},
            timeout=60,
        )
        _check_error(resp)

    async def download_file(
        self, box_id: str, remote_path: str, local_path: str
    ) -> None:
        """Download a file from a box to the host."""
        resp = await self._http.post(
            f"/v1/boxes/{box_id}/files/download",
            json={"path": remote_path},
            timeout=60,
        )
        _check_error(resp)
        Path(local_path).write_bytes(resp.content)

    # ------------------------------------------------------------------
    # Convenience
    # ------------------------------------------------------------------

    async def run(
        self,
        image: str,
        cmd: list[str],
        *,
        env: dict[str, str] | None = None,
        timeout_ms: int | None = None,
        disk_size_gb: int = 8,
        volumes: list[dict] | None = None,
    ) -> RunResult:
        """Ephemeral one-shot: create a box, run a command, return result, destroy."""
        body: dict[str, Any] = {"image": image, "cmd": cmd, "disk_size_gb": disk_size_gb}
        if env is not None:
            body["env"] = env
        if timeout_ms is not None:
            body["timeout_ms"] = timeout_ms
        if volumes is not None:
            body["volumes"] = volumes
        resp = await self._http.post("/v1/run", json=body, timeout=300)
        _check_error(resp)
        data = resp.json()
        return RunResult(
            exit_code=data.get("exit_code"),
            stdout=data.get("stdout", ""),
            stderr=data.get("stderr", ""),
            error_message=data.get("error_message"),
        )

    async def gc(self, older_than: int = 3600) -> int:
        """Garbage-collect stopped boxes. Returns number removed."""
        resp = await self._http.post(
            "/v1/gc", json={"older_than": older_than}, timeout=30
        )
        _check_error(resp)
        return resp.json().get("removed", 0)


class BoxHandle:
    """Handle to a specific box, returned by :meth:`BoxRunClient.create`."""

    def __init__(self, client: BoxRunClient, info: BoxInfo) -> None:
        self._client = client
        self.info = info

    @property
    def id(self) -> str:
        return self.info.id

    @property
    def name(self) -> str | None:
        return self.info.name

    async def refresh(self) -> BoxInfo:
        """Re-fetch box info from the server."""
        self.info = await self._client.get_box(self.id)
        return self.info

    async def exec(
        self,
        cmd: list[str],
        *,
        env: dict[str, str] | None = None,
        timeout_ms: int | None = None,
    ) -> ExecInfo:
        """Execute a command and wait for completion."""
        ei = await self._client.start_exec(
            self.id, cmd, env=env, timeout_ms=timeout_ms
        )
        deadline = (timeout_ms / 1000 + 30) if timeout_ms else 600
        elapsed = 0.0
        while ei.status == "running" and elapsed < deadline:
            await asyncio.sleep(_POLL_INTERVAL)
            elapsed += _POLL_INTERVAL
            ei = await self._client.get_exec(self.id, ei.id)
        return ei

    async def exec_stream(
        self,
        cmd: list[str],
        *,
        env: dict[str, str] | None = None,
        timeout_ms: int | None = None,
    ) -> AsyncIterator[ExecEvent]:
        """Execute a command and stream output events."""
        ei = await self._client.start_exec(
            self.id, cmd, env=env, timeout_ms=timeout_ms
        )
        async for event in self._client.exec_stream(self.id, ei.id):
            yield event

    async def upload(self, local_path: str, dest_path: str) -> None:
        """Upload a file from the host to this box."""
        await self._client.upload_file(self.id, local_path, dest_path)

    async def download(self, remote_path: str, local_path: str) -> None:
        """Download a file from this box to the host."""
        await self._client.download_file(self.id, remote_path, local_path)

    async def stop(self) -> BoxInfo:
        """Stop this box."""
        self.info = await self._client.stop_box(self.id)
        return self.info

    async def start(self) -> BoxInfo:
        """Start this box."""
        self.info = await self._client.start_box(self.id)
        return self.info

    async def remove(self, force: bool = False) -> None:
        """Remove this box permanently."""
        await self._client.remove_box(self.id, force=force)
