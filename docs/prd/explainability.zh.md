# PRD：决策可解释与回放

> 状态：A 部分现行（已实现）；B 部分规划（P5-C，见 [路线图](../roadmap.zh.md)）。2026-11-17。
> 语义权威：[内核设计文档](../archive/rewrite-v2-design.zh.md) §4.7、§9.2、§8.3、§11.4。
> 产品定位：第二差异化——竞品的 trace 是事后展示，我们的可解释是**契约**：每个决定可确定性回放。

## 1. 背景与目标

用户把模型选择交给网关后，必须能回答三个问题：**做了什么决定？凭什么？当时是什么配置？** 否则网关就是黑箱，出问题无法定位，成本无法归因。

目标：
1. 每个请求留存完整决策输入与过程，且**可重演验证**（不是"看起来合理"，是"逐字段一致"）。
2. 成本分账到"回答本身"与"路由开销"，归因到 agent/模型/账号/会话。
3. （P5-C）把账本数据变成非技术用户可读的时间线与图表。

## 2. 用户与场景

| 场景 | 期望 |
|---|---|
| "这个请求为什么用了贵模型？" | 详情页看到级联质量门失败/判官 verdict/latch 状态与 reason |
| "升级后行为变了，是 bug 吗？" | 用旧快照回放，对照当时配置 digest，区分配置变化与行为变化 |
| "这个月钱花在哪了？" | 按 agent/模型/账号/会话出账，判官开销单独可见 |
| "网关说的统计可信吗？" | 统计与直接查账本 SQL 同源，可对照 |

## 3. A 部分：已实现需求

### 3.1 决策快照与回放（§4.7）

| ID | 需求 | 验收 |
|---|---|---|
| R-EX-01 | DecisionSnapshot 含全部决策输入：selection/virtual_model/reason/previous/failed/context_hold/router_state/时钟/健康/verdicts | 快照字段覆盖审计清单 |
| R-EX-02 | 确定性契约：BTreeMap 稳定排序、固定哈希（SHA-256/BLAKE3）与浮点精度；投影排除 latency_ms/request_id/created_at | `core::decide(&snapshot)` 两次回放逐字段一致（G2 测试） |
| R-EX-03 | `tr-cli replay` 只读回放快照（唯一直调 core 的例外） | 回放输出与原始决策一致 |
| R-EX-04 | `config_digest`（规范化配置 sha256）与 profile_revision/plan_revision 随快照/账本落库 | 任一请求可查到"当时是哪份配置" |

### 3.2 账本（§9.2、§8.3）

| ID | 需求 | 验收 |
|---|---|---|
| R-EX-10 | 逐请求记录：selection/dispatch、plan_mode、preference/failover、attempts_json（逐 attempt 候选/ErrorClass/耗时/是否提交）、switches、reason_codes、outcome、tokens、ttft/duration | 字段齐全；outcome ∈ Success/UpstreamError/QualityFail/Cancelled/Rejected |
| R-EX-11 | judge_json/contributions_json/authority_json 留痕（migration 2/3/4） | 判官与各策略贡献可查 |
| R-EX-12 | **成本分账**：answer_cost_micros 与 routing_cost_micros 分离（判官成本入 routing），cost_equiv=两者和；金额整数 micros 禁 REAL | 分账列单测；无浮点金额 |
| R-EX-13 | 保留期：账本 30 天、breaker 24h、protocol_state 7 天 | 过期清理用例 |
| R-EX-14 | 统计只聚合账本，与直接 SQL 同源 | admin_query 单测对照 |

### 3.3 查询界面（P4 已交付）

| ID | 需求 | 验收 |
|---|---|---|
| R-EX-20 | Requests 页按会话/模型/outcome/协议/原因码过滤 | 各过滤维度 e2e |
| R-EX-21 | 详情页展示 plan/judge/contributions/authority 结构化数据 | 与账本字段一致 |
| R-EX-22 | 时间窗查询最大 90 天（超限 413 window_too_large），游标分页 | 契约用例 |
| R-EX-23 | UI 不直读 SQLite/日志，全部经 `/v2/admin` 代理 | 白名单测试 |

## 4. B 部分：规划需求（P5-C）

> 数据全部已在账本，B 部分是呈现与聚合增量，不动内核。验收基准见路线图 P5-C。

| ID | 需求 | 验收 |
|---|---|---|
| R-EX-30 | **路由时间线**：Requests 详情页可视化 Selection→Strategies→Plan→Dispatch→(Failover/Judge) 每步 reason/耗时 | 时间线与账本 attempts 逐条一致；latch/sticky 让位等仲裁可见 |
| R-EX-31 | 用量聚合 `GET /v2/admin/usage?by={agent,model,candidate,account,session}` | 四维+会话出账，与 SQL 同源 |
| R-EX-32 | 热力图（小时×星期，色=请求数；参考 OmniRoute 口径） | 七档时间范围；数据与账本一致 |
| R-EX-33 | 缓存命中分解，cache write 区分 Anthropic 5m/1h 两档 | 价格表加档后分账正确 |
| R-EX-34 | OTLP 导出（可选 Langfuse generation 映射）；凭据头永远排除 | 导出含成本属性；无明文 key |
| R-EX-35 | SSE 实时事件驱动时间线刷新（复用既有 events 机制） | Last-Event-ID 续传不丢事件 |

## 5. 非功能需求

- 账本写入单事务（Outcome 与状态同提交），不产生半记录。
- 查询 p95 目标：10 万行账本下可用（索引已就位，压测为 GA 前置项）。
- 脱敏：账本与快照不含明文 key/正文（shadow 记录 30 天脱敏）。

## 6. 边界

- 不做：用 LLM 判任务成功（评测用自带 checker）；判官自然语言理由参与算术；跨实例账本合并。
- 回放是**决策回放**，不是上游响应回放（不重发真实请求）。

## 7. 现状与依赖

- A 部分全部已实现；B 部分依赖 P5-C 排期，R-EX-33 依赖价格表加档（contracts 只增不改规则）。
- 对外宣传注意：回放契约是独有深度，但需时间线 UI 落地后才对普通用户可感知。