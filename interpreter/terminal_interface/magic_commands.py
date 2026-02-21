import json
import os
import subprocess
import sys
import time
from datetime import datetime

from ..core.utils.system_debug_info import system_info
from ..legal import SKILLS_DIR
from ..legal.document_parser import extract_text
from ..legal.playbook import load_playbook
from .utils.count_tokens import count_messages_tokens
from .utils.export_to_markdown import export_to_markdown


def handle_undo(self, arguments):
    # Removes all messages after the most recent user entry (and the entry itself).
    # Therefore user can jump back to the latest point of conversation.
    # Also gives a visual representation of the messages removed.

    if len(self.messages) == 0:
        return
    # Find the index of the last 'role': 'user' entry
    last_user_index = None
    for i, message in enumerate(self.messages):
        if message.get("role") == "user":
            last_user_index = i

    removed_messages = []

    # Remove all messages after the last 'role': 'user'
    if last_user_index is not None:
        removed_messages = self.messages[last_user_index:]
        self.messages = self.messages[:last_user_index]

    print("")  # Aesthetics.

    # Print out a preview of what messages were removed.
    for message in removed_messages:
        if "content" in message and message["content"] != None:
            self.display_message(
                f"**Removed message:** `\"{message['content'][:30]}...\"`"
            )
        elif "function_call" in message:
            self.display_message(
                f"**Removed codeblock**"
            )  # TODO: Could add preview of code removed here.

    print("")  # Aesthetics.


def handle_help(self, arguments):
    commands_description = {
        "%% [commands]": "Run commands in system shell",
        "%verbose [true/false]": "Toggle verbose mode. Without arguments or with 'true', it enters verbose mode. With 'false', it exits verbose mode.",
        "%reset": "Resets the current session.",
        "%undo": "Remove previous messages and its response from the message history.",
        "%save_message [path]": "Saves messages to a specified JSON path. If no path is provided, it defaults to 'messages.json'.",
        "%load_message [path]": "Loads messages from a specified JSON path. If no path is provided, it defaults to 'messages.json'.",
        "%tokens [prompt]": "EXPERIMENTAL: Calculate the tokens used by the next request based on the current conversation's messages and estimate the cost of that request; optionally provide a prompt to also calculate the tokens used by that prompt and the total amount of tokens that will be sent with the next request",
        "%help": "Show this help message.",
        "%info": "Show system and interpreter information",
        "%jupyter": "Export the conversation to a Jupyter notebook file",
        "%markdown [path]": "Export the conversation to a specified Markdown path. If no path is provided, it will be saved to the Downloads folder with a generated conversation name.",
        # Legal commands
        "%review-contract [file]": "Review a contract with clause-by-clause GREEN/YELLOW/RED analysis",
        "%triage-nda [file]": "Quick NDA triage with sign/negotiate/reject recommendation",
        "%compliance-check [framework]": "Check compliance against GDPR, CCPA, HIPAA, SOX, etc.",
        "%risk-assess [context]": "Perform a structured legal risk assessment",
        "%brief [context]": "Prepare a meeting briefing document",
        "%respond [context]": "Draft a professional legal communication",
        "%playbook [init]": "View or initialize your legal playbook",
    }

    base_message = ["> **Available Commands:**\n\n"]

    # Add each command and its description to the message
    for cmd, desc in commands_description.items():
        base_message.append(f"- `{cmd}`: {desc}\n")

    additional_info = [
        "\n\nFor further assistance, please join our community Discord or consider contributing to the project's development."
    ]

    # Combine the base message with the additional info
    full_message = base_message + additional_info

    self.display_message("".join(full_message))


def handle_verbose(self, arguments=None):
    if arguments == "" or arguments == "true":
        self.display_message("> Entered verbose mode")
        print("\n\nCurrent messages:\n")
        for message in self.messages:
            message = message.copy()
            if message["type"] == "image" and message.get("format") not in [
                "path",
                "description",
            ]:
                message["content"] = (
                    message["content"][:30] + "..." + message["content"][-30:]
                )
            print(message, "\n")
        print("\n")
        self.verbose = True
    elif arguments == "false":
        self.display_message("> Exited verbose mode")
        self.verbose = False
    else:
        self.display_message("> Unknown argument to verbose command.")


