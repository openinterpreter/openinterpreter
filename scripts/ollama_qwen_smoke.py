#!/usr/bin/env python3
"""Run a bounded CPU-only Ollama/Qwen tool-use smoke test.

The harness is deliberately self-contained: it can use an existing Ollama
server and model cache, or install and start a user-local Ollama binary. It
records the exact command, bounded output, and on-disk result so a successful
assistant message cannot be mistaken for a successful file operation.
"""

from __future__ import annotations

import argparse
from dataclasses import asdict, dataclass
import json
import os
from pathlib import Path, PurePosixPath
import platform
import select
import shutil
import signal
import subprocess
import sys
import tarfile
import time
from typing import Any, Iterable, Sequence
from urllib.error import URLError
from urllib.request import Request, urlopen


DEFAULT_MODEL = "qwen2.5-coder:1.5b"
VERIFIED_QWEN_MODELS = (DEFAULT_MODEL, "qwen2.5-coder:7b")
DEFAULT_OLLAMA_VERSION = "0.13.4"
DEFAULT_HOST = "127.0.0.1:11434"
DEFAULT_CONTEXT_LENGTH = 32768
DEFAULT_STARTUP_TIMEOUT_SECONDS = 180
DEFAULT_PULL_TIMEOUT_SECONDS = 7200
DEFAULT_RUN_TIMEOUT_SECONDS = 3600
DEFAULT_MAX_TRACE_BYTES = 10 * 1024 * 1024
DEFAULT_MAX_LOG_BYTES = 2 * 1024 * 1024
DEFAULT_EXPECTED_FILE = "oix-ollama-smoke.txt"
DEFAULT_EXPECTED_CONTENT = "open-interpreter ollama qwen smoke\n"


class HarnessError(RuntimeError):
    """An expected setup or execution failure in the smoke harness."""


@dataclass(frozen=True)
class CommandResult:
    returncode: int | None
    timed_out: bool
    output: bytes
    total_bytes: int
    truncated: bool


@dataclass(frozen=True)
class SideEffectResult:
    verified: bool
    path: str
    reason: str


class BoundedCapture:
    """Retain a bounded trace prefix while continuing to drain the process."""

    def __init__(self, max_bytes: int) -> None:
        if max_bytes < 128:
            raise ValueError("max_bytes must be at least 128")
        self.max_bytes = max_bytes
        self.total_bytes = 0
        self._data = bytearray()
        self.truncated = False

    def add(self, chunk: bytes) -> None:
        self.total_bytes += len(chunk)
        available = self.max_bytes - len(self._data)
        if available > 0:
            self._data.extend(chunk[:available])
        if len(chunk) > available:
            self.truncated = True

    def data(self) -> bytes:
        if not self.truncated:
            return bytes(self._data)

        omitted = self.total_bytes - self.max_bytes
        marker = f"\n... {omitted} bytes omitted from bounded trace ...\n".encode()
        return bytes(self._data[: self.max_bytes - len(marker)]) + marker


def _terminate_process(process: subprocess.Popen[bytes]) -> None:
    if os.name == "posix":
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            return
    else:
        process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        if os.name == "posix":
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                return
        else:
            process.kill()
        process.wait()


def run_bounded_command(
    args: Sequence[str],
    *,
    timeout_seconds: float,
    env: dict[str, str] | None = None,
    cwd: Path | None = None,
    max_bytes: int = DEFAULT_MAX_LOG_BYTES,
) -> CommandResult:
    """Run a command while draining and bounding combined stdout/stderr."""

    process = subprocess.Popen(
        list(args),
        cwd=cwd,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        start_new_session=os.name == "posix",
    )
    assert process.stdout is not None
    capture = BoundedCapture(max_bytes)
    deadline = time.monotonic() + timeout_seconds
    timed_out = False

    try:
        while True:
            if not timed_out and time.monotonic() >= deadline:
                timed_out = True
                _terminate_process(process)

            ready, _, _ = select.select([process.stdout], [], [], 0.1)
            if ready:
                # `BufferedReader.read()` may wait for its requested size or
                # EOF even after select says that a smaller amount is ready.
                # Read directly from the descriptor so sparse output cannot
                # hold the deadline loop hostage.
                chunk = os.read(process.stdout.fileno(), 65536)
                if chunk:
                    capture.add(chunk)
                else:
                    process.stdout.close()
                    break
            elif process.poll() is not None:
                # A descendant can keep the pipe open after the leader exits.
                # Treat that as a leaked command tree and terminate its group.
                _terminate_process(process)
                break
    finally:
        if process.poll() is None:
            _terminate_process(process)
        process.wait()
        process.stdout.close()

    return CommandResult(
        returncode=process.returncode,
        timed_out=timed_out,
        output=capture.data(),
        total_bytes=capture.total_bytes,
        truncated=capture.truncated,
    )


