#!/bin/bash
set -e

echo "Starting Open Interpreter installation..."
sleep 2
echo "This will take approximately 5 minutes..."
sleep 2

# Define pyenv location
pyenv_root="$HOME/.pyenv/bin/pyenv"

#!/bin/bash

# Check if Git is installed
if command -v git >/dev/null; then
    echo "Git is already installed."
else
    # Detect the operating system
    OS="$(uname -s)"

    case "$OS" in
        Linux)
            # Assume a Debian-based or Fedora-based system
            if command -v apt >/dev/null; then
                echo "Installing Git on Debian-based Linux..."
                # Check and install sudo if not present
                if ! command -v sudo &> /dev/null; then
                    apt-get update && apt-get install -y sudo
                fi
                sudo apt install -y git-all
            elif command -v dnf >/dev/null; then
                echo "Installing Git on Fedora-based Linux..."
                # Check and install sudo if not present
                if ! command -v sudo &> /dev/null; then
                    dnf install -y sudo
                fi
                sudo dnf install -y git-all
            else
                echo "Package manager not supported. Please install Git manually."
            fi
            ;;
        Darwin)
            echo "Installing Git on macOS..."
            # Install Git using Xcode Command Line Tools
            xcode-select --install || echo "Xcode Command Line Tools installation failed or already installed"
            ;;
        *)
            echo "Unsupported OS: $OS"
            ;;
    esac
fi

echo "Starting installation of pyenv..."

INSTALL_URL="https://pyenv.run"

# Check if pyenv is already installed
if command -v pyenv &> /dev/null; then
    echo "pyenv is already installed."
else
    # Try to download and install pyenv using available commands
    if command -v curl &> /dev/null; then
        echo "Using curl to download pyenv..."
        curl -L "$INSTALL_URL" | sh
    # elif command -v wget &> /dev/null; then
    #     echo "Using wget to download pyenv..."
    #     wget -O- "$INSTALL_URL" | sh
    # elif command -v python &> /dev/null; then
    #     echo "Using Python to download pyenv..."
    #     python -c "import urllib.request; exec(urllib.request.urlopen('$INSTALL_URL').read())"
    # elif command -v perl &> /dev/null; then
    #     echo "Using Perl to download pyenv..."
    #     perl -e "use LWP::Simple; exec(get('$INSTALL_URL'))"
    else
        echo "Neither curl nor wget is available."
        if [ "$(uname -s)" = "Linux" ]; then
            echo "Linux detected. Attempting to install sudo and curl..."

            # Check and install sudo if not present
            if ! command -v sudo &> /dev/null; then
                apt-get update && apt-get install -y sudo
            fi

            # Install curl using sudo
            if command -v sudo &> /dev/null; then
                sudo apt-get update && sudo apt-get install -y curl
                if command -v curl &> /dev/null; then
                    echo "Using curl to download pyenv..."
                    curl -L "$INSTALL_URL" | sh
                else
                    echo "Failed to install curl. Installation of pyenv cannot proceed."
                fi
            else
                echo "Unable to install sudo. Manual installation required."
            fi
        else
            echo "Failed to install curl. Installation of pyenv cannot proceed."
        fi
    fi
fi

# Install Python and remember the version
python_version=3.11
$pyenv_root install $python_version --skip-existing

# Explicitly use the installed Python version for commands
installed_version=$($pyenv_root exec python$python_version --version)
echo "Installed Python version: $installed_version"
if [[ $installed_version != *"$python_version"* ]]; then
    echo "Python $python_version was not installed correctly. Please open an issue at https://github.com/openinterpreter/universal-python/."
    exit 1
fi

# Use the specific Python version to install open-interpreter
$pyenv_root exec python$python_version -m pip install open-interpreter

echo "Open Interpreter has been installed. Run the following command to use it:"
echo "interpreter"