def handle_debug(self, arguments=None):
    if arguments == "" or arguments == "true":
        self.display_message("> Entered debug mode")
        print("\n\nCurrent messages:\n")
        for message in self.messages:
            message = message.copy()
            if message["type"] == "image" and message.get("format") not in [
                "path",
                "description",
            ]:
                message["content"] = (
                    message["content"][:30] + "..." + message["content"][-30:]
                )
            print(message, "\n")
        print("\n")
        self.debug = True
    elif arguments == "false":
        self.display_message("> Exited verbose mode")
        self.debug = False
    else:
        self.display_message("> Unknown argument to debug command.")


def handle_auto_run(self, arguments=None):
    if arguments == "" or arguments == "true":
        self.display_message("> Entered auto_run mode")
        self.auto_run = True
    elif arguments == "false":
        self.display_message("> Exited auto_run mode")
        self.auto_run = False
    else:
        self.display_message("> Unknown argument to auto_run command.")


def handle_info(self, arguments):
    system_info(self)


def handle_reset(self, arguments):
    self.reset()
    self.display_message("> Reset Done")


def default_handle(self, arguments):
    self.display_message("> Unknown command")
    handle_help(self, arguments)


def handle_save_message(self, json_path):
    if json_path == "":
        json_path = "messages.json"
    if not json_path.endswith(".json"):
        json_path += ".json"
    with open(json_path, "w") as f:
        json.dump(self.messages, f, indent=2)

    self.display_message(f"> messages json export to {os.path.abspath(json_path)}")


def handle_load_message(self, json_path):
    if json_path == "":
        json_path = "messages.json"
    if not json_path.endswith(".json"):
        json_path += ".json"
    with open(json_path, "r") as f:
        self.messages = json.load(f)

    self.display_message(f"> messages json loaded from {os.path.abspath(json_path)}")


def handle_count_tokens(self, prompt):
    messages = [{"role": "system", "message": self.system_message}] + self.messages

    outputs = []

    if len(self.messages) == 0:
        (conversation_tokens, conversation_cost) = count_messages_tokens(
            messages=messages, model=self.llm.model
        )
    else:
        (conversation_tokens, conversation_cost) = count_messages_tokens(
            messages=messages, model=self.llm.model
        )

    outputs.append(
        (
            f"> Tokens sent with next request as context: {conversation_tokens} (Estimated Cost: ${conversation_cost})"
        )
    )

    if prompt:
        (prompt_tokens, prompt_cost) = count_messages_tokens(
            messages=[prompt], model=self.llm.model
        )
        outputs.append(
            f"> Tokens used by this prompt: {prompt_tokens} (Estimated Cost: ${prompt_cost})"
        )

        total_tokens = conversation_tokens + prompt_tokens
        total_cost = conversation_cost + prompt_cost

        outputs.append(
            f"> Total tokens for next request with this prompt: {total_tokens} (Estimated Cost: ${total_cost})"
        )

    outputs.append(
        f"**Note**: This functionality is currently experimental and may not be accurate. Please report any issues you find to the [Open Interpreter GitHub repository](https://github.com/OpenInterpreter/open-interpreter)."
    )

    self.display_message("\n".join(outputs))


def get_downloads_path():
    if os.name == "nt":
        # For Windows
        downloads = os.path.join(os.environ["USERPROFILE"], "Downloads")
    else:
        # For MacOS and Linux
        downloads = os.path.join(os.path.expanduser("~"), "Downloads")
        # For some GNU/Linux distros, there's no '~/Downloads' dir by default
        if not os.path.exists(downloads):
            os.makedirs(downloads)
    return downloads


