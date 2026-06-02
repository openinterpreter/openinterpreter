"""
This is an Open Interpreter profile to use MiniMax.

Please set the MINIMAX_API_KEY environment variable.

MiniMax provides powerful language models with up to a 512,000 token context window.
Available models:
  - MiniMax-M3 (default): Latest flagship model with 512K context, up to 128K output, and image input support.
  - MiniMax-M2.7: Previous generation flagship model with enhanced reasoning and coding.
  - MiniMax-M2.7-highspeed: High-speed version of M2.7 for low-latency scenarios.

See https://platform.minimax.io/docs/api-reference/text-openai-api for more information.
"""

from interpreter import interpreter
import os

# LLM settings
interpreter.llm.model = "openai/MiniMax-M3"
interpreter.llm.api_key = os.environ.get("MINIMAX_API_KEY")
interpreter.llm.api_base = "https://api.minimax.io/v1"
interpreter.llm.supports_functions = True
interpreter.llm.supports_vision = True
interpreter.llm.max_tokens = 4096
interpreter.llm.context_window = 512000
interpreter.llm.temperature = 1.0

# Computer settings
interpreter.computer.import_computer_api = True

# Misc settings
interpreter.offline = False
interpreter.auto_run = False
