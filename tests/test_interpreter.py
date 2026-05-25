import os
import platform
import signal
import time
from random import randint

import pytest

#####
from interpreter import AsyncInterpreter, OpenInterpreter
from interpreter.terminal_interface.magic_commands import handle_magic_command
from interpreter.terminal_interface.utils.count_tokens import (
    count_messages_tokens,
    count_tokens,
)

interpreter = OpenInterpreter()
#####

import multiprocessing
import threading
import time

import pytest
from websocket import create_connection


def test_hallucinations():
    # We should be resiliant to common hallucinations.

    code = """10+12executeexecute\n"""

    interpreter.messages = [
        {"role": "assistant", "type": "code", "format": "python", "content": code}
    ]
    for chunk in interpreter._respond_and_store():
        if chunk.get("format") == "output":
            assert chunk.get("content") == "22"
            break

    code = """{                                                                             
    "language": "python",                                                        
    "code": "10+12"                                                        
  }"""

    interpreter.messages = [
        {"role": "assistant", "type": "code", "format": "python", "content": code}
    ]
    for chunk in interpreter._respond_and_store():
        if chunk.get("format") == "output":
            assert chunk.get("content") == "22"
            break

    code = """functions.execute({                                                                             
    "language": "python",                                                        
    "code": "10+12"                                                        
  })"""

    interpreter.messages = [
        {"role": "assistant", "type": "code", "format": "python", "content": code}
    ]
    for chunk in interpreter._respond_and_store():
        if chunk.get("format") == "output":
            assert chunk.get("content") == "22"
            break

    code = """{language: "python", code: "print('hello')" }"""

    interpreter.messages = [
        {"role": "assistant", "type": "code", "format": "python", "content": code}
    ]
    for chunk in interpreter._respond_and_store():
        if chunk.get("format") == "output":
            assert chunk.get("content").strip() == "hello"
            break


def run_auth_server():
    os.environ["INTERPRETER_REQUIRE_ACKNOWLEDGE"] = "True"
    os.environ["INTERPRETER_API_KEY"] = "testing"
    async_interpreter = AsyncInterpreter()
    async_interpreter.print = False
    async_interpreter.server.run()


# @pytest.mark.skip(reason="Requires uvicorn, which we don't require by default")
def test_authenticated_acknowledging_breaking_server():
    """
    Test the server when we have authentication and acknowledging one.

    I know this is bad, just trying to test quickly!
    """

    # Start the server in a new process

    process = multiprocessing.Process(target=run_auth_server)
    process.start()

    # Give the server a moment to start
    time.sleep(2)

    import asyncio
    import json

    import requests
    import websockets

    async def test_fastapi_server():
        import asyncio

        async with websockets.connect("ws://localhost:8000/") as websocket:
            # Connect to the websocket
            print("Connected to WebSocket")

            # Sending message via WebSocket
            await websocket.send(json.dumps({"auth": "testing"}))

            # Sending POST request
            post_url = "http://localhost:8000/settings"
            settings = {
                "llm": {
                    "model": "gpt-4o",
                    "execution_instructions": "",
                    "supports_functions": False,
                },
                "system_message": "You are a poem writing bot. Do not do anything but respond with a poem.",
                "auto_run": True,
            }
            response = requests.post(
                post_url, json=settings, headers={"X-API-KEY": "testing"}
            )
            print("POST request sent, response:", response.json())

            # Sending messages via WebSocket
            await websocket.send(
                json.dumps({"role": "user", "type": "message", "start": True})
            )
            await websocket.send(
                json.dumps(
                    {
                        "role": "user",
                        "type": "message",
                        "content": "Write a short poem about Seattle.",
                    }
                )
            )
            await websocket.send(
                json.dumps({"role": "user", "type": "message", "end": True})
            )
            print("WebSocket chunks sent")

            max_chunks = 5

            poem = ""
            while True:
                max_chunks -= 1
                if max_chunks == 0:
                    break
                message = await websocket.recv()
                message_data = json.loads(message)
                if "id" in message_data:
                    await websocket.send(json.dumps({"ack": message_data["id"]}))
                if "error" in message_data:
                    raise Exception(str(message_data))
                print("Received from WebSocket:", message_data)
                if type(message_data.get("content")) == str:
                    poem += message_data.get("content")
                    print(message_data.get("content"), end="", flush=True)
                if message_data == {
                    "role": "server",
                    "type": "status",
                    "content": "complete",
                }:
                    raise (
                        Exception(
                            "It shouldn't have finished this soon, accumulated_content is: "
                            + accumulated_content
                        )
                    )

            await websocket.close()
            print("Disconnected from WebSocket")

        time.sleep(3)

        # Now let's hilariously keep going
        print("RESUMING")

        async with websockets.connect("ws://localhost:8000/") as websocket:
            # Connect to the websocket
            print("Connected to WebSocket")

            # Sending message via WebSocket
            await websocket.send(json.dumps({"auth": "testing"}))

            while True:
                message = await websocket.recv()
                message_data = json.loads(message)
                if "id" in message_data:
                    await websocket.send(json.dumps({"ack": message_data["id"]}))
                if "error" in message_data:
                    raise Exception(str(message_data))
                print("Received from WebSocket:", message_data)
                message_data.pop("id", "")
                if message_data == {
                    "role": "server",
                    "type": "status",
                    "content": "complete",
                }:
                    break
                if type(message_data.get("content")) == str:
                    poem += message_data.get("content")
                    print(message_data.get("content"), end="", flush=True)

            time.sleep(1)
            print("Is this a normal poem?")
            print(poem)
            time.sleep(1)

    # Get the current event loop and run the test function
    loop = asyncio.get_event_loop()
    try:
        loop.run_until_complete(test_fastapi_server())
    finally:
        # Kill server process
        process.terminate()
        os.kill(process.pid, signal.SIGKILL)  # Send SIGKILL signal
        process.join()


