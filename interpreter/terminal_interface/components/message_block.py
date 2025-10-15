import re

from rich.box import MINIMAL
from rich.markdown import Markdown
from rich.panel import Panel
from rich.text import Text
from rich.console import Group

from .base_block import BaseBlock
from ..utils.streaming_markdown import (
    detect_complete_block,
    calculate_window_size,
    create_sliding_window_display,
    create_live_display,
    textify_markdown_code_blocks,
)


class MessageBlock(BaseBlock):
    def __init__(self):
        super().__init__()

        # Override the Live display with our streaming configuration
        self.live.stop()  # Stop the base Live display
        self.live = create_live_display(self.live.console)  # Use our streaming Live display
        self.live.start()

        self.type = "message"
        self.message = ""
        self.buffer = ""
        self.completed_blocks = []
        self.viewport_fraction = 0.3  # Increase from 0.2 to 0.3 for better visibility
        self.debug = False  # Disable debug mode

    def refresh(self, cursor=True):
        """Process new content and render complete blocks incrementally."""
        # Try to detect a complete block
        block_result = detect_complete_block(self.buffer)

        if block_result:
            block_text, next_line_begin = block_result

            # De-stylize any code blocks in markdown to differentiate from Code Blocks
            content = textify_markdown_code_blocks(block_text)

            # Render the complete block directly to console (above the Live viewport)
            markdown = Markdown(content.strip())
            panel = Panel(markdown, box=MINIMAL)
            self.live.console.print(panel)

            # Store the completed block
            self.completed_blocks.append(content)

            # Remove the rendered block from buffer using line numbers
            lines = self.buffer.split('\n')
            remaining_lines = lines[next_line_begin:]
            self.buffer = '\n'.join(remaining_lines)

            # If we removed content, refresh the viewport with remaining content
            if remaining_lines:
                # Continue to the streaming section below
                pass

        # Stream the remaining buffer content in the Live viewport
        if self.buffer.strip():
            # Calculate viewport size
            viewport_lines = calculate_window_size(self.live.console, self.viewport_fraction)

            # Ensure we have a reasonable viewport size
            if viewport_lines < 1:
                viewport_lines = 3  # Minimum viewport size

            # Create sliding window display for the buffer
            formatted_buffer = create_sliding_window_display(
                self.live.console, self.buffer.split('\n'), viewport_lines, self.debug)

            # Add cursor if requested
            if cursor and isinstance(formatted_buffer, Text):
                formatted_buffer += "●"
            elif cursor and isinstance(formatted_buffer, Group):
                # If it's a Group with ellipsis, add cursor to the text part
                formatted_buffer.renderables[-1] += "●"

            self.live.update(formatted_buffer)
        else:
            # Clear the live display if no buffer content
            self.live.update("")

    def add_content(self, content):
        """Add new content to the buffer and process it."""
        self.buffer += content
        self.refresh(cursor=True)

    def finalize(self):
        """Render any remaining content when the message is complete."""
        if self.buffer.strip():
            try:
                # De-stylize any code blocks in markdown
                content = textify_markdown_code_blocks(self.buffer)
                markdown = Markdown(content.strip())
                panel = Panel(markdown, box=MINIMAL)
                self.live.console.print(panel)
            except (IndexError, ValueError, TypeError):
                # Fallback to plain text if markdown parsing fails
                self.live.console.print(self.buffer)

        # Clear the live display
        self.live.update("")


