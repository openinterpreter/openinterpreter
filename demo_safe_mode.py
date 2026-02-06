#!/usr/bin/env python3
"""
Demonstration of Safe Mode functionality without requiring Ollama.
This script simulates the safe mode wrapper behavior.
"""

import sys
import os
from pathlib import Path
import tempfile

# Add current directory to path
sys.path.insert(0, str(Path(__file__).parent))

from safe_mode import SafeMode


def demo_banner(title):
    """Print a demo section banner."""
    print("\n" + "=" * 60)
    print(f"  {title}")
    print("=" * 60 + "\n")


def demo_file_operations():
    """Demonstrate file operations."""
    demo_banner("DEMO 1: File Operations")
    
    config = {
        'workspace': '~/model_workspace',
        'allowed_extensions': ['.txt', '.py', '.json', '.md'],
        'blocked_modules': [],
        'blocked_keywords': [],
    }
    
    safe_mode = SafeMode(config)
    workspace = safe_mode.workspace_path
    
    print(f"📁 Workspace: {workspace}\n")
    
    # Create a file
    print("▶️  Creating file 'demo.txt'...")
    success, msg = safe_mode.file_manager.create_file("demo.txt", "Hello from Safe Mode!")
    print(f"   {msg}\n")
    
    # Read the file
    print("▶️  Reading file 'demo.txt'...")
    success, content = safe_mode.file_manager.read_file("demo.txt")
    if success:
        print(f"   Content: {content}\n")
    
    # List files
    print("▶️  Listing all files...")
    success, files = safe_mode.file_manager.list_files()
    if success:
        print(f"   {files}\n")
    
    # Try to escape workspace (will fail)
    print("▶️  Attempting to access '../outside.txt' (should fail)...")
    success, msg = safe_mode.file_manager.create_file("../outside.txt", "This should fail")
    print(f"   {msg}\n")
    
    # Delete the file
    print("▶️  Deleting file 'demo.txt'...")
    success, msg = safe_mode.file_manager.delete_file("demo.txt")
    print(f"   {msg}\n")


def demo_code_validation():
    """Demonstrate code validation."""
    demo_banner("DEMO 2: Code Validation")
    
    config = {
        'workspace': '~/model_workspace',
        'allowed_extensions': ['.txt', '.py'],
        'blocked_modules': ['subprocess', 'os.system', 'socket'],
        'blocked_keywords': ['eval', 'exec', 'system'],
    }
    
    safe_mode = SafeMode(config)
    
    test_cases = [
        ("Safe Python code", "python", "x = 1 + 2\nprint(x)", True),
        ("Shell command", "shell", "ls -la", False),
        ("Bash command", "bash", "echo hello", False),
        ("Import subprocess", "python", "import subprocess\nsubprocess.run(['ls'])", False),
        ("Using eval", "python", "eval('print(1)')", False),
        ("Using create_file", "python", "create_file('test.txt', 'hello')", True),
        ("Direct file open", "python", "with open('test.txt') as f:\n    f.read()", False),
    ]
    
    for name, language, code, should_pass in test_cases:
        print(f"▶️  Testing: {name}")
        print(f"   Language: {language}")
        print(f"   Code: {code[:50]}...")
        is_valid, error = safe_mode.validate_code(code, language)
        
        if should_pass:
            status = "✅ PASS" if is_valid else f"❌ FAIL (expected pass): {error}"
        else:
            status = "✅ PASS" if not is_valid else "❌ FAIL (expected block)"
            if not is_valid:
                status += f"\n   Reason: {error}"
        
        print(f"   {status}\n")


