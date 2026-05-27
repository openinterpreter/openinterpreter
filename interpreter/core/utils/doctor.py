import os
import sys
import subprocess
import importlib.util

# Results of diagnosis
diagnosis_results = []
failures = []

PASSES = "PASS"
FAILS = "FAIL"

MAX_TIMEOUT = 5


# Section title
def print_section(title):
    print(f"\n{title}")
    print("-" * len(title))


# Report check
def report_check(name, passed, details=""):
    status = PASSES if passed else FAILS

    print(f"[{status}] {name}")

    if details:
        print(f"       {details}")

    diagnosis_results.append(
        {
            "name": name,
            "passed": passed,
            "details": details,
        }
    )


# Verify python environment
def verify_python_environment():
    report_check(
        "Python executable exists",
        os.path.exists(sys.executable),
        sys.executable,
    )

    if sys.executable:
        print("Executable:", sys.executable)
        print("Version:", sys.version.split()[0])


# Verify internet connectivity
def verify_internet_connection():
    try:
        command = (
            ["ping", "8.8.8.8", "-n", "1"]
            if os.name == "nt"
            else ["ping", "-c", "1", "8.8.8.8"]
        )

        result = subprocess.run(
            command,
            capture_output=True,
            text=True,
            timeout=MAX_TIMEOUT,
        )

        report_check(
            "Internet connection",
            result.returncode == 0,
        )

    except Exception as e:
        report_check(
            "Internet connection",
            False,
            str(e),
        )


# Verify Open Interpreter import works
def verify_interpreter_import():
    try:
        import interpreter

        report_check(
            "Open Interpreter import",
            True,
        )

    except Exception as e:
        report_check(
            "Open Interpreter import",
            False,
            str(e),
        )


# Check whether package exists
def check_package(name):
    return importlib.util.find_spec(name) is not None


# Output package statuses
def output_package_statuses(packages):
    for package in packages:
        status = check_package(package)

        report_check(
            package,
            status,
        )


# Check environment variable exists
def get_env_variable_status(variable):
    return os.getenv(variable) is not None


# Output environment variable statuses
def output_env_variable_statuses(env_variables):
    for variable in env_variables:
        status = get_env_variable_status(variable)

        report_check(
            variable,
            status,
        )


# Verify OpenAI API key format
def verify_openai_key_format():
    api_key = os.getenv("OPENAI_API_KEY")

    if not api_key:
        report_check(
            "OpenAI API key",
            False,
            "OPENAI_API_KEY not found",
        )

        return

    valid_format = api_key.startswith("sk-")

    report_check(
        "OpenAI API key format",
        valid_format,
    )


# Verify LiteLLM provider resolution
def verify_litellm_model(interpreter):
    try:
        import litellm

        model = interpreter.llm.model

        litellm.get_llm_provider(model)

        report_check(
            "LiteLLM provider resolution",
            True,
            f"Resolved provider for: {model}",
        )

    except Exception as e:
        report_check(
            "LiteLLM provider resolution",
            False,
            str(e),
        )


# Verify shell execution
def verify_shell_execution():
    try:
        result = subprocess.run(
            ["python", "--version"],
            capture_output=True,
            text=True,
            timeout=MAX_TIMEOUT,
        )

        report_check(
            "Shell execution",
            result.returncode == 0,
        )

    except Exception as e:
        report_check(
            "Shell execution",
            False,
            str(e),
        )


# Verify write permissions
def verify_write_permissions():
    try:
        test_file = "oi_doctor_test.tmp"

        with open(test_file, "w") as f:
            f.write("test")

        os.remove(test_file)

        report_check(
            "Filesystem write access",
            True,
        )

    except Exception as e:
        report_check(
            "Filesystem write access",
            False,
            str(e),
        )


# Verify configured model
def verify_current_model(interpreter):
    has_model = hasattr(interpreter.llm, "model")

    details = ""

    if has_model:
        details = f"Current model: {interpreter.llm.model}"

    report_check(
        "Configured model",
        has_model,
        details,
    )


