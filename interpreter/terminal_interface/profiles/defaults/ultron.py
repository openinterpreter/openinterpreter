"""
This is an Open Interpreter profile. It configures Open Interpreter to run `devstral-ultron` using Ollama.
"""

from interpreter import interpreter

# LLM settings
interpreter.llm.model = "ollama/devstral-ultron"
interpreter.llm.context_window = 32000
interpreter.llm.max_tokens = 4096

# Final message
interpreter.display_message(
    "> Model set to `ultron` (devstral-ultron)\n\n**Open Interpreter** will require approval before running code.\n\nUse `interpreter -y` to bypass this.\n\nPress `CTRL-C` to exit.\n"
)
