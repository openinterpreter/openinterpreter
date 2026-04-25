"""
Regression tests for `interpreter.core.utils.system_debug_info.interpreter_info`.

Bug (issue #1218 / surfaced in #1083): when `interpreter.offline` is True and
`interpreter.llm.api_base` is set, the diagnostic helper invokes

    subprocess.check_output(f"curl {interpreter.llm.api_base}")

with a single string but no `shell=True`. `subprocess` then treats the entire
string as the path to an executable and raises `FileNotFoundError`. The
exception is caught and stringified, so `%info` shows e.g.
`Curl output: [Errno 2] No such file or directory: 'curl http://...'`
instead of the actual server response.

These tests load the module by file path (so the heavy `interpreter` package
__init__ is not evaluated) and stub `subprocess.check_output` to capture how
curl is invoked.
"""

import importlib.util
import os
import sys
from types import SimpleNamespace
from unittest.mock import patch

import pytest


REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
MODULE_PATH = os.path.join(
    REPO_ROOT, "interpreter", "core", "utils", "system_debug_info.py"
)


def _load_module():
    spec = importlib.util.spec_from_file_location("sdi_under_test", MODULE_PATH)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


@pytest.fixture
def sdi():
    return _load_module()


def _fake_interpreter(api_base="http://localhost:1234/v1"):
    llm = SimpleNamespace(
        api_base=api_base,
        supports_vision=False,
        model="test-model",
        supports_functions=False,
        context_window=2048,
        max_tokens=512,
    )
    computer = SimpleNamespace(import_computer_api=False)
    return SimpleNamespace(
        offline=True,
        llm=llm,
        computer=computer,
        auto_run=False,
        messages=[],
        system_message="sys",
    )


def test_interpreter_info_invokes_curl_with_argv_list(sdi):
    """
    The curl subprocess call MUST split args (or use shell=True) so the
    binary 'curl' can actually be found and executed.
    """
    captured = {}

    def fake_check_output(cmd, *args, **kwargs):
        captured["cmd"] = cmd
        captured["kwargs"] = kwargs
        return b"PONG"

    interp = _fake_interpreter()
    with patch.object(sdi.subprocess, "check_output", side_effect=fake_check_output):
        out = sdi.interpreter_info(interp)

    cmd = captured.get("cmd")
    # Either a list of args, or a string with shell=True. Both invoke curl correctly.
    if isinstance(cmd, str):
        assert captured["kwargs"].get("shell") is True, (
            "string command must use shell=True so curl is resolved"
        )
        assert cmd.split()[0] == "curl"
    else:
        assert isinstance(cmd, (list, tuple)), f"unexpected cmd type: {type(cmd)}"
        assert cmd[0] == "curl"
        assert "http://localhost:1234/v1" in cmd

    # Real output should appear in the rendered info string, not an exception text.
    assert "PONG" in out
    assert "No such file or directory" not in out


def test_interpreter_info_does_not_hang_on_unresponsive_server(sdi):
    """
    A timeout should be passed so that an unresponsive api_base does not hang
    the `%info` magic command (root cause behind #1218 'no output').
    """
    captured = {}

    def fake_check_output(cmd, *args, **kwargs):
        captured["kwargs"] = kwargs
        return b"OK"

    interp = _fake_interpreter()
    with patch.object(sdi.subprocess, "check_output", side_effect=fake_check_output):
        sdi.interpreter_info(interp)

    assert "timeout" in captured["kwargs"], (
        "curl call must pass a timeout to avoid hanging %info on a dead api_base"
    )
    assert captured["kwargs"]["timeout"] is not None
    assert captured["kwargs"]["timeout"] > 0


def test_interpreter_info_handles_curl_failure_gracefully(sdi):
    """
    If curl fails (network down, server off), interpreter_info still returns
    a string and does not propagate the exception.
    """
    import subprocess as real_subprocess

    def fake_check_output(cmd, *args, **kwargs):
        raise real_subprocess.CalledProcessError(7, cmd, b"", b"connection refused")

    interp = _fake_interpreter()
    with patch.object(sdi.subprocess, "check_output", side_effect=fake_check_output):
        out = sdi.interpreter_info(interp)

    assert isinstance(out, str)
    assert "Curl output" in out


def test_interpreter_info_skips_curl_when_not_offline(sdi):
    """
    When the interpreter is online, curl must not be called at all.
    """
    interp = _fake_interpreter()
    interp.offline = False

    def fake_check_output(cmd, *args, **kwargs):  # pragma: no cover - must not run
        raise AssertionError("curl must not be invoked when offline=False")

    with patch.object(sdi.subprocess, "check_output", side_effect=fake_check_output):
        out = sdi.interpreter_info(interp)

    assert "Not local" in out
