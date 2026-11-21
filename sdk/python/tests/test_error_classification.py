from __future__ import annotations

import pytest

from openai_codex.errors import (
    CodexRpcError,
    RetryLimitExceededError,
    ServerBusyError,
    is_retryable_error,
    map_jsonrpc_error,
)
from openai_codex.retry import retry_on_overload

# codex-rs/app-server/src/error_code.rs
OVERLOADED_ERROR_CODE = -32001


@pytest.mark.parametrize(
    ("code", "data"),
    [
        # The runtime signals queue backpressure with a bare reserved code.
        (OVERLOADED_ERROR_CODE, None),
        # CodexErrorInfo is serialized as camelCase by the app-server protocol.
        (-32000, {"codexErrorInfo": "serverOverloaded"}),
        (-32000, {"codex_error_info": "server_overloaded"}),
        (-32000, {"codexErrorInfo": {"serverOverloaded": None}}),
        (-32000, ["serverOverloaded"]),
    ],
)
def test_overloaded_errors_map_to_server_busy(code: int, data: object) -> None:
    error = map_jsonrpc_error(code, "the server is overloaded", data)

    assert isinstance(error, ServerBusyError)
    assert is_retryable_error(error)


def test_overloaded_code_with_retry_limit_text_maps_to_retry_limit_exceeded() -> None:
    error = map_jsonrpc_error(OVERLOADED_ERROR_CODE, "retry limit reached", None)

    assert isinstance(error, RetryLimitExceededError)


def test_unrelated_server_error_stays_generic_and_is_not_retryable() -> None:
    error = map_jsonrpc_error(-32000, "expected active turn id", {"reason": "turnTransition"})

    assert type(error) is CodexRpcError
    assert not is_retryable_error(error)


def test_retry_on_overload_retries_a_real_overload_response() -> None:
    attempts: list[int] = []

    def flaky() -> str:
        attempts.append(len(attempts) + 1)
        if len(attempts) < 3:
            raise map_jsonrpc_error(
                OVERLOADED_ERROR_CODE, "in-process app-server request queue is full", None
            )
        return "done"

    result = retry_on_overload(flaky, max_attempts=3, initial_delay_s=0.0, max_delay_s=0.0)

    assert result == "done"
    assert len(attempts) == 3