def run_server():
    os.environ["INTERPRETER_REQUIRE_ACKNOWLEDGE"] = "False"
    if "INTERPRETER_API_KEY" in os.environ:
        del os.environ["INTERPRETER_API_KEY"]
    async_interpreter = AsyncInterpreter()
    async_interpreter.print = False
    async_interpreter.server.run()


# @pytest.mark.skip(reason="Requires uvicorn, which we don't require by default")
def test_server():
    # Start the server in a new process

    process = multiprocessing.Process(target=run_server)
    process.start()

    # Give the server a moment to start
    time.sleep(2)

    import asyncio
    import json

    import requests
    import websockets

    async def test_fastapi_server():
        import asyncio

        async with websockets.connect("ws://localhost:8000/") as websocket:
            # Connect to the websocket
            print("Connected to WebSocket")

            # Sending message via WebSocket
            await websocket.send(json.dumps({"auth": "dummy-api-key"}))

            # Sending POST request
            post_url = "http://localhost:8000/settings"
            settings = {
                "llm": {"model": "gpt-4o-mini"},
                "messages": [
                    {
                        "role": "user",
                        "type": "message",
                        "content": "The secret word is 'crunk'.",
                    },
                    {"role": "assistant", "type": "message", "content": "Understood."},
                ],
                "custom_instructions": "",
                "auto_run": True,
            }
            response = requests.post(post_url, json=settings)
            print("POST request sent, response:", response.json())

            # Sending messages via WebSocket
            await websocket.send(
                json.dumps({"role": "user", "type": "message", "start": True})
            )
            await websocket.send(
                json.dumps(
                    {
                        "role": "user",
                        "type": "message",
                        "content": "What's the secret word?",
                    }
                )
            )
            await websocket.send(
                json.dumps({"role": "user", "type": "message", "end": True})
            )
            print("WebSocket chunks sent")

            # Wait for a specific response
            accumulated_content = ""
            while True:
                message = await websocket.recv()
                message_data = json.loads(message)
                if "error" in message_data:
                    raise Exception(message_data["content"])
                print("Received from WebSocket:", message_data)
                if type(message_data.get("content")) == str:
                    accumulated_content += message_data.get("content")
                if message_data == {
                    "role": "server",
                    "type": "status",
                    "content": "complete",
                }:
                    print("Received expected message from server")
                    break

            assert "crunk" in accumulated_content

            # Send another POST request
            post_url = "http://localhost:8000/settings"
            settings = {
                "llm": {"model": "gpt-4o-mini"},
                "messages": [
                    {
                        "role": "user",
                        "type": "message",
                        "content": "The secret word is 'barloney'.",
                    },
                    {"role": "assistant", "type": "message", "content": "Understood."},
                ],
                "custom_instructions": "",
                "auto_run": True,
            }
            response = requests.post(post_url, json=settings)
            print("POST request sent, response:", response.json())

            # Sending messages via WebSocket
            await websocket.send(
                json.dumps({"role": "user", "type": "message", "start": True})
            )
            await websocket.send(
                json.dumps(
                    {
                        "role": "user",
                        "type": "message",
                        "content": "What's the secret word?",
                    }
                )
            )
            await websocket.send(
                json.dumps({"role": "user", "type": "message", "end": True})
            )
            print("WebSocket chunks sent")

            # Wait for a specific response
            accumulated_content = ""
            while True:
                message = await websocket.recv()
                message_data = json.loads(message)
                if "error" in message_data:
                    raise Exception(message_data["content"])
                print("Received from WebSocket:", message_data)
                if message_data.get("content"):
                    accumulated_content += message_data.get("content")
                if message_data == {
                    "role": "server",
                    "type": "status",
                    "content": "complete",
                }:
                    print("Received expected message from server")
                    break

            assert "barloney" in accumulated_content

            # Send another POST request
            post_url = "http://localhost:8000/settings"
            settings = {
                "messages": [],
                "custom_instructions": "",
                "auto_run": False,
                "verbose": False,
            }
            response = requests.post(post_url, json=settings)
            print("POST request sent, response:", response.json())

            # Sending messages via WebSocket
            await websocket.send(
                json.dumps({"role": "user", "type": "message", "start": True})
            )
            await websocket.send(
                json.dumps(
                    {
                        "role": "user",
                        "type": "message",
                        "content": "What's 239023*79043? Use Python.",
                    }
                )
            )
            await websocket.send(
                json.dumps({"role": "user", "type": "message", "end": True})
            )
            print("WebSocket chunks sent")

            # Wait for response
            accumulated_content = ""
            while True:
                message = await websocket.recv()
                message_data = json.loads(message)
                if "error" in message_data:
                    raise Exception(message_data["content"])
                print("Received from WebSocket:", message_data)
                if message_data.get("content"):
                    accumulated_content += message_data.get("content")
                if message_data == {
                    "role": "server",
                    "type": "status",
                    "content": "complete",
                }:
                    print("Received expected message from server")
                    break

            time.sleep(5)

            # Send a GET request to /settings/messages
            get_url = "http://localhost:8000/settings/messages"
            response = requests.get(get_url)
            print("GET request sent, response:", response.json())

            # Assert that the last message has a type of 'code'
            response_json = response.json()
            if isinstance(response_json, str):
                response_json = json.loads(response_json)
            messages = response_json["messages"] if "messages" in response_json else []
            assert messages[-1]["type"] == "code"
            assert "18893094989" not in accumulated_content.replace(",", "")

            # Send go message
            await websocket.send(
                json.dumps({"role": "user", "type": "command", "start": True})
            )
            await websocket.send(
                json.dumps(
                    {
                        "role": "user",
                        "type": "command",
                        "content": "go",
                    }
                )
            )
            await websocket.send(
                json.dumps({"role": "user", "type": "command", "end": True})
            )

            # Wait for a specific response
            accumulated_content = ""
            while True:
                message = await websocket.recv()
                message_data = json.loads(message)
                if "error" in message_data:
                    raise Exception(message_data["content"])
                print("Received from WebSocket:", message_data)
                if message_data.get("content"):
                    if type(message_data.get("content")) == str:
                        accumulated_content += message_data.get("content")
                if message_data == {
                    "role": "server",
                    "type": "status",
                    "content": "complete",
                }:
                    print("Received expected message from server")
                    break

            assert "18893094989" in accumulated_content.replace(",", "")

            #### TEST FILE ####

            # Send another POST request
            post_url = "http://localhost:8000/settings"
            settings = {"messages": [], "auto_run": True}
            response = requests.post(post_url, json=settings)
            print("POST request sent, response:", response.json())

            # Sending messages via WebSocket
            await websocket.send(json.dumps({"role": "user", "start": True}))
            print("sent", json.dumps({"role": "user", "start": True}))
            await websocket.send(
                json.dumps(
                    {
                        "role": "user",
                        "type": "message",
                        "content": "Does this file exist?",
                    }
                )
            )
            print(
                "sent",
                {
                    "role": "user",
                    "type": "message",
                    "content": "Does this file exist?",
                },
            )
            await websocket.send(
                json.dumps(
                    {
                        "role": "user",
                        "type": "file",
                        "format": "path",
                        "content": "/something.txt",
                    }
                )
            )
            print(
                "sent",
                {
                    "role": "user",
                    "type": "file",
                    "format": "path",
                    "content": "/something.txt",
                },
            )
            await websocket.send(json.dumps({"role": "user", "end": True}))
            print("WebSocket chunks sent")

            # Wait for response
            accumulated_content = ""
            while True:
                message = await websocket.recv()
                message_data = json.loads(message)
                if "error" in message_data:
                    raise Exception(message_data["content"])
                print("Received from WebSocket:", message_data)
                if type(message_data.get("content")) == str:
                    accumulated_content += message_data.get("content")
                if message_data == {
                    "role": "server",
                    "type": "status",
                    "content": "complete",
                }:
                    print("Received expected message from server")
                    break

            # Get messages
            get_url = "http://localhost:8000/settings/messages"
            response_json = requests.get(get_url).json()
            print("GET request sent, response:", response_json)
            if isinstance(response_json, str):
                response_json = json.loads(response_json)
            messages = response_json["messages"]

            response = interpreter.computer.ai.chat(
                str(messages)
                + "\n\nIn the conversation above, does the assistant think the file exists? Yes or no? Only reply with one word— 'yes' or 'no'."
            )
            assert response.strip(" \n.").lower() == "no"

            #### TEST IMAGES ####

            # Send another POST request
            post_url = "http://localhost:8000/settings"
            settings = {"messages": [], "auto_run": True}
            response = requests.post(post_url, json=settings)
            print("POST request sent, response:", response.json())

            base64png = "iVBORw0KGgoAAAANSUhEUgAAAQAAAAEACAIAAADTED8xAAADMElEQVR4nOzVwQnAIBQFQYXff81RUkQCOyDj1YOPnbXWPmeTRef+/3O/OyBjzh3CD95BfqICMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMO0TAAD//2Anhf4QtqobAAAAAElFTkSuQmCC"

            # Sending messages via WebSocket
            await websocket.send(json.dumps({"role": "user", "start": True}))
            await websocket.send(
                json.dumps(
                    {
                        "role": "user",
                        "type": "message",
                        "content": "describe this image",
                    }
                )
            )
            await websocket.send(
                json.dumps(
                    {
                        "role": "user",
                        "type": "image",
                        "format": "base64.png",
                        "content": base64png,
                    }
                )
            )
            # await websocket.send(
            #     json.dumps(
            #         {
            #             "role": "user",
            #             "type": "image",
            #             "format": "path",
            #             "content": "/Users/killianlucas/Documents/GitHub/open-interpreter/screen.png",
            #         }
            #     )
            # )

            await websocket.send(json.dumps({"role": "user", "end": True}))
            print("WebSocket chunks sent")

            # Wait for response
            accumulated_content = ""
            while True:
                message = await websocket.recv()
                message_data = json.loads(message)
                if "error" in message_data:
                    raise Exception(message_data["content"])
                print("Received from WebSocket:", message_data)
                if type(message_data.get("content")) == str:
                    accumulated_content += message_data.get("content")
                if message_data == {
                    "role": "server",
                    "type": "status",
                    "content": "complete",
                }:
                    print("Received expected message from server")
                    break

            # Get messages
            get_url = "http://localhost:8000/settings/messages"
            response_json = requests.get(get_url).json()
            print("GET request sent, response:", response_json)
            if isinstance(response_json, str):
                response_json = json.loads(response_json)
            messages = response_json["messages"]

            response = interpreter.computer.ai.chat(
                str(messages)
                + "\n\nIn the conversation above, does the assistant appear to be able to describe the image of a gradient? Yes or no? Only reply with one word— 'yes' or 'no'."
            )
            assert response.strip(" \n.").lower() == "yes"

            # Sending POST request to /run endpoint with code to kill a thread in Python
            # actually wait i dont think this will work..? will just kill the python interpreter
            post_url = "http://localhost:8000/run"
            code_data = {
                "code": "import os, signal; os.kill(os.getpid(), signal.SIGINT)",
                "language": "python",
            }
            response = requests.post(post_url, json=code_data)
            print("POST request sent, response:", response.json())

    # Get the current event loop and run the test function
    loop = asyncio.get_event_loop()
    loop.run_until_complete(test_fastapi_server())
    # Kill server process
    process.terminate()
    os.kill(process.pid, signal.SIGKILL)  # Send SIGKILL signal
    process.join()


