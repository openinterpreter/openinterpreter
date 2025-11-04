# Logging in Open Interpreter

Open Interpreter now includes a structured logging system for better debugging and monitoring.

## Quick Start

```python
from interpreter.core.utils.logging_config import get_logger

logger = get_logger(__name__)

# Use standard logging levels
logger.debug("Detailed information for debugging")
logger.info("General informational messages")
logger.warning("Warning messages")
logger.error("Error messages", exc_info=True)  # Include stack trace
logger.critical("Critical errors")
```

## Configuration

### Environment Variable

Set the log level using the `OI_LOG_LEVEL` environment variable:

```bash
export OI_LOG_LEVEL=DEBUG    # Show all messages
export OI_LOG_LEVEL=INFO     # Show info and above
export OI_LOG_LEVEL=WARNING  # Show warnings and errors (default)
export OI_LOG_LEVEL=ERROR    # Show only errors
```

### Programmatic Configuration

```python
from interpreter.core.utils.logging_config import setup_logging
import logging

# Basic setup
setup_logging(level=logging.DEBUG)

# With verbose format (includes file and line numbers)
setup_logging(level=logging.DEBUG, verbose=True)

# With log file
setup_logging(
    level=logging.DEBUG,
    log_file="/path/to/open-interpreter.log"
)
```

## Log Levels Guide

- **DEBUG**: Detailed information for diagnosing problems
  - Variable values, state changes, flow control
  - Only visible when explicitly enabled

- **INFO**: Confirmation that things are working as expected
  - Major operation starts/completions
  - Configuration changes

- **WARNING**: Something unexpected happened, but the program continues
  - Deprecated features
  - Recoverable errors
  - Missing optional dependencies

- **ERROR**: Serious problem, program may not be able to perform some function
  - Failed operations
  - Unrecoverable errors
  - Use `exc_info=True` to include stack traces

- **CRITICAL**: Program may be unable to continue running
  - Fatal errors
  - System-level failures

## Best Practices

1. **Use appropriate log levels**
   ```python
   logger.debug(f"Processing {len(items)} items")  # NOT info
   logger.info("Model changed to gpt-4")
   logger.warning("Using fallback model due to API error")
   logger.error("Failed to execute code", exc_info=True)
   ```

2. **Include context in messages**
   ```python
   # Good
   logger.error(f"Failed to load profile '{profile_name}': {error}")

   # Bad
   logger.error("Error loading profile")
   ```

3. **Use structured logging for important data**
   ```python
   logger.info(
       f"LLM request: model={model}, tokens={tokens}, "
       f"temperature={temperature}"
   )
   ```

4. **Don't log sensitive information**
   ```python
   # Bad - logs API key
   logger.debug(f"Using API key: {api_key}")

   # Good - masks sensitive data
   logger.debug(f"Using API key: {api_key[:8]}...")
   ```

5. **Use exc_info for exceptions**
   ```python
   try:
       risky_operation()
   except Exception as e:
       logger.error(f"Operation failed: {e}", exc_info=True)
   ```

## Migration from Print Statements

When replacing `print()` statements:

- Status updates → `logger.info()`
- Debug output → `logger.debug()`
- Warnings → `logger.warning()`
- Errors → `logger.error()`
- User-facing messages → Keep as `print()` or use display functions

## Example

```python
from interpreter.core.utils.logging_config import get_logger

logger = get_logger(__name__)

def process_message(message):
    logger.debug(f"Processing message: {message[:50]}...")

    try:
        result = complex_operation(message)
        logger.info(f"Message processed successfully, result size: {len(result)}")
        return result
    except ValueError as e:
        logger.warning(f"Invalid message format: {e}")
        return None
    except Exception as e:
        logger.error("Unexpected error during processing", exc_info=True)
        raise
```
