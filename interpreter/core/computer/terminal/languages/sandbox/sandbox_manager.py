"""
SandboxManager - Manages OpenSandbox lifecycle and code execution.

Owns a single sandbox instance and CodeInterpreterSync, providing
streaming code execution that yields LMC-format chunks.
"""

import os
import queue
import threading
import traceback

LANGUAGE_MAP = {
    "python": "python",
    "py": "python",
    "shell": "bash",
    "bash": "bash",
    "sh": "bash",
    "zsh": "bash",
    "javascript": "javascript",
    "js": "javascript",
}

# Default sandbox image for code interpreter
DEFAULT_IMAGE = "opensandbox/code-interpreter:latest"


class SandboxManager:
    def __init__(
        self,
        api_key=None,
        domain=None,
        image=None,
        timeout_minutes=10,
    ):
        self.api_key = api_key or os.environ.get("OPEN_SANDBOX_API_KEY")
        self.domain = domain or os.environ.get("OPEN_SANDBOX_DOMAIN")
        self.image = image or os.environ.get("OPEN_SANDBOX_IMAGE", DEFAULT_IMAGE)
        self.timeout_minutes = timeout_minutes

        self._sandbox = None
        self._code_interpreter = None
        self._contexts = {}  # language_name -> CodeContextSync
        self._current_execution_id = None
        self._lock = threading.Lock()

    def _ensure_sandbox(self):
        """Lazily create the sandbox and code interpreter on first use."""
        if self._sandbox is not None:
            return

        try:
            from opensandbox.sync.sandbox import SandboxSync
            from opensandbox.config import ConnectionConfig
            from code_interpreter.sync.code_interpreter import CodeInterpreterSync
        except ImportError:
            raise ImportError(
                "OpenSandbox packages are required for sandbox mode. "
                "Install with: pip install opensandbox opensandbox-code-interpreter"
            )

        if not self.api_key:
            raise ValueError(
                "OpenSandbox API key is required. "
                "Set via --sandbox_api_key or OPEN_SANDBOX_API_KEY env var."
            )
        if not self.domain:
            raise ValueError(
                "OpenSandbox domain is required. "
                "Set via --sandbox_domain or OPEN_SANDBOX_DOMAIN env var."
            )

        from datetime import timedelta

        config = ConnectionConfig(
            api_key=self.api_key,
            domain=self.domain,
        )

        self._sandbox = SandboxSync.create(
            self.image,
            connection_config=config,
            timeout=timedelta(minutes=self.timeout_minutes),
        )
        self._code_interpreter = CodeInterpreterSync.create(sandbox=self._sandbox)

    def _get_context(self, language):
        """Get or create an execution context for the given language."""
        sandbox_lang = LANGUAGE_MAP.get(language.lower())
        if sandbox_lang is None:
            raise ValueError(
                f"Language '{language}' is not supported in sandbox mode. "
                f"Supported: {list(LANGUAGE_MAP.keys())}"
            )

        if sandbox_lang not in self._contexts:
            self._contexts[sandbox_lang] = (
                self._code_interpreter.codes.create_context(sandbox_lang)
            )
        return self._contexts[sandbox_lang], sandbox_lang

    def execute(self, language, code):
        """
        Execute code in the sandbox. Generator yielding LMC-format dicts.

        Mirrors the streaming pattern from SubprocessLanguage.run().
        """
        try:
            self._ensure_sandbox()
        except Exception:
            yield {
                "type": "console",
                "format": "output",
                "content": traceback.format_exc(),
            }
            return

        try:
            context, sandbox_lang = self._get_context(language)
        except Exception:
            yield {
                "type": "console",
                "format": "output",
                "content": traceback.format_exc(),
            }
            return

        from opensandbox.models.execd_sync import ExecutionHandlersSync

        message_queue = queue.Queue()
        done_event = threading.Event()
        execution_result = [None]  # mutable container for thread result

        def on_stdout(msg):
            message_queue.put({
                "type": "console",
                "format": "output",
                "content": msg.text,
            })

        def on_stderr(msg):
            message_queue.put({
                "type": "console",
                "format": "output",
                "content": msg.text,
            })

        def on_error(err):
            tb = "\n".join(err.traceback) if err.traceback else ""
            content = f"{err.name}: {err.value}"
            if tb:
                content = f"{tb}\n{content}"
            message_queue.put({
                "type": "console",
                "format": "output",
                "content": content,
            })

        def on_execution_complete(complete):
            done_event.set()

        handlers = ExecutionHandlersSync(
            on_stdout=on_stdout,
            on_stderr=on_stderr,
            on_error=on_error,
            on_execution_complete=on_execution_complete,
        )

        def run_in_thread():
            try:
                result = self._code_interpreter.codes.run(
                    code,
                    context=context,
                    handlers=handlers,
                )
                execution_result[0] = result
                if result and result.id:
                    self._current_execution_id = result.id
            except Exception:
                message_queue.put({
                    "type": "console",
                    "format": "output",
                    "content": traceback.format_exc(),
                })
            finally:
                done_event.set()

        thread = threading.Thread(target=run_in_thread, daemon=True)
        thread.start()

        # Yield output as it arrives, same pattern as SubprocessLanguage
        while True:
            try:
                output = message_queue.get(timeout=0.3)
                yield output
            except queue.Empty:
                if done_event.is_set():
                    # Drain remaining items
                    while not message_queue.empty():
                        yield message_queue.get()
                    break

        # If execution produced results (e.g. expression values), yield them
        result = execution_result[0]
        if result and result.result:
            for r in result.result:
                if r.text:
                    yield {
                        "type": "console",
                        "format": "output",
                        "content": r.text,
                    }

        self._current_execution_id = None

    def stop(self):
        """Interrupt currently running execution."""
        exec_id = self._current_execution_id
        if exec_id and self._code_interpreter:
            try:
                self._code_interpreter.codes.interrupt(exec_id)
            except Exception:
                pass

    def terminate(self):
        """Kill the sandbox and release all resources."""
        if self._sandbox:
            try:
                self._sandbox.kill()
            except Exception:
                pass
            try:
                self._sandbox.close()
            except Exception:
                pass
        self._sandbox = None
        self._code_interpreter = None
        self._contexts = {}
        self._current_execution_id = None
