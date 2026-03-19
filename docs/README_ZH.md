<h1 align="center">● Open Interpreter</h1>

<p align="center">
    <a href="https://discord.gg/Hvz9Axh84z">
        <img alt="Discord" src="https://img.shields.io/discord/1146610656779440188?logo=discord&style=flat&logoColor=white"/></a>
    <a href="README_JA.md"><img src="https://img.shields.io/badge/ドキュメント-日本語-white.svg" alt="JA doc"/></a>
    <a href="README_ZH.md"><img src="https://img.shields.io/badge/文档-中文版-white.svg" alt="ZH doc"/></a>
    <a href="README_ES.md"> <img src="https://img.shields.io/badge/Español-white.svg" alt="ES doc"/></a>
    <a href="README_UK.md"><img src="https://img.shields.io/badge/Українська-white.svg" alt="UK doc"/></a>
    <a href="README_IN.md"><img src="https://img.shields.io/badge/Hindi-white.svg" alt="IN doc"/></a>
    <a href="../README.md"><img src="https://img.shields.io/badge/english-document-white.svg" alt="EN doc"></a>
    <a href="../LICENSE"><img src="https://img.shields.io/static/v1?label=license&message=AGPL&color=white&style=flat" alt="License"/></a>
    <br>
    <br><a href="https://0ggfznkwh4j.typeform.com/to/G21i9lJ2">登记以提前获取桌面应用程序</a>‎ ‎ |‎ ‎ <a href="https://docs.openinterpreter.com/">文档</a><br>
</p>

<br>

<img alt="local_explorer" src="https://github.com/OpenInterpreter/open-interpreter/assets/63927363/d941c3b4-b5ad-4642-992c-40edf31e2e7a">

<br>
</p>
<br>

**Open Interpreter** 可以让大语言模型（LLMs）在本地运行代码（比如 Python、Javascript、Shell 等）。安装后，在终端上运行 `$ interpreter` 即可通过类似 ChatGPT 的界面与 Open Interpreter 聊天。

本软件为计算机的通用功能提供了一个自然语言界面，比如：

- 创建和编辑照片、视频、PDF 等
- 控制 Chrome 浏览器进行研究
- 绘制、清理和分析大型数据集
- ...等

**⚠️ 注意：在代码运行前都会要求您批准执行代码。**

<br>

## 演示

https://github.com/OpenInterpreter/open-interpreter/assets/63927363/37152071-680d-4423-9af3-64836a6f7b60

#### Google Colab 上也提供了交互式演示：

