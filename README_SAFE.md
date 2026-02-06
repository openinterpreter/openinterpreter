# 🔒 Open Interpreter - Safe Mode

Safe Mode — это обёртка над Open Interpreter с жёсткими ограничениями безопасности для предотвращения опасных операций.

> **💡 Важно:** AI **НЕ МОЖЕТ** видеть ваши личные файлы! Все операции ограничены папкой `~/model_workspace`. Подробнее: [PRIVACY_PROTECTION_RU.md](PRIVACY_PROTECTION_RU.md)

## 🎯 Цель

Позволить использовать Open Interpreter в ограниченном безопасном окружении, где AI может:
- ✅ Работать с файлами только в изолированной папке
- ✅ Выполнять Python код с ограничениями
- ✅ Искать информацию в интернете (только чтение)
- ❌ НЕ может выполнять shell команды
- ❌ НЕ может обращаться к системным файлам
- ❌ НЕ может устанавливать пакеты

## 🔐 Функции безопасности

### 1. Песочница для файлов (`~/model_workspace`)

Все файловые операции ограничены папкой `~/model_workspace`:

- ✅ Создание, чтение, удаление файлов только внутри workspace
- ✅ Автоматическая проверка путей (защита от `../` и абсолютных путей)
- ✅ Whitelist расширений файлов: `.txt`, `.py`, `.json`, `.md`, `.csv`, `.html`, `.css`, `.js`, `.yaml`, `.yml`
- ❌ Доступ за пределы workspace заблокирован
- ❌ Системные папки недоступны (`/etc`, `/sys`, `/usr`, `/home`, и т.д.)

### 2. Блокировка Shell команд

- ❌ Shell/Bash/Zsh/PowerShell команды полностью заблокированы
- ❌ `subprocess`, `os.system()`, `eval()`, `exec()` заблокированы
- ❌ Опасные модули не могут быть импортированы

### 3. Whitelist операций

Доступны только эти 5 безопасных функций:

```python
# Создать файл
success, message = create_file("test.txt", "Hello World!")

# Прочитать файл
success, content = read_file("test.txt")

# Удалить файл
success, message = delete_file("test.txt")

# Список файлов
success, file_list = list_files()
success, file_list = list_files("subdir")

# Поиск в интернете
success, results = search_web("Python tutorials")
```

### 4. Аудит лог

Все действия логируются в `~/model_workspace/.audit.log` в формате JSON:

```json
{
  "timestamp": "2026-02-06T20:30:00.123456",
  "operation": "code_execution",
  "params": {"language": "python", "code": "create_file('test.txt', ...)"},
  "result": "Execution started",
  "success": true
}
```

### 5. Настройка для Ollama

По умолчанию используется локальная модель Ollama:

- Модель: `qwen3:14b`
- API URL: `http://localhost:11434`
- `auto_run: false` — всегда спрашивает перед выполнением кода

## 📦 Установка

### Предварительные требования

1. **Python 3.9+**
   ```bash
   python3 --version
   ```

2. **Ollama с моделью qwen3:14b**
   ```bash
   # Установить Ollama (если ещё не установлен)
   curl https://ollama.ai/install.sh | sh
   
   # Загрузить модель
   ollama pull qwen3:14b
   
   # Запустить сервер
   ollama serve
   ```

### Установка Safe Mode

```bash
# Клонировать репозиторий (если ещё не клонирован)
git clone https://github.com/frank-rikert/open-interpreter.git
cd open-interpreter

# Запустить установку
chmod +x install_safe.sh
./install_safe.sh
```

Скрипт установки:
- ✅ Создаст виртуальное окружение `venv_safe`
- ✅ Установит зависимости (open-interpreter, requests, pyyaml)
- ✅ Создаст папку `~/model_workspace`
- ✅ Создаст удобный launcher `start_safe.sh`

## 🚀 Использование

### Запуск

```bash
# Простой способ
./start_safe.sh

# Или вручную
source venv_safe/bin/activate
python run_safe.py
```

### Примеры использования

#### 1. Работа с файлами

```
You: Создай файл hello.txt с текстом "Hello, Safe Mode!"

AI: (выполняет)
success, message = create_file("hello.txt", "Hello, Safe Mode!")
# ✅ File created: hello.txt

You: Прочитай файл hello.txt

AI: (выполняет)
success, content = read_file("hello.txt")
# Содержимое: Hello, Safe Mode!

You: Покажи список всех файлов

AI: (выполняет)
success, files = list_files()
# 📄 hello.txt (21 bytes)
```

#### 2. Поиск в интернете

```
You: Найди информацию о Python asyncio

AI: (выполняет)
success, results = search_web("Python asyncio")
# 📌 asyncio is a library to write concurrent code using async/await syntax...
# 🔗 https://docs.python.org/3/library/asyncio.html
```

#### 3. Работа с данными

```
You: Создай CSV файл с данными о продажах

AI: (выполняет)
import json

data = """
Name,Price,Quantity
Apple,1.50,100
Banana,0.75,200
Orange,2.00,150
"""

success, msg = create_file("sales.csv", data)
print(msg)
# ✅ File created: sales.csv
```

### Что НЕ работает (и это правильно!)

```
You: Установи библиотеку pandas

AI: (попытка)
# ❌ Shell execution is blocked in safe mode
# 💡 Use only the approved functions

You: Выполни команду ls

AI: (попытка)
# ❌ Shell execution is blocked in safe mode. Language: shell

You: Прочитай файл /etc/passwd

AI: (попытка)
# ❌ Absolute paths are not allowed: /etc/passwd
```

## ⚙️ Конфигурация