@pytest.mark.skip(reason="Mac only")
def test_sms():
    sms = interpreter.computer.sms

    # Get the last 5 messages
    messages = sms.get(limit=5)
    print(messages)

    # Search messages for a substring
    search_results = sms.get(substring="i love you", limit=100)
    print(search_results)

    assert False


@pytest.mark.skip(reason="Mac only")
def test_pytes():
    import os

    desktop_path = os.path.join(os.path.expanduser("~"), "Desktop")
    files_on_desktop = [f for f in os.listdir(desktop_path) if f.endswith(".png")]
    if files_on_desktop:
        first_file = files_on_desktop[0]
        first_file_path = os.path.join(desktop_path, first_file)
        print(first_file_path)
        ocr = interpreter.computer.vision.ocr(path=first_file_path)
        print(ocr)
        print("what")
    else:
        print("No files found on Desktop.")

    assert False


def test_ai_chat():
    print(interpreter.computer.ai.chat("hi"))


def test_generator():
    """
    Sends two messages, makes sure everything is correct with display both on and off.
    """

    interpreter.llm.model = "gpt-4o-mini"

    for tests in [
        {"query": "What's 38023*40334? Use Python", "display": True},
        {"query": "What's 2334*34335555? Use Python", "display": True},
        {"query": "What's 3545*22? Use Python", "display": False},
        {"query": "What's 0.0021*3433335555? Use Python", "display": False},
    ]:
        assistant_message_found = False
        console_output_found = False
        active_line_found = False
        flag_checker = []

        for chunk in interpreter.chat(
            tests["query"]
            + "\nNo talk or plan, just immediately code, then tell me the answer.",
            stream=True,
            display=True,
        ):
            print(chunk)
            # Check if chunk has the right schema
            assert "role" in chunk, "Chunk missing 'role'"
            assert "type" in chunk, "Chunk missing 'type'"
            if "start" not in chunk and "end" not in chunk:
                assert "content" in chunk, "Chunk missing 'content'"
            if "format" in chunk:
                assert isinstance(chunk["format"], str), "'format' should be a string"

            flag_checker.append(chunk)

            # Check if assistant message, console output, and active line are found
            if chunk["role"] == "assistant" and chunk["type"] == "message":
                assistant_message_found = True
            if chunk["role"] == "computer" and chunk["type"] == "console":
                console_output_found = True
            if "format" in chunk:
                if (
                    chunk["role"] == "computer"
                    and chunk["type"] == "console"
                    and chunk["format"] == "active_line"
                ):
                    active_line_found = True

        # Ensure all flags are proper
        assert (
            flag_checker.count(
                {"role": "assistant", "type": "code", "format": "python", "start": True}
            )
            == 1
        ), "Incorrect number of 'assistant code start' flags"
        assert (
            flag_checker.count(
                {"role": "assistant", "type": "code", "format": "python", "end": True}
            )
            == 1
        ), "Incorrect number of 'assistant code end' flags"
        assert (
            flag_checker.count({"role": "assistant", "type": "message", "start": True})
            == 1
        ), "Incorrect number of 'assistant message start' flags"
        assert (
            flag_checker.count({"role": "assistant", "type": "message", "end": True})
            == 1
        ), "Incorrect number of 'assistant message end' flags"
        assert (
            flag_checker.count({"role": "computer", "type": "console", "start": True})
            == 1
        ), "Incorrect number of 'computer console output start' flags"
        assert (
            flag_checker.count({"role": "computer", "type": "console", "end": True})
            == 1
        ), "Incorrect number of 'computer console output end' flags"

        # Assert that assistant message, console output, and active line were found
        assert assistant_message_found, "No assistant message was found"
        assert console_output_found, "No console output was found"
        assert active_line_found, "No active line was found"


