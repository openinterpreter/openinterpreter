#!/usr/bin/env python3
"""
oi_os_mode.py -- Launch OpenInterpreter as a managed subprocess for full
autonomous computer use (OS Mode) or local agent mode (Ollama).

OS Mode: OI runs its own screenshot → Claude API → pyautogui loop.
Local Mode: OI runs in classic code-execution mode with Ollama as backend.

Usage:
    oi_os_mode.py "Open Calculator and compute 2+2"
    oi_os_mode.py --provider anthropic "Change the wallpaper"
    oi_os_mode.py --local "What apps are open?"
    oi_os_mode.py --local --model llama3.2-vision "Describe the screen"
    oi_os_mode.py --timeout 120 "Fill out the form"
"""

import argparse
import os
import subprocess
import sys


def find_interpreter():
    """Find the interpreter CLI."""
    import shutil
    path = shutil.which("interpreter")
    if path:
        return path
    # Try common locations
    for candidate in [
        os.path.expanduser("~/.local/bin/interpreter"),
        "/usr/local/bin/interpreter",
    ]:
        if os.path.exists(candidate):
            return candidate
    return None


def run_os_mode(task, provider="anthropic", timeout=300):
    """Run OI in OS Mode (screenshot-driven, Claude API)."""
    interpreter_path = find_interpreter()
    if not interpreter_path:
        print("Error: 'interpreter' CLI not found. Run: ~/.claude/skills/open-interpreter/scripts/oi_install.sh",
              file=sys.stderr)
        sys.exit(1)

    # Check API key
    if provider == "anthropic" and not os.environ.get("ANTHROPIC_API_KEY"):
        print("Error: ANTHROPIC_API_KEY not set. Required for OS Mode.", file=sys.stderr)
        sys.exit(1)

    cmd = [interpreter_path, "--os", "-y"]

    print(f"[oi] OS Mode: provider={provider}, task={repr(task)}", file=sys.stderr)

    try:
        proc = subprocess.Popen(
            cmd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

        # Send task and close stdin to signal end of input
        stdout, stderr = proc.communicate(input=task + "\n", timeout=timeout)

        if stdout:
            print(stdout)
        if stderr:
            print(stderr, file=sys.stderr)

        return proc.returncode

    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()
        print(f"Error: OS Mode timed out after {timeout}s", file=sys.stderr)
        return 1
    except FileNotFoundError:
        print("Error: 'interpreter' CLI not found", file=sys.stderr)
        return 1


def run_local_mode(task, model="llama3.2-vision", api_base="http://localhost:11434", timeout=300):
    """Run OI in classic mode with Ollama as backend."""
    interpreter_path = find_interpreter()
    if not interpreter_path:
        print("Error: 'interpreter' CLI not found. Run: ~/.claude/skills/open-interpreter/scripts/oi_install.sh",
              file=sys.stderr)
        sys.exit(1)

    # Verify Ollama is running
    try:
        import urllib.request
        urllib.request.urlopen(f"{api_base}/api/tags", timeout=3)
    except Exception:
        print(f"Error: Ollama not reachable at {api_base}. Start with: ollama serve", file=sys.stderr)
        sys.exit(1)

    cmd = [
        interpreter_path,
        "--model", f"ollama/{model}",
        "--api_base", api_base,
        "-y",
    ]

    print(f"[oi] Local Mode: model=ollama/{model}, task={repr(task)}", file=sys.stderr)

    try:
        proc = subprocess.Popen(
            cmd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

        stdout, stderr = proc.communicate(input=task + "\n", timeout=timeout)

        if stdout:
            print(stdout)
        if stderr:
            print(stderr, file=sys.stderr)

        return proc.returncode

    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()
        print(f"Error: Local Mode timed out after {timeout}s", file=sys.stderr)
        return 1


def main():
    parser = argparse.ArgumentParser(
        description="Launch OpenInterpreter for autonomous computer use"
    )
    parser.add_argument("task", help="Task description for OI to execute")
    parser.add_argument("--local", action="store_true",
                        help="Use local Ollama model instead of Claude API")
    parser.add_argument("--model", default="llama3.2-vision",
                        help="Ollama model for local mode (default: llama3.2-vision)")
    parser.add_argument("--provider", default="anthropic",
                        help="API provider for OS Mode (default: anthropic). Currently only validates anthropic.")
    parser.add_argument("--api-base", default="http://localhost:11434",
                        help="Ollama API base URL (default: http://localhost:11434)")
    parser.add_argument("--timeout", type=int, default=300,
                        help="Timeout in seconds (default: 300)")

    args = parser.parse_args()

    if args.local:
        rc = run_local_mode(
            args.task,
            model=args.model,
            api_base=args.api_base,
            timeout=args.timeout,
        )
    else:
        rc = run_os_mode(
            args.task,
            provider=args.provider,
            timeout=args.timeout,
        )

    sys.exit(rc)


if __name__ == "__main__":
    main()