# Verify Ollama installation
def verify_ollama_exists():
    try:
        result = subprocess.run(
            ["ollama", "list"],
            capture_output=True,
            text=True,
            timeout=MAX_TIMEOUT,
        )

        report_check(
            "Ollama installed",
            result.returncode == 0,
        )

    except Exception as e:
        report_check(
            "Ollama installed",
            False,
            str(e),
        )


# Verify Ollama service runs
def verify_ollama_runs():
    try:
        result = subprocess.run(
            ["ollama", "ps"],
            capture_output=True,
            text=True,
            timeout=MAX_TIMEOUT,
        )

        report_check(
            "Ollama running",
            result.returncode == 0,
        )

    except Exception as e:
        report_check(
            "Ollama running",
            False,
            str(e),
        )


# Verify Ollama models exist
def verify_ollama_models():
    try:
        result = subprocess.run(
            ["ollama", "list"],
            capture_output=True,
            text=True,
            timeout=MAX_TIMEOUT,
        )

        output = result.stdout.strip()

        has_models = len(output.splitlines()) > 1

        report_check(
            "Ollama models installed",
            has_models,
        )

    except Exception as e:
        report_check(
            "Ollama models installed",
            False,
            str(e),
        )


# Verify CUDA support
def verify_cuda():
    try:
        import torch

        available = torch.cuda.is_available()

        details = ""

        if available:
            details = torch.cuda.get_device_name(0)

        report_check(
            "CUDA support",
            available,
            details,
        )

    except Exception as e:
        report_check(
            "CUDA support",
            False,
            str(e),
        )


# Check whether doctor passed
def doctor_passed():
    return all(result["passed"] for result in diagnosis_results)


# Generate summary of diagnosis
def generate_doctor_summary():
    failures.clear()

    passing_tests = 0
    failing_tests = 0

    for result in diagnosis_results:
        if result["passed"]:
            passing_tests += 1
        else:
            failing_tests += 1
            failures.append(result)

    print(f"PASS: {passing_tests}")
    print(f"FAIL: {failing_tests}")

    if failures:
        print("\nFailures")
        print("-" * 8)

        for failure in failures:
            print(f"- {failure['name']}")

            if failure["details"]:
                print(f"  {failure['details']}")

    if doctor_passed():
        sys.exit(0)
    else:
        sys.exit(1)


# Run doctor diagnostics
def run_doctor(interpreter):
    diagnosis_results.clear()
    failures.clear()

    print("Running Open Interpreter diagnostics...")
    print("=" * 30)

    # Python
    print_section("Python")
    verify_python_environment()

    # Internet
    print_section("Internet Connectivity")
    verify_internet_connection()

    # Open Interpreter
    print_section("Open Interpreter")
    verify_interpreter_import()

    # Packages
    print_section("Packages")

    packages = [
        "openai",
        "litellm",
        "tiktoken",
        "torch",
    ]

    output_package_statuses(packages)

    # Environment variables
    print_section("Environment Variables")

    env_vars = [
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "GROQ_API_KEY",
    ]

    output_env_variable_statuses(env_vars)

    # OpenAI API key
    verify_openai_key_format()

    # LiteLLM
    print_section("LiteLLM")
    verify_litellm_model(interpreter)

    # File permissions
    print_section("File Permissions")
    verify_write_permissions()

    # Shell execution
    print_section("Shell Execution")
    verify_shell_execution()

    # Model configuration
    print_section("Model Configurations")
    verify_current_model(interpreter)

    # Ollama
    print_section("Ollama")
    verify_ollama_exists()
    verify_ollama_runs()
    verify_ollama_models()

    # CUDA
    print_section("CUDA and GPU")
    verify_cuda()

    # Summary
    print_section("Doctor Summary")
    generate_doctor_summary()