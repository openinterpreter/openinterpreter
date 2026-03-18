"""
This file defines the Interpreter class.
It's the main file. `from interpreter import interpreter` will import an instance of this class.
"""
import json
import os
import threading
import time
from datetime import datetime

from ..terminal_interface.local_setup import local_setup
from ..terminal_interface.terminal_interface import terminal_interface
from ..terminal_interface.utils.display_markdown_message import display_markdown_message
from ..terminal_interface.utils.local_storage_path import get_storage_path
from ..terminal_interface.utils.oi_dir import oi_dir
from .computer.computer import Computer
from .default_system_message import default_system_message
from .llm.llm import Llm
from .respond import respond
from .utils.telemetry import send_telemetry
from .utils.truncate_output import truncate_output


class OpenInterpreter:
    """
    This class (one instance is called an `interpreter`) is the "grand central station" of this project.

    Its responsibilities are to:

    1. Given some user input, prompt the language model.
    2. Parse the language models responses, converting them into LMC Messages.
    3. Send code to the computer.
    4. Parse the computer's response (which will already be LMC Messages).
    5. Send the computer's response back to the language model.
    ...

    The above process should repeat—going back and forth between the language model and the computer— until:

    6. Decide when the process is finished based on the language model's response.
    """

    def __init__(
        self,
        messages=None,
        offline=False,
        auto_run=False,
        verbose=False,
        debug=False,
        max_output=2800,
        safe_mode="off",
        shrink_images=True,
        loop=False,
        loop_message="""Proceed. You CAN run code on my machine. If the entire task I asked for is done, say exactly 'The task is done.' If you need some specific information (like username or password) say EXACTLY 'Please provide more information.' If it's impossible, say 'The task is impossible.' (If I haven't provided a task, say exactly 'Let me know what you'd like to do next.') Otherwise keep going.""",
        loop_breakers=[
            "The task is done.",
            "The task is impossible.",
            "Let me know what you'd like to do next.",
            "Please provide more information.",
        ],
        disable_telemetry=False,
        in_terminal_interface=False,
        conversation_history=True,
        conversation_filename=None,
        conversation_history_path=get_storage_path("conversations"),
        os=False,
        speak_messages=False,
        llm=None,
        system_message=default_system_message,
        custom_instructions="",
        user_message_template="{content}",
        always_apply_user_message_template=False,
        code_output_template="Code output: {content}\n\nWhat does this output mean / what's next (if anything, or are we done)?",
        empty_code_output_template="The code above was executed on my machine. It produced no text output. what's next (if anything, or are we done?)",
        code_output_sender="user",
        computer=None,
        sync_computer=False,
        import_computer_api=False,
        skills_path=None,
        import_skills=False,
        multi_line=True,
        contribute_conversation=False,
        plain_text_display=False,
        sandbox_mode=False,
        sandbox_api_key=None,
        sandbox_domain=None,
    ):
        # State
        self.messages = [] if messages is None else messages
        self.responding = False
        self.last_messages_count = 0

        # Settings
        self.offline = offline
        self.auto_run = auto_run
        self.verbose = verbose
        self.debug = debug
        self.max_output = max_output
        self.safe_mode = safe_mode
        self.shrink_images = shrink_images
        self.disable_telemetry = disable_telemetry
        self.in_terminal_interface = in_terminal_interface
        self.multi_line = multi_line
        self.contribute_conversation = contribute_conversation
        self.plain_text_display = plain_text_display
        self.highlight_active_line = True  # additional setting to toggle active line highlighting. Defaults to True

        # Sandbox settings
        self.sandbox_mode = sandbox_mode
        self.sandbox_api_key = sandbox_api_key
        self.sandbox_domain = sandbox_domain

        # Loop messages
        self.loop = loop
        self.loop_message = loop_message
        self.loop_breakers = loop_breakers

        # Conversation history
        self.conversation_history = conversation_history
        self.conversation_filename = conversation_filename
        self.conversation_history_path = conversation_history_path

        # OS control mode related attributes
        self.os = os
        self.speak_messages = speak_messages

        # Computer
        self.computer = Computer(self) if computer is None else computer
        self.sync_computer = sync_computer
        self.computer.import_computer_api = import_computer_api

        # Skills
        if skills_path:
            self.computer.skills.path = skills_path

        self.computer.import_skills = import_skills

        # LLM
        self.llm = Llm(self) if llm is None else llm

        # These are LLM related
        self.system_message = system_message
        self.custom_instructions = custom_instructions
        self.user_message_template = user_message_template
        self.always_apply_user_message_template = always_apply_user_message_template
        self.code_output_template = code_output_template
        self.empty_code_output_template = empty_code_output_template
        self.code_output_sender = code_output_sender

    def local_setup(self):
        """
        Opens a wizard that lets terminal users pick a local model.
        """
        self = local_setup(self)

    def wait(self):
        while self.responding:
            time.sleep(0.2)
        # Return new messages
        return self.messages[self.last_messages_count :]

    @property
    def anonymous_telemetry(self) -> bool:
        return not self.disable_telemetry and not self.offline

    @property
    def will_contribute(self):
        overrides = (
            self.offline or not self.conversation_history or self.disable_telemetry
        )
        return self.contribute_conversation and not overrides

    def chat(self, message=None, display=True, stream=False, blocking=True):
        try:
            self.responding = True
            if self.anonymous_telemetry:
                message_type = type(
                    message
                ).__name__  # Only send message type, no content
                send_telemetry(
                    "started_chat",
                    properties={
                        "in_terminal_interface": self.in_terminal_interface,
                        "message_type": message_type,
                        "os_mode": self.os,
                    },
                )

            if not blocking:
                chat_thread = threading.Thread(
                    target=self.chat, args=(message, display, stream, True)
                )  # True as in blocking = True
                chat_thread.start()
                return

            if stream:
                return self._streaming_chat(message=message, display=display)

            # If stream=False, *pull* from the stream.
            for _ in self._streaming_chat(message=message, display=display):
                pass

            # Return new messages
            self.responding = False
            return self.messages[self.last_messages_count :]

        except GeneratorExit:
            self.responding = False
            # It's fine
        except Exception as e:
            self.responding = False
            if self.anonymous_telemetry:
                message_type = type(message).__name__
                send_telemetry(
                    "errored",
                    properties={
                        "error": str(e),
                        "in_terminal_interface": self.in_terminal_interface,
                        "message_type": message_type,
                        "os_mode": self.os,
                    },
                )

            raise

    def _streaming_chat(self, message=None, display=True):
        # Sometimes a little more code -> a much better experience!
        # Display mode actually runs interpreter.chat(display=False, stream=True) from within the terminal_interface.
        # wraps the vanilla .chat(display=False) generator in a display.
        # Quite different from the plain generator stuff. So redirect to that
        if display:
            yield from terminal_interface(self, message)
            return

        # One-off message
        if message or message == "":
            ## We support multiple formats for the incoming message:
            # Dict (these are passed directly in)
            if isinstance(message, dict):
                if "role" not in message:
                    message["role"] = "user"
                self.messages.append(message)
            # String (we construct a user message dict)
            elif isinstance(message, str):
                self.messages.append(
                    {"role": "user", "type": "message", "content": message}
                )
            # List (this is like the OpenAI API)
            elif isinstance(message, list):
                self.messages = message

            # Now that the user's messages have been added, we set last_messages_count.
            # This way we will only return the messages after what they added.
            self.last_messages_count = len(self.messages)

            # DISABLED because I think we should just not transmit images to non-multimodal models?
            # REENABLE this when multimodal becomes more common:

            # Make sure we're using a model that can handle this
            # if not self.llm.supports_vision:
            #     for message in self.messages:
            #         if message["type"] == "image":
            #             raise Exception(
            #                 "Use a multimodal model and set `interpreter.llm.supports_vision` to True to handle image messages."
            #             )

            # This is where it all happens!
            yield from self._respond_and_store()

            # Save conversation if we've turned conversation_history on
            if self.conversation_history:
                # If it's the first message, set the conversation name
                if not self.conversation_filename:
                    first_few_words_list = self.messages[0]["content"][:25].split(" ")
                    if (
                        len(first_few_words_list) >= 2
                    ):  # for languages like English with blank between words
                        first_few_words = "_".join(first_few_words_list[:-1])
                    else:  # for languages like Chinese without blank between words
                        first_few_words = self.messages[0]["content"][:15]
                    for char in '<>:"/\\|?*!\n':  # Invalid characters for filenames
                        first_few_words = first_few_words.replace(char, "")

                    date = datetime.now().strftime("%B_%d_%Y_%H-%M-%S")
                    self.conversation_filename = (
                        "__".join([first_few_words, date]) + ".json"
                    )

                # Check if the directory exists, if not, create it
                if not os.path.exists(self.conversation_history_path):
                    os.makedirs(self.conversation_history_path)
                # Write or overwrite the file
                with open(
                    os.path.join(
                        self.conversation_history_path, self.conversation_filename
                    ),
                    "w",
                ) as f:
                    json.dump(self.messages, f)
            return

        raise Exception(
            "`interpreter.chat()` requires a display. Set `display=True` or pass a message into `interpreter.chat(message)`."
        )

    def _respond_and_store(self):
        """
        Pulls from the respond stream, adding delimiters. Some things, like active_line, console, confirmation... these act specially.
        Also assembles new messages and adds them to `self.messages`.
        """
        self.verbose = False

        # Utility function
        def is_ephemeral(chunk):
            """
            Ephemeral = this chunk doesn't contribute to a message we want to save.
            """
            if "format" in chunk and chunk["format"] == "active_line":
                return True
            if chunk["type"] == "review":
                return True
            return False

        last_flag_base = None

        try:
            for chunk in respond(self):
                # For async usage
                if hasattr(self, "stop_event") and self.stop_event.is_set():
                    print("Open Interpreter stopping.")
                    break

                if chunk["content"] == "":
                    continue

                # If active_line is None, we finished running code.
                if (
                    chunk.get("format") == "active_line"
                    and chunk.get("content", "") == None
                ):
                    # If output wasn't yet produced, add an empty output
                    if self.messages[-1]["role"] != "computer":
                        self.messages.append(
                            {
                                "role": "computer",
                                "type": "console",
                                "format": "output",
                                "content": "",
                            }
                        )

                # Handle the special "confirmation" chunk, which neither triggers a flag or creates a message
                if chunk["type"] == "confirmation":
                    # Emit a end flag for the last message type, and reset last_flag_base
                    if last_flag_base:
                        yield {**last_flag_base, "end": True}
                        last_flag_base = None

                    if self.auto_run == False:
                        yield chunk

                    # We want to append this now, so even if content is never filled, we know that the execution didn't produce output.
                    # ... rethink this though.
                    # self.messages.append(
                    #     {
                    #         "role": "computer",
                    #         "type": "console",
                    #         "format": "output",
                    #         "content": "",
                    #     }
                    # )
                    continue

                # Check if the chunk's role, type, and format (if present) match the last_flag_base
                if (
                    last_flag_base
                    and "role" in chunk
                    and "type" in chunk
                    and last_flag_base["role"] == chunk["role"]
                    and last_flag_base["type"] == chunk["type"]
                    and (
                        "format" not in last_flag_base
                        or (
                            "format" in chunk
                            and chunk["format"] == last_flag_base["format"]
                        )
                    )
                ):
                    # If they match, append the chunk's content to the current message's content
                    # (Except active_line, which shouldn't be stored)
                    if not is_ephemeral(chunk):
                        if any(
                            [
                                (property in self.messages[-1])
                                and (
                                    self.messages[-1].get(property)
                                    != chunk.get(property)
                                )
                                for property in ["role", "type", "format"]
                            ]
                        ):
                            self.messages.append(chunk)
                        else:
                            self.messages[-1]["content"] += chunk["content"]
                else:
                    # If they don't match, yield a end message for the last message type and a start message for the new one
                    if last_flag_base:
                        yield {**last_flag_base, "end": True}

                    last_flag_base = {"role": chunk["role"], "type": chunk["type"]}

                    # Don't add format to type: "console" flags, to accommodate active_line AND output formats
                    if "format" in chunk and chunk["type"] != "console":
                        last_flag_base["format"] = chunk["format"]

                    yield {**last_flag_base, "start": True}

                    # Add the chunk as a new message
                    if not is_ephemeral(chunk):
                        self.messages.append(chunk)

                # Yield the chunk itself
                yield chunk

                # Truncate output if it's console output
                if chunk["type"] == "console" and chunk["format"] == "output":
                    self.messages[-1]["content"] = truncate_output(
                        self.messages[-1]["content"],
                        self.max_output,
                        add_scrollbars=self.computer.import_computer_api,  # I consider scrollbars to be a computer API thing
                    )

            # Yield a final end flag
            if last_flag_base:
                yield {**last_flag_base, "end": True}
        except GeneratorExit:
            raise  # gotta pass this up!

    def reset(self):
        self.computer.terminate()  # Terminates all languages
        self.computer._has_imported_computer_api = False  # Flag reset
        self.messages = []
        self.last_messages_count = 0

    def display_message(self, markdown):
        # This is just handy for start_script in profiles.
        if self.plain_text_display:
            print(markdown)
        else:
            display_markdown_message(markdown)

    def get_oi_dir(self):
        # Again, just handy for start_script in profiles.
        return oi_dir
