"""
Centralized logging configuration for Open Interpreter.

Usage:
    from interpreter.core.utils.logging_config import get_logger

    logger = get_logger(__name__)
    logger.info("Message")
    logger.warning("Warning message")
    logger.error("Error message", exc_info=True)
"""

import logging
import os
import sys
from typing import Optional


# Default log format
DEFAULT_FORMAT = "%(asctime)s - %(name)s - %(levelname)s - %(message)s"
VERBOSE_FORMAT = "%(asctime)s - %(name)s - %(levelname)s - %(filename)s:%(lineno)d - %(message)s"


def get_log_level_from_env() -> int:
    """Get log level from environment variable."""
    level_name = os.environ.get("OI_LOG_LEVEL", "WARNING").upper()
    return getattr(logging, level_name, logging.WARNING)


def setup_logging(
    level: Optional[int] = None,
    format_string: Optional[str] = None,
    log_file: Optional[str] = None,
    verbose: bool = False
) -> None:
    """
    Configure logging for Open Interpreter.

    Args:
        level: Logging level (default: from OI_LOG_LEVEL env or WARNING)
        format_string: Custom format string (default: DEFAULT_FORMAT)
        log_file: Optional log file path
        verbose: Use verbose format with file and line numbers
    """
    if level is None:
        level = get_log_level_from_env()

    if format_string is None:
        format_string = VERBOSE_FORMAT if verbose else DEFAULT_FORMAT

    # Remove any existing handlers
    root_logger = logging.getLogger("interpreter")
    for handler in root_logger.handlers[:]:
        root_logger.removeHandler(handler)

    # Configure root logger for interpreter package
    root_logger.setLevel(level)

    # Console handler
    console_handler = logging.StreamHandler(sys.stdout)
    console_handler.setLevel(level)
    console_formatter = logging.Formatter(format_string)
    console_handler.setFormatter(console_formatter)
    root_logger.addHandler(console_handler)

    # File handler if specified
    if log_file:
        try:
            os.makedirs(os.path.dirname(log_file), exist_ok=True)
            file_handler = logging.FileHandler(log_file)
            file_handler.setLevel(level)
            file_formatter = logging.Formatter(VERBOSE_FORMAT)  # Always verbose in files
            file_handler.setFormatter(file_formatter)
            root_logger.addHandler(file_handler)
        except (OSError, IOError) as e:
            root_logger.warning(f"Failed to create log file {log_file}: {e}")

    # Prevent propagation to avoid duplicate logs
    root_logger.propagate = False


def get_logger(name: str) -> logging.Logger:
    """
    Get a logger for the specified module.

    Args:
        name: Module name (typically __name__)

    Returns:
        Configured logger instance
    """
    # Ensure the logger is under the interpreter namespace
    if not name.startswith("interpreter"):
        name = f"interpreter.{name}"

    return logging.getLogger(name)


# Initialize default logging configuration on import
if not logging.getLogger("interpreter").handlers:
    setup_logging()
