# S0 客户端冒烟（Hermes / Codex / Claude Code）

**状态：未执行 —— 等待客户端与上游凭据。**

本文件是 S0-5 的产物占位与执行说明。没有真实客户端时，不把 mock/curl 的
结果记成“客户端冒烟通过”。

## 为什么没有执行

- 需要本机安装 Hermes、Codex、Claude Code（或等价客户端）；
- 需要至少一个真实上游凭据（OpenAI 兼容 / Anthropic）；
- 需要用客户端自己的配置把 base URL 指向 v2。

## 执行步骤（已就绪）

每条客户端至少跑一次**流式**和一次**非流式**：

1. 保存配置（示例见 `v2/example/config.minimal.toml`），把候选指向真实上游；
2. 启动服务：
   ```bash
   cargo run -p tr-server -- --config <config.toml> --db token-router-v2.db
   ```
3. 客户端 base URL 指向 `http://127.0.0.1:16621/v1`（Anthropic 客户端指向 `/v1` 或 `/v1/messages`）；
4. 各发一条流式、一条非流式请求；
5. 检查账本与响应头同源：
   ```bash
   cargo run -p tr-cli -- doctor --config <config.toml> --db token-router-v2.db --json
   ```

## 验收记录表（执行后填写）

| 客户端 | 协议 | 模式 | 结果 | 关键字段（served slot / usage / SSE 顺序） |
|---|---|---|---|---|
| Hermes | Chat Completions | 流式 | 待执行 | — |
| Hermes | Chat Completions | 非流式 | 待执行 | — |
| Codex | Responses | 流式 | 待执行 | — |
| Codex | Responses | 非流式 | 待执行 | — |
| Claude Code | Anthropic Messages | 流式 | 待执行 | — |
| Claude Code | Anthropic Messages | 非流式 | 待执行 | — |

任一项失败：记录请求/响应样本，更新 `docs/s0-gap-report.md`，再决定是否缩 P0 范围。

## 已完成的替代证据（不替代客户端冒烟）

- 三协议 fixtures 19 个，请求字段覆盖率 100%；
- `crates/server/tests/http.rs` 覆盖三端点流式/非流式（mock 上游）；
- SSE 事件顺序与错误帧有 golden/结构性断言。
