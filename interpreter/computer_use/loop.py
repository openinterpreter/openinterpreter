"""
Based on Anthropic's computer use example at https://github.com/anthropics/anthropic-quickstarts/blob/main/computer-use-demo/computer_use_demo/loop.py
"""

import asyncio
import json
import os
import platform
import time
import traceback
import uuid
from collections.abc import Callable
from datetime import datetime

try:
    from enum import StrEnum
except ImportError:  # 3.10 compatibility
    from enum import Enum as StrEnum

from typing import Any, List, cast

import requests
from anthropic import Anthropic, AnthropicBedrock, AnthropicVertex, APIResponse
from anthropic.types import ToolResultBlockParam
from anthropic.types.beta import (
    BetaContentBlock,
    BetaContentBlockParam,
    BetaImageBlockParam,
    BetaMessage,
    BetaMessageParam,
    BetaRawContentBlockDeltaEvent,
    BetaRawContentBlockStartEvent,
    BetaRawContentBlockStopEvent,
    BetaTextBlockParam,
    BetaToolResultBlockParam,
)

from .tools import BashTool, ComputerTool, EditTool, ToolCollection, ToolResult

BETA_FLAG = "computer-use-2024-10-22"

from typing import List, Optional

import uvicorn
from fastapi import FastAPI
from fastapi.responses import StreamingResponse
from pydantic import BaseModel
from rich import print as rich_print
from rich.markdown import Markdown
from rich.rule import Rule

# Add this near the top of the file, with other imports and global variables
messages: List[BetaMessageParam] = []


def print_markdown(message):
    """
    Display markdown message. Works with multiline strings with lots of indentation.
    Will automatically make single line > tags beautiful.
    """

    for line in message.split("\n"):
        line = line.strip()
        if line == "":
            print("")
        elif line == "---":
            rich_print(Rule(style="white"))
        else:
            try:
                rich_print(Markdown(line))
            except UnicodeEncodeError as e:
                # Replace the problematic character or handle the error as needed
                print("Error displaying line:", line)

    if "\n" not in message and message.startswith(">"):
        # Aesthetic choice. For these tags, they need a space below them
        print("")


class APIProvider(StrEnum):
    ANTHROPIC = "anthropic"
    BEDROCK = "bedrock"
    VERTEX = "vertex"


PROVIDER_TO_DEFAULT_MODEL_NAME: dict[APIProvider, str] = {
    APIProvider.ANTHROPIC: "claude-3-5-sonnet-20241022",
    APIProvider.BEDROCK: "anthropic.claude-3-5-sonnet-20241022-v2:0",
    APIProvider.VERTEX: "claude-3-5-sonnet-v2@20241022",
}


# This system prompt is optimized for the Docker environment in this repository and
# specific tool combinations enabled.
# We encourage modifying this system prompt to ensure the model has context for the
# environment it is running in, and to provide any additional information that may be
# helpful for the task at hand.

SYSTEM_PROMPT = f"""<SYSTEM_CAPABILITY>
* You are an AI assistant with access to a virtual machine running on {"Mac OS" if platform.system() == "Darwin" else platform.system()} with internet access.
* When using your computer function calls, they take a while to run and send back to you. Where possible/feasible, try to chain multiple of these calls all into one function calls request.
* The current date is {datetime.today().strftime('%A, %B %d, %Y')}.
</SYSTEM_CAPABILITY>"""

# Update the SYSTEM_PROMPT for Mac OS
if platform.system() == "Darwin":
    SYSTEM_PROMPT += """
<IMPORTANT>
* Open applications using Spotlight by using the computer tool to simulate pressing Command+Space, typing the application name, and pressing Enter.
</IMPORTANT>"""


