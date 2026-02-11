# boxrun-sdk

Python SDK for [BoxRun](https://github.com/boxlite-ai/boxrun) — ultra-lightweight sandbox platform for developers and AI agents.

## Install

```bash
pip install boxrun-sdk
```

## Quick Start

```python
import asyncio
from boxrun_sdk import BoxRunClient

async def main():
    async with BoxRunClient() as client:
        box = await client.create("ubuntu:24.04", name="dev")

        result = await box.exec(["echo", "hello"])
        print(f"Exit code: {result.exit_code}")

        async for event in box.exec_stream(["apt-get", "update"]):
            if event.type == "log":
                print(event.data, end="")

        await box.remove()

asyncio.run(main())
```

## Documentation

- [SDK Reference](https://github.com/boxlite-ai/boxrun/blob/main/docs/sdk.md)
- [Examples](https://github.com/boxlite-ai/boxrun/tree/main/examples)
- [REST API](https://github.com/boxlite-ai/boxrun/blob/main/docs/api.md)

## License

Apache-2.0
