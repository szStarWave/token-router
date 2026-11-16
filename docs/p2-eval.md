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

## 成本闸校准（P2-9，仍未完成）

校准不依赖上述任务集，而依赖带“估算 delta / 实际 delta / 是否误拒”的观测集：

```bash
cargo run -p tr-cli -- eval calibrate --observations <observations.json> --json
```

- 门槛：MAPE ≤ 25%、false deny ≤ 5%、样本数 ≥ 30；
- 不达标时输出 `cost_gate_mode = "shadow"`，成本闸不得 active；
- 观测集字段：`estimated_delta_micros`、`actual_delta_micros`、`denied`、`would_have_succeeded`；
- 本次评测没有逐切换的“估算 vs 实际”记录，因此不能替代校准观测集。