def install_and_import(package):
    try:
        module = __import__(package)
    except ImportError:
        try:
            # Install the package silently with pip
            print("")
            print(f"Installing {package}...")
            print("")
            subprocess.check_call(
                [sys.executable, "-m", "pip", "install", package],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            module = __import__(package)
        except subprocess.CalledProcessError:
            # If pip fails, try pip3
            try:
                subprocess.check_call(
                    [sys.executable, "-m", "pip3", "install", package],
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
            except subprocess.CalledProcessError:
                print(f"Failed to install package {package}.")
                return
    finally:
        globals()[package] = module
    return module


def jupyter(self, arguments):
    # Dynamically install nbformat if not already installed
    nbformat = install_and_import("nbformat")
    from nbformat.v4 import new_code_cell, new_markdown_cell, new_notebook

    downloads = get_downloads_path()
    current_time = datetime.now()
    formatted_time = current_time.strftime("%m-%d-%y-%I%M%p")
    filename = f"open-interpreter-{formatted_time}.ipynb"
    notebook_path = os.path.join(downloads, filename)
    nb = new_notebook()
    cells = []

    for msg in self.messages:
        if msg["role"] == "user" and msg["type"] == "message":
            # Prefix user messages with '>' to render them as block quotes, so they stand out
            content = f"> {msg['content']}"
            cells.append(new_markdown_cell(content))
        elif msg["role"] == "assistant" and msg["type"] == "message":
            cells.append(new_markdown_cell(msg["content"]))
        elif msg["type"] == "code":
            # Handle the language of the code cell
            if "format" in msg and msg["format"]:
                language = msg["format"]
            else:
                language = "python"  # Default to Python if no format specified
            code_cell = new_code_cell(msg["content"])
            code_cell.metadata.update({"language": language})
            cells.append(code_cell)

    nb["cells"] = cells

    with open(notebook_path, "w", encoding="utf-8") as f:
        nbformat.write(nb, f)

    print("")
    self.display_message(
        f"Jupyter notebook file exported to {os.path.abspath(notebook_path)}"
    )


def markdown(self, export_path: str):
    # If it's an empty conversations
    if len(self.messages) == 0:
        print("No messages to export.")
        return

    # If user doesn't specify the export path, then save the exported PDF in '~/Downloads'
    if not export_path:
        export_path = get_downloads_path() + f"/{self.conversation_filename[:-4]}md"

    export_to_markdown(self.messages, export_path)


def _load_skill(skill_name):
    """Load a legal skill markdown file by name."""
    skill_path = os.path.join(SKILLS_DIR, f"{skill_name}.md")
    if os.path.exists(skill_path):
        with open(skill_path, "r", encoding="utf-8") as f:
            return f.read()
    return ""


def _build_legal_prompt(skill_name, task_description, arguments=""):
    """Build a prompt that combines a legal skill with user input."""
    skill_content = _load_skill(skill_name)
    playbook = load_playbook()

    prompt_parts = [task_description]

    if arguments.strip():
        prompt_parts.append(f"\n\n## User Input\n\n{arguments}")

    # If arguments reference a file path, try to extract text
    if arguments.strip():
        potential_path = arguments.strip().strip("'\"")
        if os.path.isfile(potential_path):
            try:
                doc_text = extract_text(potential_path)
                prompt_parts.append(
                    f"\n\n## Document Content\n\n```\n{doc_text}\n```"
                )
            except (ValueError, ImportError) as e:
                prompt_parts.append(f"\n\n(Could not extract document text: {e})")

    if playbook:
        prompt_parts.append(f"\n\n## Playbook\n\n{playbook}")

    if skill_content:
        prompt_parts.append(f"\n\n## Skill Reference\n\n{skill_content}")

    return "\n".join(prompt_parts)


def handle_review_contract(self, arguments):
    """Handle /review-contract command."""
    prompt = _build_legal_prompt(
        "contract_review",
        "Please perform a comprehensive contract review on the following document. "
        "Provide a clause-by-clause analysis with GREEN/YELLOW/RED risk flags, "
        "an executive summary, and priority negotiation items.",
        arguments,
    )
    self.messages.append({"role": "user", "type": "message", "content": prompt})
    # Let the interpreter process this message
    for chunk in self.chat(prompt, display=True, stream=True):
        pass


def handle_triage_nda(self, arguments):
    """Handle /triage-nda command."""
    prompt = _build_legal_prompt(
        "nda_triage",
        "Please triage this NDA. Classify it (mutual/one-way), assess key risk areas, "
        "and provide a triage decision (SIGN AS-IS / SIGN WITH MINOR EDITS / NEGOTIATE / REJECT).",
        arguments,
    )
    self.messages.append({"role": "user", "type": "message", "content": prompt})
    for chunk in self.chat(prompt, display=True, stream=True):
        pass


def handle_compliance_check(self, arguments):
    """Handle /compliance-check command."""
    prompt = _build_legal_prompt(
        "compliance",
        "Please perform a compliance review. Identify applicable regulatory frameworks "
        "and assess the document/practice against each requirement. "
        "Provide a gap analysis with compliance scores and priority actions.",
        arguments,
    )
    self.messages.append({"role": "user", "type": "message", "content": prompt})
    for chunk in self.chat(prompt, display=True, stream=True):
        pass


def handle_risk_assess(self, arguments):
    """Handle /risk-assess command."""
    prompt = _build_legal_prompt(
        "legal_risk_assessment",
        "Please perform a legal risk assessment. Identify risks across all relevant categories, "
        "score them by likelihood and impact, and provide a mitigation roadmap with a decision recommendation.",
        arguments,
    )
    self.messages.append({"role": "user", "type": "message", "content": prompt})
    for chunk in self.chat(prompt, display=True, stream=True):
        pass


def handle_brief(self, arguments):
    """Handle /brief command for meeting preparation."""
    prompt = _build_legal_prompt(
        "meeting_briefing",
        "Please prepare a comprehensive meeting briefing document. "
        "Include key issues, talking points, questions to ask, fallback positions, and follow-up actions.",
        arguments,
    )
    self.messages.append({"role": "user", "type": "message", "content": prompt})
    for chunk in self.chat(prompt, display=True, stream=True):
        pass


def handle_respond(self, arguments):
    """Handle /respond command for drafting legal communications."""
    prompt = _build_legal_prompt(
        "canned_responses",
        "Please draft a professional legal communication based on the following context. "
        "Use an appropriate template and customize it with the specific details provided.",
        arguments,
    )
    self.messages.append({"role": "user", "type": "message", "content": prompt})
    for chunk in self.chat(prompt, display=True, stream=True):
        pass


def handle_playbook(self, arguments):
    """Handle /playbook command to view or initialize the playbook."""
    from ..legal.playbook import find_playbook, get_default_playbook_template, save_playbook

    existing = find_playbook()
    if existing:
        self.display_message(f"> Playbook found at: `{existing}`")
        playbook_content = load_playbook(existing)
        # Show first few lines as preview
        lines = playbook_content.split("\n")[:10]
        preview = "\n".join(lines)
        self.display_message(f"```\n{preview}\n...\n```")
        self.display_message("> Edit the file above to customize your legal positions.")
    else:
        if arguments.strip() == "init":
            template = get_default_playbook_template()
            path = save_playbook(template)
            self.display_message(f"> Playbook created at: `{path}`")
            self.display_message("> Edit this file to customize your organization's legal positions.")
        else:
            self.display_message("> No playbook found. Run `/playbook init` to create one from the default template.")


def handle_magic_command(self, user_input):
    # Handle shell
    if user_input.startswith("%%"):
        code = user_input[2:].strip()
        self.computer.run("shell", code, stream=False, display=True)
        print("")
        return

    # split the command into the command and the arguments, by the first whitespace
    switch = {
        "help": handle_help,
        "verbose": handle_verbose,
        "debug": handle_debug,
        "auto_run": handle_auto_run,
        "reset": handle_reset,
        "save_message": handle_save_message,
        "load_message": handle_load_message,
        "undo": handle_undo,
        "tokens": handle_count_tokens,
        "info": handle_info,
        "jupyter": jupyter,
        "markdown": markdown,
        # Legal commands
        "review-contract": handle_review_contract,
        "triage-nda": handle_triage_nda,
        "compliance-check": handle_compliance_check,
        "risk-assess": handle_risk_assess,
        "brief": handle_brief,
        "respond": handle_respond,
        "playbook": handle_playbook,
    }

    user_input = user_input[1:].strip()  # Capture the part after the `%`
    command = user_input.split(" ")[0]
    arguments = user_input[len(command) :].strip()

    if command == "debug":
        print(
            "\n`%debug` / `--debug_mode` has been renamed to `%verbose` / `--verbose`.\n"
        )
        time.sleep(1.5)
        command = "verbose"

    action = switch.get(
        command, default_handle
    )  # Get the function from the dictionary, or default_handle if not found
    action(self, arguments)  # Execute the function
