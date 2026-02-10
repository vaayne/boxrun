"""Spin up multiple VMs, each running Claude Code as an AI agent.

Each VM is an isolated sandbox with Claude Code installed. You provide
the prompts — this script handles creating VMs, installing Claude Code,
and running agents in parallel.

Prerequisites:
  - boxrun serve
  - ANTHROPIC_API_KEY environment variable set

Usage:
  # Run 3 agents with your own prompts:
  ANTHROPIC_API_KEY=sk-ant-... python examples/multi_claude_agents.py \
    "Write a Python function that sorts a list" \
    "Write a bash script that finds large files" \
    "Write a Makefile for a Go project"
"""

import asyncio
import os
import shlex
import sys

from boxrun_sdk import BoxRunClient


async def setup_claude_code(box):
    """Install Claude Code CLI in a box using the official native installer."""
    print(f"  [{box.info.name}] Installing Claude Code...")

    install_cmd = (
        "apt-get update -qq && "
        "apt-get install -y -qq curl > /dev/null 2>&1 && "
        "curl -fsSL https://claude.ai/install.sh | bash && "
        'export PATH="$HOME/.local/bin:$PATH" && '
        "claude --version"
    )
    result = await box.exec(
        ["sh", "-c", install_cmd],
        env={"DEBIAN_FRONTEND": "noninteractive"},
        timeout_ms=180_000,  # 3 min for install
    )
    if result.exit_code != 0:
        print(f"  [{box.info.name}] Install failed (exit {result.exit_code})")
        return False

    print(f"  [{box.info.name}] Claude Code ready")
    return True


async def run_agent(client: BoxRunClient, index: int, prompt: str, api_key: str):
    """Create a VM, install Claude Code, and run a single prompt."""
    name = f"agent-{index}"

    box = await client.create(
        "ubuntu:24.04",
        name=name,
        cpu=2,
        memory_mb=1024,
        network=True,
    )
    print(f"[{name}] Created box {box.id}")

    try:
        ok = await setup_claude_code(box)
        if not ok:
            return {"name": name, "status": "install_failed"}

        # Run Claude Code in headless mode.
        # --dangerously-skip-permissions: required for non-interactive use
        # --max-turns 10: prevent runaway agents
        print(f"[{name}] Running: {prompt[:60]}...")
        claude_cmd = (
            'export PATH="$HOME/.local/bin:$PATH" && '
            "claude -p "
            "--dangerously-skip-permissions "
            "--output-format text "
            "--max-turns 10 "
            f"{shlex.quote(prompt)}"
        )
        result = await box.exec(
            ["sh", "-c", claude_cmd],
            env={"ANTHROPIC_API_KEY": api_key},
            timeout_ms=300_000,  # 5 min
        )
        print(f"[{name}] Done (exit {result.exit_code})")

        # Show what the agent produced
        async for event in box.exec_stream(["ls", "-la", "/root/"]):
            if event.type == "log" and event.stream == "stdout":
                for line in event.data.splitlines():
                    print(f"  [{name}] {line}")

        return {
            "name": name,
            "status": "success" if result.exit_code == 0 else "failed",
            "exit_code": result.exit_code,
        }
    finally:
        await box.remove(force=True)
        print(f"[{name}] Box removed")


async def main():
    api_key = os.environ.get("ANTHROPIC_API_KEY")
    if not api_key:
        print("Error: ANTHROPIC_API_KEY environment variable is required")
        sys.exit(1)

    prompts = sys.argv[1:]
    if not prompts:
        print("Usage: python examples/multi_claude_agents.py <prompt1> <prompt2> ...")
        print('Example: python examples/multi_claude_agents.py "Write hello world in Go"')
        sys.exit(1)

    print(f"Launching {len(prompts)} agent(s) in parallel...\n")

    async with BoxRunClient() as client:
        tasks = [
            asyncio.create_task(run_agent(client, i, prompt, api_key))
            for i, prompt in enumerate(prompts)
        ]
        results = await asyncio.gather(*tasks, return_exceptions=True)

        print("\n" + "=" * 60)
        print("RESULTS")
        print("=" * 60)
        for r in results:
            if isinstance(r, Exception):
                print(f"  ERROR: {r}")
            else:
                print(f"  {r['name']}: {r['status']}")


if __name__ == "__main__":
    asyncio.run(main())
