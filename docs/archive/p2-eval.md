> **已归档（2026-11-17）**：本文档为历史档案，不再维护。现行文档见 [docs/README.md](../README.md)。

# P2 评测对照（三列）

**状态：已运行（2026-11-16，真实上游）。**

- 上游：本机 gateway `http://127.0.0.1:3425/v1`（真实模型，非 mock）
- 任务集：`v2/tests/eval/tasks.json`，33 个任务
- 模式：stream
- router 配置：`weak = auto/glm-5.3-flash`（strength 10）、`strong = auto/deepseek-v4.1-flash`（strength 80），cascade，`candidate_pool = ["weak","strong"]`
- 基线配置：同一 `weak` 模型，`candidate_pool = ["weak"]`

## 三列对照

| 配置 | 成功率 | 等价成本 (micros) | 首 token 延迟 (ms) |
|---|---|---|---|
| router | 66.7% | 1167 | 1880.0 |
| baseline | 66.7% | 897 | 4111.3 |

原始文件由命令生成（本次写入 `/tmp/tr-smoke/p2-eval-real.md`）：

```bash
MAGPIE_KEY=magpie cargo run -p tr-cli -- eval \
  --config /tmp/tr-smoke/router.toml \
  --baseline-config /tmp/tr-smoke/baseline.toml \
  --tasks tests/eval/tasks.json \
  --out /tmp/tr-smoke/p2-eval-real.md --json
```

## 解读与限制

- 成功率相同（22/33）：checker 是严格文本/工具名匹配，两个配置失败的题目相同。
- router 的首 token 更快（1880ms vs 4111ms），但等价成本更高（1167 vs 897 micros）：
  weak 模型有 1 次流式响应解码失败，触发接力到 strong；这是真实 failover 成本，
  不应被隐藏。
- 该结果只覆盖本机 gateway 的两个模型与一个价格表；不同价格/模型下三列会变化。
- 运行结束后 cost gate 仍保持 `shadow`：本表不是 P2-9 校准数据。

## 任务集（P2-7）

`v2/tests/eval/tasks.json`：33 个任务，覆盖

| 难度 | 数量 | checker |
|---|---|---|
| simple | 8 | 文本包含 |
| medium | 11 | 文本包含 |
| hard | 8 | 文本包含 |
| tool | 6 | 指定工具调用 |

checker 不用 LLM 判成功；单测锁定 ≥30 与四类覆盖。

## 成本闸校准（P2-9）

用同一次真实评测的 **同任务 weak/strong 配对成本**构造观测集（33 条）：

- `estimated_delta_micros`：按 `estimate_input_tokens` 的字符启发式 + 两个候选的输入价格估算的切换价差；
- `actual_delta_micros`：router 实际成本 − baseline 实际成本（账本 `answer_cost`，真实模型）；
- `denied=false`：本次没有成本闸拒绝决策，因此 false deny 无样本。

```bash
cargo run -p tr-cli -- eval calibrate \
  --observations /tmp/tr-smoke/observations.json --json
```

结果：

```json
{
  "samples": 33,
  "mape": 1.1688221809130457,
  "denied": 0,
  "false_denies": 0,
  "false_deny_rate": null,
  "passed": false,
  "cost_gate_mode": "shadow"
}
```

**结论：MAPE 116.9% ≫ 25%，校准不达标 → 成本闸保持 `shadow`，不得 active。**

限制：本次观测集没有“被拒绝的切换”，false deny 无法度量；下一步需要真实
gate 拒绝样本（`denied=true` + 事后成功/失败）才能完成 false deny 门槛。