async def sampling_loop(
    *,
    model: str,
    provider: APIProvider,
    system_prompt_suffix: str,
    messages: list[BetaMessageParam],
    output_callback: Callable[[BetaContentBlock], None],
    tool_output_callback: Callable[[ToolResult, str], None],
    api_key: str,
    only_n_most_recent_images: int | None = None,
    max_tokens: int = 4096,
):
    """
    Agentic sampling loop for the assistant/tool interaction of computer use.
    """
    tool_collection = ToolCollection(
        ComputerTool(),
        # BashTool(),
        # EditTool(),
    )
    system = (
        f"{SYSTEM_PROMPT}{' ' + system_prompt_suffix if system_prompt_suffix else ''}"
    )

    while True:
        if only_n_most_recent_images:
            _maybe_filter_to_n_most_recent_images(messages, only_n_most_recent_images)

        if provider == APIProvider.ANTHROPIC:
            client = Anthropic(api_key=api_key, timeout=60.0, max_retries=3)
        elif provider == APIProvider.VERTEX:
            client = AnthropicVertex()
        elif provider == APIProvider.BEDROCK:
            client = AnthropicBedrock()

        # Call the API
        # we use raw_response to provide debug information to streamlit. Your
        # implementation may be able call the SDK directly with:
        # `response = client.messages.create(...)` instead.
        raw_response = client.beta.messages.create(
            max_tokens=max_tokens,
            messages=messages,
            model=model,
            system=system,
            tools=tool_collection.to_params(),
            betas=["computer-use-2024-10-22"],
            stream=True,
        )

        response_content = []
        current_block = None

        for chunk in raw_response:
            if isinstance(chunk, BetaRawContentBlockStartEvent):
                current_block = chunk.content_block
            elif isinstance(chunk, BetaRawContentBlockDeltaEvent):
                if chunk.delta.type == "text_delta":
                    print(f"{chunk.delta.text}", end="", flush=True)
                    yield {"type": "chunk", "chunk": chunk.delta.text}
                    await asyncio.sleep(0)
                    if current_block and current_block.type == "text":
                        current_block.text += chunk.delta.text
                elif chunk.delta.type == "input_json_delta":
                    print(f"{chunk.delta.partial_json}", end="", flush=True)
                    if current_block and current_block.type == "tool_use":
                        if not hasattr(current_block, "partial_json"):
                            current_block.partial_json = ""
                        current_block.partial_json += chunk.delta.partial_json
            elif isinstance(chunk, BetaRawContentBlockStopEvent):
                if current_block:
                    if hasattr(current_block, "partial_json"):
                        # Finished a tool call
                        # print()
                        current_block.input = json.loads(current_block.partial_json)
                        # yield {"type": "chunk", "chunk": current_block.input}
                        delattr(current_block, "partial_json")
                    else:
                        # Finished a message
                        print("\n")
                        yield {"type": "chunk", "chunk": "\n"}
                        await asyncio.sleep(0)
                    response_content.append(current_block)
                    current_block = None

        response = BetaMessage(
            id=str(uuid.uuid4()),
            content=response_content,
            role="assistant",
            model=model,
            stop_reason=None,
            stop_sequence=None,
            type="message",
            usage={
                "input_tokens": 0,
                "output_tokens": 0,
            },  # Add a default usage dictionary
        )

        messages.append(
            {
                "role": "assistant",
                "content": cast(list[BetaContentBlockParam], response.content),
            }
        )

        tool_result_content: list[BetaToolResultBlockParam] = []
        for content_block in cast(list[BetaContentBlock], response.content):
            output_callback(content_block)
            if content_block.type == "tool_use":
                result = await tool_collection.run(
                    name=content_block.name,
                    tool_input=cast(dict[str, Any], content_block.input),
                )
                tool_result_content.append(
                    _make_api_tool_result(result, content_block.id)
                )
                tool_output_callback(result, content_block.id)

        if not tool_result_content:
            # Done!
            yield {"type": "messages", "messages": messages}
            break

        messages.append({"content": tool_result_content, "role": "user"})


def _maybe_filter_to_n_most_recent_images(
    messages: list[BetaMessageParam],
    images_to_keep: int,
    min_removal_threshold: int = 5,
):
    """
    With the assumption that images are screenshots that are of diminishing value as
    the conversation progresses, remove all but the final `images_to_keep` tool_result
    images in place, with a chunk of min_removal_threshold to reduce the amount we
    break the implicit prompt cache.
    """
    if images_to_keep is None:
        return messages

    tool_result_blocks = cast(
        list[ToolResultBlockParam],
        [
            item
            for message in messages
            for item in (
                message["content"] if isinstance(message["content"], list) else []
            )
            if isinstance(item, dict) and item.get("type") == "tool_result"
        ],
    )

    total_images = sum(
        1
        for tool_result in tool_result_blocks
        for content in tool_result.get("content", [])
        if isinstance(content, dict) and content.get("type") == "image"
    )

    images_to_remove = total_images - images_to_keep
    # for better cache behavior, we want to remove in chunks
    images_to_remove -= images_to_remove % min_removal_threshold

    for tool_result in tool_result_blocks:
        if isinstance(tool_result.get("content"), list):
            new_content = []
            for content in tool_result.get("content", []):
                if isinstance(content, dict) and content.get("type") == "image":
                    if images_to_remove > 0:
                        images_to_remove -= 1
                        continue
                new_content.append(content)
            tool_result["content"] = new_content


