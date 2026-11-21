from __future__ import annotations

from typing import Any

# The app-server reserves this code for overload backpressure and emits it without
# a data payload (codex-rs/app-server/src/error_code.rs).
OVERLOADED_ERROR_CODE = -32001


class CodexError(Exception):
    """Base exception for SDK errors."""


class JsonRpcError(CodexError):
    """Raw JSON-RPC error wrapper from the server."""

    def __init__(self, code: int, message: str, data: Any = None):
        super().__init__(f"JSON-RPC error {code}: {message}")
        self.code = code
        self.message = message
        self.data = data


class TransportClosedError(CodexError):
    """Raised when the Codex transport closes unexpectedly."""


class CodexRpcError(JsonRpcError):
    """Base typed error for JSON-RPC failures."""


class ParseError(CodexRpcError):
    """Raised when a request or response cannot be parsed."""


class InvalidRequestError(CodexRpcError):
    """Raised when the runtime rejects the request shape."""


class MethodNotFoundError(CodexRpcError):
    """Raised when the requested operation is unavailable."""


class InvalidParamsError(CodexRpcError):
    """Raised when an operation receives invalid parameters."""


class InternalRpcError(CodexRpcError):
    """Raised when the runtime reports an internal RPC failure."""


class ServerBusyError(CodexRpcError):
    """Server is overloaded / unavailable and caller should retry."""


class RetryLimitExceededError(ServerBusyError):
    """Server exhausted internal retry budget for a retryable operation."""


def _contains_retry_limit_text(message: str) -> bool:
    lowered = message.lower()
    return "retry limit" in lowered or "too many failed attempts" in lowered


def _is_overloaded_info(value: Any) -> bool:
    """Match the overloaded ``CodexErrorInfo`` value in either wire spelling.

    The runtime serializes the enum as camelCase (``serverOverloaded``) because
    ``CodexErrorInfo`` derives ``#[serde(rename_all = "camelCase")]``, while older
    payloads used snake_case.
    """

    if not isinstance(value, str):
        return False
    return value.replace("_", "").lower() == "serveroverloaded"


def _is_server_overloaded(data: Any) -> bool:
    if data is None:
        return False

    if _is_overloaded_info(data):
        return True

    if isinstance(data, dict):
        return any(
            _is_overloaded_info(key) or _is_server_overloaded(value) for key, value in data.items()
        )

    if isinstance(data, list):
        return any(_is_server_overloaded(value) for value in data)

    return False


def map_jsonrpc_error(code: int, message: str, data: Any = None) -> JsonRpcError:
    """Map a raw JSON-RPC error into a richer SDK exception class."""

    if code == -32700:
        return ParseError(code, message, data)
    if code == -32600:
        return InvalidRequestError(code, message, data)
    if code == -32601:
        return MethodNotFoundError(code, message, data)
    if code == -32602:
        return InvalidParamsError(code, message, data)
    if code == -32603:
        return InternalRpcError(code, message, data)

    if -32099 <= code <= -32000:
        if code == OVERLOADED_ERROR_CODE or _is_server_overloaded(data):
            if _contains_retry_limit_text(message):
                return RetryLimitExceededError(code, message, data)
            return ServerBusyError(code, message, data)
        if _contains_retry_limit_text(message):
            return RetryLimitExceededError(code, message, data)
        return CodexRpcError(code, message, data)

    return JsonRpcError(code, message, data)


def is_retryable_error(exc: BaseException) -> bool:
    """True if the exception is a transient overload-style error."""

    if isinstance(exc, ServerBusyError):
        return True

    if isinstance(exc, JsonRpcError):
        return exc.code == OVERLOADED_ERROR_CODE or _is_server_overloaded(exc.data)

    return False