def normalize_host(host: str) -> str:
    value = host.strip().rstrip("/")
    if not value:
        raise ValueError("Ollama host must not be empty")
    if "://" not in value:
        value = f"http://{value}"
    return value


def ollama_is_ready(host: str, timeout_seconds: float = 2) -> bool:
    try:
        request = Request(f"{normalize_host(host)}/api/version")
        with urlopen(request, timeout=timeout_seconds) as response:
            payload = json.load(response)
    except (OSError, URLError, ValueError, json.JSONDecodeError):
        return False
    return isinstance(payload, dict) and isinstance(payload.get("version"), str)


def parse_ollama_list(output: str) -> dict[str, str | None]:
    models: dict[str, str | None] = {}
    for line in output.splitlines():
        fields = line.split()
        if not fields or fields[0].upper() == "NAME":
            continue
        models[fields[0]] = fields[1] if len(fields) > 1 else None
    return models


def select_model(requested: str | None, installed: Iterable[str]) -> str:
    if requested:
        if not requested.lower().startswith("qwen"):
            raise ValueError(f"model must be a Qwen model: {requested}")
        return requested

    installed_set = set(installed)
    for model in VERIFIED_QWEN_MODELS:
        if model in installed_set:
            return model
    return DEFAULT_MODEL


def validate_relative_file(value: str) -> str:
    path = PurePosixPath(value)
    if not value or path.is_absolute() or ".." in path.parts:
        raise ValueError("expected file must be a non-empty path inside the workspace")
    return path.as_posix()


def build_prompt(
    user_prompt: str | None, expected_file: str, expected_content: str
) -> str:
    task = (
        user_prompt
        or "Perform a direct file-write smoke test in the current workspace."
    )
    return (
        f"{task}\n\n"
        "Required observable check: use the available file tool to create the file "
        f"`{expected_file}` in the current workspace. Write exactly this UTF-8 "
        f"content, including the final newline:\n{expected_content}"
        "Do not merely describe the action; perform it before finishing."
    )


def build_interpreter_command(
    interpreter: Path,
    workspace: Path,
    model: str,
    prompt: str,
) -> list[str]:
    return [
        str(interpreter),
        "exec",
        "--json",
        "--ephemeral",
        "--skip-git-repo-check",
        "--cd",
        str(workspace),
        "--oss",
        "--local-provider",
        "ollama",
        "--chat-completions",
        "--model",
        model,
        "--sandbox",
        "workspace-write",
        prompt,
    ]


TOOL_EVENT_TYPES = {
    "command_execution",
    "custom_tool_call",
    "file_change",
    "function_call",
    "mcp_tool_call",
    "tool_call",
    "tool_use",
}
TOOL_KEYS = {
    "command_execution",
    "custom_tool_call",
    "file_change",
    "function_call",
    "mcp_tool_call",
    "tool_call",
    "tool_name",
    "toolName",
    "tool_use",
}


def _tool_signals(value: Any, signals: list[str]) -> None:
    if isinstance(value, dict):
        event_type = str(value.get("type", "")).lower()
        if event_type in TOOL_EVENT_TYPES:
            signals.append(event_type)
        for key, child in value.items():
            if key in TOOL_KEYS and child not in (None, "", [], {}):
                signals.append(key)
            if key == "tool_calls" and isinstance(child, list):
                signals.extend("tool_calls" for _ in child)
            _tool_signals(child, signals)
    elif isinstance(value, list):
        for item in value:
            _tool_signals(item, signals)


def detect_tool_calls(trace: str) -> list[str]:
    signals: list[str] = []
    for line in trace.splitlines():
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        _tool_signals(value, signals)
    return signals


CRASH_MARKERS = (
    "panicked at",
    "segmentation fault",
    "sigsegv",
    "abort trap",
    "fatal runtime error",
)


def classify_attempt(
    *,
    returncode: int | None,
    timed_out: bool,
    trace: str,
    side_effect: SideEffectResult,
) -> str:
    if timed_out:
        return "interpreter_loop"
    if (
        returncode is None
        or returncode < 0
        or any(marker in trace.lower() for marker in CRASH_MARKERS)
    ):
        return "interpreter_crash"
    if returncode != 0:
        return "interpreter_failure"
    if not detect_tool_calls(trace):
        return "model_incompatible_no_tool_call"
    if not side_effect.verified:
        return "tool_call_without_requested_side_effect"
    return "passed"


