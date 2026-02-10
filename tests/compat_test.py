#!/usr/bin/env python3
"""
BoxRun Rust Server — Python SDK Compatibility Test
===================================================
Verifies the Rust server is 100% API-compatible with the Python SDK.
Runs real BoxLite VMs against a live server.

Prerequisites:
    pip install boxrun          # Python SDK

Usage:
    # 1. Start the Rust server
    boxrun serve --port 9099

    # 2. Run this test
    python tests/compat_test.py                          # default: http://127.0.0.1:9099
    BOXRUN_TEST_URL=http://host:port python tests/compat_test.py  # custom server
"""
import asyncio
import json
import os
import sys
import tempfile
import time
import traceback
from pathlib import Path

from boxrun.sdk.client import BoxHandle, BoxRunClient
from boxrun.sdk.types import BoxInfo, ExecEvent, ExecInfo, RunResult
from boxrun.common.errors import BoxRunError

SERVER_URL = os.environ.get("BOXRUN_TEST_URL", "http://127.0.0.1:9099")
passed = 0
failed = 0
errors = []


def report(name, ok, detail=""):
    global passed, failed
    if ok:
        passed += 1
        print(f"  PASS  {name}")
    else:
        failed += 1
        errors.append((name, detail))
        print(f"  FAIL  {name}: {detail}")