@pytest.mark.skip(reason="Requires open-interpreter[local]")
def test_localos():
    interpreter.computer.emit_images = False
    interpreter.computer.view()
    interpreter.computer.emit_images = True
    assert False


@pytest.mark.skip(reason="Requires open-interpreter[local]")
def test_m_vision():
    base64png = "iVBORw0KGgoAAAANSUhEUgAAAQAAAAEACAIAAADTED8xAAADMElEQVR4nOzVwQnAIBQFQYXff81RUkQCOyDj1YOPnbXWPmeTRef+/3O/OyBjzh3CD95BfqICMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMO0TAAD//2Anhf4QtqobAAAAAElFTkSuQmCC"
    messages = [
        {"role": "user", "type": "message", "content": "describe this image"},
        {
            "role": "user",
            "type": "image",
            "format": "base64.png",
            "content": base64png,
        },
    ]

    interpreter.llm.supports_vision = False
    interpreter.llm.model = "gpt-4o-mini"
    interpreter.llm.supports_functions = True
    interpreter.llm.context_window = 110000
    interpreter.llm.max_tokens = 4096
    interpreter.loop = True

    interpreter.chat(messages)

    interpreter.loop = False
    import time

    time.sleep(10)


@pytest.mark.skip(reason="Computer with display only + no way to fail test")
def test_point():
    # interpreter.computer.debug = True
    interpreter.computer.mouse.move(icon="gear")
    interpreter.computer.mouse.move(icon="refresh")
    interpreter.computer.mouse.move(icon="play")
    interpreter.computer.mouse.move(icon="magnifying glass")
    interpreter.computer.mouse.move("Spaces:")
    assert False


