import asyncio
import json
import os
import shutil
import socket
import threading
import time
import traceback
from collections import deque
from datetime import datetime
from typing import Any, Dict, List, Optional, Union

import shortuuid
from pydantic import BaseModel
from starlette.websockets import WebSocketState

from .core import OpenInterpreter

last_start_time = 0

try:
    import janus
    import uvicorn
    from fastapi import (
        APIRouter,
        FastAPI,
        File,
        Form,
        HTTPException,
        Request,
        UploadFile,
        WebSocket,
    )
    from fastapi.responses import JSONResponse, PlainTextResponse, StreamingResponse
    from starlette.status import HTTP_403_FORBIDDEN
except:
    # Server dependencies are not required by the main package.
    pass


complete_message = {"role": "server", "type": "status", "content": "complete"}


class AsyncInterpreter(OpenInterpreter):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)

        self.respond_thread = None
        self.stop_event = threading.Event()
        self.output_queue = None
        self.unsent_messages = deque()
        self.id = os.getenv("INTERPRETER_ID", datetime.now().timestamp())
        self.print = False  # Will print output

        self.require_acknowledge = (
            os.getenv("INTERPRETER_REQUIRE_ACKNOWLEDGE", "False").lower() == "true"
        )
        self.acknowledged_outputs = []

        self.server = Server(self)

        # For the 01. This lets the OAI compatible server accumulate context before responding.
        self.context_mode = False

    async def input(self, chunk):
        """
        Accumulates LMC chunks onto interpreter.messages.
        When it hits an "end" flag, calls interpreter.respond().
        """

        if "start" in chunk:
            # If the user is starting something, the interpreter should stop.
            if self.respond_thread is not None and self.respond_thread.is_alive():
                self.stop_event.set()
                self.respond_thread.join()
            self.accumulate(chunk)
        elif "content" in chunk:
            self.accumulate(chunk)
        elif "end" in chunk:
            # If the user is done talking, the interpreter should respond.

            run_code = None  # Will later default to auto_run unless the user makes a command here

            # But first, process any commands.
            if self.messages[-1].get("type") == "command":
                command = self.messages[-1]["content"]
                self.messages = self.messages[:-1]

                if command == "stop":
                    # Any start flag would have stopped it a moment ago, but to be sure:
                    self.stop_event.set()
                    self.respond_thread.join()
                    return
                if command == "go":
                    # This is to approve code.
                    run_code = True
                    pass

            self.stop_event.clear()
            self.respond_thread = threading.Thread(
                target=self.respond, args=(run_code,)
            )
            self.respond_thread.start()

    async def output(self):
        if self.output_queue == None:
            self.output_queue = janus.Queue()
        return await self.output_queue.async_q.get()

    def respond(self, run_code=None):
        for attempt in range(5):  # 5 attempts
            try:
                if run_code == None:
                    run_code = self.auto_run

                sent_chunks = False

                for chunk_og in self._respond_and_store():
                    chunk = (
                        chunk_og.copy()
                    )  # This fixes weird double token chunks. Probably a deeper problem?

                    if chunk["type"] == "confirmation":
                        if run_code:
                            run_code = False
                            continue
                        else:
                            break

                    if self.stop_event.is_set():
                        return

                    if self.print:
                        if "start" in chunk:
                            print("\n")
                        if chunk["type"] in ["code", "console"] and "format" in chunk:
                            if "start" in chunk:
                                print(
                                    "\n------------\n\n```" + chunk["format"],
                                    flush=True,
                                )
                            if "end" in chunk:
                                print("\n```\n\n------------\n\n", flush=True)
                        if chunk.get("format") != "active_line":
                            if "format" in chunk and "base64" in chunk["format"]:
                                print("\n[An image was produced]")
                            else:
                                content = chunk.get("content", "")
                                content = (
                                    str(content)
                                    .encode("ascii", "ignore")
                                    .decode("ascii")
                                )
                                print(content, end="", flush=True)

                    if self.debug:
                        print("Interpreter produced this chunk:", chunk)

                    self.output_queue.sync_q.put(chunk)
                    sent_chunks = True

                if not sent_chunks:
                    print("ERROR. NO CHUNKS SENT. TRYING AGAIN.")
                    print("Messages:", self.messages)
                    messages = [
                        "Hello? Answer please.",
                        "Just say something, anything.",
                        "Are you there?",
                        "Can you respond?",
                        "Please reply.",
                    ]
                    self.messages.append(
                        {
                            "role": "user",
                            "type": "message",
                            "content": messages[attempt % len(messages)],
                        }
                    )
                    time.sleep(1)
                else:
                    self.output_queue.sync_q.put(complete_message)
                    if self.debug:
                        print("\nServer response complete.\n")
                    return

            except Exception as e:
                error = traceback.format_exc() + "\n" + str(e)
                error_message = {
                    "role": "server",
                    "type": "error",
                    "content": traceback.format_exc() + "\n" + str(e),
                }
                self.output_queue.sync_q.put(error_message)
                self.output_queue.sync_q.put(complete_message)
                print("\n\n--- SENT ERROR: ---\n\n")
                print(error)
                print("\n\n--- (ERROR ABOVE WAS SENT) ---\n\n")
                return

        error_message = {
            "role": "server",
            "type": "error",
            "content": "No chunks sent or unknown error.",
        }
        self.output_queue.sync_q.put(error_message)
        self.output_queue.sync_q.put(complete_message)
        raise Exception("No chunks sent or unknown error.")

    def accumulate(self, chunk):
        """
        Accumulates LMC chunks onto interpreter.messages.
        """
        if type(chunk) == str:
            chunk = json.loads(chunk)

        if type(chunk) == dict:
            if chunk.get("format") == "active_line":
                # We don't do anything with these.
                pass

            elif "content" in chunk and not (
                len(self.messages) > 0
                and (
                    (
                        "type" in self.messages[-1]
                        and chunk.get("type") != self.messages[-1].get("type")
                    )
                    or (
                        "format" in self.messages[-1]
                        and chunk.get("format") != self.messages[-1].get("format")
                    )
                )
            ):
                if len(self.messages) == 0:
                    raise Exception(
                        "You must send a 'start: True' chunk first to create this message."
                    )
                # Append to an existing message
                if (
                    "type" not in self.messages[-1]
                ):  # It was created with a type-less start message
                    self.messages[-1]["type"] = chunk["type"]
                if (
                    chunk.get("format") and "format" not in self.messages[-1]
                ):  # It was created with a type-less start message
                    self.messages[-1]["format"] = chunk["format"]
                if "content" not in self.messages[-1]:
                    self.messages[-1]["content"] = chunk["content"]
                else:
                    self.messages[-1]["content"] += chunk["content"]

            # elif "content" in chunk and (len(self.messages) > 0 and self.messages[-1] == {'role': 'user', 'start': True}):
            #     # Last message was {'role': 'user', 'start': True}. Just populate that with this chunk
            #     self.messages[-1] = chunk.copy()

            elif "start" in chunk or (
                len(self.messages) > 0
                and (
                    chunk.get("type") != self.messages[-1].get("type")
                    or chunk.get("format") != self.messages[-1].get("format")
                )
            ):
                # Create a new message
                chunk_copy = (
                    chunk.copy()
                )  # So we don't modify the original chunk, which feels wrong.
                if "start" in chunk_copy:
                    chunk_copy.pop("start")
                if "content" not in chunk_copy:
                    chunk_copy["content"] = ""
                self.messages.append(chunk_copy)

        elif type(chunk) == bytes:
            if self.messages[-1]["content"] == "":  # We initialize as an empty string ^
                self.messages[-1]["content"] = b""  # But it actually should be bytes
            self.messages[-1]["content"] += chunk


