"""Upload a file, process it inside the VM, and download the result.

Prerequisites: boxrun serve
"""

import asyncio
import tempfile
from pathlib import Path

from boxrun import BoxRunClient


async def main():
    async with BoxRunClient() as client:
        box = await client.create("ubuntu:24.04", name="file-example")

        try:
            # Create a local file to upload
            with tempfile.NamedTemporaryFile(
                mode="w", suffix=".txt", delete=False
            ) as f:
                f.write("hello world\nfoo bar baz\nhello again\n")
                local_input = f.name

            # Upload it to the box
            await box.upload(local_input, "/root/input.txt")
            print(f"Uploaded {local_input} -> /root/input.txt")

            # Process the file inside the VM
            result = await box.exec(
                ["sh", "-c", "grep -c hello /root/input.txt > /root/output.txt"]
            )
            print(f"Processing exit code: {result.exit_code}")

            # Download the result
            local_output = tempfile.mktemp(suffix=".txt")
            await box.download("/root/output.txt", local_output)
            content = Path(local_output).read_text()
            print(f"Result: {content.strip()} lines matched 'hello'")

            # Clean up local temp files
            Path(local_input).unlink(missing_ok=True)
            Path(local_output).unlink(missing_ok=True)
        finally:
            await box.remove(force=True)
            print("Box removed.")


if __name__ == "__main__":
    asyncio.run(main())