async def run_tests():
    global passed, failed

    async with BoxRunClient(base_url=SERVER_URL) as client:

        # ============================================================
        # 1. Server Info
        # ============================================================
        print("\n--- Server Info ---")
        try:
            resp = await client._http.get("/v1/info", timeout=10)
            data = resp.json()
            report("GET /v1/info returns 200", resp.status_code == 200)
            report("info has max_cpu", "max_cpu" in data and data["max_cpu"] > 0, str(data))
            report("info has max_memory_mb", "max_memory_mb" in data and data["max_memory_mb"] > 0, str(data))
        except Exception as e:
            report("GET /v1/info", False, str(e))

        # ============================================================
        # 2. Create Box
        # ============================================================
        print("\n--- Create Box ---")
        box: BoxHandle | None = None
        try:
            box = await client.create("python", name="compat-test", cpu=2, memory_mb=1024)
            report("create box returns BoxHandle", isinstance(box, BoxHandle))
            report("box has id (box_ prefix)", box.id.startswith("box_"), box.id)
            report("box name matches", box.name == "compat-test", box.name)
            report("box status is running", box.info.status == "running", box.info.status)
            report("box image is resolved", "python" in box.info.image.lower() or "ubuntu" in box.info.image.lower(), box.info.image)
            report("box cpu matches", box.info.cpu == 2, str(box.info.cpu))
            report("box memory_mb matches", box.info.memory_mb == 1024, str(box.info.memory_mb))
            report("box disk_size_gb present", box.info.disk_size_gb > 0, str(box.info.disk_size_gb))
            report("box workdir is /root", box.info.workdir == "/root", box.info.workdir)
            report("box created_at not empty", len(box.info.created_at) > 0, box.info.created_at)
        except Exception as e:
            report("create box", False, f"{e}\n{traceback.format_exc()}")

        if not box:
            print("FATAL: Cannot create box, aborting remaining tests")
            return

        box_id = box.id

        # ============================================================
        # 3. Get Box
        # ============================================================
        print("\n--- Get Box ---")
        try:
            info = await client.get_box(box_id)
            report("get_box by ID", isinstance(info, BoxInfo))
            report("get_box id matches", info.id == box_id)

            info2 = await client.get_box("compat-test")
            report("get_box by name", info2.id == box_id)
        except Exception as e:
            report("get_box", False, str(e))

        # ============================================================
        # 4. List Boxes
        # ============================================================
        print("\n--- List Boxes ---")
        try:
            boxes = await client.list_boxes()
            report("list_boxes returns list", isinstance(boxes, list))
            report("list_boxes contains our box", any(b.id == box_id for b in boxes))
            report("BoxInfo fields parse", all(isinstance(b, BoxInfo) for b in boxes))

            running = await client.list_boxes(status="running")
            report("list_boxes status filter", all(b.status == "running" for b in running))
        except Exception as e:
            report("list_boxes", False, str(e))

        # ============================================================
        # 5. Exec (blocking)
        # ============================================================
        print("\n--- Exec (blocking) ---")
        try:
            result = await box.exec(["echo", "hello world"])
            report("exec returns ExecInfo", isinstance(result, ExecInfo))
            report("exec status is succeeded", result.status == "succeeded", result.status)
            report("exec exit_code is 0", result.exit_code == 0, str(result.exit_code))
            report("exec has id", result.id.startswith("exec_"), result.id)
            report("exec box_id matches", result.box_id == box_id, result.box_id)
            report("exec cmd matches", result.cmd == ["echo", "hello world"], str(result.cmd))
            report("exec created_at set", len(result.created_at) > 0)
            report("exec finished_at set", result.finished_at is not None)
        except Exception as e:
            report("exec blocking", False, f"{e}\n{traceback.format_exc()}")

        # ============================================================
        # 6. Exec with non-zero exit code
        # ============================================================
        print("\n--- Exec with non-zero exit ---")
        try:
            result = await box.exec(["sh", "-c", "exit 42"])
            report("non-zero exit code", result.exit_code == 42, str(result.exit_code))
            report("status is failed", result.status == "failed", result.status)
        except Exception as e:
            report("exec non-zero exit", False, str(e))

        # ============================================================
        # 7. Exec with env
        # ============================================================
        print("\n--- Exec with env ---")
        try:
            result = await box.exec(["sh", "-c", "echo $MY_VAR"], env={"MY_VAR": "test123"})
            report("exec with env succeeds", result.exit_code == 0, str(result.exit_code))
        except Exception as e:
            report("exec with env", False, str(e))

        # ============================================================
        # 8. Exec stream (SSE)
        # ============================================================
        print("\n--- Exec Stream (SSE) ---")
        try:
            events = []
            async for event in box.exec_stream(["echo", "streamed output"]):
                events.append(event)
                if event.type == "exit":
                    break
            report("stream returns events", len(events) > 0, str(len(events)))

            log_events = [e for e in events if e.type == "log"]
            exit_events = [e for e in events if e.type == "exit"]
            report("has log events", len(log_events) > 0, f"log={len(log_events)}")
            report("has exit event", len(exit_events) == 1, f"exit={len(exit_events)}")

            if log_events:
                report("log event has stream field", log_events[0].stream in ("stdout", "stderr"), log_events[0].stream)
                report("log event has data", len(log_events[0].data) > 0)
                report("log event has seq", isinstance(log_events[0].seq, int))

            if exit_events:
                exit_data = exit_events[0].data
                report("exit event has data", len(exit_data) > 0, exit_data)
                try:
                    d = json.loads(exit_data) if isinstance(exit_data, str) else exit_data
                    if isinstance(d, dict):
                        report("exit data has exit_code", "exit_code" in d, str(d))
                    else:
                        report("exit data is parseable", True)
                except (json.JSONDecodeError, TypeError):
                    report("exit data format", True, f"raw: {exit_data}")
        except Exception as e:
            report("exec stream", False, f"{e}\n{traceback.format_exc()}")

        # ============================================================
        # 9. List Execs
        # ============================================================
        print("\n--- List Execs ---")
        execs = []
        try:
            resp = await client._http.get(f"/v1/boxes/{box_id}/execs", timeout=10)
            execs = resp.json()
            report("list execs returns array", isinstance(execs, list))
            report("list execs has entries", len(execs) >= 1, str(len(execs)))
            if execs:
                report("exec entry has id", "id" in execs[0])
                report("exec entry has status", "status" in execs[0])
        except Exception as e:
            report("list execs", False, str(e))

        # ============================================================
        # 10. Get Exec
        # ============================================================
        print("\n--- Get Exec ---")
        try:
            if execs:
                exec_id = execs[0]["id"]
                resp = await client._http.get(f"/v1/boxes/{box_id}/exec/{exec_id}", timeout=10)
                data = resp.json()
                ei = ExecInfo(**data)
                report("get exec parses as ExecInfo", isinstance(ei, ExecInfo))
                report("get exec has fields", ei.id == exec_id and ei.box_id == box_id)
        except Exception as e:
            report("get exec", False, str(e))

        # ============================================================
        # 11. File Upload
        # ============================================================
        print("\n--- File Upload ---")
        try:
            with tempfile.NamedTemporaryFile(mode="w", suffix=".txt", delete=False) as f:
                f.write("compat test content\n")
                upload_path = f.name
            await box.upload(upload_path, "/root/compat_test.txt")
            report("upload succeeds", True)
            os.unlink(upload_path)

            verify = await box.exec(["cat", "/root/compat_test.txt"])
            report("uploaded file readable", verify.exit_code == 0, str(verify.exit_code))
        except Exception as e:
            report("file upload", False, f"{e}\n{traceback.format_exc()}")

        # ============================================================
        # 12. File Download
        # ============================================================
        print("\n--- File Download ---")
        try:
            dl_path = tempfile.mktemp(suffix=".txt")
            await box.download("/root/compat_test.txt", dl_path)
            content = Path(dl_path).read_text()
            report("download succeeds", True)
            report("download content matches", "compat test content" in content, repr(content[:50]))
            os.unlink(dl_path)
        except Exception as e:
            report("file download", False, f"{e}\n{traceback.format_exc()}")

        # ============================================================
        # 13. Stop Box
        # ============================================================
        print("\n--- Stop Box ---")
        try:
            stopped = await box.stop()
            report("stop returns BoxInfo", isinstance(stopped, BoxInfo))
            report("stop status is stopped", stopped.status == "stopped", stopped.status)
        except Exception as e:
            report("stop box", False, str(e))

        # ============================================================
        # 14. Start Box
        # ============================================================
        print("\n--- Start Box ---")
        try:
            started = await box.start()
            report("start returns BoxInfo", isinstance(started, BoxInfo))
            report("start status is running", started.status == "running", started.status)
        except Exception as e:
            report("start box", False, str(e))

        # ============================================================
        # 15. Remove running box without force (should fail)
        # ============================================================
        print("\n--- Remove Box (running, no force) ---")
        try:
            await box.remove(force=False)
            report("remove running box without force", False, "should have raised")
        except BoxRunError as e:
            report("remove running box rejected", e.code == "BOX_ALREADY_RUNNING", f"code={e.code}")
        except Exception as e:
            report("remove running box error type", False, str(e))

        # ============================================================
        # 16. Remove Box (force)
        # ============================================================
        print("\n--- Remove Box (force) ---")
        try:
            await box.remove(force=True)
            report("force remove succeeds", True)

            try:
                await client.get_box(box_id)
                report("box is gone after remove", False, "still exists")
            except BoxRunError as e:
                report("box is gone after remove", e.code == "BOX_NOT_FOUND", f"code={e.code}")
        except Exception as e:
            report("force remove", False, str(e))

        # ============================================================
        # 17. Error: Get non-existent box
        # ============================================================
        print("\n--- Error Responses ---")
        try:
            await client.get_box("nonexistent_box_id")
            report("404 for missing box", False, "should have raised")
        except BoxRunError as e:
            report("404 code is BOX_NOT_FOUND", e.code == "BOX_NOT_FOUND", f"code={e.code}")
            report("404 has message", len(e.message) > 0, e.message)
        except Exception as e:
            report("404 error format", False, str(e))

        # ============================================================
        # 18. Error: Invalid create request
        # ============================================================
        try:
            resp = await client._http.post("/v1/boxes", json={"image": "python", "cpu": -1}, timeout=10)
            report("invalid request returns 4xx", resp.status_code >= 400, str(resp.status_code))
            data = resp.json()
            report("error has code field", "code" in data, str(data))
            report("error has message field", "message" in data, str(data))
        except Exception as e:
            report("validation error", False, str(e))

        # ============================================================
        # 19. Duplicate name
        # ============================================================
        print("\n--- Duplicate Name ---")
        dup_box = None
        try:
            dup_box = await client.create("python", name="dup-test")
            try:
                await client.create("python", name="dup-test")
                report("duplicate name rejected", False, "should have raised")
            except BoxRunError as e:
                report("duplicate name error code", e.code == "NAME_ALREADY_EXISTS", f"code={e.code}")
        except Exception as e:
            report("duplicate name test", False, str(e))
        finally:
            if dup_box:
                try:
                    await dup_box.remove(force=True)
                except Exception:
                    pass

        # ============================================================
        # 20. Ephemeral Run (/v1/run)
        # ============================================================
        print("\n--- Ephemeral Run ---")
        try:
            result = await client.run("python", ["echo", "ephemeral test"])
            report("run returns RunResult", isinstance(result, RunResult))
            report("run exit_code is 0", result.exit_code == 0, str(result.exit_code))
            report("run has stdout", "ephemeral test" in result.stdout, repr(result.stdout[:50]))
            report("run stderr is string", isinstance(result.stderr, str))
        except Exception as e:
            report("ephemeral run", False, f"{e}\n{traceback.format_exc()}")

        # ============================================================
        # 21. Ephemeral Run with non-zero exit
        # ============================================================
        print("\n--- Ephemeral Run (non-zero exit) ---")
        try:
            result = await client.run("python", ["sh", "-c", "echo err >&2; exit 7"])
            report("run non-zero exit_code", result.exit_code == 7, str(result.exit_code))
            report("run has stderr", "err" in result.stderr, repr(result.stderr[:50]))
        except Exception as e:
            report("ephemeral run non-zero", False, f"{e}\n{traceback.format_exc()}")

        # ============================================================
        # 22. GC
        # ============================================================
        print("\n--- Garbage Collection ---")
        try:
            removed = await client.gc(older_than=999999)
            report("gc returns int", isinstance(removed, int))
            report("gc removed >= 0", removed >= 0, str(removed))
        except Exception as e:
            report("gc", False, str(e))

        # ============================================================
        # 23. Create with volumes
        # ============================================================
        print("\n--- Volumes ---")
        vol_box = None
        try:
            tmp_dir = tempfile.mkdtemp()
            Path(tmp_dir, "voltest.txt").write_text("volume data")
            vols = [{"host_path": tmp_dir, "guest_path": "/mnt/data", "readonly": False}]
            vol_box = await client.create("python", volumes=vols)
            report("create with volumes", isinstance(vol_box, BoxHandle))
            report("volumes in response", vol_box.info.volumes is not None)
            if vol_box.info.volumes:
                report("volume guest_path matches", vol_box.info.volumes[0]["guest_path"] == "/mnt/data")
        except Exception as e:
            report("volumes create", False, f"{e}\n{traceback.format_exc()}")
        finally:
            if vol_box:
                try:
                    await vol_box.remove(force=True)
                except Exception:
                    pass

        # ============================================================
        # 24. Web UI
        # ============================================================
        print("\n--- Web UI ---")
        try:
            resp = await client._http.get("/ui", timeout=10, follow_redirects=True)
            report("GET /ui returns 200", resp.status_code == 200, str(resp.status_code))
            report("UI is HTML", "html" in resp.headers.get("content-type", "").lower() or "<html" in resp.text[:200].lower(),
                   resp.headers.get("content-type", ""))
        except Exception as e:
            report("web ui", False, str(e))

        # ============================================================
        # 25. Root redirect
        # ============================================================
        try:
            resp = await client._http.get("/", timeout=10, follow_redirects=False)
            report("GET / redirects", resp.status_code in (301, 302, 303, 307, 308), str(resp.status_code))
        except Exception as e:
            report("root redirect", False, str(e))


async def main():
    print("=" * 60)
    print("BoxRun Rust Server - Python SDK Compatibility Test")
    print(f"Target: {SERVER_URL}")
    print("=" * 60)

    start = time.time()
    await run_tests()
    elapsed = time.time() - start

    print("\n" + "=" * 60)
    print(f"Results: {passed} passed, {failed} failed ({elapsed:.1f}s)")
    if errors:
        print(f"\nFailed tests:")
        for name, detail in errors:
            print(f"  - {name}: {detail}")
    print("=" * 60)

    sys.exit(1 if failed > 0 else 0)


if __name__ == "__main__":
    asyncio.run(main())
