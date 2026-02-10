"""Basic BoxRun SDK usage: create a box, run a command, and clean up.

Prerequisites: boxrun serve
"""

import asyncio

from boxrun_sdk import BoxRunClient


async def main():
    async with BoxRunClient() as client:
        # Create a box
        box = await client.create("ubuntu:24.04", name="basic-example")
        print(f"Created box: {box.id} (status: {box.info.status})")

        try:
            # Run a command and stream its output
            async for event in box.exec_stream(["echo", "Hello from BoxRun!"]):
                if event.type == "log":
                    print(event.data, end="")

            # Run another command
            async for event in box.exec_stream(["uname", "-a"]):
                if event.type == "log":
                    print(event.data, end="")
        finally:
            # Clean up
            await box.remove(force=True)
            print("Box removed.")


if __name__ == "__main__":
    asyncio.run(main())