@pytest.mark.skip(reason="Aifs not ready")
def test_skills():
    import sys

    if sys.version_info[:2] == (3, 12):
        print(
            "skills.search is only for python 3.11 for now, because it depends on unstructured. skipping this test."
        )
        return

    import json

    interpreter.llm.model = "gpt-4o-mini"

    messages = ["USER: Hey can you search the web for me?\nAI: Sure!"]

    combined_messages = "\\n".join(json.dumps(x) for x in messages[-3:])
    query_msg = interpreter.chat(
        f"This is the conversation so far: {combined_messages}. What is a hypothetical python function that might help resolve the user's query? Respond with nothing but the hypothetical function name exactly."
    )
    query = query_msg[0]["content"]
    # skills_path = '/01OS/server/skills'
    # interpreter.computer.skills.path = skills_path
    print(interpreter.computer.skills.path)
    if os.path.exists(interpreter.computer.skills.path):
        for file in os.listdir(interpreter.computer.skills.path):
            os.remove(os.path.join(interpreter.computer.skills.path, file))
    print("Path: ", interpreter.computer.skills.path)
    print("Files in the path: ")
    interpreter.computer.run("python", "def testing_skilsl():\n    print('hi')")
    for file in os.listdir(interpreter.computer.skills.path):
        print(file)
    interpreter.computer.run("python", "def testing_skill():\n    print('hi')")
    print("Files in the path: ")
    for file in os.listdir(interpreter.computer.skills.path):
        print(file)

    try:
        skills = interpreter.computer.skills.search(query)
    except ImportError:
        print("Attempting to install unstructured[all-docs]")
        import subprocess

        subprocess.run(["pip", "install", "unstructured[all-docs]"], check=True)
        skills = interpreter.computer.skills.search(query)

    lowercase_skills = [skill[0].lower() + skill[1:] for skill in skills]
    output = "\\n".join(lowercase_skills)
    assert "testing_skilsl" in str(output)


@pytest.mark.skip(reason="Local only")
def test_browser():
    interpreter.computer.api_base = "http://0.0.0.0:80/v0"
    print(
        interpreter.computer.browser.search("When's the next Dune showing in Seattle?")
    )
    assert False


