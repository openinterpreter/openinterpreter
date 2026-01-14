"""
This is an Open Interpreter profile. It configures Open Interpreter to run `ministral-8b` using the Mistral API.
"""

from interpreter import interpreter

# LLM settings
interpreter.llm.model = "mistral/ministral-8b-latest"
interpreter.llm.api_base = "https://api.mistral.ai/v1"
interpreter.llm.context_window = 32000
interpreter.llm.max_tokens = 4096

# Final message
interpreter.display_message(
    "> Model set to `ministral`\n\n**Open Interpreter** will require approval before running code.\n\nUse `interpreter -y` to bypass this.\n\nPress `CTRL-C` to exit.\n"
)