def _make_api_tool_result(
    result: ToolResult, tool_use_id: str
) -> BetaToolResultBlockParam:
    """Convert an agent ToolResult to an API ToolResultBlockParam."""
    tool_result_content: list[BetaTextBlockParam | BetaImageBlockParam] | str = []
    is_error = False
    if result.error:
        is_error = True
        tool_result_content = _maybe_prepend_system_tool_result(result, result.error)
    else:
        if result.output:
            tool_result_content.append(
                {
                    "type": "text",
                    "text": _maybe_prepend_system_tool_result(result, result.output),
                }
            )
        if result.base64_image:
            tool_result_content.append(
                {
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": "image/png",
                        "data": result.base64_image,
                    },
                }
            )
    return {
        "type": "tool_result",
        "content": tool_result_content,
        "tool_use_id": tool_use_id,
        "is_error": is_error,
    }


def _maybe_prepend_system_tool_result(result: ToolResult, result_text: str):
    if result.system:
        result_text = f"<system>{result.system}</system>\n{result_text}"
    return result_text


async def main():
    global exit_flag
    messages: List[BetaMessageParam] = []
    model = PROVIDER_TO_DEFAULT_MODEL_NAME[APIProvider.ANTHROPIC]
    provider = APIProvider.ANTHROPIC
    system_prompt_suffix = ""

    # Check if running in server mode
    if "--server" in sys.argv:
        app = FastAPI()

        # Start the mouse position checking thread when in server mode
        mouse_thread = threading.Thread(target=check_mouse_position)
        mouse_thread.daemon = True
        mouse_thread.start()

        # Get API key from environment variable
        api_key = os.environ.get("ANTHROPIC_API_KEY")
        if not api_key:
            raise ValueError(
                "ANTHROPIC_API_KEY environment variable must be set when running in server mode"
            )

        @app.post("/openai/chat/completions")
        async def chat_completion(request: ChatCompletionRequest):
            print("BRAND NEW REQUEST")
            # Check exit flag before processing request
            if exit_flag:
                return {"error": "Server shutting down due to mouse in corner"}

            async def stream_response():
                print("is this even happening")

                # Instead of creating converted_messages, append the last message to global messages
                global messages
                messages.append(
                    {
                        "role": request.messages[-1].role,
                        "content": [
                            {"type": "text", "text": request.messages[-1].content}
                        ],
                    }
                )

                response_chunks = []

                async def output_callback(content_block: BetaContentBlock):
                    chunk = f"data: {json.dumps({'choices': [{'delta': {'content': content_block.text}}]})}\n\n"
                    response_chunks.append(chunk)
                    yield chunk

                async def tool_output_callback(result: ToolResult, tool_id: str):
                    if result.output or result.error:
                        content = result.output if result.output else result.error
                        chunk = f"data: {json.dumps({'choices': [{'delta': {'content': content}}]})}\n\n"
                        response_chunks.append(chunk)
                        yield chunk

                try:
                    yield f"data: {json.dumps({'choices': [{'delta': {'role': 'assistant'}}]})}\n\n"

                    messages = [m for m in messages if m["content"]]
                    print(str(messages)[-100:])
                    await asyncio.sleep(4)

                    async for chunk in sampling_loop(
                        model=model,
                        provider=provider,
                        system_prompt_suffix=system_prompt_suffix,
                        messages=messages,  # Now using global messages
                        output_callback=output_callback,
                        tool_output_callback=tool_output_callback,
                        api_key=api_key,
                    ):
                        if chunk["type"] == "chunk":
                            await asyncio.sleep(0)
                            yield f"data: {json.dumps({'choices': [{'delta': {'content': chunk['chunk']}}]})}\n\n"
                        if chunk["type"] == "messages":
                            messages = chunk["messages"]

                    yield f"data: {json.dumps({'choices': [{'delta': {'content': '', 'finish_reason': 'stop'}}]})}\n\n"

                except Exception as e:
                    print("Error: An exception occurred.")
                    print(traceback.format_exc())
                    pass
                    # raise
                    # print(f"Error: {e}")
                    # yield f"data: {json.dumps({'error': str(e)})}\n\n"

            return StreamingResponse(stream_response(), media_type="text/event-stream")

        # Instead of running uvicorn here, we'll return the app
        return app

    # Original CLI code continues here...
    print()
    print_markdown("Welcome to **Open Interpreter**.\n")
    print_markdown("---")
    time.sleep(0.5)

    # Check for API key in environment variable
    api_key = os.environ.get("ANTHROPIC_API_KEY")
    if not api_key:
        api_key = input(
            "\nAn Anthropic API is required for OS mode.\n\nEnter your Anthropic API key: "
        )
        print_markdown("\n---")
        time.sleep(0.5)

    import random

    tips = [
        # "You can type `i` in your terminal to use Open Interpreter.",
        "**Tip:** Type `wtf` in your terminal to have Open Interpreter fix the last error.",
        # "You can type prompts after `i` in your terminal, for example, `i want you to install node`. (Yes, really.)",
        "We recommend using our desktop app for the best experience. Type `d` for early access.",
        "**Tip:** Reduce display resolution for better performance.",
    ]

    random_tip = random.choice(tips)

    markdown_text = f"""> Model set to `Claude 3.5 Sonnet (New)`, OS control enabled

{random_tip}

**Warning:** This AI has full system access and can modify files, install software, and execute commands. By continuing, you accept all risks and responsibility.

Move your mouse to any corner of the screen to exit.
"""

    print_markdown(markdown_text)

    # Start the mouse position checking thread
    mouse_thread = threading.Thread(target=check_mouse_position)
    mouse_thread.daemon = True
    mouse_thread.start()

    while not exit_flag:
        user_input = input("> ")
        print()
        if user_input.lower() in ["exit", "quit", "q"]:
            break
        elif user_input.lower() in ["d"]:
            print_markdown(
                "---\nTo get early access to the **Open Interpreter Desktop App**, please provide the following information:\n"
            )
            first_name = input("What's your first name? ").strip()
            email = input("What's your email? ").strip()

            url = "https://neyguovvcjxfzhqpkicj.supabase.co/functions/v1/addEmailToWaitlist"
            data = {"first_name": first_name, "email": email}

            try:
                response = requests.post(url, json=data)
            except requests.RequestException as e:
                pass

            print_markdown("\nWe'll email you shortly. ✓\n---\n")
            continue

        messages.append(
            {"role": "user", "content": [{"type": "text", "text": user_input}]}
        )

        def output_callback(content_block: BetaContentBlock):
            pass

        def tool_output_callback(result: ToolResult, tool_id: str):
            if result.output:
                print(f"---\n{result.output}\n---")
            if result.error:
                print(f"---\n{result.error}\n---")

        try:
            async for chunk in sampling_loop(
                model=model,
                provider=provider,
                system_prompt_suffix=system_prompt_suffix,
                messages=messages,
                output_callback=output_callback,
                tool_output_callback=tool_output_callback,
                api_key=api_key,
            ):
                if chunk["type"] == "messages":
                    messages = chunk["messages"]
        except Exception as e:
            raise

    # The thread will automatically terminate when the main program exits