def authenticate_function(key):
    """
    This function checks if the provided key is valid for authentication.

    Returns True if the key is valid, False otherwise.
    """
    # Fetch the API key from the environment variables. If it's not set, return True.
    api_key = os.getenv("INTERPRETER_API_KEY", None)

    # If the API key is not set in the environment variables, return True.
    # Otherwise, check if the provided key matches the fetched API key.
    # Return True if they match, False otherwise.
    if api_key is None:
        return True
    else:
        return key == api_key


def create_router(async_interpreter):
    router = APIRouter()

    @router.get("/heartbeat")
    async def heartbeat():
        return {"status": "alive"}

    @router.get("/")
    async def home():
        return PlainTextResponse(
            """
            <!DOCTYPE html>
            <html>
            <head>
                <title>Chat</title>
            </head>
            <body>
                <form action="" onsubmit="sendMessage(event)">
                    <textarea id="messageInput" rows="10" cols="50" autocomplete="off"></textarea>
                    <button>Send</button>
                </form>
                <button id="approveCodeButton">Approve Code</button>
                <button id="authButton">Send Auth</button>
                <div id="messages"></div>
                <script>
                    var ws = new WebSocket("ws://"""
            + async_interpreter.server.host
            + ":"
            + str(async_interpreter.server.port)
            + """/");
                    var lastMessageElement = null;

                    ws.onmessage = function(event) {

                        var eventData = JSON.parse(event.data);

                        """
            + (
                """
                        
                        // Acknowledge receipt
                        var acknowledge_message = {
                            "ack": eventData.id
                        };
                        ws.send(JSON.stringify(acknowledge_message));

                        """
                if async_interpreter.require_acknowledge
                else ""
            )
            + """

                        if (lastMessageElement == null) {
                            lastMessageElement = document.createElement('p');
                            document.getElementById('messages').appendChild(lastMessageElement);
                            lastMessageElement.innerHTML = "<br>"
                        }

                        if ((eventData.role == "assistant" && eventData.type == "message" && eventData.content) ||
                            (eventData.role == "computer" && eventData.type == "console" && eventData.format == "output" && eventData.content) ||
                            (eventData.role == "assistant" && eventData.type == "code" && eventData.content)) {
                            lastMessageElement.innerHTML += eventData.content;
                        } else {
                            lastMessageElement.innerHTML += "<br><br>" + JSON.stringify(eventData) + "<br><br>";
                        }
                    };
                    function sendMessage(event) {
                        event.preventDefault();
                        var input = document.getElementById("messageInput");
                        var message = input.value;
                        if (message.startsWith('{') && message.endsWith('}')) {
                            message = JSON.stringify(JSON.parse(message));
                            ws.send(message);
                        } else {
                            var startMessageBlock = {
                                "role": "user",
                                //"type": "message",
                                "start": true
                            };
                            ws.send(JSON.stringify(startMessageBlock));

                            var messageBlock = {
                                "role": "user",
                                "type": "message",
                                "content": message
                            };
                            ws.send(JSON.stringify(messageBlock));

                            var endMessageBlock = {
                                "role": "user",
                                //"type": "message",
                                "end": true
                            };
                            ws.send(JSON.stringify(endMessageBlock));
                        }
                        var userMessageElement = document.createElement('p');
                        userMessageElement.innerHTML = '<b>' + input.value + '</b><br>';
                        document.getElementById('messages').appendChild(userMessageElement);
                        lastMessageElement = document.createElement('p');
                        document.getElementById('messages').appendChild(lastMessageElement);
                        input.value = '';
                    }
                function approveCode() {
                    var startCommandBlock = {
                        "role": "user",
                        "type": "command",
                        "start": true
                    };
                    ws.send(JSON.stringify(startCommandBlock));

                    var commandBlock = {
                        "role": "user",
                        "type": "command",
                        "content": "go"
                    };
                    ws.send(JSON.stringify(commandBlock));

                    var endCommandBlock = {
                        "role": "user",
                        "type": "command",
                        "end": true
                    };
                    ws.send(JSON.stringify(endCommandBlock));
                }
                function authenticate() {
                    var authBlock = {
                        "auth": "dummy-api-key"
                    };
                    ws.send(JSON.stringify(authBlock));
                }

                document.getElementById("approveCodeButton").addEventListener("click", approveCode);
                document.getElementById("authButton").addEventListener("click", authenticate);
                </script>
            </body>
            </html>
            """,
            media_type="text/html",
        )

    @router.websocket("/")
    async def websocket_endpoint(websocket: WebSocket):
        await websocket.accept()

        try:  # solving it ;)/ # killian super wrote this

            async def receive_input():
                authenticated = False
                while True:
                    try:
                        if websocket.client_state != WebSocketState.CONNECTED:
                            return
                        data = await websocket.receive()

                        if (
                            not authenticated
                            and os.getenv("INTERPRETER_REQUIRE_AUTH") != "False"
                        ):
                            if "text" in data:
                                data = json.loads(data["text"])
                                if "auth" in data:
                                    if async_interpreter.server.authenticate(
                                        data["auth"]
                                    ):
                                        authenticated = True
                                        await websocket.send_text(
                                            json.dumps({"auth": True})
                                        )
                            if not authenticated:
                                await websocket.send_text(json.dumps({"auth": False}))
                            continue

                        if data.get("type") == "websocket.receive":
                            if "text" in data:
                                data = json.loads(data["text"])
                                if (
                                    async_interpreter.require_acknowledge
                                    and "ack" in data
                                ):
                                    async_interpreter.acknowledged_outputs.append(
                                        data["ack"]
                                    )
                                    continue
                            elif "bytes" in data:
                                data = data["bytes"]
                            await async_interpreter.input(data)
                        elif data.get("type") == "websocket.disconnect":
                            print("Client wants to disconnect, that's fine..")
                            return
                        else:
                            print("Invalid data:", data)
                            continue

                    except Exception as e:
                        error = traceback.format_exc() + "\n" + str(e)
                        error_message = {
                            "role": "server",
                            "type": "error",
                            "content": traceback.format_exc() + "\n" + str(e),
                        }
                        if websocket.client_state == WebSocketState.CONNECTED:
                            await websocket.send_text(json.dumps(error_message))
                            await websocket.send_text(json.dumps(complete_message))
                            print("\n\n--- SENT ERROR: ---\n\n")
                        else:
                            print(
                                "\n\n--- ERROR (not sent due to disconnected state): ---\n\n"
                            )
                        print(error)
                        print("\n\n--- (ERROR ABOVE) ---\n\n")

            async def send_output():
                while True:
                    if websocket.client_state != WebSocketState.CONNECTED:
                        return
                    try:
                        # First, try to send any unsent messages
                        while async_interpreter.unsent_messages:
                            output = async_interpreter.unsent_messages[0]
                            if async_interpreter.debug:
                                print("This was unsent, sending it again:", output)

                            success = await send_message(output)
                            if success:
                                async_interpreter.unsent_messages.popleft()

                        # If we've sent all unsent messages, get a new output
                        if not async_interpreter.unsent_messages:
                            output = await async_interpreter.output()
                            success = await send_message(output)
                            if not success:
                                async_interpreter.unsent_messages.append(output)
                                if async_interpreter.debug:
                                    print(
                                        f"Added message to unsent_messages queue after failed attempts: {output}"
                                    )

                    except Exception as e:
                        error = traceback.format_exc() + "\n" + str(e)
                        error_message = {
                            "role": "server",
                            "type": "error",
                            "content": error,
                        }
                        async_interpreter.unsent_messages.append(error_message)
                        async_interpreter.unsent_messages.append(complete_message)
                        print("\n\n--- ERROR (will be sent when possible): ---\n\n")
                        print(error)
                        print(
                            "\n\n--- (ERROR ABOVE WILL BE SENT WHEN POSSIBLE) ---\n\n"
                        )

            async def send_message(output):
                if isinstance(output, dict) and "id" in output:
                    id = output["id"]
                else:
                    id = shortuuid.uuid()
                    if (
                        isinstance(output, dict)
                        and async_interpreter.require_acknowledge
                    ):
                        output["id"] = id

                for attempt in range(20):
                    # time.sleep(0.5)

                    if websocket.client_state != WebSocketState.CONNECTED:
                        return False

                    try:
                        # print("sending:", output)

                        if isinstance(output, bytes):
                            await websocket.send_bytes(output)
                            return True  # Haven't set up ack for this
                        else:
                            if async_interpreter.require_acknowledge:
                                output["id"] = id
                            if async_interpreter.debug:
                                print("Sending this over the websocket:", output)
                            await websocket.send_text(json.dumps(output))

                        if async_interpreter.require_acknowledge:
                            acknowledged = False
                            for _ in range(100):
                                if id in async_interpreter.acknowledged_outputs:
                                    async_interpreter.acknowledged_outputs.remove(id)
                                    acknowledged = True
                                    if async_interpreter.debug:
                                        print("This output was acknowledged:", output)
                                    break
                                await asyncio.sleep(0.0001)

                            if acknowledged:
                                return True
                            else:
                                if async_interpreter.debug:
                                    print("Acknowledgement not received for:", output)
                                return False
                        else:
                            return True

                    except Exception as e:
                        print(
                            f"Failed to send output on attempt number: {attempt + 1}. Output was: {output}"
                        )
                        print(f"Error: {str(e)}")
                        traceback.print_exc()
                        await asyncio.sleep(0.01)

                # If we've reached this point, we've failed to send after 100 attempts
                if output not in async_interpreter.unsent_messages:
                    print("Failed to send message:", output)
                else:
                    print(
                        "Failed to send message, also it was already in unsent queue???:",
                        output,
                    )

                return False

            await asyncio.gather(receive_input(), send_output())

        except Exception as e:
            error = traceback.format_exc() + "\n" + str(e)
            error_message = {
                "role": "server",
                "type": "error",
                "content": error,
            }
            async_interpreter.unsent_messages.append(error_message)
            async_interpreter.unsent_messages.append(complete_message)
            print("\n\n--- ERROR (will be sent when possible): ---\n\n")
            print(error)
            print("\n\n--- (ERROR ABOVE WILL BE SENT WHEN POSSIBLE) ---\n\n")

    # TODO
    @router.post("/")
    async def post_input(payload: Dict[str, Any]):
        try:
            async_interpreter.input(payload)
            return {"status": "success"}
        except Exception as e:
            return {"error": str(e)}, 500

    @router.post("/settings")
    async def set_settings(payload: Dict[str, Any]):
        for key, value in payload.items():
            print("Updating settings...")
            # print(f"Updating settings: {key} = {value}")
            if key in ["llm", "computer"] and isinstance(value, dict):
                if key == "auto_run":
                    return {
                        "error": f"The setting {key} is not modifiable through the server due to security constraints."
                    }, 403
                if hasattr(async_interpreter, key):
                    for sub_key, sub_value in value.items():
                        if hasattr(getattr(async_interpreter, key), sub_key):
                            setattr(getattr(async_interpreter, key), sub_key, sub_value)
                        else:
                            return {
                                "error": f"Sub-setting {sub_key} not found in {key}"
                            }, 404
                else:
                    return {"error": f"Setting {key} not found"}, 404
            elif hasattr(async_interpreter, key):
                setattr(async_interpreter, key, value)
            else:
                return {"error": f"Setting {key} not found"}, 404

        return {"status": "success"}

    @router.get("/settings/{setting}")
    async def get_setting(setting: str):
        if hasattr(async_interpreter, setting):
            setting_value = getattr(async_interpreter, setting)
            try:
                return json.dumps({setting: setting_value})
            except TypeError:
                return {"error": "Failed to serialize the setting value"}, 500
        else:
            return json.dumps({"error": "Setting not found"}), 404

    if os.getenv("INTERPRETER_INSECURE_ROUTES", "").lower() == "true":

        @router.post("/run")
        async def run_code(payload: Dict[str, Any]):
            language, code = payload.get("language"), payload.get("code")
            if not (language and code):
                return {"error": "Both 'language' and 'code' are required."}, 400
            try:
                print(f"Running {language}:", code)
                output = async_interpreter.computer.run(language, code)
                print("Output:", output)
                return {"output": output}
            except Exception as e:
                return {"error": str(e)}, 500

        @router.post("/upload")
        async def upload_file(file: UploadFile = File(...), path: str = Form(...)):
            try:
                with open(path, "wb") as output_file:
                    shutil.copyfileobj(file.file, output_file)
                return {"status": "success"}
            except Exception as e:
                return {"error": str(e)}, 500

        @router.get("/download/{filename}")
        async def download_file(filename: str):
            try:
                return StreamingResponse(
                    open(filename, "rb"), media_type="application/octet-stream"
                )
            except Exception as e:
                return {"error": str(e)}, 500

    ### OPENAI COMPATIBLE ENDPOINT

    class ChatMessage(BaseModel):
        role: str
        content: Union[str, List[Dict[str, Any]]]

    class ChatCompletionRequest(BaseModel):
        model: str = "default-model"
        messages: List[ChatMessage]
        max_tokens: Optional[int] = None
        temperature: Optional[float] = None
        stream: Optional[bool] = False

    async def openai_compatible_generator(run_code):
        if run_code:
            print("Running code.\n")
            for i, chunk in enumerate(async_interpreter._respond_and_store()):
                if "content" in chunk:
                    print(chunk["content"], end="")  # Sorry! Shitty display for now
                if "start" in chunk:
                    print("\n")

                output_content = None

                if chunk["type"] == "message" and "content" in chunk:
                    output_content = chunk["content"]
                if chunk["type"] == "code" and "start" in chunk:
                    output_content = "```" + chunk["format"] + "\n"
                if chunk["type"] == "code" and "content" in chunk:
                    output_content = chunk["content"]
                if chunk["type"] == "code" and "end" in chunk:
                    output_content = "\n```\n"

                if output_content:
                    await asyncio.sleep(0)
                    output_chunk = {
                        "id": i,
                        "object": "chat.completion.chunk",
                        "created": time.time(),
                        "model": "open-interpreter",
                        "choices": [{"delta": {"content": output_content}}],
                    }
                    yield f"data: {json.dumps(output_chunk)}\n\n"

            return

        made_chunk = False

        for message in [
            ".",
            "Just say something, anything.",
            "Hello? Answer please.",
            "Are you there?",
            "Can you respond?",
            "Please reply.",
        ]:
            for i, chunk in enumerate(
                async_interpreter.chat(message=message, stream=True, display=True)
            ):
                await asyncio.sleep(0)  # Yield control to the event loop
                made_chunk = True

                if (
                    chunk["type"] == "confirmation"
                    and async_interpreter.auto_run == False
                ):
                    await asyncio.sleep(0)
                    output_content = "Do you want to run this code?"
                    output_chunk = {
                        "id": i,
                        "object": "chat.completion.chunk",
                        "created": time.time(),
                        "model": "open-interpreter",
                        "choices": [{"delta": {"content": output_content}}],
                    }
                    yield f"data: {json.dumps(output_chunk)}\n\n"
                    break

                if async_interpreter.stop_event.is_set():
                    break

                output_content = None

                if chunk["type"] == "message" and "content" in chunk:
                    output_content = chunk["content"]
                if chunk["type"] == "code" and "start" in chunk:
                    output_content = "```" + chunk["format"] + "\n"
                if chunk["type"] == "code" and "content" in chunk:
                    output_content = chunk["content"]
                if chunk["type"] == "code" and "end" in chunk:
                    output_content = "\n```\n"

                if output_content:
                    await asyncio.sleep(0)
                    output_chunk = {
                        "id": i,
                        "object": "chat.completion.chunk",
                        "created": time.time(),
                        "model": "open-interpreter",
                        "choices": [{"delta": {"content": output_content}}],
                    }
                    yield f"data: {json.dumps(output_chunk)}\n\n"

            if made_chunk:
                break

    @router.post("/openai/chat/completions")
    async def chat_completion(request: ChatCompletionRequest):
        global last_start_time

        # Convert to LMC
        last_message = request.messages[-1]

        if last_message.role != "user":
            raise ValueError("Last message must be from the user.")

        if last_message.content == "{STOP}":
            # Handle special STOP token
            async_interpreter.stop_event.set()
            time.sleep(5)
            async_interpreter.stop_event.clear()
            return

        if last_message.content in ["{CONTEXT_MODE_ON}", "{REQUIRE_START_ON}"]:
            async_interpreter.context_mode = True
            return

        if last_message.content in ["{CONTEXT_MODE_OFF}", "{REQUIRE_START_OFF}"]:
            async_interpreter.context_mode = False
            return

        if last_message.content == "{AUTO_RUN_ON}":
            async_interpreter.auto_run = True
            return

        if last_message.content == "{AUTO_RUN_OFF}":
            async_interpreter.auto_run = False
            return

        run_code = False
        if (
            async_interpreter.messages
            and async_interpreter.messages[-1]["type"] == "code"
            and last_message.content.lower().strip(".!?").strip() == "yes"
        ):
            run_code = True
        elif type(last_message.content) == str:
            async_interpreter.messages.append(
                {
                    "role": "user",
                    "type": "message",
                    "content": last_message.content,
                }
            )
            print(">", last_message.content)
        elif type(last_message.content) == list:
            for content in last_message.content:
                if content["type"] == "text":
                    async_interpreter.messages.append(
                        {"role": "user", "type": "message", "content": str(content)}
                    )
                    print(">", content)
                elif content["type"] == "image_url":
                    if "url" not in content["image_url"]:
                        raise Exception("`url` must be in `image_url`.")
                    url = content["image_url"]["url"]
                    print("> [user sent an image]", url[:100])
                    if "base64," not in url:
                        raise Exception(
                            '''Image must be in the format: "data:image/jpeg;base64,{base64_image}"'''
                        )

                    # data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAA6oA...

                    data = url.split("base64,")[1]
                    format = "base64." + url.split(";")[0].split("/")[1]
                    async_interpreter.messages.append(
                        {
                            "role": "user",
                            "type": "image",
                            "format": format,
                            "content": data,
                        }
                    )

        else:
            if async_interpreter.context_mode:
                # In context mode, we only respond if we received a {START} message
                # Otherwise, we're just accumulating context
                if last_message.content == "{START}":
                    if async_interpreter.messages[-1]["content"] == "{START}":
                        # Remove that {START} message that would have just been added
                        async_interpreter.messages = async_interpreter.messages[:-1]
                    last_start_time = time.time()
                    if (
                        async_interpreter.messages
                        and async_interpreter.messages[-1].get("role") != "user"
                    ):
                        return
                else:
                    # Check if we're within 6 seconds of last_start_time
                    current_time = time.time()
                    if current_time - last_start_time <= 6:
                        # Continue processing
                        pass
                    else:
                        # More than 6 seconds have passed, so return
                        return

            else:
                if last_message.content == "{START}":
                    # This just sometimes happens I guess
                    # Remove that {START} message that would have just been added
                    async_interpreter.messages = async_interpreter.messages[:-1]
                    return

        async_interpreter.stop_event.set()
        time.sleep(0.1)
        async_interpreter.stop_event.clear()

        if request.stream:
            return StreamingResponse(
                openai_compatible_generator(run_code), media_type="application/x-ndjson"
            )
        else:
            messages = async_interpreter.chat(message=".", stream=False, display=True)
            content = messages[-1]["content"]
            return {
                "id": "200",
                "object": "chat.completion",
                "created": time.time(),
                "model": request.model,
                "choices": [{"message": {"role": "assistant", "content": content}}],
            }

    return router