def verify_side_effect(
    workspace: Path, expected_file: str, expected_content: str
) -> SideEffectResult:
    path = workspace / expected_file
    relative = Path(expected_file).as_posix()
    if not path.is_file():
        return SideEffectResult(False, relative, "file_missing")
    actual = path.read_bytes()
    if actual != expected_content.encode():
        return SideEffectResult(False, relative, "content_mismatch")
    return SideEffectResult(True, relative, "exact_content_match")


def cpu_only_environment(
    base: dict[str, str], *, host: str, models_dir: Path, context_length: int
) -> dict[str, str]:
    env = dict(base)
    ollama_host = host.removeprefix("http://").removeprefix("https://")
    env.update(
        {
            "OLLAMA_HOST": ollama_host,
            "OLLAMA_MODELS": str(models_dir),
            "OLLAMA_CONTEXT_LENGTH": str(context_length),
            "CUDA_VISIBLE_DEVICES": "",
            "ROCR_VISIBLE_DEVICES": "",
        }
    )
    return env


def _default_cache_dir() -> Path:
    root = os.environ.get("XDG_CACHE_HOME")
    return (
        Path(root) / "open-interpreter" / "ollama"
        if root
        else Path.home() / ".cache" / "open-interpreter" / "ollama"
    )


def _ollama_archive_name() -> str:
    machine = platform.machine().lower()
    if machine in {"x86_64", "amd64"}:
        return "ollama-linux-amd64.tgz"
    if machine in {"aarch64", "arm64"}:
        return "ollama-linux-arm64.tgz"
    raise HarnessError(f"unsupported Linux architecture for Ollama: {machine}")


def find_or_install_ollama(
    *,
    explicit: str | None,
    version: str,
    cache_dir: Path,
) -> Path:
    candidates = [explicit] if explicit else []
    if not explicit:
        found = shutil.which("ollama")
        if found:
            candidates.append(found)
        candidates.append(str(cache_dir / "ollama" / version / "bin" / "ollama"))
    for candidate in candidates:
        if candidate and Path(candidate).is_file() and os.access(candidate, os.X_OK):
            return Path(candidate).resolve()
    install_dir = cache_dir / "ollama" / version
    binary = install_dir / "bin" / "ollama"
    if not binary.exists():
        install_dir.mkdir(parents=True, exist_ok=True)
        archive = install_dir / _ollama_archive_name()
        url = (
            "https://github.com/ollama/ollama/releases/download/"
            f"v{version}/{archive.name}"
        )
        try:
            with urlopen(url, timeout=60) as response, archive.open("wb") as output:
                shutil.copyfileobj(response, output)
        except (OSError, URLError) as error:
            raise HarnessError(
                f"failed to download Ollama from {url}: {error}"
            ) from error
        with tarfile.open(archive, "r:gz") as tar:
            root = install_dir.resolve()
            for member in tar.getmembers():
                target = (install_dir / member.name).resolve()
                if target != root and root not in target.parents:
                    raise HarnessError(f"unsafe path in Ollama archive: {member.name}")
            tar.extractall(install_dir)
    if not binary.is_file():
        raise HarnessError(f"Ollama archive did not provide {binary}")
    binary.chmod(binary.stat().st_mode | 0o111)
    return binary.resolve()


def _start_ollama(
    ollama: Path,
    *,
    env: dict[str, str],
    startup_timeout: float,
) -> subprocess.Popen[bytes]:
    process = subprocess.Popen(
        [str(ollama), "serve"],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.STDOUT,
        start_new_session=os.name == "posix",
    )
    deadline = time.monotonic() + startup_timeout
    while time.monotonic() < deadline:
        if ollama_is_ready(env["OLLAMA_HOST"]):
            return process
        if process.poll() is not None:
            raise HarnessError(
                f"Ollama exited before becoming ready (exit {process.returncode})"
            )
        time.sleep(1)
    _terminate_process(process)
    raise HarnessError(f"Ollama did not become ready within {startup_timeout:g}s")


