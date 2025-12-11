import platform
import subprocess

from importlib.metadata import version, PackageNotFoundError
from importlib.metadata import distributions
from packaging.requirements import Requirement
from packaging.version import Version
import psutil
import toml


def get_python_version():
    return platform.python_version()


def get_pip_version():
    try:
        pip_version = subprocess.check_output(["pip", "--version"]).decode().split()[1]
    except Exception as e:
        pip_version = str(e)
    return pip_version


def get_oi_version():
    try:
        oi_version_cmd = subprocess.check_output(
            ["interpreter", "--version"], text=True
        )
    except Exception as e:
        oi_version_cmd = str(e)
    try:
        pkg_ver = version("open-interpreter")
    except PackageNotFoundError:
        pkg_ver = None
    oi_version = oi_version_cmd, pkg_ver
    return oi_version


def get_os_version():
    return platform.platform()


def get_cpu_info():
    return platform.processor()


def get_ram_info():
    vm = psutil.virtual_memory()
    used_ram_gb = vm.used / (1024**3)
    free_ram_gb = vm.free / (1024**3)
    total_ram_gb = vm.total / (1024**3)
    return f"{total_ram_gb:.2f} GB, used: {used_ram_gb:.2f}, free: {free_ram_gb:.2f}"


def get_package_mismatches(file_path="pyproject.toml"):
    with open(file_path, "r") as file:
        pyproject = toml.load(file)

    project_dependencies = pyproject.get("project", {}).get("dependencies", [])
    dev_dependencies = pyproject.get("dependency-groups", {}).get("dev", [])

    requirements = []
    unparsable = []
    for raw_requirement in project_dependencies + dev_dependencies:
        try:
            requirements.append(Requirement(raw_requirement))
        except Exception:
            unparsable.append(raw_requirement)

    installed_packages = {
        dist.metadata["Name"].lower(): dist.version
        for dist in distributions()
    }
    mismatches = []
    for requirement in requirements:
        installed_version = installed_packages.get(requirement.name.lower())
        if not installed_version:
            mismatches.append(f"\t  {requirement.name}: Not found in pip list")
            continue

        if requirement.specifier and not requirement.specifier.contains(
            Version(installed_version), prereleases=True
        ):
            mismatches.append(
                f"\t  {requirement.name}: Mismatch, pyproject.toml={requirement.specifier}, pip={installed_version}"
            )

    mismatches.extend(
        [f"\t  {requirement}: Unable to parse requirement" for requirement in unparsable]
    )

    return "\n" + "\n".join(mismatches)


def interpreter_info(interpreter):
    try:
        if interpreter.offline and interpreter.llm.api_base:
            try:
                curl = subprocess.check_output(f"curl {interpreter.llm.api_base}")
            except Exception as e:
                curl = str(e)
        else:
            curl = "Not local"

        messages_to_display = []
        for message in interpreter.messages:
            message = str(message.copy())
            try:
                if len(message) > 2000:
                    message = message[:1000]
            except Exception as e:
                print(str(e), "for message:", message)
            messages_to_display.append(message)

        return f"""

        # Interpreter Info
        
        Vision: {interpreter.llm.supports_vision}
        Model: {interpreter.llm.model}
        Function calling: {interpreter.llm.supports_functions}
        Context window: {interpreter.llm.context_window}
        Max tokens: {interpreter.llm.max_tokens}
        Computer API: {interpreter.computer.import_computer_api}

        Auto run: {interpreter.auto_run}
        API base: {interpreter.llm.api_base}
        Offline: {interpreter.offline}

        Curl output: {curl}

        # Messages

        System Message: {interpreter.system_message}

        """ + "\n\n".join(
            [str(m) for m in messages_to_display]
        )
    except:
        return "Error, couldn't get interpreter info"


def system_info(interpreter):
    oi_version = get_oi_version()
    print(
        f"""
        Python Version: {get_python_version()}
        Pip Version: {get_pip_version()}
        Open-interpreter Version: cmd: {oi_version[0]}, pkg: {oi_version[1]}
        OS Version and Architecture: {get_os_version()}
        CPU Info: {get_cpu_info()}
        RAM Info: {get_ram_info()}
        {interpreter_info(interpreter)}
    """
    )

    # Removed the following, as it causes `FileNotFoundError: [Errno 2] No such file or directory: 'pyproject.toml'`` on prod
    # (i think it works on dev, but on prod the pyproject.toml will not be in the cwd. might not be accessible at all)
    # Package Version Mismatches:
    # {get_package_mismatches()}
