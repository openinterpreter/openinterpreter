---
title: 非交互模式
description: 使用 interpreter exec 在脚本、CI 和流水线中运行 Open Interpreter。
---

当你希望某个任务在不启动全屏 TUI 的情况下完整执行时，使用 `interpreter exec`。

```bash
interpreter exec "summarize the changes in the last commit"
```

可读的最终答案会打印到 stdout。进度和诊断信息使用 stderr，除非你选择 JSON 输出。

## 输入

将提示作为参数传入：

```bash
interpreter exec "find one bug in src/parser.rs"
```

从 stdin 读取提示：

```bash
cat task.md | interpreter exec -
```

将上下文通过管道传入提示：

```bash
git diff | interpreter exec "explain this diff and flag risky changes"
```

为第一个提示附加图片：

```bash
interpreter exec -i screenshot.png "describe the UI problem"
```

## 常用标志

| 标志 | 用途 |
| ---- | ---- |
| `--json` | 输出换行分隔的 JSON 事件。 |
| `--output-schema <file>` | 要求最终答案符合 JSON Schema。 |
| `--output-last-message, -o <file>` | 将最终的助手消息写入文件。 |
| `--color always|never|auto` | 控制 ANSI 颜色。 |
| `--sandbox <mode>` | 覆盖沙箱模式。 |
| `--ask-for-approval <mode>` | 覆盖批准策略。 |
| `--profile <name>` | 使用指定的配置文件。 |
| `--ephemeral` | 不持久化会话记录。 |
| `--skip-git-repo-check` | 允许在非 Git 仓库中运行。 |
| `--ignore-user-config` | 跳过本次运行的用户配置。 |
| `--ignore-rules` | 跳过 execpolicy 规则。 |
| `--verify` | 在退出前额外运行一次完成检查。 |
| `--timeout <seconds>` | 在运行期间发送剩余时间提醒。 |

## JSON 事件

自动化场景请使用 `--json`：

```bash
interpreter exec --json "list the files this task would touch"
```

每一行都是一个 JSON 事件，表示进度、工具调用、文件更改、推理摘要或最终消息。

## 完成状态与退出码

退出码为零表示 agent 回合已到达协议定义的完成状态，并不能证明任意指令所要求的副作用已经发生。例如，本地模型可能返回正常的最终消息，却没有创建提示词要求的文件。

适用时可使用 `--verify` 请求额外的完成检查回合，同时让自动化流程直接检查必要的输出：

```bash
interpreter exec --verify "create NOTES.md summarizing this project"
test -s NOTES.md
```

使用本地模型时，还应确保模型服务器实际启用的上下文窗口足以容纳完整的 agent 提示词和工具 schema。请参阅[本地模型的工具调用](/docs/models#本地模型的工具调用)。

## 结构化输出

配合 `--output-schema` 使用模式文件：

```json schema.json
{
  "type": "object",
  "properties": {
    "risk": { "type": "string" },
    "recommended_fix": { "type": "string" }
  },
  "required": ["risk", "recommended_fix"]
}
```

```bash
interpreter exec --output-schema schema.json \
  "inspect the current diff and return the highest risk"
```

## 恢复 Exec 工作

继续最近的非交互会话：

```bash
interpreter exec resume --last "now apply the plan"
```

或恢复指定的会话 ID：

```bash
interpreter exec resume <SESSION_ID> "continue"
```

添加 `--all` 可搜索当前工作目录之外的会话。

## 来自 Exec 的审查

在不打开 TUI 的情况下运行代码审查：

```bash
interpreter exec review --uncommitted
interpreter exec review --base main
interpreter exec review --commit abc123
```

对于自定义审查指令，可传入文本或使用 `-` 从 stdin 读取。

## CI 模式

使用 API Key 认证并保持沙箱范围窄：

```yaml
- name: Review patch
  env:
    OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
  run: |
    interpreter exec --json --sandbox read-only \
      "review this pull request diff for regressions" \
      < pr.diff > review.jsonl
```
