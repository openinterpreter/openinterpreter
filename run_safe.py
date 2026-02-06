#!/usr/bin/env python3
"""
Safe Mode Entry Point for Open Interpreter

This script runs Open Interpreter with strict security controls:
- All file operations sandboxed to ~/model_workspace
- Shell execution completely blocked
- Only whitelisted operations allowed
- All actions logged to audit log
"""

import sys
import os
from pathlib import Path
import yaml

# Add the current directory to path to import safe_mode
sys.path.insert(0, str(Path(__file__).parent))

from safe_mode import SafeMode, create_safe_environment
from interpreter import OpenInterpreter


SAFE_SYSTEM_MESSAGE = """
You are Open Interpreter running in SAFE MODE with strict security restrictions.

# SECURITY CONSTRAINTS

You can ONLY use these approved functions:

1. **create_file(filename, content)** - Create a file in the workspace
   - Only these extensions allowed: .txt, .py, .json, .md, .csv, .html, .css, .js, .yaml, .yml
   - Example: create_file("test.txt", "Hello world")

2. **read_file(filename)** - Read a file from the workspace
   - Example: read_file("test.txt")

3. **delete_file(filename)** - Delete a file from the workspace
   - Example: delete_file("test.txt")

4. **list_files(subdirectory="")** - List files in the workspace
   - Example: list_files() or list_files("subdir")

5. **search_web(query)** - Search the web using DuckDuckGo
   - Example: search_web("Python tutorials")

# RESTRICTIONS

❌ **FORBIDDEN OPERATIONS:**
- Shell/bash commands (blocked)
- Direct file operations with open(), os.remove, etc. (use the functions above instead)
- Network operations except search_web()
- Installing packages with pip
- System commands with subprocess
- Any operations outside ~/model_workspace

✅ **ALLOWED:**
- Python code using standard library (math, json, datetime, etc.)
- Data processing with built-in functions
- Using the 5 safe functions listed above

# WORKSPACE

All your files are in: ~/model_workspace
You cannot access any files outside this directory.

# IMPORTANT

When the user asks you to work with files, ALWAYS use create_file(), read_file(), delete_file(), and list_files().
NEVER use open(), os.path, pathlib.Path, or any direct file operations.

When you need information from the web, use search_web().

Be helpful and creative within these constraints!
""".strip()


def load_config():
    """Load safe mode configuration."""
    config_path = Path(__file__).parent / "safe_config.yaml"
    
    if not config_path.exists():
        print(f"❌ Configuration file not found: {config_path}")
        sys.exit(1)
    
    with open(config_path, 'r') as f:
        config = yaml.safe_load(f)
    
    return config


def setup_interpreter_with_safe_mode(config):
    """
    Setup Open Interpreter with safe mode enabled.
    """
    # Initialize SafeMode
    safe_mode = SafeMode(config)
    
    print(f"✅ Safe mode initialized")
    print(f"📁 Workspace: {safe_mode.workspace_path}")
    print(f"🔒 Security: Enabled")
    print()
    
    # Create interpreter instance
    interpreter = OpenInterpreter(
        auto_run=config['execution']['auto_run'],
        safe_mode=config['execution']['safe_mode'],
    )
    
    # Configure for Ollama
    ollama_config = config['ollama']
    interpreter.llm.model = f"ollama/{ollama_config['model']}"
    interpreter.llm.api_base = ollama_config['api_base']
    interpreter.llm.supports_functions = False
    
    # Set custom system message
    interpreter.system_message = SAFE_SYSTEM_MESSAGE
    
    # Store safe_mode instance for later use
    interpreter._safe_mode = safe_mode
    interpreter._safe_functions = create_safe_environment(interpreter, safe_mode)
    
    return interpreter, safe_mode


