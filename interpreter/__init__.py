import sys
import warnings

# Suppress Pydantic serialization warnings often caused by Litellm streaming
warnings.filterwarnings("ignore", message=".*PydanticSerializationUnexpectedValue.*")

if "--os" in sys.argv:
    from rich import print as rich_print
    from rich.markdown import Markdown
    from rich.rule import Rule

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
                except UnicodeEncodeError:
                    # Replace the problematic character or handle the error as needed
                    print("Error displaying line:", line)

        if "\n" not in message and message.startswith(">"):
            # Aesthetic choice. For these tags, they need a space below them
            print("")

    from importlib.metadata import version as pkg_version
    import requests
    from packaging import version as packaging_version

    def check_for_update():
        # Fetch the latest version from the PyPI API
        response = requests.get("https://pypi.org/pypi/open-interpreter/json")
        latest_version = response.json()["info"]["version"]

        # Get the current version using importlib.metadata
        current_version = pkg_version("open-interpreter")

        return packaging_version.parse(latest_version) > packaging_version.parse(
            current_version
        )

    if check_for_update():
        print_markdown(
            "> **A new version of Open Interpreter is available.**\n>Please run: `pip install --upgrade open-interpreter`\n\n---"
        )

    if "--voice" in sys.argv:
        print("Coming soon...")
    from .computer_use.loop import run_async_main

    run_async_main()
    exit()

from .core.async_core import AsyncInterpreter
from .core.computer.terminal.base_language import BaseLanguage
from .core.core import OpenInterpreter

interpreter = OpenInterpreter()
computer = interpreter.computer

#     ____                      ____      __                            __
#    / __ \____  ___  ____     /  _/___  / /____  _________  ________  / /____  _____
#   / / / / __ \/ _ \/ __ \    / // __ \/ __/ _ \/ ___/ __ \/ ___/ _ \/ __/ _ \/ ___/
#  / /_/ / /_/ /  __/ / / /  _/ // / / / /_/  __/ /  / /_/ / /  /  __/ /_/  __/ /
#  \____/ .___/\___/_/ /_/  /___/_/ /_/\__/\___/_/  / .___/_/   \___/\__/\___/_/
#      /_/                                         /_/
