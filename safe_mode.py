"""
Safe Mode Module for Open Interpreter

This module provides strict security controls for Open Interpreter:
- Sandboxed file operations only in ~/model_workspace
- Blocked shell/bash command execution
- Whitelisted operations only
- Audit logging
"""

import os
import re
import json
from pathlib import Path
from datetime import datetime
from typing import Optional, Dict, Any, List
import requests


class SafeMode:
    """
    Main security controller that enforces safe mode restrictions.
    """
    
    def __init__(self, config: Dict[str, Any]):
        self.config = config
        self.workspace_path = Path(config['workspace']).expanduser().resolve()
        self.allowed_extensions = config.get('allowed_extensions', [])
        self.blocked_keywords = config.get('blocked_keywords', [])
        self.blocked_modules = config.get('blocked_modules', [])
        
        # Create workspace if it doesn't exist
        self.workspace_path.mkdir(parents=True, exist_ok=True)
        
        # Initialize components
        self.file_manager = SafeFileManager(self.workspace_path, self.allowed_extensions)
        self.web_search = SafeWebSearch()
        
    def validate_code(self, code: str, language: str) -> tuple[bool, Optional[str]]:
        """
        Validate code before execution.
        
        Returns:
            (is_valid, error_message) tuple
        """
        # Block all shell/bash execution
        if language.lower() in ['shell', 'bash', 'zsh', 'powershell', 'cmd']:
            return False, f"❌ Shell execution is blocked in safe mode. Language: {language}"
        
        # For Python code, check for dangerous patterns
        if language.lower() == 'python':
            # Check for blocked modules
            for module in self.blocked_modules:
                patterns = [
                    rf'\bimport\s+{re.escape(module)}\b',
                    rf'\bfrom\s+{re.escape(module)}\b',
                    rf'\b__import__\([\'\"]{re.escape(module)}[\'\"]',
                ]
                for pattern in patterns:
                    if re.search(pattern, code):
                        return False, f"❌ Blocked module detected: {module}"
            
            # Check for blocked keywords/functions
            for keyword in self.blocked_keywords:
                if re.search(rf'\b{re.escape(keyword)}\b', code):
                    return False, f"❌ Blocked keyword detected: {keyword}"
            
            # Check for file operations outside allowed functions
            file_ops = ['open(', 'os.remove', 'os.unlink', 'os.rmdir', 'pathlib.Path']
            for op in file_ops:
                if op in code and not self._is_using_safe_functions(code):
                    return False, f"❌ Direct file operation detected: {op}. Use create_file(), read_file(), delete_file() instead."
        
        return True, None
    
    def _is_using_safe_functions(self, code: str) -> bool:
        """Check if code only uses safe wrapper functions."""
        safe_functions = ['create_file', 'read_file', 'delete_file', 'list_files', 'search_web']
        return any(func in code for func in safe_functions)
    
    def audit_log(self, operation: str, params: Dict[str, Any], result: Any, success: bool):
        """
        Log all operations to audit log.
        """
        log_file = self.workspace_path / '.audit.log'
        log_entry = {
            'timestamp': datetime.now().isoformat(),
            'operation': operation,
            'params': params,
            'result': str(result)[:200],  # Truncate long results
            'success': success
        }
        
        try:
            with open(log_file, 'a') as f:
                f.write(json.dumps(log_entry) + '\n')
        except Exception as e:
            print(f"Warning: Failed to write audit log: {e}")


