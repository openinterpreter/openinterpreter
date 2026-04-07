"""
Sends anonymous telemetry to posthog. This helps us know how people are
using OI / what needs our focus.

Disable anonymous telemetry by execute one of below:
1. Running `interpreter --disable_telemetry` in command line.
2. Executing `interpreter.disable_telemetry = True` in Python.
3. Setting the `DISABLE_TELEMETRY` os var to `true`.

based on ChromaDB's telemetry:
https://github.com/chroma-core/chroma/tree/main/chromadb/telemetry/product
"""

import contextlib
import json
import os
import re
import threading
import uuid

from importlib.metadata import version, PackageNotFoundError
import requests


def get_or_create_uuid():
    try:
        uuid_file_path = os.path.join(
            os.path.expanduser("~"),
            ".cache", "open-interpreter", "telemetry_user_id"
        )
        os.makedirs(
            os.path.dirname(uuid_file_path), exist_ok=True
        )  # Ensure the directory exists

        if os.path.exists(uuid_file_path):
            with open(uuid_file_path, "r") as file:
                return file.read()
        else:
            new_uuid = str(uuid.uuid4())
            with open(uuid_file_path, "w") as file:
                file.write(new_uuid)
            return new_uuid
    except:
        # Non blocking
        return "idk"


user_id = get_or_create_uuid()


# --- Sanitization helpers ---

# Matches common absolute file paths (Unix and Windows)
_PATH_PATTERN = re.compile(
    r'(?:[A-Za-z]:\\|/)(?:[\w.\-]+[/\\])*[\w.\-]+'
)

# Environment variable references like $HOME, %USERPROFILE%
_ENV_VAR_PATTERN = re.compile(
    r'(?:\$[A-Z_]+|%[A-Z_]+%)'
)

# Sensitive keys whose values should be redacted
_SENSITIVE_KEYS = frozenset({
    "api_key", "api_secret", "token", "password", "secret",
    "authorization", "credential", "private_key",
})


def _sanitize_value(value):
    """Recursively sanitize a value, stripping file paths and sensitive data."""
    if isinstance(value, str):
        # Redact absolute file paths
        sanitized = _PATH_PATTERN.sub("<path>", value)
        # Redact environment variable references
        sanitized = _ENV_VAR_PATTERN.sub("<env>", sanitized)
        return sanitized
    elif isinstance(value, dict):
        return {
            k: "<redacted>" if k.lower() in _SENSITIVE_KEYS else _sanitize_value(v)
            for k, v in value.items()
        }
    elif isinstance(value, (list, tuple)):
        return [_sanitize_value(item) for item in value]
    return value


def _sanitize_properties(properties):
    """
    Sanitize telemetry properties to prevent accidental leakage of
    file paths, credentials, or other sensitive information in
    exception stack traces or user-supplied data.
    """
    if not isinstance(properties, dict):
        return properties
    return _sanitize_value(properties)


def send_telemetry(event_name, properties=None):
    if properties is None:
        properties = {}
    properties["oi_version"] = version("open-interpreter")

    # Sanitize all properties before sending
    properties = _sanitize_properties(properties)

    try:
        url = "https://app.posthog.com/capture"
        headers = {"Content-Type": "application/json"}
        data = {
            "api_key": "phc_6cmXy4MEbLfNGezqGjuUTY8abLu0sAwtGzZFpQW97lc",
            "event": event_name,
            "properties": properties,
            "distinct_id": user_id,
        }
        requests.post(url, headers=headers, data=json.dumps(data))
    except:
        pass
