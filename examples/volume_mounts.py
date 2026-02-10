"""Create a box with a volume mount and demonstrate shared filesystem access.

Prerequisites: boxrun serve
"""

import asyncio
import tempfile
from pathlib import Path

from boxrun_sdk import BoxRunClient


async def main():
    # Create a temporary host directory with a file
    with tempfile.TemporaryDirectory() as host_dir:
        (Path(host_dir) / "greeting.txt").write_text("Hello from the host!\n")

        async with BoxRunClient() as client:
            # Create a box with the host directory mounted at /root/shared
            box = await client.create(
                "ubuntu:24.04",
                name="volume-example",
                volumes=[
                    {
                        "host_path": host_dir,
                        "guest_path": "/root/shared",
                        "readonly": False,
                    }
                ],
            )

            try:
                # Read the host file from inside the VM
                result = await box.exec(["cat", "/root/shared/greeting.txt"])
                print(f"Read from VM (exit {result.exit_code})")

                # Write a file from inside the VM
                await box.exec(
                    ["sh", "-c", "echo 'Hello from the VM!' > /root/shared/reply.txt"]
                )

                # Verify it's visible on the host
                reply = (Path(host_dir) / "reply.txt").read_text()
                print(f"Host sees reply: {reply.strip()}")
            finally:
                await box.remove(force=True)
                print("Box removed.")


if __name__ == "__main__":
    asyncio.run(main())
