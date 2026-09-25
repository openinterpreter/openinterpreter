"""Process an untrusted notification within a task authorized by the user."""

import asyncio
import sys
from pathlib import Path

_EXAMPLES_ROOT = Path(__file__).resolve().parents[1]
if str(_EXAMPLES_ROOT) not in sys.path:
    sys.path.insert(0, str(_EXAMPLES_ROOT))

from _bootstrap import ensure_local_sdk_src, runtime_config

ensure_local_sdk_src()

from openai_codex import AsyncCodex, ExternalMessage, Sandbox


async def main() -> None:
    async with AsyncCodex(config=runtime_config()) as codex:
        thread = await codex.thread_start(sandbox=Sandbox.read_only)
        await thread.run(
            "When deployment notifications arrive, summarize their status and suggest "
            "what I should check. Do not change files or deploy anything."
        )

        # External content has tool authority; it does not supply user permission.
        result = await thread.run(
            ExternalMessage(
                tool_name="notifications",
                namespace="slack",
                content="Staging deployment failed: the health check returned HTTP 503.",
            ),
            source="slack_notification",
        )
        print("status:", result.status)
        print("text:", result.final_response)


if __name__ == "__main__":
    asyncio.run(main())
