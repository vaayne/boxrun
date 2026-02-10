"""Full AI agent workflow: create a workspace, upload code, stream
execution, and download results.

Uses python:3.12-slim image which has Python pre-installed,
so no apt-get install needed.

Prerequisites: boxrun serve
"""

import asyncio
import json
import tempfile
from pathlib import Path

from boxrun import BoxRunClient


async def main():
    async with BoxRunClient() as client:
        # 1. Create a workspace box with Python pre-installed
        box = await client.create(
            "python:3.12-slim", name="agent-workspace", memory_mb=1024
        )
        print(f"Created workspace: {box.id}")

        try:
            # 2. Upload a script
            with tempfile.NamedTemporaryFile(
                mode="w", suffix=".py", delete=False
            ) as f:
                f.write(
                    'import json\n'
                    'result = {"numbers": [i**2 for i in range(10)], "status": "ok"}\n'
                    'with open("/root/output.json", "w") as out:\n'
                    '    json.dump(result, out)\n'
                    'print("Done! Output written to /root/output.json")\n'
                )
                script_path = f.name

            await box.upload(script_path, "/root/task.py")
            Path(script_path).unlink()
            print("Uploaded task.py")

            # 3. Run the script, streaming output
            print("\n--- Running task ---")
            async for event in box.exec_stream(["python3", "/root/task.py"]):
                if event.type == "log":
                    print(event.data, end="")

            # 4. Download results
            local_output = tempfile.mktemp(suffix=".json")
            await box.download("/root/output.json", local_output)
            result = json.loads(Path(local_output).read_text())
            print(f"\nResult: {result}")
            Path(local_output).unlink()

        finally:
            # 5. Clean up
            await box.remove(force=True)
            print("Workspace removed.")


if __name__ == "__main__":
    asyncio.run(main())
