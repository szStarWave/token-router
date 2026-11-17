> **已归档（2026-11-17）**：本文档为历史档案，不再维护。现行文档见 [docs/README.md](../README.md)。

# S0 客户端冒烟（Hermes / Codex / Claude Code）

**状态：已执行（2026-11-16）。** 三个客户端都通过 v2 完成真实请求；上游为本机
gateway `http://127.0.0.1:3425/v1`（真实模型，非 mock）。

## 结果

| 客户端 | 协议 | 模式 | 结果 | 说明 |
|---|---|---|---|---|
| Hermes `-z` | Chat Completions | 流式 | 通过 | `model.streaming` 默认开启；输出 `hermes-smoke` |
| Hermes `-z` | Chat Completions | 非流式 | 通过 | 临时 `model.streaming=false`；输出 `hermes-nonstream-smoke` |
| Codex `exec` | Responses | 流式 | 通过 | `stream_committed`，TTFT≈1.9s，`tokens used` 正常 |
| Claude Code `-p` | Anthropic Messages | 流式 | 通过 | 输出 `claude-v2-smoke`；`unrecognized_model` 仅提示模型元数据缺失 |
| curl / server tests | Chat / Anthropic / Responses | 非流式 | 通过 | 三协议 HTTP 层各一次；Codex/Claude 的 CLI 未暴露关闭流式开关 |

## 执行方式

```bash
# 1. v2 指向真实上游
#    candidate.base_url = http://127.0.0.1:3425/v1
#    credential = "env:MAGPIE_KEY"

# 2. 启动 v2（本机 smoke 用 auth_enabled=false）
MAGPIE_KEY=magpie cargo run -p tr-server -- \
  --config <smoke.toml> --db <smoke.db> --listen 127.0.0.1:16631

# 3. 三个客户端
# Hermes：临时把 providers.magpie.base_url 指到 v2；跑完恢复
hermes -z "Reply with exactly: hermes-smoke" -m auto --provider magpie --yolo
hermes config set model.streaming false   # 非流式那一轮
# Codex：base_url 指到 v2（Responses 协议）
codex exec -c model="auto" \
  -c model_providers.magpie.base_url="http://127.0.0.1:16631/v1" \
  --skip-git-repo-check "Reply with exactly: codex-smoke"
# Claude Code：用 --settings 注入 env，避免 ~/.claude/settings.json 覆盖
claude --settings <v2-settings.json> -p "Reply with exactly: claude-smoke" --output-format text
```

## 冒烟中发现并修复的协议问题

| 客户端 | 现象 | 修复 |
|---|---|---|
| Codex | `unsupported value: developer` | Responses `developer` message 映射为 `System` |
| Codex | `missing field: tool.name` | Responses 只把 `type=function` 当工具；内置工具（web_search/local_shell）跳过；兼容 `function.name` 嵌套 |
| Claude Code | `unsupported value: system` | Anthropic messages 中出现 `role=system` 时映射为 `System` |

这三处都有 proto 单测锁定；修复后三客户端均通过。

## 已知限制

- Codex / Claude Code 的 CLI 没有关闭流式的开关，因此它们的“非流式”由协议层
  请求覆盖（server tests + curl），不是客户端自身选项。
- 客户端使用的 `model=auto` 不在 Codex 的模型元数据表里，会有
  `unrecognized_model` 警告；不影响请求。
