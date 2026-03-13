"""
This is an Open Interpreter profile to use MiniMax.

Please set the MINIMAX_API_KEY environment variable.

MiniMax provides powerful language models with a 204,800 token context window.
Available models:
  - MiniMax-M2.5 (default): Peak Performance. Ultimate Value.
  - MiniMax-M2.5-highspeed: Same performance, faster and more agile.

See https://platform.minimax.io/docs/api-reference/text-openai-api for more information.
"""

from interpreter import interpreter
import os

# LLM settings
interpreter.llm.model = "openai/MiniMax-M2.5"
interpreter.llm.api_key = os.environ.get("MINIMAX_API_KEY")
interpreter.llm.api_base = "https://api.minimax.io/v1"
interpreter.llm.supports_functions = True
interpreter.llm.supports_vision = False
interpreter.llm.max_tokens = 4096
interpreter.llm.context_window = 204800
interpreter.llm.temperature = 1.0

# Computer settings
interpreter.computer.import_computer_api = True

# Misc settings
interpreter.offline = False
interpreter.auto_run = False