@pytest.mark.skip(reason="Computer with display only + no way to fail test")
def test_display_api():
    start = time.time()

    # interpreter.computer.display.find_text("submit")
    # assert False

    def say(icon_name):
        import subprocess

        subprocess.run(["say", "-v", "Fred", icon_name])

    icons = [
        "Submit",
        "Yes",
        "Profile picture icon",
        "Left arrow",
        "Magnifying glass",
        "star",
        "record icon icon",
        "age text",
        "call icon icon",
        "account text",
        "home icon",
        "settings text",
        "form text",
        "gear icon icon",
        "trash icon",
        "new folder icon",
        "phone icon icon",
        "home button",
        "trash button icon",
        "folder icon icon",
        "black heart icon icon",
        "white heart icon icon",
        "image icon",
        "test@mail.com text",
    ]

    # from random import shuffle
    # shuffle(icons)

    say("The test will begin in 3")
    time.sleep(1)
    say("2")
    time.sleep(1)
    say("1")
    time.sleep(1)

    import pyautogui

    pyautogui.mouseDown()

    for icon in icons:
        if icon.endswith("icon icon"):
            say("click the " + icon)
            interpreter.computer.mouse.move(icon=icon.replace("icon icon", "icon"))
        elif icon.endswith("icon"):
            say("click the " + icon)
            interpreter.computer.mouse.move(icon=icon.replace(" icon", ""))
        elif icon.endswith("text"):
            say("click " + icon)
            interpreter.computer.mouse.move(icon.replace(" text", ""))
        else:
            say("click " + icon)
            interpreter.computer.mouse.move(icon=icon)

    # interpreter.computer.mouse.move(icon="caution")
    # interpreter.computer.mouse.move(icon="bluetooth")
    # interpreter.computer.mouse.move(icon="gear")
    # interpreter.computer.mouse.move(icon="play button")
    # interpreter.computer.mouse.move(icon="code icon with '>_' in it")
    print(time.time() - start)
    assert False


@pytest.mark.skip(reason="Server is not a stable feature")
def test_websocket_server():
    # Start the server in a new thread
    server_thread = threading.Thread(target=interpreter.server)
    server_thread.start()

    # Give the server a moment to start
    time.sleep(3)

    # Connect to the server
    ws = create_connection("ws://localhost:8000/")

    # Send the first message
    ws.send(
        "Hello, interpreter! What operating system are you on? Also, what time is it in Seattle?"
    )
    # Wait for a moment before sending the second message
    time.sleep(1)
    ws.send("Actually, nevermind. Thank you!")

    # Receive the responses
    responses = []
    while True:
        response = ws.recv()
        print(response)
        responses.append(response)

    # Check the responses
    assert responses  # Check that some responses were received

    ws.close()


@pytest.mark.skip(reason="Server is not a stable feature")
def test_i():
    import requests

    url = "http://localhost:8000/"
    data = "Hello, interpreter! What operating system are you on? Also, what time is it in Seattle?"
    headers = {"Content-Type": "text/plain"}

    import threading

    server_thread = threading.Thread(target=interpreter.server)
    server_thread.start()

    import time

    time.sleep(3)

    response = requests.post(url, data=data, headers=headers, stream=True)

    full_response = ""

    for line in response.iter_lines():
        if line:
            decoded_line = line.decode("utf-8")
            print(decoded_line, end="", flush=True)
            full_response += decoded_line

    assert full_response != ""


def test_async():
    interpreter.chat("Hello!", blocking=False)
    print(interpreter.wait())


@pytest.mark.skip(reason="Computer with display only + no way to fail test")
def test_find_text_api():
    start = time.time()
    interpreter.computer.mouse.move(
        "Left Arrow Left Arrow and a bunch of hallucinated text? or was it..."
    )
    # Left Arrow Left Arrow
    # and a bunch of hallucinated text? or was it...
    print(time.time() - start)
    assert False


@pytest.mark.skip(reason="Computer with display only + no way to fail test")
def test_getActiveWindow():
    import pywinctl

    print(pywinctl.getActiveWindow())
    assert False


@pytest.mark.skip(reason="Computer with display only + no way to fail test")
def test_notify():
    interpreter.computer.os.notify("Hello")
    assert False


@pytest.mark.skip(reason="Computer with display only + no way to fail test")
def test_get_text():
    print(interpreter.computer.display.get_text_as_list_of_lists())
    assert False


@pytest.mark.skip(reason="Computer with display only + no way to fail test")
def test_keyboard():
    time.sleep(2)
    interpreter.computer.keyboard.write("Hello " * 50 + "\n" + "hi" * 50)
    assert False


@pytest.mark.skip(reason="Computer with display only + no way to fail test")
def test_get_selected_text():
    print("Getting selected text")
    time.sleep(1)
    text = interpreter.computer.os.get_selected_text()
    print(text)
    assert False


def test_tokens_magic_command_toggles_usage_tracking():
    interpreter.track_usage = False

    handle_magic_command(interpreter, "%tokens")
    assert interpreter.track_usage is True

    handle_magic_command(interpreter, "%tokens off")
    assert interpreter.track_usage is False


