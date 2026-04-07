"""
Utility for verifying the integrity of downloaded model files.

Downloads from external sources (e.g. HuggingFace) should be verified
against known SHA-256 checksums to prevent tampered or corrupted files
from being executed.
"""

import hashlib
import os


def compute_sha256(file_path, chunk_size=8192):
    """Compute SHA-256 hash of a file."""
    sha256 = hashlib.sha256()
    with open(file_path, "rb") as f:
        while True:
            chunk = f.read(chunk_size)
            if not chunk:
                break
            sha256.update(chunk)
    return sha256.hexdigest()


def verify_model_integrity(model_path, expected_hash=None, model_name="model"):
    """
    Verify the integrity of a downloaded model file.

    Args:
        model_path: Path to the downloaded file
        expected_hash: Expected SHA-256 hex digest (or None if unknown)
        model_name: Human-readable model name for log messages

    Returns:
        True if verification passed (or no hash to verify against)
        False if hash mismatch detected

    Raises:
        No exceptions — always returns a boolean. Callers decide policy.
    """
    if not os.path.exists(model_path):
        print(f"\n⚠️  Warning: Model file not found at {model_path}")
        return False

    actual_hash = compute_sha256(model_path)

    if expected_hash is None:
        print(
            f"\n⚠️  No SHA-256 checksum available for '{model_name}'."
            f"\n   Downloaded file hash: {actual_hash}"
            f"\n   Consider verifying this hash manually against the official source."
        )
        return True  # No hash to verify against — pass with warning

    if actual_hash.lower() == expected_hash.lower():
        print(f"\n✅ Integrity verified for '{model_name}' (SHA-256 match)")
        return True
    else:
        print(
            f"\n🚨 INTEGRITY CHECK FAILED for '{model_name}'!"
            f"\n   Expected: {expected_hash}"
            f"\n   Actual:   {actual_hash}"
            f"\n   The downloaded file may be corrupted or tampered with."
            f"\n   Removing the suspicious file..."
        )
        try:
            os.remove(model_path)
        except OSError:
            pass
        return False