def run_async_main():
    if "--server" in sys.argv:
        # Start uvicorn server directly without asyncio.run()
        app = asyncio.run(main())
        uvicorn.run(app, host="0.0.0.0", port=8000)
    else:
        asyncio.run(main())


if __name__ == "__main__":
    run_async_main()

import sys
import threading

# Replace the pynput and screeninfo imports with pyautogui
import pyautogui

# Replace the global variables and functions related to mouse tracking
exit_flag = False


def check_mouse_position():
    global exit_flag
    corner_threshold = 10
    screen_width, screen_height = pyautogui.size()

    while not exit_flag:
        x, y = pyautogui.position()
        if (
            (x <= corner_threshold and y <= corner_threshold)
            or (x <= corner_threshold and y >= screen_height - corner_threshold)
            or (x >= screen_width - corner_threshold and y <= corner_threshold)
            or (
                x >= screen_width - corner_threshold
                and y >= screen_height - corner_threshold
            )
        ):
            exit_flag = True
            print("\nMouse moved to corner. Exiting...")
            os._exit(0)
        threading.Event().wait(0.1)  # Check every 100ms


class ChatMessage(BaseModel):
    role: str
    content: str


class ChatCompletionRequest(BaseModel):
    messages: List[ChatMessage]
    stream: Optional[bool] = False
