"""
Streaming markdown utilities for OpenInterpreter.

This module provides block-based incremental rendering for streaming markdown content,
similar to the approach demonstrated in dev_examples/rich_markdown_example.py.
"""

import re
from markdown_it import MarkdownIt
from rich.align import Align
from rich.console import Console, Group
from rich.live import Live
from rich.markdown import Markdown
from rich.panel import Panel
from rich.text import Text


def detect_complete_block(markdown_text):
    """
    Detect complete blocks by finding when a new top-level block starts.
    Returns (block_text, next_line_begin) when a complete block is found, or None.
    """
    # Enable non-default features to match Rich's parser configuration
    md = MarkdownIt().enable("strikethrough").enable("table")
    md_tokens = md.parse(markdown_text)

    lines = markdown_text.split('\n')

    # Find all top-level block tokens (level 0)
    top_level_tokens = []

    for md_token in md_tokens:
        # Only collect top-level structural blocks
        # (paragraph_open, heading_open, fence, etc.)
        if md_token.block and md_token.level == 0:
            # Only count opening (nesting=1) or self-closing (nesting=0) blocks
            if md_token.nesting in (0, 1):
                top_level_tokens.append(md_token)

    # If we have at least 2 top-level opening tokens, the first
    # opening-closing token pair is a complete block.
    if len(top_level_tokens) >= 2:
        first_token = top_level_tokens[0]
        second_token = top_level_tokens[1]

        line_begin, line_end = first_token.map
        next_line_begin = second_token.map[0]

        # Include the block content AND any blank lines between this block and the next
        # This preserves the original markdown spacing
        block_lines = lines[line_begin:next_line_begin]
        block_text = '\n'.join(block_lines)
        return block_text, next_line_begin

    return None


def calculate_window_size(console, viewport_fraction):
    """Calculate viewport size based on terminal height and fraction.

    Args:
        console: Rich Console instance
        viewport_fraction: Fraction of terminal height (0 to 1)

    Returns:
        Number of lines for viewport (minimum 1)
    """
    terminal_height = console.size.height
    return max(1, int(terminal_height * viewport_fraction))


def create_sliding_window_display(console, current_lines, viewport_lines, debug=False):
    """Create display text with sliding viewport and upper ellipsis when needed.

    Args:
        console: Rich Console instance
        current_lines: List of all current text lines
        viewport_lines: Maximum number of lines to display
        debug: If True, wrap content in a bordered panel to show Live area boundaries

    Returns:
        Rich Text, Group, or Panel renderable showing the viewport
    """
    # Get last N lines (or all lines if fewer than N)
    display_lines = current_lines[-viewport_lines:]
    text = Text('\n'.join(display_lines))

    # Wrap with red ellipsis at top if content was truncated, mimicking
    # the bottom red ellipsis in a rich Live display in `ellipsis` mode.
    if len(current_lines) > viewport_lines:
        text = Group(
            Align.center(Text("...", style="red"), width=console.size.width),
            text
        )

    # Wrap in a panel with border only in debug mode
    if debug:
        return Panel(text, title="Streaming Buffer", border_style="blue")
    else:
        return text


def create_live_display(console):
    """Create a Live display with standard settings.

    Args:
        console: Rich Console instance

    Returns:
        Rich Live display object configured for streaming
    """
    return Live(console=console, refresh_per_second=20,
                vertical_overflow="ellipsis")


def textify_markdown_code_blocks(text):
    """
    To distinguish CodeBlocks from markdown code, we simply turn all markdown code
    (like '```python...') into text code blocks ('```text') which makes the code black and white.
    """
    replacement = "```text"
    lines = text.split("\n")
    inside_code_block = False

    for i in range(len(lines)):
        # If the line matches ``` followed by optional language specifier
        if re.match(r"^```(\w*)$", lines[i].strip()):
            inside_code_block = not inside_code_block

            # If we just entered a code block, replace the marker
            if inside_code_block:
                lines[i] = replacement

    return "\n".join(lines)
