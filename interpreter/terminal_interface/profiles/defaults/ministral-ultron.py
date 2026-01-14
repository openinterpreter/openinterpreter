"""
This is an Open Interpreter profile. It configures Open Interpreter to run the local `ministral-ultron` model via Ollama.
"""

from interpreter import interpreter

# LLM settings
interpreter.llm.model = "ollama/ministral-ultron"
interpreter.llm.context_window = 32000
interpreter.llm.max_tokens = 4096
interpreter.llm.supports_functions = False

# Final message
interpreter.display_message(
    "> Model set to `ministral-ultron` (Local Ollama)\n\n**Open Interpreter** will require approval before running code.\n\nUse `interpreter -y` to bypass this.\n\nPress `CTRL-C` to exit.\n"
)
