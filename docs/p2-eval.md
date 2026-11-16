# P2 评测对照（三列）

**状态：未运行 —— 等待真实上游。**

本文件是 P2-8 的产物占位与执行说明。按计划，mock 上游的对照没有意义，
因此这里不预填任何成功率/成本/延迟数字。

## 运行方式（已就绪）

```bash
# router 配置 + 单模型基线配置，同一任务集各跑一遍
cargo run -p tr-cli -- eval \
  --config example/config.minimal.toml \
  --baseline-config <single-model.toml> \
  --tasks tests/eval/tasks.json \
  --out docs/p2-eval.md
```

- 默认流式：首 token 延迟 = 上游第一个流式事件的到达时间；
- 等价成本 = 账本 `answer_cost + routing_cost`，流式结束后按 `plan_id` 回查；
- 未提供 `--baseline-config` 时，命令会明确提示“不可比较成本/质量收益”（设计 §11.5）。

## 任务集（P2-7）

`v2/tests/eval/tasks.json`：33 个任务，覆盖

| 难度 | 数量 | checker |
|---|---|---|
| simple | 8 | 文本包含 |
| medium | 11 | 文本包含 |
| hard | 8 | 文本包含 |
| tool | 6 | 指定工具调用 |

checker 不用 LLM 判成功；单测锁定 ≥30 与四类覆盖。

## 表结构（运行后填写）

| 配置 | 成功率 | 等价成本 (micros) | 首 token 延迟 (ms) |
|---|---|---|---|
| router | 待运行 | 待运行 | 待运行 |
| baseline | 待运行 | 待运行 | 待运行 |

## 成本闸校准（P2-9）

校准不依赖上述任务集，而依赖带“估算 delta / 实际 delta / 是否误拒”的观测集：

```bash
cargo run -p tr-cli -- eval calibrate --observations <observations.json> --json
```

- 门槛：MAPE ≤ 25%、false deny ≤ 5%、样本数 ≥ 30；
- 不达标时输出 `cost_gate_mode = "shadow"`，成本闸不得 active；
- 观测集字段：`estimated_delta_micros`、`actual_delta_micros`、`denied`、`would_have_succeeded`。

## 数据到位后

把实际运行的表、任务集版本、所用配置 digest、上游模型与价格写入本文件，
并更新计划文档 P2-8 / P2-9 行。
