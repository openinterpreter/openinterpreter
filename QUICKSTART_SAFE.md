# 🚀 Quick Start Guide - Safe Mode

This guide helps you get started with Open Interpreter Safe Mode in 5 minutes.

## Prerequisites

- Python 3.9 or higher
- [Ollama](https://ollama.ai) installed and running
- Terminal access

## Step 1: Install Ollama and Model

```bash
# Install Ollama (if not already installed)
curl https://ollama.ai/install.sh | sh

# Pull the qwen3:14b model
ollama pull qwen3:14b

# Start Ollama server (in a separate terminal)
ollama serve
```

## Step 2: Install Safe Mode

```bash
# Navigate to the repository
cd open-interpreter

# Run the installation script
chmod +x install_safe.sh
./install_safe.sh
```

The installation script will:
- ✅ Create a virtual environment (`venv_safe`)
- ✅ Install all dependencies
- ✅ Create the workspace directory (`~/model_workspace`)
- ✅ Create a convenience launcher script (`start_safe.sh`)

## Step 3: Start Safe Mode

```bash
# Option 1: Use the convenience script
./start_safe.sh

# Option 2: Manual start
source venv_safe/bin/activate
python run_safe.py
```

## Step 4: Try It Out!

Once started, you'll see:

```
============================================================
🔒 OPEN INTERPRETER - SAFE MODE
============================================================

✅ Safe mode initialized
📁 Workspace: /home/user/model_workspace
🔒 Security: Enabled

📋 Available functions:
  • create_file(filename, content)
  • read_file(filename)
  • delete_file(filename)
  • list_files(subdirectory='')
  • search_web(query)

⚠️  All other operations are blocked for security.
📝 All actions are logged to ~/model_workspace/.audit.log

============================================================
```

### Example Conversation

```
> Create a file called greeting.txt with "Hello, Safe Mode!"

The interpreter will show you the code it wants to run:

```python
success, message = create_file("greeting.txt", "Hello, Safe Mode!")
print(message)
```

Press Enter to approve, and you'll see:
✅ File created: greeting.txt
```

```
> List all files

```python
success, files = list_files()
print(files)
```

Output:
📄 greeting.txt (18 bytes)
```

```
> Search for Python tutorials

```python
success, results = search_web("Python tutorials")
print(results)
```

Output:
📌 Python tutorials for beginners...
🔗 https://docs.python.org/3/tutorial/
```

## What You CANNOT Do (By Design)

These will be blocked for security:

```
> Install pandas
❌ Shell execution is blocked in safe mode

> Run ls command
❌ Shell execution is blocked in safe mode

> Access /etc/passwd
❌ Absolute paths are not allowed

> Use subprocess module
❌ Blocked module detected: subprocess
```

## Configuration

Edit `safe_config.yaml` to customize:

```yaml
# Change workspace location
workspace: ~/my_custom_workspace

# Add more allowed extensions
allowed_extensions:
  - .txt
  - .py
  - .xml  # Add this

# Change Ollama model
ollama:
  model: llama3:8b  # Use a different model
```

## Testing

Run the test suite:

```bash
source venv_safe/bin/activate
python test_safe_mode.py
```

Run the interactive demo:

```bash
python demo_safe_mode.py
```

## Troubleshooting

### Ollama Not Running

```
Error: Connection refused to localhost:11434
```

**Solution:** Start Ollama in another terminal:
```bash
ollama serve
```

### Model Not Found

```
Error: Model qwen3:14b not found
```

**Solution:** Pull the model:
```bash
ollama pull qwen3:14b
```

### Permission Denied

```
Error: Permission denied: ~/model_workspace
```

**Solution:** Check directory permissions:
```bash
chmod 755 ~/model_workspace
```

### Import Errors

```
ModuleNotFoundError: No module named 'yaml'
```

**Solution:** Activate the virtual environment:
```bash
source venv_safe/bin/activate
```

## Monitoring Activity

View the audit log:

```bash
# View last 10 actions
tail -10 ~/model_workspace/.audit.log

# View with pretty formatting
cat ~/model_workspace/.audit.log | jq .
```

## Stopping Safe Mode

1. Press `Ctrl+C` in the terminal
2. Stop Ollama if desired: `pkill ollama`

## Next Steps

- 📖 Read the full documentation: [README_SAFE.md](README_SAFE.md)
- 📝 See usage examples: [EXAMPLES_SAFE.md](EXAMPLES_SAFE.md)
- 🧪 Run tests: `python test_safe_mode.py`
- 🎮 Try the demo: `python demo_safe_mode.py`

## Getting Help

If you encounter issues:

1. Check that Ollama is running: `curl http://localhost:11434`
2. Verify the model: `ollama list | grep qwen3`
3. Check Python version: `python3 --version` (need 3.9+)
4. Review logs: `cat ~/model_workspace/.audit.log`

## Security Notes

Safe Mode enforces these restrictions:

- ✅ All file operations in `~/model_workspace` only
- ✅ No shell command execution
- ✅ No dangerous Python modules (subprocess, socket, etc.)
- ✅ No direct file operations (must use safe functions)
- ✅ All actions logged
- ✅ Code reviewed before execution (auto_run=false)

**Remember:** Safe Mode is designed for constrained environments. It's not meant to be a complete sandbox but provides reasonable security for AI code execution.

---

**Happy Safe Coding! 🔒**