class SafeFileManager:
    """
    Manages all file operations within the sandboxed workspace.
    """
    
    def __init__(self, workspace_path: Path, allowed_extensions: List[str]):
        self.workspace_path = workspace_path
        self.allowed_extensions = allowed_extensions
    
    def _validate_path(self, filename: str) -> tuple[bool, Optional[str], Optional[Path]]:
        """
        Validate that the path is within workspace and uses allowed extension.
        
        Returns:
            (is_valid, error_message, resolved_path) tuple
        """
        try:
            # Create full path
            if os.path.isabs(filename):
                # Reject absolute paths
                return False, f"❌ Absolute paths are not allowed: {filename}", None
            
            full_path = (self.workspace_path / filename).resolve()
            
            # Check if path is within workspace (prevent directory traversal)
            if not str(full_path).startswith(str(self.workspace_path)):
                return False, f"❌ Path is outside workspace: {filename}", None
            
            # Check file extension
            extension = full_path.suffix.lower()
            if extension and extension not in self.allowed_extensions:
                return False, f"❌ File extension not allowed: {extension}. Allowed: {', '.join(self.allowed_extensions)}", None
            
            return True, None, full_path
            
        except Exception as e:
            return False, f"❌ Invalid path: {str(e)}", None
    
    def create_file(self, filename: str, content: str) -> tuple[bool, str]:
        """
        Create a file in the workspace.
        
        Args:
            filename: Name or relative path of the file
            content: Content to write
            
        Returns:
            (success, message) tuple
        """
        is_valid, error, full_path = self._validate_path(filename)
        if not is_valid:
            return False, error
        
        try:
            # Create parent directories if needed
            full_path.parent.mkdir(parents=True, exist_ok=True)
            
            # Write file
            full_path.write_text(content, encoding='utf-8')
            
            return True, f"✅ File created: {filename}"
        except Exception as e:
            return False, f"❌ Failed to create file: {str(e)}"
    
    def read_file(self, filename: str) -> tuple[bool, str]:
        """
        Read a file from the workspace.
        
        Args:
            filename: Name or relative path of the file
            
        Returns:
            (success, content_or_error) tuple
        """
        is_valid, error, full_path = self._validate_path(filename)
        if not is_valid:
            return False, error
        
        try:
            if not full_path.exists():
                return False, f"❌ File not found: {filename}"
            
            if not full_path.is_file():
                return False, f"❌ Not a file: {filename}"
            
            content = full_path.read_text(encoding='utf-8')
            return True, content
            
        except Exception as e:
            return False, f"❌ Failed to read file: {str(e)}"
    
    def delete_file(self, filename: str) -> tuple[bool, str]:
        """
        Delete a file from the workspace.
        
        Args:
            filename: Name or relative path of the file
            
        Returns:
            (success, message) tuple
        """
        is_valid, error, full_path = self._validate_path(filename)
        if not is_valid:
            return False, error
        
        try:
            if not full_path.exists():
                return False, f"❌ File not found: {filename}"
            
            if not full_path.is_file():
                return False, f"❌ Not a file: {filename}"
            
            full_path.unlink()
            return True, f"✅ File deleted: {filename}"
            
        except Exception as e:
            return False, f"❌ Failed to delete file: {str(e)}"
    
    def list_files(self, subdirectory: str = "") -> tuple[bool, str]:
        """
        List files in the workspace or a subdirectory.
        
        Args:
            subdirectory: Optional subdirectory to list
            
        Returns:
            (success, file_list_or_error) tuple
        """
        try:
            if subdirectory:
                is_valid, error, list_path = self._validate_path(subdirectory)
                if not is_valid:
                    return False, error
            else:
                list_path = self.workspace_path
            
            if not list_path.exists():
                return False, f"❌ Directory not found: {subdirectory or '.'}"
            
            if not list_path.is_dir():
                return False, f"❌ Not a directory: {subdirectory or '.'}"
            
            # List all files and directories
            items = []
            for item in sorted(list_path.iterdir()):
                if item.name.startswith('.') and item.name != '.audit.log':
                    continue  # Skip hidden files except audit log
                
                relative_path = item.relative_to(self.workspace_path)
                item_type = "📁" if item.is_dir() else "📄"
                size = f"({item.stat().st_size} bytes)" if item.is_file() else ""
                items.append(f"{item_type} {relative_path} {size}")
            
            if not items:
                return True, "📂 Empty directory"
            
            return True, "\n".join(items)
            
        except Exception as e:
            return False, f"❌ Failed to list files: {str(e)}"


class SafeWebSearch:
    """
    Safe web search using only DuckDuckGo API (HTTP GET only).
    """
    
    def __init__(self, max_results: int = 5):
        self.max_results = max_results
    
    def search(self, query: str) -> tuple[bool, str]:
        """
        Search the web using DuckDuckGo.
        
        Args:
            query: Search query
            
        Returns:
            (success, results_or_error) tuple
        """
        try:
            # Use DuckDuckGo Instant Answer API
            url = "https://api.duckduckgo.com/"
            params = {
                'q': query,
                'format': 'json',
                'no_html': '1',
                'skip_disambig': '1'
            }
            
            response = requests.get(url, params=params, timeout=10)
            response.raise_for_status()
            
            data = response.json()
            
            # Format results
            results = []
            
            # Abstract (main answer)
            if data.get('Abstract'):
                results.append(f"📌 {data['AbstractText']}")
                if data.get('AbstractURL'):
                    results.append(f"   🔗 {data['AbstractURL']}")
            
            # Related topics
            if data.get('RelatedTopics'):
                results.append("\n🔍 Related topics:")
                for i, topic in enumerate(data['RelatedTopics'][:self.max_results], 1):
                    if isinstance(topic, dict) and 'Text' in topic:
                        results.append(f"{i}. {topic.get('Text', '')}")
                        if topic.get('FirstURL'):
                            results.append(f"   🔗 {topic['FirstURL']}")
            
            if not results:
                return True, "No results found for this query."
            
            return True, "\n".join(results)
            
        except requests.exceptions.RequestException as e:
            return False, f"❌ Search failed: {str(e)}"
        except Exception as e:
            return False, f"❌ Unexpected error: {str(e)}"


def create_safe_environment(interpreter, safe_mode: SafeMode):
    """
    Inject safe functions into the Python environment.
    """
    safe_functions = {
        'create_file': safe_mode.file_manager.create_file,
        'read_file': safe_mode.file_manager.read_file,
        'delete_file': safe_mode.file_manager.delete_file,
        'list_files': safe_mode.file_manager.list_files,
        'search_web': safe_mode.web_search.search,
    }
    
    return safe_functions
