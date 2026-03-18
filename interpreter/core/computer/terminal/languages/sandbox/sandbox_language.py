"""
SandboxLanguage - Base class for sandbox-backed language execution.

Extends BaseLanguage and delegates all execution to a shared SandboxManager.
"""

from ...base_language import BaseLanguage


class SandboxLanguage(BaseLanguage):
    """
    Base class for languages that execute code in an OpenSandbox container.

    Subclasses set `name`, `aliases`, `file_extension`, and `sandbox_lang`
    to define which language they handle.
    """

    _is_sandbox_language = True  # Marker for Terminal instantiation logic
    sandbox_lang = None  # Override in subclasses: "python", "bash", "javascript"

    def __init__(self, sandbox_manager):
        self.sandbox_manager = sandbox_manager

    def run(self, code):
        yield from self.sandbox_manager.execute(self.sandbox_lang, code)

    def stop(self):
        self.sandbox_manager.stop()

    def terminate(self):
        # Don't kill the whole sandbox from one language handler.
        # The SandboxManager.terminate() is called by Terminal.terminate().
        pass
