# Example Usage of Safe Mode

This document provides examples of how to use Open Interpreter in Safe Mode.

## Starting Safe Mode

```bash
# Start with the convenience script
./start_safe.sh

# Or manually
source venv_safe/bin/activate
python run_safe.py
```

## Example 1: Create and Read Files

```
You: Create a file called hello.txt with the content "Hello from Safe Mode!"

AI will execute:
```python
success, message = create_file("hello.txt", "Hello from Safe Mode!")
print(message)
```

Output: ✅ File created: hello.txt

---

You: Now read that file

AI will execute:
```python
success, content = read_file("hello.txt")
if success:
    print(content)
else:
    print(content)  # This will contain the error message
```

Output: Hello from Safe Mode!
```

## Example 2: Work with CSV Data

```
You: Create a CSV file with sales data for January

AI will execute:
```python
csv_data = """Date,Product,Amount,Quantity
2024-01-01,Widget A,1250.50,25
2024-01-02,Widget B,890.75,15
2024-01-03,Widget A,2100.00,42
2024-01-04,Widget C,550.25,11
2024-01-05,Widget B,1780.50,30"""

success, message = create_file("sales_january.csv", csv_data)
print(message)
```

Output: ✅ File created: sales_january.csv

---

You: Calculate total sales from that CSV

AI will execute:
```python
success, content = read_file("sales_january.csv")
if not success:
    print("Error:", content)
else:
    lines = content.strip().split('\n')
    total = 0
    for line in lines[1:]:  # Skip header
        parts = line.split(',')
        amount = float(parts[2])
        total += amount
    print(f"Total sales: ${total:.2f}")
```

Output: Total sales: $6571.00
```

## Example 3: Search the Web

```
You: Search for information about Python list comprehensions

AI will execute:
```python
success, results = search_web("Python list comprehensions")
print(results)
```

Output: 
```
📌 List comprehensions provide a concise way to create lists in Python...
   🔗 https://docs.python.org/3/tutorial/datastructures.html

🔍 Related topics:
1. List comprehension is a syntactic construct available in Python...
   🔗 https://en.wikipedia.org/wiki/List_comprehension
```
```

## Example 4: List Files

```
You: Show me all files in the workspace

AI will execute:
```python
success, file_list = list_files()
print(file_list)
```

Output:
```
📄 hello.txt (24 bytes)
📄 sales_january.csv (185 bytes)
```
```

## Example 5: Create JSON Data

```
You: Create a JSON file with configuration data

AI will execute:
```python
import json

config = {
    "app_name": "MyApp",
    "version": "1.0.0",
    "settings": {
        "debug": False,
        "max_connections": 100,
        "timeout": 30
    },
    "features": ["feature1", "feature2", "feature3"]
}

json_content = json.dumps(config, indent=2)
success, message = create_file("config.json", json_content)
print(message)
```

Output: ✅ File created: config.json
```

## Example 6: Data Processing

```
You: Read the sales CSV, calculate average sale amount, and create a summary JSON file

AI will execute:
```python
import json

# Read CSV
success, content = read_file("sales_january.csv")
if not success:
    print("Error reading CSV:", content)
else:
    # Parse and calculate
    lines = content.strip().split('\n')
    amounts = []
    for line in lines[1:]:
        parts = line.split(',')
        amounts.append(float(parts[2]))
    
    avg_amount = sum(amounts) / len(amounts)
    total_amount = sum(amounts)
    num_sales = len(amounts)
    
    # Create summary
    summary = {
        "period": "January 2024",
        "total_sales": total_amount,
        "num_transactions": num_sales,
        "average_sale": round(avg_amount, 2),
        "min_sale": min(amounts),
        "max_sale": max(amounts)
    }
    
    # Write to JSON
    json_content = json.dumps(summary, indent=2)
    success, message = create_file("sales_summary.json", json_content)
    print(message)
    print("\nSummary:")
    print(json_content)
```

Output:
```
✅ File created: sales_summary.json

Summary:
{
  "period": "January 2024",
  "total_sales": 6571.0,
  "num_transactions": 5,
  "average_sale": 1314.2,
  "min_sale": 550.25,
  "max_sale": 2100.0
}
```
```

## Example 7: HTML Generation

```
You: Create an HTML page showing the sales data

AI will execute:
```python
# Read the sales data
success, csv_content = read_file("sales_january.csv")
if not success:
    print("Error:", csv_content)
else:
    lines = csv_content.strip().split('\n')
    header = lines[0].split(',')
    data_rows = [line.split(',') for line in lines[1:]]
    
    # Create HTML
    html = """<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Sales Report - January 2024</title>
    <style>
        body { font-family: Arial, sans-serif; margin: 20px; }
        h1 { color: #333; }
        table { border-collapse: collapse; width: 100%; }
        th, td { border: 1px solid #ddd; padding: 8px; text-align: left; }
        th { background-color: #4CAF50; color: white; }
        tr:nth-child(even) { background-color: #f2f2f2; }
    </style>
</head>
<body>
    <h1>Sales Report - January 2024</h1>
    <table>
        <thead>
            <tr>"""
    
    # Add headers
    for h in header:
        html += f"\n                <th>{h}</th>"
    
    html += """
            </tr>
        </thead>
        <tbody>"""
    
    # Add data rows
    for row in data_rows:
        html += "\n            <tr>"
        for cell in row:
            html += f"\n                <td>{cell}</td>"
        html += "\n            </tr>"
    
    html += """
        </tbody>
    </table>
</body>
</html>"""
    
    success, message = create_file("sales_report.html", html)
    print(message)
```

Output: ✅ File created: sales_report.html
```

## What Gets Blocked

### Blocked: Shell Commands

```
You: Run ls to see files

AI tries:
```bash
ls -la
```

Output: ❌ Shell execution is blocked in safe mode. Language: shell
```

### Blocked: Package Installation

```
You: Install pandas

AI tries:
```bash
pip install pandas
```

Output: ❌ Shell execution is blocked in safe mode. Language: bash
```

### Blocked: Subprocess Module

```
You: Execute a system command

AI tries:
```python
import subprocess
subprocess.run(['ls', '-la'])
```

Output: ❌ Blocked module detected: subprocess
```

### Blocked: File Access Outside Workspace

```
You: Read /etc/passwd

AI tries:
```python
success, content = read_file("/etc/passwd")
print(content)
```

Output: (False, '❌ Absolute paths are not allowed: /etc/passwd')
```

### Blocked: Direct File Operations

```
You: Create a file using open()

AI tries:
```python
with open('test.txt', 'w') as f:
    f.write('hello')
```

Output: ❌ Direct file operation detected: open(. Use create_file(), read_file(), delete_file() instead.
```

## Tips

1. **Always use the safe functions**: `create_file()`, `read_file()`, `delete_file()`, `list_files()`, `search_web()`
2. **Check return values**: All functions return `(success, result)` tuples
3. **File extensions matter**: Only allowed extensions can be used
4. **Relative paths only**: No absolute paths or `../` traversal
5. **Review code before execution**: Safe mode is set to `auto_run=false` by default

## Audit Log

All actions are logged to `~/model_workspace/.audit.log`:

```bash
cat ~/model_workspace/.audit.log | tail -5
```

Example log entry:
```json
{
  "timestamp": "2026-02-06T20:30:00.123456",
  "operation": "code_execution",
  "params": {"language": "python", "code": "success, message = create_file('test.txt', 'hello')..."},
  "result": "Execution started",
  "success": true
}
```
