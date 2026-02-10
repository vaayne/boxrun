"""Stream command output in real time using exec_stream().

Prerequisites: boxrun serve
"""

import asyncio

from boxrun import BoxRunClient


async def main():
    async with BoxRunClient() as client:
        box = await client.create("ubuntu:24.04", name="stream-example")

        try:
            # Stream a command that produces output over time
            print("--- Streaming output ---")
            async for event in box.exec_stream(
                ["sh", "-c", "for i in 1 2 3 4 5; do echo \"Line $i\"; sleep 0.5; done"]
            ):
                if event.type == "log":
                    stream_label = f"[{event.stream}]" if event.stream else ""
                    print(f"{stream_label} {event.data}", end="")
                elif event.type == "exit":
                    print(f"\n--- Command finished ---")
        finally:
            await box.remove(force=True)
            print("Box removed.")


if __name__ == "__main__":
    asyncio.run(main())