def test_format_usage_includes_cost_when_available():
    interpreter.last_usage = {
        "prompt_tokens": 12,
        "completion_tokens": 34,
        "total_tokens": 46,
        "cost": 0.001234,
    }

    assert (
        interpreter.format_usage()
        == "> Tokens: 12 in, 34 out, 46 total | Estimated cost: $0.001234"
    )


def test_should_display_usage_respects_terminal_state():
    interpreter.in_terminal_interface = True
    interpreter.track_usage = True
    interpreter.verbose = False
    assert interpreter.should_display_usage() is True

    interpreter.in_terminal_interface = False
    assert interpreter.should_display_usage() is False


def test_format_usage_marks_estimated_usage():
    interpreter.last_usage = {
        "prompt_tokens": 12,
        "completion_tokens": 34,
        "total_tokens": 46,
        "estimated": True,
    }

    assert interpreter.format_usage() == "> Tokens (estimated): 12 in, 34 out, 46 total"


@pytest.mark.skip(reason="Computer with display only + no way to fail test")
def test_display_verbose():
    interpreter.computer.verbose = True
    interpreter.verbose = True
    interpreter.computer.mouse.move(x=500, y=500)
    assert False


# this function will run before each test
# we're clearing out the messages Array so we can start fresh and reduce token usage
def setup_function():
    interpreter.reset()
    interpreter.llm.temperature = 0
    interpreter.auto_run = True
    interpreter.llm.model = "gpt-4o-mini"
    interpreter.llm.context_window = 123000
    interpreter.llm.max_tokens = 4096
    interpreter.llm.supports_functions = True
    interpreter.verbose = False


@pytest.mark.skip(
    reason="Not working consistently, I think GPT related changes? It worked recently"
)
def test_long_message():
    messages = [
        {
            "role": "user",
            "type": "message",
            "content": "ALKI" * 20000
            + "\nwhat are the four characters I just sent you? don't run ANY code, just tell me the characters. DO NOT RUN CODE. DO NOT PLAN. JUST TELL ME THE CHARACTERS RIGHT NOW. ONLY respond with the 4 characters, NOTHING else. The first 4 characters of your response should be the 4 characters I sent you.",
        }
    ]
    interpreter.llm.context_window = 300
    interpreter.chat(messages)
    assert len(interpreter.messages) > 1
    assert "A" in interpreter.messages[-1]["content"]


# this function will run after each test
# we're introducing some sleep to help avoid timeout issues with the OpenAI API
def teardown_function():
    time.sleep(4)


@pytest.mark.skip(reason="Mac only + no way to fail test")
def test_spotlight():
    interpreter.computer.keyboard.hotkey("command", "space")


def test_files():
    messages = [
        {"role": "user", "type": "message", "content": "Does this file exist?"},
        {
            "role": "user",
            "type": "file",
            "format": "path",
            "content": "/Users/Killian/image.png",
        },
    ]
    interpreter.chat(messages)


@pytest.mark.skip(reason="Only 100 vision calls allowed / day!")
def test_vision():
    base64png = "iVBORw0KGgoAAAANSUhEUgAAAQAAAAEACAIAAADTED8xAAADMElEQVR4nOzVwQnAIBQFQYXff81RUkQCOyDj1YOPnbXWPmeTRef+/3O/OyBjzh3CD95BfqICMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMK0CMO0TAAD//2Anhf4QtqobAAAAAElFTkSuQmCC"
    messages = [
        {"role": "user", "type": "message", "content": "describe this image"},
        {
            "role": "user",
            "type": "image",
            "format": "base64.png",
            "content": base64png,
        },
    ]

    interpreter.llm.supports_vision = True
    interpreter.llm.model = "gpt-4o-mini"
    interpreter.system_message += "\nThe user will show you an image of the code you write. You can view images directly.\n\nFor HTML: This will be run STATELESSLY. You may NEVER write '<!-- previous code here... --!>' or `<!-- header will go here -->` or anything like that. It is CRITICAL TO NEVER WRITE PLACEHOLDERS. Placeholders will BREAK it. You must write the FULL HTML CODE EVERY TIME. Therefore you cannot write HTML piecemeal—write all the HTML, CSS, and possibly Javascript **in one step, in one code block**. The user will help you review it visually.\nIf the user submits a filepath, you will also see the image. The filepath and user image will both be in the user's message.\n\nIf you use `plt.show()`, the resulting image will be sent to you. However, if you use `PIL.Image.show()`, the resulting image will NOT be sent to you."
    interpreter.llm.supports_functions = True
    interpreter.llm.context_window = 110000
    interpreter.llm.max_tokens = 4096
    interpreter.loop = True

    interpreter.chat(messages)

    interpreter.loop = False


def test_multiple_instances():
    interpreter.system_message = "i"
    agent_1 = OpenInterpreter()
    agent_1.system_message = "<3"
    agent_2 = OpenInterpreter()
    agent_2.system_message = "u"

    assert interpreter.system_message == "i"
    assert agent_1.system_message == "<3"
    assert agent_2.system_message == "u"