def wrap_code_execution(original_run, safe_mode, safe_functions):
    """
    Wrap the computer.run() method to enforce safe mode.
    """
    def safe_run(language, code, *args, **kwargs):
        # Validate code
        is_valid, error_msg = safe_mode.validate_code(code, language)
        
        if not is_valid:
            # Log the blocked attempt
            safe_mode.audit_log(
                operation='code_execution_blocked',
                params={'language': language, 'code': code[:200]},
                result=error_msg,
                success=False
            )
            
            # Return error message as generator (to match original_run signature)
            yield {
                "type": "console",
                "format": "output",
                "content": error_msg + "\n\n💡 Use only the approved functions: create_file(), read_file(), delete_file(), list_files(), search_web()"
            }
            return
        
        # For Python code, prepend safe function definitions
        if language.lower() == 'python':
            # Inline implementation of safe functions for the kernel
            safe_functions_code = """
# Safe Mode Functions (auto-injected)
def create_file(filename, content):
    '''Create a file in the workspace. Returns (success, message).'''
    import os
    workspace = os.path.expanduser('~/model_workspace')
    os.makedirs(workspace, exist_ok=True)
    filepath = os.path.join(workspace, filename)
    # Validate path
    if not os.path.abspath(filepath).startswith(workspace):
        return (False, "❌ Path outside workspace")
    # Check extension
    allowed = ['.txt', '.py', '.json', '.md', '.csv', '.html', '.css', '.js', '.yaml', '.yml']
    ext = os.path.splitext(filename)[1].lower()
    if ext and ext not in allowed:
        return (False, f"❌ Extension not allowed: {ext}")
    # Write file
    try:
        with open(filepath, 'w', encoding='utf-8') as f:
            f.write(content)
        return (True, f"✅ File created: {filename}")
    except Exception as e:
        return (False, f"❌ Error: {str(e)}")

def read_file(filename):
    '''Read a file from the workspace. Returns (success, content).'''
    import os
    workspace = os.path.expanduser('~/model_workspace')
    filepath = os.path.join(workspace, filename)
    if not os.path.abspath(filepath).startswith(workspace):
        return (False, "❌ Path outside workspace")
    try:
        with open(filepath, 'r', encoding='utf-8') as f:
            content = f.read()
        return (True, content)
    except FileNotFoundError:
        return (False, f"❌ File not found: {filename}")
    except Exception as e:
        return (False, f"❌ Error: {str(e)}")

def delete_file(filename):
    '''Delete a file from the workspace. Returns (success, message).'''
    import os
    workspace = os.path.expanduser('~/model_workspace')
    filepath = os.path.join(workspace, filename)
    if not os.path.abspath(filepath).startswith(workspace):
        return (False, "❌ Path outside workspace")
    try:
        os.remove(filepath)
        return (True, f"✅ File deleted: {filename}")
    except FileNotFoundError:
        return (False, f"❌ File not found: {filename}")
    except Exception as e:
        return (False, f"❌ Error: {str(e)}")

def list_files(subdirectory=""):
    '''List files in the workspace. Returns (success, file_list).'''
    import os
    workspace = os.path.expanduser('~/model_workspace')
    if subdirectory:
        listdir = os.path.join(workspace, subdirectory)
    else:
        listdir = workspace
    if not os.path.abspath(listdir).startswith(workspace):
        return (False, "❌ Path outside workspace")
    try:
        items = []
        for item in sorted(os.listdir(listdir)):
            if item.startswith('.') and item != '.audit.log':
                continue
            path = os.path.join(listdir, item)
            if os.path.isdir(path):
                items.append(f"📁 {item}/")
            else:
                size = os.path.getsize(path)
                items.append(f"📄 {item} ({size} bytes)")
        return (True, "\\n".join(items) if items else "📂 Empty directory")
    except Exception as e:
        return (False, f"❌ Error: {str(e)}")

def search_web(query):
    '''Search the web using DuckDuckGo. Returns (success, results).'''
    try:
        import requests
        url = "https://api.duckduckgo.com/"
        params = {'q': query, 'format': 'json', 'no_html': '1', 'skip_disambig': '1'}
        response = requests.get(url, params=params, timeout=10)
        response.raise_for_status()
        data = response.json()
        results = []
        if data.get('Abstract'):
            results.append(f"📌 {data['AbstractText']}")
            if data.get('AbstractURL'):
                results.append(f"   🔗 {data['AbstractURL']}")
        if data.get('RelatedTopics'):
            results.append("\\n🔍 Related topics:")
            for i, topic in enumerate(data['RelatedTopics'][:5], 1):
                if isinstance(topic, dict) and 'Text' in topic:
                    results.append(f"{i}. {topic.get('Text', '')}")
                    if topic.get('FirstURL'):
                        results.append(f"   🔗 {topic['FirstURL']}")
        return (True, "\\n".join(results) if results else "No results found")
    except Exception as e:
        return (False, f"❌ Search failed: {str(e)}")

"""
            # Prepend safe functions to the user's code
            code = safe_functions_code + "\n" + code
        
        # Log the execution
        safe_mode.audit_log(
            operation='code_execution',
            params={'language': language, 'code': code[:200]},
            result='Execution started',
            success=True
        )
        
        # Call original run method (it's a generator)
        try:
            for output in original_run(language, code, *args, **kwargs):
                yield output
            
            # Log success
            safe_mode.audit_log(
                operation='code_execution_complete',
                params={'language': language},
                result='Execution completed',
                success=True
            )
            
        except Exception as e:
            # Log error
            safe_mode.audit_log(
                operation='code_execution_error',
                params={'language': language},
                result=str(e),
                success=False
            )
            raise
    
    return safe_run


def main():
    """Main entry point."""
    print("=" * 60)
    print("🔒 OPEN INTERPRETER - SAFE MODE")
    print("=" * 60)
    print()
    
    # Load configuration
    config = load_config()
    
    # Setup interpreter
    interpreter, safe_mode = setup_interpreter_with_safe_mode(config)
    
    # Wrap the execution method
    original_run = interpreter.computer.run
    interpreter.computer.run = wrap_code_execution(
        original_run,
        safe_mode,
        interpreter._safe_functions
    )
    
    print("📋 Available functions:")
    print("  • create_file(filename, content)")
    print("  • read_file(filename)")
    print("  • delete_file(filename)")
    print("  • list_files(subdirectory='')")
    print("  • search_web(query)")
    print()
    print("⚠️  All other operations are blocked for security.")
    print("📝 All actions are logged to ~/model_workspace/.audit.log")
    print()
    print("=" * 60)
    print()
    
    # Start the chat interface
    try:
        interpreter.chat()
    except KeyboardInterrupt:
        print("\n\n👋 Goodbye!")
    except Exception as e:
        print(f"\n❌ Error: {e}")
        import traceback
        traceback.print_exc()


if __name__ == "__main__":
    main()