Настройки находятся в `safe_config.yaml`:

```yaml
# Workspace directory
workspace: ~/model_workspace

# Allowed file extensions
allowed_extensions:
  - .txt
  - .py
  - .json
  - .md
  - .csv
  - .html
  - .css
  - .js

# Blocked modules
blocked_modules:
  - subprocess
  - os.system
  - socket
  - shutil.rmtree
  # ... и другие опасные модули

# Blocked keywords
blocked_keywords:
  - eval
  - exec
  - __import__
  - system
  - chmod
  - sudo
  # ... и другие опасные функции

# Ollama settings
ollama:
  model: qwen3:14b
  api_url: http://localhost:11434
```

## 📁 Структура проекта

```
open-interpreter/
├── safe_mode.py          # Основной модуль безопасности
├── run_safe.py           # Точка входа для безопасного режима
├── safe_config.yaml      # Конфигурация
├── install_safe.sh       # Скрипт установки
├── start_safe.sh         # Launcher (создаётся при установке)
├── README_SAFE.md        # Эта документация
├── venv_safe/            # Виртуальное окружение (создаётся при установке)
└── ~/model_workspace/    # Workspace для файлов
    └── .audit.log        # Лог всех действий
```

## 🔍 Компоненты

### `safe_mode.py`

- **`SafeMode`** — главный контроллер безопасности
- **`SafeFileManager`** — управление файлами в песочнице
- **`SafeWebSearch`** — безопасный поиск через DuckDuckGo API
- **`audit_log()`** — функция логирования

### `run_safe.py`

- Точка входа для безопасного режима
- Настраивает Open Interpreter с ограничениями
- Перехватывает выполнение кода
- Инжектирует safe functions в Python окружение
- Применяет custom system message

### `safe_config.yaml`

- Конфигурация workspace
- Whitelist и blacklist
- Настройки Ollama
- Параметры выполнения

## 🛡️ Как это работает

1. **Загрузка конфигурации**: Читает `safe_config.yaml`

2. **Инициализация SafeMode**: Создаёт workspace, инициализирует компоненты

3. **Настройка интерпретатора**: Создаёт instance Open Interpreter с Ollama

4. **Обёртка выполнения**: Перехватывает `interpreter.computer.run()`:
   - Проверяет язык (блокирует shell)
   - Валидирует Python код (блокирует опасные модули/функции)
   - Инжектирует safe functions
   - Логирует все действия

5. **Custom system message**: Инструктирует AI использовать только безопасные функции

6. **Валидация путей**: Проверяет все файловые операции перед выполнением

## 🔧 Расширение

### Добавление новых безопасных функций

1. Добавьте функцию в `SafeMode` или соответствующий класс:

```python
class SafeFileManager:
    def copy_file(self, src: str, dst: str) -> tuple[bool, str]:
        # Валидация путей
        # Копирование
        # Возврат результата
        pass
```

2. Добавьте в `create_safe_environment()`:

```python
safe_functions = {
    'create_file': safe_mode.file_manager.create_file,
    'copy_file': safe_mode.file_manager.copy_file,  # Новая функция
    # ...
}
```

3. Обновите system message в `run_safe.py`

### Изменение модели

Отредактируйте `safe_config.yaml`:

```yaml
ollama:
  model: llama3:8b  # Или другая модель
  api_url: http://localhost:11434
```

## 🐛 Отладка

### Включить verbose режим

```python
interpreter = OpenInterpreter(
    auto_run=False,
    safe_mode='ask',
    verbose=True,  # Добавить это
    debug=True,    # И это для детальных логов
)
```

### Просмотр audit log

```bash
cat ~/model_workspace/.audit.log | jq .
```

### Проверка Ollama

```bash
# Проверить, что Ollama работает
curl http://localhost:11434/api/generate -d '{
  "model": "qwen3:14b",
  "prompt": "Hello"
}'
```

## ⚠️ Ограничения

1. **Только Python код** — другие языки (JavaScript, R, и т.д.) заблокированы кроме Python
2. **Нет pip install** — нельзя устанавливать пакеты во время работы
3. **Ограниченный интернет** — только HTTP GET через DuckDuckGo API
4. **Нет многопоточности** — subprocess и threading заблокированы
5. **Статический анализ** — блокируются опасные ключевые слова, но AI всё ещё может найти обходные пути

## 🔐 Рекомендации безопасности

1. **Всегда проверяйте код** перед выполнением (auto_run=false)
2. **Регулярно просматривайте** audit log
3. **Ограничьте размер** workspace (квота диска)
4. **Используйте отдельного пользователя** для запуска (опционально)
5. **Изолируйте сеть** если не нужен веб-поиск

## 📝 Лицензия

Этот код является расширением Open Interpreter и распространяется под той же лицензией AGPL-3.0.

## 🤝 Вклад

Если вы нашли способ обойти ограничения безопасности, пожалуйста, сообщите об этом!

## 📞 Поддержка

При возникновении проблем:
1. Проверьте, что Ollama запущен: `curl http://localhost:11434`
2. Проверьте логи: `cat ~/model_workspace/.audit.log`
3. Запустите с `verbose=True` для детальных логов

## 🔐 Защита Личных Файлов

**Вопрос:** "AI может видеть мои личные файлы?"

**Ответ:** **НЕТ!** Все личные файлы полностью защищены. AI имеет доступ **ТОЛЬКО** к папке `~/model_workspace`.

Подробная информация о защите: **[PRIVACY_PROTECTION_RU.md](PRIVACY_PROTECTION_RU.md)**

---

**Создано для безопасного использования Open Interpreter в ограниченной среде.**
