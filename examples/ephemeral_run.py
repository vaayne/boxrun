"""One-shot ephemeral run — the shortest possible BoxRun example.

Creates a box, runs a command, prints output, and destroys the box
all in a single call.

Prerequisites: boxrun serve
"""

import asyncio

from boxrun_sdk import BoxRunClient


async def main():
    async with BoxRunClient() as client:
        result = await client.run("ubuntu:24.04", ["echo", "Hello from an ephemeral box!"])
        print(f"stdout: {result.stdout}")
        print(f"stderr: {result.stderr}")
        print(f"exit code: {result.exit_code}")


if __name__ == "__main__":
    asyncio.run(main())
