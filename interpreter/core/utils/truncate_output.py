import re
import tempfile
from pathlib import Path


ANSI_ESCAPE_RE = re.compile(r"\x1B\[[0-?]*[ -/]*[@-~]")
TRUNCATION_PREFIX = "Output truncated ("


def _strip_ansi_codes(data):
    return ANSI_ESCAPE_RE.sub("", data)


def _latest_output_path():
    return Path(tempfile.gettempdir()) / "oi-output-latest.txt"


def _build_truncation_message(total_chars, chars_per_end, output_path):
    return (
        f"Output truncated ({total_chars:,} visible characters total). "
        f"Showing {chars_per_end:,} visible characters from start/end. "
        f"Full output saved to `{output_path}`. "
        "To inspect it, use tools like `head`, `tail`, or `grep`, "
        "or rerun the command in smaller steps.\n\n"
    )


def truncate_output(data, max_output_chars=2800, add_scrollbars=False):
    # if "@@@DO_NOT_TRUNCATE@@@" in data:
    #     return data

    needs_truncation = False

    # Calculate how much to show from start and end
    chars_per_end = max_output_chars // 2
    visible_data = _strip_ansi_codes(data)

    # This won't work because truncated code is stored in interpreter.messages :/
    # If the full code was stored, we could do this:
    if add_scrollbars:
        extra_scrollbar_message = (
            f" Run `get_last_output()[0:{max_output_chars}]` to see the first page."
        )
    else:
        extra_scrollbar_message = ""
    # Then we have code in `terminal.py` which makes that function work. It should be a computer tool though to just access messages IMO. Or like, self.messages.

    # Remove previous truncation message if it exists
    if data.startswith(TRUNCATION_PREFIX):
        _, separator, remainder = data.partition("\n\n")
        if separator:
            data = remainder
            visible_data = _strip_ansi_codes(data)
        needs_truncation = True

    # If data exceeds max length, truncate it and add message
    if len(visible_data) > max_output_chars or needs_truncation:
        output_path = _latest_output_path()
        output_path.write_text(data)
        message = _build_truncation_message(
            len(visible_data), chars_per_end, output_path
        )
        if extra_scrollbar_message:
            message = message.strip() + extra_scrollbar_message + "\n\n"
        first_part = visible_data[:chars_per_end]
        last_part = visible_data[-chars_per_end:]
        data = message + first_part + "\n[...]\n" + last_part

    return data
