# P1 shadow 报告（真实流量）

**状态：未产出 —— 0 / 500 样本。**

本文件是 P1-13 的产物占位与执行说明。没有真实 dogfood 流量时，这里不写任何数字：
mock 上游的一致率/延迟不构成 D-1 决策证据。

## 为什么没有数据

- 需要真实客户端（Hermes / Codex / Claude Code 或任意 OpenAI 兼容客户端）把流量指向 v2；
- 需要至少一个真实上游凭据；
- judge 以 `shadow` 模式运行：判官给建议，但 Plan 仍由内核决定。

## 采集与门禁（已就绪）

```bash
# 1. 配置 judge 为 shadow（[judge] mode = "shadow"），启动服务
cargo run -p tr-server -- --config <config.toml> --db token-router-v2.db

# 2. 让真实客户端持续跑任务，至少积累 500 条 shadow 记录

# 3. 出报告（样本不足会非零退出，可直接做 dogfood 门禁）
cargo run -p tr-cli -- shadow-report \
  --db token-router-v2.db --min-samples 500 --require --json
```

## 报告字段（由 `shadow_summary` 同源计算）

| 字段 | 含义 |
|---|---|
| `total` | shadow 样本数 |
| `agreed` / `disagreed` / `unknown` | 判官建议与内核结论的一致/不一致/无法比较 |
| `agreement_rate` | 有判定样本的一致率 |
| `avg_latency_ms` / `p95_latency_ms` | 判官调用延迟 |
| `total_cost_micros` | shadow 期间判官成本合计（来自账本 `routing_cost`） |

## D-1 active 门槛（设计 §16.2）

同时满足才允许把 judge 从 `shadow` 改为 `active`：

1. 样本数 ≥ 500；
2. 判官 P95 ≤ 300ms；
3. escalation precision ≥ 0.7（由 shadow 一致率与人工抽查共同确认）。

未达标时保持 `off`/`shadow`，不得用“看起来还行”替代数据。

## 补录后

把命令输出、样本数、一致率、P95、成本写进本文件，并在
`docs/rewrite-v2-implementation-plan.zh.md` 的 P1-13 / P1-15 行更新结论。
