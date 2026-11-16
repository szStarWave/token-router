# S0 协议覆盖报告

> 由 `cargo xtask fixtures-check --report docs/s0-gap-report.md` 生成，请勿手改。
> 覆盖探针只统计**请求**字段；响应形状与流式事件由 fixture 自检断言。
> 本报告不是“兼容承诺”：v2 的 canonical 面见设计文档 §4.1。

## 数据来源与已知限制

- 样本为**自建协议样本**（`v2/tests/fixtures/`），不是真实客户端录制；
因此 100% 覆盖率只证明“映射表覆盖了这些形状”，不证明真实客户端不会带来新字段。
- 三客户端冒烟（Hermes / Codex / Claude Code）尚未执行，需在具备客户端与上游凭据的
环境补录真实样本后重跑本门禁；届时新增字段会出现在“未映射字段”里。
- provider 私有字段靠 `passthrough` 兜底；探针里映射到 `passthrough.*` 的字段视为“有处置”。
- mock upstream / v1 重放尚未接入：P0a 的 e2e 需要它，届时补 `crates/mock-upstream`。

## 覆盖率

| 协议 | fixtures | 流式 | 错误 | wire 叶子 | 已映射 | 覆盖率 |
|---|---:|---:|---:|---:|---:|---:|
| anthropic_messages | 6 | 2 | 1 | 55 | 55 | 100.0% |
| chat_completions | 7 | 2 | 2 | 55 | 55 | 100.0% |
| responses | 6 | 2 | 1 | 64 | 64 | 100.0% |

## 未映射字段

（无）

## 断言失败

（无）

门禁阈值：每个协议覆盖率 ≥ 95%。
