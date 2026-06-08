"""
Tests for exporting conversations to Markdown.

These guard against a Windows-specific regression: on Windows, the text-mode
default for open() is the locale codepage (e.g. cp1252), not UTF-8. Any
conversation containing emoji or non-Latin-1 characters (CJK, accents, ...)
therefore raised UnicodeEncodeError on export, which silently failed for
Windows users while working fine on the maintainers' POSIX machines (whose
default is already UTF-8). The fix is to pass encoding="utf-8" explicitly.
"""

import builtins

from interpreter.terminal_interface.utils.export_to_markdown import export_to_markdown

# A user/assistant exchange that mixes an accent, CJK and an emoji - all of
# which are unrepresentable in cp1252 and so trip the bug.
NON_ASCII_MESSAGES = [
    {"role": "user", "type": "message", "content": "Comment dit-on « café » ? 你好 🎉"},
    {
        "role": "assistant",
        "type": "message",
        "content": "On dit « café ». 干杯 🥂",
    },
]


def _force_windows_default_encoding(monkeypatch):
    """Make open() behave like Windows: default text encoding is cp1252.

    Python resolves the default text encoding from the OS, so on macOS/Linux
    the bug is invisible. We wrap builtins.open so that any text-mode call
    without an explicit ``encoding`` falls back to cp1252, reproducing the
    Windows codepage behaviour deterministically and cross-platform.
    """
    real_open = builtins.open

    def windows_open(file, mode="r", *args, **kwargs):
        if "b" not in mode and "encoding" not in kwargs and len(args) < 3:
            kwargs["encoding"] = "cp1252"
        return real_open(file, mode, *args, **kwargs)

    monkeypatch.setattr(builtins, "open", windows_open)
    return real_open


def test_export_markdown_unicode(tmp_path, monkeypatch):
    """Exporting non-ASCII content must not crash and must round-trip as UTF-8.

    Under the simulated Windows codepage, the pre-fix bare open(path, "w")
    raised UnicodeEncodeError. With encoding="utf-8" the export succeeds and
    the emoji/CJK/accented text survives a UTF-8 read.
    """
    real_open = _force_windows_default_encoding(monkeypatch)
    export_path = tmp_path / "conversation.md"

    # Pre-fix this line raises UnicodeEncodeError under the cp1252 default.
    export_to_markdown(NON_ASCII_MESSAGES, str(export_path))

    # Read back with the real open so the wrapper's cp1252 fallback can't mask
    # a wrongly-encoded file - the bytes on disk must be valid UTF-8.
    with real_open(export_path, "r", encoding="utf-8") as f:
        contents = f.read()

    assert "café" in contents
    assert "你好" in contents
    assert "🎉" in contents
    assert "干杯" in contents