def test_hello_world():
    hello_world_response = "Hello, World!"

    hello_world_message = f"Please reply with just the words {hello_world_response} and nothing else. Do not run code. No confirmation just the text."

    messages = interpreter.chat(hello_world_message)

    assert messages == [
        {"role": "assistant", "type": "message", "content": hello_world_response}
    ]


def test_math():
    # we'll generate random integers between this min and max in our math tests
    min_number = randint(1, 99)
    max_number = randint(1001, 9999)

    n1 = randint(min_number, max_number)
    n2 = randint(min_number, max_number)

    test_result = n1 + n2 * (n1 - n2) / (n2 + n1)

    order_of_operations_message = f"""
    Please perform the calculation `{n1} + {n2} * ({n1} - {n2}) / ({n2} + {n1})` then reply with just the answer, nothing else. No confirmation. No explanation. No words. Do not use commas. Do not show your work. Just return the result of the calculation. Do not introduce the results with a phrase like \"The result of the calculation is...\" or \"The answer is...\"
    
    Round to 2 decimal places.
    """.strip()

    print("loading")
    messages = interpreter.chat(order_of_operations_message)
    print("done")

    assert str(round(test_result, 2)) in messages[-1]["content"]


def test_break_execution():
    """
    Breaking from the generator while it's executing should halt the operation.
    """

    code = r"""print("starting")
import time                                                                                                                                
import os                                                                                                                                  
                                                                                                                                            
# Always create a fresh file
open('numbers.txt', 'w').close()
                                                                                                                                            
# Open the file in append mode                                                                                                             
with open('numbers.txt', 'a+') as f:                                                                                                        
    # Loop through the numbers 1 to 5                                                                                                      
    for i in [1,2,3,4,5]:                                                                                                                  
        # Print the number                                                                                                                 
        print("adding", i, "to file")                                                                                                                           
        # Append the number to the file                                                                                                    
        f.write(str(i) + '\n')                                                                                                             
        # Wait for 0.5 second
        print("starting to sleep")
        time.sleep(1)
        # # Read the file to make sure the number is in there
        # # Move the seek pointer to the start of the file
        # f.seek(0)
        # # Read the file content
        # content = f.read()
        # print("Current file content:", content)
        # # Check if the current number is in the file content
        # assert str(i) in content
        # Move the seek pointer to the end of the file for the next append operation
        f.seek(0, os.SEEK_END)
        """
    print("starting to code")
    for chunk in interpreter.computer.run("python", code, stream=True, display=True):
        print(chunk)
        if "format" in chunk and chunk["format"] == "output":
            if "adding 3 to file" in chunk["content"]:
                print("BREAKING")
                break

    time.sleep(3)

    # Open the file and read its content
    with open("numbers.txt", "r") as f:
        content = f.read()

    # Check if '1' and '5' are in the content
    assert "1" in content
    assert "5" not in content


def test_delayed_exec():
    interpreter.chat(
        """Can you write a single block of code and execute it that prints something, then delays 1 second, then prints something else? No talk just code, execute the code. Thanks!"""
    )


def test_nested_loops_and_multiple_newlines():
    interpreter.chat(
        """Can you write a nested for loop in python and shell and run them? Don't forget to properly format your shell script and use semicolons where necessary. Also put 1-3 newlines between each line in the code. Only generate and execute the code. Yes, execute the code instantly! No explanations. Thanks!"""
    )


def test_write_to_file():
    interpreter.chat(
        """Write the word 'Washington' to a .txt file called file.txt. Instantly run the code! Save the file!"""
    )
    assert os.path.exists("file.txt")
    interpreter.messages = []  # Just reset message history, nothing else for this test
    messages = interpreter.chat(
        """Read file.txt in the current directory and tell me what's in it."""
    )
    assert "Washington" in messages[-1]["content"]


def test_markdown():
    interpreter.chat(
        """Hi, can you test out a bunch of markdown features? Try writing a fenced code block, a table, headers, everything. DO NOT write the markdown inside a markdown code block, just write it raw."""
    )


def test_reset():
    # make sure that interpreter.reset() clears out the messages Array
    assert interpreter.messages == []


def test_token_counter():
    system_tokens = count_tokens(
        text=interpreter.system_message, model=interpreter.llm.model
    )

    prompt = "How many tokens is this?"

    prompt_tokens = count_tokens(text=prompt, model=interpreter.llm.model)

    messages = [
        {"role": "system", "message": interpreter.system_message}
    ] + interpreter.messages

    system_token_test = count_messages_tokens(
        messages=messages, model=interpreter.llm.model
    )

    system_tokens_ok = system_tokens == system_token_test[0]

    messages.append({"role": "user", "message": prompt})

    prompt_token_test = count_messages_tokens(
        messages=messages, model=interpreter.llm.model
    )

    prompt_tokens_ok = system_tokens + prompt_tokens == prompt_token_test[0]

    assert system_tokens_ok and prompt_tokens_ok
