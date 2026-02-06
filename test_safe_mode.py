#!/usr/bin/env python3
"""
Test script for Safe Mode functionality
"""

import sys
import os
from pathlib import Path

# Add parent directory to path
sys.path.insert(0, str(Path(__file__).parent))

from safe_mode import SafeMode, SafeFileManager, SafeWebSearch
import tempfile

def test_safe_file_manager():
    """Test SafeFileManager operations."""
    print("Testing SafeFileManager...")
    
    # Create a temporary workspace for testing
    with tempfile.TemporaryDirectory() as tmpdir:
        workspace = Path(tmpdir)
        manager = SafeFileManager(workspace, ['.txt', '.py', '.json'])
        
        # Test 1: Create file
        success, msg = manager.create_file("test.txt", "Hello, Safe Mode!")
        assert success, f"Failed to create file: {msg}"
        print(f"✅ Create file: {msg}")
        
        # Test 2: Read file
        success, content = manager.read_file("test.txt")
        assert success and content == "Hello, Safe Mode!", f"Failed to read file: {content}"
        print(f"✅ Read file: content matches")
        
        # Test 3: List files
        success, file_list = manager.list_files()
        assert success and "test.txt" in file_list, f"File not in list: {file_list}"
        print(f"✅ List files: {file_list}")
        
        # Test 4: Delete file
        success, msg = manager.delete_file("test.txt")
        assert success, f"Failed to delete file: {msg}"
        print(f"✅ Delete file: {msg}")
        
        # Test 5: Try to access parent directory (should fail)
        success, msg = manager.create_file("../outside.txt", "Should fail")
        assert not success, "Should have blocked parent directory access"
        print(f"✅ Blocked parent directory access: {msg}")
        
        # Test 6: Try disallowed extension (should fail)
        success, msg = manager.create_file("bad.exe", "Should fail")
        assert not success, "Should have blocked .exe extension"
        print(f"✅ Blocked disallowed extension: {msg}")
        
        # Test 7: Try absolute path (should fail)
        success, msg = manager.create_file("/etc/passwd", "Should fail")
        assert not success, "Should have blocked absolute path"
        print(f"✅ Blocked absolute path: {msg}")
    
    print("✅ All SafeFileManager tests passed!\n")


def test_safe_mode_validation():
    """Test SafeMode code validation."""
    print("Testing SafeMode validation...")
    
    config = {
        'workspace': '~/model_workspace',
        'allowed_extensions': ['.txt', '.py'],
        'blocked_modules': ['subprocess', 'os.system', 'socket'],
        'blocked_keywords': ['eval', 'exec', 'system'],
    }
    
    safe_mode = SafeMode(config)
    
    # Test 1: Block shell execution
    is_valid, error = safe_mode.validate_code("ls -la", "shell")
    assert not is_valid, "Should have blocked shell execution"
    print(f"✅ Blocked shell: {error}")
    
    # Test 2: Block bash execution
    is_valid, error = safe_mode.validate_code("echo hello", "bash")
    assert not is_valid, "Should have blocked bash execution"
    print(f"✅ Blocked bash: {error}")
    
    # Test 3: Block subprocess import
    is_valid, error = safe_mode.validate_code("import subprocess", "python")
    assert not is_valid, "Should have blocked subprocess import"
    print(f"✅ Blocked subprocess: {error}")
    
    # Test 4: Block eval
    is_valid, error = safe_mode.validate_code("eval('print(1)')", "python")
    assert not is_valid, "Should have blocked eval"
    print(f"✅ Blocked eval: {error}")
    
    # Test 5: Allow safe Python code
    is_valid, error = safe_mode.validate_code("x = 1 + 2\nprint(x)", "python")
    assert is_valid, f"Should have allowed safe code: {error}"
    print("✅ Allowed safe Python code")
    
    # Test 6: Allow create_file usage
    is_valid, error = safe_mode.validate_code("create_file('test.txt', 'hello')", "python")
    assert is_valid, f"Should have allowed create_file: {error}"
    print("✅ Allowed create_file function")
    
    print("✅ All SafeMode validation tests passed!\n")


def test_web_search():
    """Test SafeWebSearch (basic test without actual network call)."""
    print("Testing SafeWebSearch...")
    
    search = SafeWebSearch()
    
    # Just test that the object is created properly
    assert hasattr(search, 'search'), "SafeWebSearch should have search method"
    print("✅ SafeWebSearch initialized correctly")
    
    # Note: We don't test actual search here to avoid network dependency
    print("✅ SafeWebSearch tests passed!\n")


def main():
    """Run all tests."""
    print("=" * 60)
    print("🧪 Safe Mode Tests")
    print("=" * 60)
    print()
    
    try:
        test_safe_file_manager()
        test_safe_mode_validation()
        test_web_search()
        
        print("=" * 60)
        print("✅ All tests passed!")
        print("=" * 60)
        return 0
    except AssertionError as e:
        print(f"\n❌ Test failed: {e}")
        import traceback
        traceback.print_exc()
        return 1
    except Exception as e:
        print(f"\n❌ Unexpected error: {e}")
        import traceback
        traceback.print_exc()
        return 1


if __name__ == "__main__":
    sys.exit(main())
