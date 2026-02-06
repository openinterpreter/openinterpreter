#!/bin/bash

# Installation script for Open Interpreter Safe Mode

set -e  # Exit on error

echo "=================================================="
echo "🔒 Open Interpreter - Safe Mode Installation"
echo "=================================================="
echo ""

# Get the directory where the script is located
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
WORKSPACE_DIR="$HOME/model_workspace"

# Check if Python is installed
if ! command -v python3 &> /dev/null; then
    echo "❌ Python 3 is not installed. Please install Python 3.9 or higher."
    exit 1
fi

# Check Python version
PYTHON_VERSION=$(python3 -c 'import sys; print(".".join(map(str, sys.version_info[:2])))')
echo "✅ Python version: $PYTHON_VERSION"

# Create virtual environment
echo ""
echo "📦 Creating virtual environment..."
VENV_DIR="$SCRIPT_DIR/venv_safe"

if [ -d "$VENV_DIR" ]; then
    echo "   Virtual environment already exists at $VENV_DIR"
    read -p "   Do you want to recreate it? (y/N): " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        rm -rf "$VENV_DIR"
        python3 -m venv "$VENV_DIR"
    fi
else
    python3 -m venv "$VENV_DIR"
fi

# Activate virtual environment
echo "   Activating virtual environment..."
source "$VENV_DIR/bin/activate"

# Upgrade pip
echo ""
echo "📦 Upgrading pip..."
pip install --upgrade pip

# Install dependencies
echo ""
echo "📦 Installing dependencies..."
echo "   This may take a few minutes..."

# Install open-interpreter and required packages
pip install open-interpreter requests pyyaml

echo ""
echo "✅ Dependencies installed successfully!"

# Create workspace directory
echo ""
echo "📁 Creating workspace directory..."
mkdir -p "$WORKSPACE_DIR"
echo "   Workspace created at: $WORKSPACE_DIR"

# Make run_safe.py executable
echo ""
echo "🔧 Making run_safe.py executable..."
chmod +x "$SCRIPT_DIR/run_safe.py"

# Create a convenience script
echo ""
echo "📝 Creating convenience launcher script..."
cat > "$SCRIPT_DIR/start_safe.sh" << 'EOF'
#!/bin/bash
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
source "$SCRIPT_DIR/venv_safe/bin/activate"
python "$SCRIPT_DIR/run_safe.py"
EOF

chmod +x "$SCRIPT_DIR/start_safe.sh"

# Print success message
echo ""
echo "=================================================="
echo "✅ Installation Complete!"
echo "=================================================="
echo ""
echo "📋 Next Steps:"
echo ""
echo "1. Make sure Ollama is running with the qwen3:14b model:"
echo "   $ ollama pull qwen3:14b"
echo "   $ ollama serve"
echo ""
echo "2. Start Open Interpreter in safe mode:"
echo "   $ ./start_safe.sh"
echo ""
echo "   Or manually:"
echo "   $ source venv_safe/bin/activate"
echo "   $ python run_safe.py"
echo ""
echo "📁 Workspace: $WORKSPACE_DIR"
echo "   All file operations will be restricted to this directory."
echo ""
echo "🔒 Security Features:"
echo "   • File operations only in ~/model_workspace"
echo "   • Shell execution blocked"
echo "   • Only whitelisted operations allowed"
echo "   • All actions logged to .audit.log"
echo ""
echo "📖 For more information, see README_SAFE.md"
echo ""
echo "=================================================="