def _write_json(path: Path, value: Any) -> None:
    path.write_text(
        json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


def _run_attempt(
    *,
    attempt: int,
    root: Path,
    interpreter: Path,
    model: str,
    host: str,
    prompt: str,
    expected_file: str,
    expected_content: str,
    env: dict[str, str],
    timeout_seconds: float,
    max_trace_bytes: int,
) -> dict[str, Any]:
    attempt_root = root / f"attempt-{attempt}"
    workspace = attempt_root / "workspace"
    home = attempt_root / "interpreter-home"
    attempt_root.mkdir()
    workspace.mkdir()
    home.mkdir()
    (workspace / "src").mkdir()
    (workspace / "README.md").write_text(
        "# Ollama/Qwen smoke fixture\n\nThis file is intentionally small.\n",
        encoding="utf-8",
    )
    (workspace / "src" / "main.py").write_text(
        'def greeting(name: str) -> str:\n    return f"hello {name}"\n',
        encoding="utf-8",
    )
    command = build_interpreter_command(interpreter, workspace, model, prompt)
    attempt_env = dict(env)
    attempt_env.update(
        {
            "CODEX_OSS_BASE_URL": f"{normalize_host(host)}/v1",
            "INTERPRETER_HOME": str(home),
            "CODEX_HOME": str(home),
        }
    )
    result = run_bounded_command(
        command,
        timeout_seconds=timeout_seconds,
        env=attempt_env,
        cwd=workspace,
        max_bytes=max_trace_bytes,
    )
    (attempt_root / "interpreter-trace.jsonl").write_bytes(result.output)
    side_effect = verify_side_effect(workspace, expected_file, expected_content)
    trace = result.output.decode("utf-8", errors="replace")
    status = classify_attempt(
        returncode=result.returncode,
        timed_out=result.timed_out,
        trace=trace,
        side_effect=side_effect,
    )
    attempt_result = {
        "attempt": attempt,
        "status": status,
        "returncode": result.returncode,
        "timedOut": result.timed_out,
        "toolCallCount": len(detect_tool_calls(trace)),
        "traceBytes": result.total_bytes,
        "traceTruncated": result.truncated,
        "sideEffect": asdict(side_effect),
    }
    _write_json(attempt_root / "result.json", attempt_result)
    return attempt_result


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Run a bounded Linux CPU-only Ollama/Qwen tool-use smoke test."
    )
    parser.add_argument(
        "--interpreter", help="path to interpreter (default: PATH lookup)"
    )
    parser.add_argument("--model", help=f"Qwen model (default: {DEFAULT_MODEL})")
    parser.add_argument("--ollama-version", default=DEFAULT_OLLAMA_VERSION)
    parser.add_argument("--ollama", help="path to an Ollama binary")
    parser.add_argument("--host", default=os.environ.get("OLLAMA_HOST", DEFAULT_HOST))
    parser.add_argument(
        "--cache-dir",
        type=Path,
        default=Path(os.environ["OIX_OLLAMA_CACHE"])
        if os.environ.get("OIX_OLLAMA_CACHE")
        else _default_cache_dir(),
        help="durable cache for Ollama binaries and models",
    )
    parser.add_argument("--output-dir", type=Path, help="artifact directory")
    parser.add_argument("--attempts", type=int, default=1)
    parser.add_argument(
        "--startup-timeout", type=float, default=DEFAULT_STARTUP_TIMEOUT_SECONDS
    )
    parser.add_argument(
        "--pull-timeout", type=float, default=DEFAULT_PULL_TIMEOUT_SECONDS
    )
    parser.add_argument(
        "--run-timeout", type=float, default=DEFAULT_RUN_TIMEOUT_SECONDS
    )
    parser.add_argument("--max-trace-bytes", type=int, default=DEFAULT_MAX_TRACE_BYTES)
    parser.add_argument(
        "--prompt", help="additional task text before the required file write"
    )
    parser.add_argument("--expected-file", default=DEFAULT_EXPECTED_FILE)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    if platform.system() != "Linux":
        print("This harness supports Linux only.", file=sys.stderr)
        return 2
    try:
        for name in (
            "attempts",
            "startup_timeout",
            "pull_timeout",
            "run_timeout",
            "max_trace_bytes",
        ):
            if getattr(args, name) <= 0:
                raise HarnessError(f"{name} must be positive")
        if not 1 <= args.attempts <= 5:
            raise HarnessError("attempts must be between 1 and 5")
        expected_file = validate_relative_file(args.expected_file)
        root = args.output_dir or Path.cwd() / f"ollama-qwen-smoke-{int(time.time())}"
        root = root.expanduser().resolve()
        if root.exists() and any(root.iterdir()):
            raise HarnessError(f"artifact directory is not empty: {root}")
        root.mkdir(parents=True, exist_ok=False)
        cache_dir = args.cache_dir.expanduser().resolve()
        models_dir = Path(os.environ.get("OLLAMA_MODELS", cache_dir / "models"))
        host = normalize_host(args.host)
        base_env = cpu_only_environment(
            os.environ,
            host=host,
            models_dir=models_dir,
            context_length=DEFAULT_CONTEXT_LENGTH,
        )

        ollama = find_or_install_ollama(
            explicit=args.ollama,
            version=args.ollama_version,
            cache_dir=cache_dir,
        )
        version_result = run_bounded_command(
            [str(ollama), "--version"], timeout_seconds=30, env=base_env
        )
        (root / "ollama-version.txt").write_bytes(version_result.output)
        if version_result.returncode != 0:
            raise HarnessError("Ollama --version failed")

        server_process: subprocess.Popen[bytes] | None = None
        reused_server = ollama_is_ready(host)
        if not reused_server:
            server_process = _start_ollama(
                ollama,
                env=base_env,
                startup_timeout=args.startup_timeout,
            )
        try:
            list_result = run_bounded_command(
                [str(ollama), "list"], timeout_seconds=60, env=base_env
            )
            (root / "ollama-list.txt").write_bytes(list_result.output)
            installed = parse_ollama_list(
                list_result.output.decode("utf-8", errors="replace")
            )
            model = select_model(args.model, installed)
            if model not in installed:
                pull_result = run_bounded_command(
                    [str(ollama), "pull", model],
                    timeout_seconds=args.pull_timeout,
                    env=base_env,
                    max_bytes=DEFAULT_MAX_LOG_BYTES,
                )
                (root / "ollama-pull.log").write_bytes(pull_result.output)
                if pull_result.timed_out or pull_result.returncode != 0:
                    raise HarnessError(f"failed to pull model {model}")
                list_result = run_bounded_command(
                    [str(ollama), "list"], timeout_seconds=60, env=base_env
                )
                (root / "ollama-list-after-pull.txt").write_bytes(list_result.output)
                installed = parse_ollama_list(
                    list_result.output.decode("utf-8", errors="replace")
                )
            if model not in installed:
                raise HarnessError(f"Ollama does not list pulled model {model}")
            prompt = build_prompt(args.prompt, expected_file, DEFAULT_EXPECTED_CONTENT)
            interpreter_value = (
                args.interpreter
                or os.environ.get("INTERPRETER_BIN")
                or shutil.which("interpreter")
            )
            if not interpreter_value:
                raise HarnessError("interpreter was not found; pass --interpreter")
            interpreter = Path(interpreter_value).expanduser().resolve()
            if not interpreter.is_file() or not os.access(interpreter, os.X_OK):
                raise HarnessError(f"interpreter is not executable: {interpreter}")
            attempts = [
                _run_attempt(
                    attempt=attempt,
                    root=root,
                    interpreter=interpreter,
                    model=model,
                    host=host,
                    prompt=prompt,
                    expected_file=expected_file,
                    expected_content=DEFAULT_EXPECTED_CONTENT,
                    env=base_env,
                    timeout_seconds=args.run_timeout,
                    max_trace_bytes=args.max_trace_bytes,
                )
                for attempt in range(1, args.attempts + 1)
            ]
            model_digest = installed.get(model)
            summary = {
                "schemaVersion": 1,
                "status": "passed"
                if all(item["status"] == "passed" for item in attempts)
                else "failed",
                "model": model,
                "modelDigest": model_digest,
                "ollamaVersion": version_result.output.decode(
                    "utf-8", errors="replace"
                ).strip(),
                "endpoint": f"{host}/v1",
                "wireApi": "chat_completions",
                "provider": "ollama",
                "cpuOnly": True,
                "serverReused": reused_server,
                "cacheDir": str(cache_dir),
                "modelsDir": str(models_dir),
                "contextLength": DEFAULT_CONTEXT_LENGTH,
                "timeouts": {
                    "startup": args.startup_timeout,
                    "pull": args.pull_timeout,
                    "run": args.run_timeout,
                },
                "attempts": attempts,
            }
            _write_json(root / "summary.json", summary)
            return 0 if summary["status"] == "passed" else 1
        finally:
            if server_process is not None:
                _terminate_process(server_process)
    except (HarnessError, OSError, ValueError) as error:
        print(f"ollama/qwen smoke harness failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