class Server:
    DEFAULT_HOST = "127.0.0.1"
    DEFAULT_PORT = 8000

    def __init__(self, async_interpreter, host=None, port=None):
        self.app = FastAPI()
        router = create_router(async_interpreter)
        self.authenticate = authenticate_function

        # Add authentication middleware
        @self.app.middleware("http")
        async def validate_api_key(request: Request, call_next):
            # Ignore authentication for the /heartbeat route
            if request.url.path == "/heartbeat":
                return await call_next(request)

            api_key = request.headers.get("X-API-KEY")
            if self.authenticate(api_key):
                response = await call_next(request)
                return response
            else:
                return JSONResponse(
                    status_code=HTTP_403_FORBIDDEN,
                    content={"detail": "Authentication failed"},
                )

        self.app.include_router(router)
        h = host or os.getenv("INTERPRETER_HOST", Server.DEFAULT_HOST)
        p = port or int(os.getenv("INTERPRETER_PORT", Server.DEFAULT_PORT))
        self.config = uvicorn.Config(app=self.app, host=h, port=p)
        self.uvicorn_server = uvicorn.Server(self.config)

    @property
    def host(self):
        return self.config.host

    @host.setter
    def host(self, value):
        self.config.host = value
        self.uvicorn_server = uvicorn.Server(self.config)

    @property
    def port(self):
        return self.config.port

    @port.setter
    def port(self, value):
        self.config.port = value
        self.uvicorn_server = uvicorn.Server(self.config)

    def run(self, host=None, port=None, retries=5):
        if host is not None:
            self.host = host
        if port is not None:
            self.port = port

        # Print server information
        if self.host == "0.0.0.0":
            print(
                "Warning: Using host `0.0.0.0` will expose Open Interpreter over your local network."
            )
            s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            s.connect(("8.8.8.8", 80))  # Google's public DNS server
            print(f"Server will run at http://{s.getsockname()[0]}:{self.port}")
            s.close()
        else:
            print(f"Server will run at http://{self.host}:{self.port}")

        self.uvicorn_server.run()

        # for _ in range(retries):
        #     try:
        #         self.uvicorn_server.run()
        #         break
        #     except KeyboardInterrupt:
        #         break
        #     except ImportError as e:
        #         if _ == 4:  # If this is the last attempt
        #             raise ImportError(
        #                 str(e)
        #                 + """\n\nPlease ensure you have run `pip install "open-interpreter[server]"` to install server dependencies."""
        #             )
        #     except:
        #         print("An unexpected error occurred:", traceback.format_exc())
        #         print("Server restarting.")
