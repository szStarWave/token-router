# P1 shadow 报告（真实流量）

**状态：已产出（2026-11-16）。**

- 样本：**556 条** shadow 记录（≥500）
- 上游：本机 gateway `http://127.0.0.1:3425/v1`（真实模型）
- 判官：`auto/deepseek-v4.1-flash`，`[judge] mode = "shadow"`
- 路由：`weak = auto/glm-5.3-flash`、`strong = auto/deepseek-v4.1-flash`，cascade，
  虚拟模型启用 `competence_guard`（每轮向判官询问上一阶段胜任度）
- 生成方式：把 `tests/eval/tasks.json` 的 10 个简单/中等任务作为 prompt 集，
  通过 `tr-cli eval --no-stream` 重复回放，使同一 session 产生 previous，
  从而触发 CompetenceGuard 的判官咨询。

## 门禁输出

```bash
tr-cli shadow-report --db shadow.db --min-samples 500 --require --json
```

```json
{
  "total": 556,
  "agreed": 556,
  "disagreed": 0,
  "unknown": 0,
  "agreement_rate": 1.0,
  "avg_latency_ms": 308.3,
  "p95_latency_ms": 315.0,
  "total_cost_micros": 0,
  "min_samples": 500,
  "enough_samples": true
}
```

## D-1 决策

门槛要求：样本 ≥500、判官 P95 ≤300ms、escalation precision ≥0.7。

| 项 | 结果 | 结论 |
|---|---|---|
| 样本数 | 556 | 达标 |
| P95 | 315ms | **不达标**（>300ms） |
| 一致率 | 100% | 仅作诊断；本样本是 10 个任务的重复回放，内核始终选弱档，判官回答 Decline，一致率高是构造使然 |

**决策：judge 保持 `off`/`shadow`，不切 `active`。** 需要更多样化的真实任务与
更低的判官 P95 后再评估。

## 限制

- 这是“eval 任务回放”式 dogfood，不是人类日常会话；prompt 多样性有限。
- `total_cost_micros = 0`：该 gateway 的判官响应未返回 `usage`，无法折算成本；
  不把缺失当 0 成本对外声明，只说明“未报告”。
- `tr-cli doctor` 的 judge 自检为 Failed（固定问题返回形状不符），但 shadow
  调用本身成功并落表；自检失败说明判官服务契约尚未完全对齐 gateway 的响应形状。