[![Open In Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/drive/1WKmRXZgsErej2xUriKzxrEAXdxMSgWbb?usp=sharing)

#### 以及一个受电影《Her》启发的语音界面示例：

[![Open In Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/drive/1NojYGHDgxH6Y1G1oxThEBBb2AtyODBIK)

## 快速开始

### 安装

```shell
pip install git+https://github.com/OpenInterpreter/open-interpreter.git
```

> 无法运行？请阅读我们的[设置指南](https://docs.openinterpreter.com/getting-started/setup)。

### 终端

安装后，直接运行 `interpreter`：

```shell
interpreter
```

### Python

```python
from interpreter import interpreter

interpreter.chat("绘制 AAPL 和 META 的标准化股票价格图") # 执行单一命令
interpreter.chat() # 开始交互式聊天
```

### GitHub Codespaces

在本仓库的 GitHub 页面上按下 `,` 键创建一个 codespace。片刻之后，您将获得一个预装了 open-interpreter 的云端虚拟机环境。然后您可以直接开始与它交互，并自由确认它执行系统命令，无需担心损坏系统。

## 与 ChatGPT 代码解释器的比较

OpenAI 发布的带有 GPT-4 的 [Code Interpreter](https://openai.com/blog/chatgpt-plugins#code-interpreter) 提供了一个使用 ChatGPT 完成实际任务的绝佳机会。

但是，OpenAI 的服务是托管的、闭源的，并且受到严格限制：

- 无法访问互联网。
- [预装软件包数量有限](https://wfhbrian.com/mastering-chatgpts-code-interpreter-list-of-python-packages/)。
- 最大上传为 100 MB，且最大运行时间限制为 120.0 秒。
- 当环境终止时，之前的状态会被清除（包括任何生成的文件或链接）。

---

Open Interpreter 通过在您的本地环境中运行克服了这些限制。它可以完全访问互联网，不受运行时间或文件大小的限制，并且可以使用任何软件包或库。

它将 GPT-4 代码解释器的强大功能与您本地开发环境的灵活性相结合。

## 命令

**更新：** Generator 更新 (0.1.5) 引入了流式传输：

```python
message = "我们目前使用的是什么操作系统？"

for chunk in interpreter.chat(message, display=False, stream=True):
  print(chunk)
```

### 交互式聊天

要在终端中开始交互式聊天，可以通过命令行运行 `interpreter`：

```shell
interpreter
```

或者在一个 .py 文件中运行 `interpreter.chat()`：

```python
interpreter.chat()
```

**您还可以流式传输每个代码块：**

```python
message = "我们目前使用的是什么操作系统？"

for chunk in interpreter.chat(message, display=False, stream=True):
  print(chunk)
```

### 程序化聊天

为了获得更精确的控制，您可以直接将消息传递给 `.chat(message)`：

```python
interpreter.chat("为 /videos 目录下的所有视频添加字幕。")

# ... 向终端流式输出，完成任务 ...

interpreter.chat("这些看起来很棒，但你能把字幕调大点吗？")

# ...
```

### 开始新的聊天

在 Python 中，Open Interpreter 会记录对话历史。如果您想从头开始，可以重置它：

```python
interpreter.messages = []
```

### 保存和恢复聊天

`interpreter.chat()` 会返回一个消息列表，这可以用于通过 `interpreter.messages = messages` 恢复对话：

```python
messages = interpreter.chat("我的名字是 Killian。") # 将消息保存到 'messages'
interpreter.messages = [] # 重置解释器（"Killian" 将被遗忘）

interpreter.messages = messages # 从 'messages' 恢复聊天（"Killian" 将被记住）
```

### 自定义系统消息

您可以检查和配置 Open Interpreter 的系统消息，以扩展其功能、修改权限或赋予其更多上下文。

```python
interpreter.system_message += """
使用 -y 运行 shell 命令，这样用户就不必确认它们。
"""
print(interpreter.system_message)
```

### 更改语言模型

Open Interpreter 使用 [LiteLLM](https://docs.litellm.ai/docs/providers/) 连接到托管语言模型。

您可以通过设置 model 参数来更改模型：

```shell
interpreter --model gpt-3.5-turbo
interpreter --model claude-2
interpreter --model command-nightly
```

在 Python 中，在对象上设置模型：

```python
interpreter.llm.model = "gpt-3.5-turbo"
```

[在此处找到适用于您语言模型的合适 "model" 字符串。](https://docs.litellm.ai/docs/providers/)

### 在本地运行 Open Interpreter

#### 终端

Open Interpreter 可以使用兼容 OpenAI 的服务器在本地运行模型（LM Studio, jan.ai, ollama 等）。

只需使用推理服务器的 api_base URL 运行 `interpreter`（对于 LM Studio，默认是 `http://localhost:1234/v1`）：

```shell
interpreter --api_base "http://localhost:1234/v1" --api_key "fake_key"
```

或者，您无需安装任何第三方软件即可使用 Llamafile，只需运行：

```shell
interpreter --local
```

有关更详细的指南，请查看 [Mike Bird 的这个视频](https://www.youtube.com/watch?v=CEs51hGWuGU?si=cN7f6QhfT4edfG5H)

**如何在后台运行 LM Studio。**

1. 下载 [https://lmstudio.ai/](https://lmstudio.ai/) 然后启动它。
2. 选择一个模型，然后点击 **↓ Download**。
3. 点击左侧（💬下方）的 **↔️** 按钮。
4. 在顶部选择您的模型，然后点击 **Start Server**。

服务器运行后，您就可以开始与 Open Interpreter 聊天了。

> **注意：** 本地模式会将 `context_window` 设置为 3000，`max_tokens` 设置为 1000。如果您的模型有不同的要求，请手动设置这些参数（见下文）。

#### Python

我们的 Python 包含为您提供了对每个设置的更多控制。为了复现并连接到 LM Studio，请使用以下设置：

```python
from interpreter import interpreter

interpreter.offline = True # 禁用像 Open Procedures 这样的在线功能
interpreter.llm.model = "openai/x" # 告诉 OI 以 OpenAI 的格式发送消息
interpreter.llm.api_key = "fake_key" # LiteLLM 需要这个来与 LM Studio 对话
interpreter.llm.api_base = "http://localhost:1234/v1" # 将其指向任何兼容 OpenAI 的服务器

interpreter.chat()
```

#### 上下文窗口，最大 Token 数

您可以修改本地运行模型的 `max_tokens` 和 `context_window`（以 token 为单位）。

对于本地模式，较小的上下文窗口会使用更少的 RAM，因此如果它运行失败 / 运行缓慢，我们建议尝试一个短得多的窗口（约 1000）。确保 `max_tokens` 小于 `context_window`。

```shell
interpreter --local --max_tokens 1000 --context_window 3000
```

### 调试模式 (Verbose mode)

为了帮助您检查 Open Interpreter，我们提供了一个 `--verbose` 模式用于调试。

您可以使用它的标志 (`interpreter --verbose`) 或在聊天中途激活调试模式：

```shell
$ interpreter
...
> %verbose true <- 开启调试模式

> %verbose false <- 关闭调试模式
```

### 交互模式命令

在交互模式下，您可以使用以下命令来增强体验。以下是可用命令的列表：

**可用命令：**

- `%verbose [true/false]`: 切换调试模式。不带参数或带 `true` 时进入调试模式。带 `false` 时退出。
- `%reset`: 重置当前会话的对话。
- `%undo`: 从消息历史中删除上一条用户消息和 AI 的回复。
- `%tokens [prompt]`: (_实验性_) 计算作为上下文与下一个 prompt 一起发送的 token 数并估算其成本。如果提供了一个 `prompt`，也可以选择计算它的 token 数和预估成本。依赖于 [LiteLLM 的 `cost_per_token()` 方法](https://docs.litellm.ai/docs/completion/token_usage#2-cost_per_token) 进行预估。
- `%help`: 显示帮助信息。

### 配置 / 配置文件

Open Interpreter 允许您使用 `yaml` 文件设置默认行为。

这提供了一种配置解释器的灵活方式，而无需每次都更改命令行参数。

运行以下命令打开配置文件目录：

```
interpreter --profiles
```

您可以在此处添加 `yaml` 文件。默认的配置文件名为 `default.yaml`。

#### 多个配置文件

Open Interpreter 支持多个 `yaml` 文件，允许您轻松地在配置之间切换：

```
interpreter --profile my_profile.yaml
```

## 示例 FastAPI 服务器

Generator 更新使得 Open Interpreter 可以通过 HTTP REST 端点进行控制：

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

您也可以简单地运行 `interpreter.server()` 来启动一个与上述完全相同的服务器。

## Android

关于在您的 Android 设备上安装 Open Interpreter 的分步指南，可以在 [open-interpreter-termux 仓库](https://github.com/MikeBirdTech/open-interpreter-termux)中找到。

## 安全须知

由于生成的代码是在您的本地环境中执行的，它可能会与您的文件和系统设置交互，从而可能导致意想不到的后果，如数据丢失或安全风险。

**⚠️ Open Interpreter 在执行代码之前会要求用户确认。**

您可以运行 `interpreter -y` 或设置 `interpreter.auto_run = True` 来绕过此确认，在这种情况下：

- 在请求修改文件或系统设置的命令时要小心。
- 像看护自动驾驶汽车一样留意 Open Interpreter，并随时准备通过关闭终端来终止进程。
- 考虑在受限的环境中运行 Open Interpreter，例如 Google Colab 或 Replit。这些环境更加隔离，从而降低了执行任意代码的风险。

目前提供了针对[安全模式 (safe mode)](https://github.com/OpenInterpreter/open-interpreter/blob/main/docs/SAFE_MODE.md) 的**实验性**支持，以帮助缓解部分风险。

## 它是如何工作的？

Open Interpreter 为[函数调用语言模型](https://platform.openai.com/docs/guides/gpt/function-calling) 配备了一个 `exec()` 函数，该函数接受 `language`（如 "Python" 或 "JavaScript"）和要运行的 `code`。

然后，我们将模型的消息、代码以及您系统的输出以 Markdown 格式流式传输到终端。

# 离线访问文档

您可以在没有网络连接的情况下随时访问完整的[文档](https://docs.openinterpreter.com/)。

[Node](https://nodejs.org/en) 是先决条件：

- 版本 18.17.0 或任何后继的 18.x.x 版本。
- 版本 20.3.0 或任何后继的 20.x.x 版本。
- 从 21.0.0 开始的任何版本，未指定上限。

安装 [Mintlify](https://mintlify.com/):

```bash
npm i -g mintlify@latest
```

进入 docs 目录并运行相应命令：

```bash
# 假设您在项目根目录下
cd ./docs

# 运行文档服务器
mintlify dev
```

应该会打开一个新的浏览器窗口。只要文档服务器在运行，文档就可以通过 [http://localhost:3000](http://localhost:3000) 访问。

# 参与贡献

感谢您对贡献的兴趣！我们欢迎社区的参与。

请参阅我们的 [贡献指南](https://github.com/OpenInterpreter/open-interpreter/blob/main/docs/CONTRIBUTING.md) 以获取有关如何参与的更多详细信息。

# 路线图

访问 [我们的路线图](https://github.com/OpenInterpreter/open-interpreter/blob/main/docs/ROADMAP.md) 预览 Open Interpreter 的未来。

**注意**: 此软件不隶属于 OpenAI。

![thumbnail-ncu](https://github.com/OpenInterpreter/open-interpreter/assets/63927363/1b19a5db-b486-41fd-a7a1-fe2028031686)

> 拥有一个在你指尖上以极快速度工作的初级程序员...可以使新的工作流程变得轻松而高效，并向新的受众开放编程的优势。
>
> — _OpenAI 的代码解释器发布公告_

<br>