def demo_execution_wrapper():
    """Demonstrate how code execution is wrapped."""
    demo_banner("DEMO 3: Execution Wrapper Simulation")
    
    config = {
        'workspace': '~/model_workspace',
        'allowed_extensions': ['.txt', '.py', '.json'],
        'blocked_modules': ['subprocess', 'os.system'],
        'blocked_keywords': ['eval', 'exec'],
    }
    
    safe_mode = SafeMode(config)
    
    # Simulate Python code that uses safe functions
    user_code = """
# User wants to create and read a file
success, msg = create_file('example.txt', 'Hello World!')
print(msg)

success, content = read_file('example.txt')
if success:
    print(f"File content: {content}")
"""
    
    print("▶️  User code to execute:")
    print("-" * 60)
    print(user_code)
    print("-" * 60)
    print()
    
    # Validate
    print("▶️  Validating code...")
    is_valid, error = safe_mode.validate_code(user_code, "python")
    if is_valid:
        print("   ✅ Code validation passed")
    else:
        print(f"   ❌ Code validation failed: {error}")
        return
    
    # In run_safe.py, safe functions would be prepended
    print("\n▶️  Safe functions would be injected before execution:")
    print("   - create_file(filename, content)")
    print("   - read_file(filename)")
    print("   - delete_file(filename)")
    print("   - list_files(subdirectory)")
    print("   - search_web(query)")
    print()
    
    # Show audit logging
    print("▶️  Action would be logged to audit log:")
    safe_mode.audit_log(
        operation='code_execution',
        params={'language': 'python', 'code': user_code[:100]},
        result='Execution started',
        success=True
    )
    log_file = safe_mode.workspace_path / '.audit.log'
    if log_file.exists():
        with open(log_file, 'r') as f:
            last_line = f.readlines()[-1]
            print(f"   {last_line.strip()}")
    print()


def demo_security_features():
    """Demonstrate security features."""
    demo_banner("DEMO 4: Security Features")
    
    print("🔒 Safe Mode Security Features:\n")
    
    features = [
        ("File Sandbox", "All file operations restricted to ~/model_workspace"),
        ("Shell Blocking", "All shell/bash/zsh/powershell execution blocked"),
        ("Module Blacklist", "Dangerous modules like subprocess, socket blocked"),
        ("Keyword Filtering", "Dangerous keywords like eval, exec blocked"),
        ("Path Validation", "Prevents directory traversal (../) and absolute paths"),
        ("Extension Whitelist", "Only allowed file extensions (.txt, .py, .json, etc.)"),
        ("Audit Logging", "All actions logged to .audit.log in JSON format"),
        ("Code Validation", "Pre-execution validation before any code runs"),
    ]
    
    for i, (feature, description) in enumerate(features, 1):
        print(f"{i}. ✅ {feature}")
        print(f"   {description}\n")


def demo_configuration():
    """Show configuration options."""
    demo_banner("DEMO 5: Configuration (safe_config.yaml)")
    
    print("Configuration file structure:\n")
    
    config_example = """workspace: ~/model_workspace

allowed_extensions:
  - .txt
  - .py
  - .json
  - .md
  - .csv

blocked_modules:
  - subprocess
  - os.system
  - socket
  - shutil.rmtree

blocked_keywords:
  - eval
  - exec
  - system
  - chmod

ollama:
  model: qwen3:14b
  api_url: http://localhost:11434

execution:
  auto_run: false  # Always ask before executing
  safe_mode: ask
"""
    
    print(config_example)


def main():
    """Run all demonstrations."""
    print("\n" + "🔒" * 30)
    print("  OPEN INTERPRETER - SAFE MODE DEMONSTRATION")
    print("🔒" * 30)
    
    try:
        demo_security_features()
        demo_file_operations()
        demo_code_validation()
        demo_execution_wrapper()
        demo_configuration()
        
        print("\n" + "=" * 60)
        print("  ✅ Demonstration Complete!")
        print("=" * 60)
        print("\n📚 For more information:")
        print("   • README_SAFE.md - Full documentation")
        print("   • EXAMPLES_SAFE.md - Usage examples")
        print("   • test_safe_mode.py - Run tests")
        print("\n🚀 To start Safe Mode:")
        print("   1. Run: ./install_safe.sh")
        print("   2. Start Ollama: ollama serve")
        print("   3. Run: ./start_safe.sh")
        print()
        
    except Exception as e:
        print(f"\n❌ Error during demonstration: {e}")
        import traceback
        traceback.print_exc()
        return 1
    
    return 0


if __name__ == "__main__":
    sys.exit(main())
