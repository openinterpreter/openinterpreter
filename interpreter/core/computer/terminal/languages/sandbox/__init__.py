"""
Sandbox-backed language implementations for OpenSandbox execution.
"""

from .sandbox_language import SandboxLanguage


class SandboxPython(SandboxLanguage):
    file_extension = "py"
    name = "Python"
    aliases = ["py"]
    sandbox_lang = "python"


class SandboxShell(SandboxLanguage):
    file_extension = "sh"
    name = "Shell"
    aliases = ["bash", "sh", "zsh"]
    sandbox_lang = "bash"


class SandboxJavaScript(SandboxLanguage):
    file_extension = "js"
    name = "JavaScript"
    aliases = ["js"]
    sandbox_lang = "javascript"
