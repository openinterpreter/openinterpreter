<h1 align="center">● Open Interpreter</h1>

<p align="center">
    <a href="https://discord.gg/Hvz9Axh84z">
        <img alt="Discord" src="https://img.shields.io/discord/1146610656779440188?logo=discord&style=flat&logoColor=white"/></a>
    <a href="README_JA.md"><img src="https://img.shields.io/badge/ドキュメント-日本語-white.svg" alt="JA doc"/></a>
    <a href="README_ZH.md"><img src="https://img.shields.io/badge/文档-中文版-white.svg" alt="ZH doc"/></a>
    <a href="README_ES.md"> <img src="https://img.shields.io/badge/Español-white.svg" alt="ES doc"/></a>
    <a href="README_UK.md"><img src="https://img.shields.io/badge/Українська-white.svg" alt="UK doc"/></a>
    <a href="README_IN.md"><img src="https://img.shields.io/badge/Hindi-white.svg" alt="IN doc"/></a>
    <a href="README_RU.md"><img src="https://img.shields.io/badge/Русский-white.svg" alt="RU doc"/></a>
    <a href="LICENSE"><img src="https://img.shields.io/static/v1?label=license&message=AGPL&color=white&style=flat" alt="License"/></a>
    <br>
    <br><a href="https://0ggfznkwh4j.typeform.com/to/G21i9lJ2">Получить ранний доступ к ПК версии приложения</a>‎ ‎ |‎ ‎ <a href="https://docs.openinterpreter.com/">Документация</a><br>
</p>

<br>

<img alt="local_explorer" src="https://github.com/OpenInterpreter/open-interpreter/assets/63927363/d941c3b4-b5ad-4642-992c-40edf31e2e7a">

<br>

**Open Interpreter** позволяет большим языковым моделям (LLM) выполнять код (Python, Javascript, Shell и другие) локально. Вы можете общаться с Open Interpreter через интерфейс в стиле ChatGPT в вашем терминале, запустив `$ interpreter` после установки.

Это предоставляет интерфейс на естественном языке для работы с базовыми возможностями вашего компьютера:

- Создание и редактирование фотографий, видео, PDF и т.д.
- Управление браузером Chrome для проведения исследований
- Построение графиков, очистка и анализ больших наборов данных
- ...и многое другое.

**⚠️ Примечание: Перед выполнением кода вам будет предложено его подтвердить.**

<br>

## Демо

https://github.com/OpenInterpreter/open-interpreter/assets/63927363/37152071-680d-4423-9af3-64836a6f7b60

#### Интерактивное демо также доступно на Google Colab:

[![Open In Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/drive/1WKmRXZgsErej2xUriKzxrEAXdxMSgWbb?usp=sharing)

#### А также пример голосового интерфейса, вдохновлённый фильмом _«Она»_:

[![Open In Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/drive/1NojYGHDgxH6Y1G1oxThEBBb2AtyODBIK)

## Быстрый старт

### Установка

```shell
pip install git+https://github.com/OpenInterpreter/open-interpreter.git
```

> Не работает? Прочитайте наше [руководство по установке](https://docs.openinterpreter.com/getting-started/setup).

### Терминал

После установки просто запустите `interpreter`:

```shell
interpreter
```

### Python

```python
from interpreter import interpreter

interpreter.chat("Построй график нормализованных цен акций AAPL и META") # Выполняет одну команду
interpreter.chat() # Запускает интерактивный чат
```

### GitHub Codespaces

Нажмите клавишу `,` на странице репозитория GitHub, чтобы создать codespace. Через мгновение вы получите облачную виртуальную машину с предустановленным open-interpreter. Затем вы можете начать взаимодействие с ним напрямую и свободно подтверждать выполнение системных команд, не беспокоясь о повреждении системы.

## Сравнение с Code Interpreter от ChatGPT

Выпуск OpenAI [Code Interpreter](https://openai.com/blog/chatgpt-plugins#code-interpreter) с GPT-4 предоставляет отличную возможность выполнять реальные задачи с помощью ChatGPT.

Однако сервис OpenAI размещён на их серверах, является закрытым и имеет серьёзные ограничения:

- Нет доступа к интернету.
- [Ограниченный набор предустановленных пакетов](https://wfhbrian.com/mastering-chatgpts-code-interpreter-list-of-python-packages/).
- Максимальный размер загрузки 100 МБ, ограничение времени выполнения 120 секунд.
- Состояние очищается (вместе с созданными файлами или ссылками) при завершении сессии.

---

Open Interpreter преодолевает эти ограничения, работая в вашей локальной среде. Он имеет полный доступ к интернету, не ограничен по времени или размеру файлов и может использовать любой пакет или библиотеку.

Это сочетает мощь Code Interpreter от GPT-4 с гибкостью вашей локальной среды разработки.

## Команды

**Обновление:** Generator (0.1.5) представило потоковую передачу (стрим):

```python
message = "Какая у нас операционная система?"

for chunk in interpreter.chat(message, display=False, stream=True):
  print(chunk)
```

### Интерактивный чат

Чтобы запустить интерактивный чат в терминале, выполните `interpreter` из командной строки:

```shell
interpreter
```

Или `interpreter.chat()` из .py файла:

```python
interpreter.chat()
```

**Вы также можете получать данные потоком (стримом):**

```python
message = "Какая у нас операционная система?"

for chunk in interpreter.chat(message, display=False, stream=True):
  print(chunk)
```

### Программный чат

Для более точного управления вы можете передавать сообщения напрямую в `.chat(message)`:

```python
interpreter.chat("Добавь субтитры ко всем видео в папке /videos.")

# ... Вывод потоком в терминал, выполнение задачи ...

interpreter.chat("Отлично, но можешь сделать субтитры покрупнее?")

# ...
```

### Начать новый чат

В Python Open Interpreter запоминает историю разговора. Если хотите начать заново, вы можете сбросить её:

```python
interpreter.messages = []
```

### Сохранение и восстановление чатов

`interpreter.chat()` возвращает список сообщений, который можно использовать для возобновления разговора с помощью `interpreter.messages = messages`:

```python
messages = interpreter.chat("Меня зовут Иван.") # Сохраняем сообщения в 'messages'
interpreter.messages = [] # Сбрасываем интерпретатор ("Иван" будет забыт)

interpreter.messages = messages # Возобновляем чат из 'messages' ("Иван" будет помнить)
```

### Настройка системного сообщения

Вы можете просматривать и настраивать системное сообщение Open Interpreter для расширения его функциональности, изменения разрешений или предоставления дополнительного контекста.

```python
interpreter.system_message += """
Запускай команды shell с флагом -y, чтобы пользователю не нужно было их подтверждать.
"""
print(interpreter.system_message)
```

### Смена языковой модели

Open Interpreter использует [LiteLLM](https://docs.litellm.ai/docs/providers/) для подключения к размещённым языковым моделям.

Вы можете изменить модель, установив параметр model:

```shell
interpreter --model gpt-3.5-turbo
interpreter --model claude-2
interpreter --model command-nightly
```

В Python установите модель на объекте:

```python
interpreter.llm.model = "gpt-3.5-turbo"
```

[Найдите подходящую строку «model» для вашей языковой модели здесь.](https://docs.litellm.ai/docs/providers/)

### Локальный запуск Open Interpreter

#### Терминал

Open Interpreter может использовать OpenAI-совместимый сервер для локального запуска моделей. (LM Studio, jan.ai, ollama и т.д.)

Просто запустите `interpreter` с URL api_base вашего сервера вывода (для LM Studio по умолчанию это `http://localhost:1234/v1`):

```shell
interpreter --api_base "http://localhost:1234/v1" --api_key "fake_key"
```

Альтернативно, вы можете использовать Llamafile без установки стороннего ПО, просто запустив:

```shell
interpreter --local
```

Для более подробного руководства посмотрите [это видео от Mike Bird](https://www.youtube.com/watch?v=CEs51hGWuGU?si=cN7f6QhfT4edfG5H)

**Как запустить LM Studio в фоновом режиме.**

1. Скачайте [https://lmstudio.ai/](https://lmstudio.ai/) и запустите.
2. Выберите модель и нажмите **↓ Скачать**.
3. Нажмите кнопку **↔️** слева (под 💬).
4. Выберите вашу модель вверху, затем нажмите **Запустить сервер**.

Когда сервер запущен, вы можете начать разговор с Open Interpreter.

> **Примечание:** Локальный режим устанавливает `context_window` в 3000 и `max_tokens` в 1000. Если ваша модель имеет другие требования, установите эти параметры вручную (см. ниже).

#### Python

Наш Python-пакет даёт больше контроля над каждой настройкой. Для подключения к LM Studio используйте эти настройки:

```python
from interpreter import interpreter

interpreter.offline = True # Отключает онлайн-функции, такие как Open Procedures
interpreter.llm.model = "openai/x" # Указывает OI отправлять сообщения в формате OpenAI
interpreter.llm.api_key = "fake_key" # LiteLLM, который мы используем для связи с LM Studio, требует это
interpreter.llm.api_base = "http://localhost:1234/v1" # Укажите на любой OpenAI-совместимый сервер

interpreter.chat()
```

#### Контекстное окно, максимальные токены

Вы можете изменить `max_tokens` и `context_window` (в токенах) для локально запущенных моделей.

Для локального режима меньшие контекстные окна используют меньше RAM, поэтому мы рекомендуем попробовать значительно меньшее окно (~1000), если происходят сбои или работает медленно. Убедитесь, что `max_tokens` меньше, чем `context_window`.

```shell
interpreter --local --max_tokens 1000 --context_window 3000
```

### Режим подробного вывода

Для помощи в отладке Open Interpreter у нас есть режим `--verbose`.

Вы можете активировать режим verbose с помощью флага (`interpreter --verbose`) или во время чата:

```shell
$ interpreter
...
> %verbose true <- Включает режим verbose

> %verbose false <- Выключает режим verbose
```

### Команды интерактивного режима

В интерактивном режиме вы можете использовать следующие команды для улучшения работы. Вот список доступных команд:

**Доступные команды:**

- `%verbose [true/false]`: Переключает режим verbose. Без аргументов или с `true` входит в режим verbose. С `false` выходит из режима verbose.
- `%reset`: Сбрасывает разговор текущей сессии.
- `%undo`: Удаляет предыдущее сообщение пользователя и ответ ИИ из истории сообщений.
- `%tokens [prompt]`: (_Экспериментально_) Вычисляет токены, которые будут отправлены со следующим промптом в качестве контекста, и оценивает их стоимость. Опционально вычисляет токены и оценочную стоимость промпта, если он предоставлен. Использует [метод LiteLLM `cost_per_token()`](https://docs.litellm.ai/docs/completion/token_usage#2-cost_per_token) для оценки стоимости.
- `%help`: Показывает справочное сообщение.

### Конфигурация / Профили

Open Interpreter позволяет устанавливать поведение по умолчанию с помощью `yaml` файлов.

Это обеспечивает гибкий способ настройки интерпретатора без изменения аргументов командной строки каждый раз.

Выполните следующую команду, чтобы открыть директорию профилей:

```
interpreter --profiles
```

Вы можете добавлять туда `yaml` файлы. Профиль по умолчанию называется `default.yaml`.

#### Несколько профилей

Open Interpreter поддерживает несколько `yaml` файлов, позволяя легко переключаться между конфигурациями:

```
interpreter --profile my_profile.yaml
```

## Пример FastAPI сервера

Обновление Generator позволяет управлять Open Interpreter через HTTP REST эндпоинты:

```python
# server.py

from fastapi import FastAPI
from fastapi.responses import StreamingResponse
from interpreter import interpreter

app = FastAPI()

@app.get("/chat")
def chat_endpoint(message: str):
    def event_stream():
        for result in interpreter.chat(message, stream=True):
            yield f"data: {result}\n\n"

    return StreamingResponse(event_stream(), media_type="text/event-stream")

@app.get("/history")
def history_endpoint():
    return interpreter.messages
```

```shell
pip install fastapi uvicorn
uvicorn server:app --reload
```

Вы также можете запустить идентичный сервер, просто выполнив `interpreter.server()`.

## Android

Пошаговое руководство по установке Open Interpreter на Android устройство можно найти в [репозитории open-interpreter-termux](https://github.com/MikeBirdTech/open-interpreter-termux).

## Предупреждение о безопасности

Поскольку сгенерированный код выполняется в вашей локальной среде, он может взаимодействовать с вашими файлами и системными настройками, что потенциально может привести к непредвиденным последствиям, таким как потеря данных или риски безопасности.

**⚠️ Open Interpreter запросит подтверждение пользователя перед выполнением кода.**

Вы можете запустить `interpreter -y` или установить `interpreter.auto_run = True`, чтобы пропустить это подтверждение, в этом случае:

- Будьте осторожны при запросе команд, которые изменяют файлы или системные настройки.
- Наблюдайте за Open Interpreter как за самоуправляемым автомобилем и будьте готовы завершить процесс, закрыв терминал.
- Рассмотрите возможность запуска Open Interpreter в ограниченной среде, такой как Google Colab или Replit. Эти среды более изолированы, что снижает риски выполнения произвольного кода.

Существует **экспериментальная** поддержка [безопасного режима](https://github.com/OpenInterpreter/open-interpreter/blob/main/docs/SAFE_MODE.md) для снижения некоторых рисков.

## Как это работает?

Open Interpreter оснащает [языковую модель с поддержкой вызова функций](https://platform.openai.com/docs/guides/gpt/function-calling) функцией `exec()`, которая принимает `language` (например, "Python" или "JavaScript") и `code` для выполнения.

Затем мы передаём сообщения модели, код и выводы вашей системы в терминал в формате Markdown.

# Вклад в проект

Спасибо за ваш интерес к участию! Мы приветствуем вовлечение сообщества.

Пожалуйста, ознакомьтесь с нашими [правилами участия](https://github.com/OpenInterpreter/open-interpreter/blob/main/docs/CONTRIBUTING.md) для получения более подробной информации о том, как участвовать.

# Дорожная карта

Посетите [нашу дорожную карту](https://github.com/OpenInterpreter/open-interpreter/blob/main/docs/ROADMAP.md) для предпросмотра будущего Open Interpreter.

**Примечание**: Это программное обеспечение не связано с OpenAI.

![thumbnail-ncu](https://github.com/OpenInterpreter/open-interpreter/assets/63927363/1b19a5db-b486-41fd-a7a1-fe2028031686)


> Доступ к младшему программисту, работающему со скоростью ваших пальцев... может сделать новые рабочие процессы лёгкими и эффективными, а также открыть преимущества программирования для новой аудитории.
>
> — _Релиз Code Interpreter от OpenAI_

<br>
