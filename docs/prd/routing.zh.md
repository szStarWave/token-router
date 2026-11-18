# PRD：路由决策引擎

> 状态：现行需求（内核已实现并冻结，2026-11-17）。本文把已冻结的内核语义表述为产品需求；**语义权威是 [内核设计文档](../archive/rewrite-v2-design.zh.md)**（§x 均指它），本文与其冲突时以它为准。
> 产品定位：这是 Token Router 的第一差异化——竞品做资源调度（哪个账号有额度），我们做决策质量（哪个模型该答这个问题）。对外口径见 [产品文档 §3](../product.zh.md)。

## 1. 背景与目标

多 agent 用户手里有强弱/贵贱不同的多个模型入口。裸 failover 只解决"能不能答"，不解决"该不该由它答"。路由引擎的目标：

1. 在成本与质量之间做出**有纪律的折中**（级联 + 判官 + 成本闸）。
2. 保持**会话连续性**（sticky/ContextHold，保 prompt cache 与 thinking signature）。
3. 升级**有证据、不震荡**（Escalation latch）。
4. 每个决定**可解释、可回放**（见 [可解释 PRD](./explainability.zh.md)）。

非目标：流式中途换模型、虚拟模型嵌套、词表/关键词进内核、用 LLM 判任务成功（§14）。

## 2. 用户与场景

| 场景 | 期望行为 |
|---|---|
| 日常编码，想省钱 | 弱档先答，质量门不过才升级强档（cascade） |
| 长会话重构 | 粘住同一候选，prompt cache 不失效（sticky/ContextHold） |
| 连续难题 | 证据确认后闩锁到强档，不来回降级（latch） |
| 某上游 429/宕机 | 自动接力下一候选，不原地重试（failover + 熔断） |
| 流式回答进行中 | 绝不中途换模型；升级下一轮生效 |

## 3. 概念模型

- **候选（Candidate）**：一个物理模型入口 `{id, tier, strength, upstream{base_url, model, credential}, window, capability, price, enabled}`。tier 是标签不是结论；credential 只是引用。
- **虚拟模型（VirtualModel）**：agent 面向的命名入口，绑定 profile + strategies + candidate_pool。请求 `model` 字段填它；`auto`/`token-router` 内置。
- **Selection vs Dispatch**：Selection=虚拟模型+thinking level，Dispatch=物理模型+clamp 后 level；两者都进账本与响应头。

## 4. 功能需求

### 4.1 计划模式（§4.4、§10.6）

| ID | 需求 | 验收 |
|---|---|---|
| R-RT-01 | `single`：只执行首选，错误按 failover 接力 | 首选成功时不产生额外 attempt |
| R-RT-02 | `cascade` 非流式：质量门失败可换候选重试 | 质量门失败后 attempt 序列含下一候选；快照可回放该决定 |
| R-RT-03 | `cascade` 流式：仅语义提交前传输错误接力，不做内容质量门 | 提交后任何错误不触发换候选 |
| R-RT-04 | `verify_after` 非流式：可同轮改强档重试一次；流式只影响下一轮 | 重试次数 ≤1 |
| R-RT-05 | `stream_escalation`：`next_turn`（默认）/`gate_before_tool`（tool_call 前缓冲可换候选）/`off` | gate_before_tool 下换候选只发生在完整 tool_call 帧之前 |
| R-RT-06 | **语义提交边界**：首个 content delta 或首个完整 tool_call 帧之后不切换候选 | 流式中途注入上游故障，已提交字节后不换候选 |

### 4.2 判官（§18.2、§6）

| ID | 需求 | 验收 |
|---|---|---|
| R-RT-10 | `[judge]` mode `off`（默认）/`shadow`（只记录不参与）/`active` | shadow 下账本有 judge 记录但 Plan 不受影响 |
| R-RT-11 | 预算：每轮 ≤1 次、每分钟 ≤30、每会话 ≤200（可配）；deadline 默认 300ms | 超预算/超时不再咨询，记 reason |
| R-RT-12 | **fail-open**：判官超时/失败/不可解析绝不进用户错误路径 | 判官服务全挂时请求仍正常完成 |
| R-RT-13 | 判官服务 `local`/`http`/`llm` 三种；llm 需结构化输出（json_schema/json_mode/tool_call） | 三种 service 各有集成测试 |
| R-RT-14 | 判官成本计入 routing_cost（与 answer_cost 分账） | 账本两列分账，cost_equiv=两者和 |
| R-RT-15 | 判官自然语言理由只进审计，不参与算术 | core 不消费理由文本 |

### 4.3 会话保持（§4.9.1）

| ID | 需求 | 验收 |
|---|---|---|
| R-RT-20 | Sticky 硬复用：Retry 复用 failed 候选、Continuation 复用 previous 候选 | 复用成功时 prompt cache/thinking signature 保持 |
| R-RT-21 | `ErrorClass ∈ {Overloaded, ContextOverflow}` 跳过 StickyFailed 回退 previous；纯 enum 判断 | 不匹配错误文本；两类错误各有用例 |
| R-RT-22 | usable 检查（在池/enabled/凭据/模态/window/健康）任一不满足：不阻塞，回退 ContextHold/Plan 并记 `STICKY_*` reason | 每种不满足有对应 reason code |
| R-RT-23 | ContextHold 软亲和：User 轮上一成功候选置 preference 首位，可被改选；生效需同虚拟模型/候选组/plan_revision/profile/入口/身份 + TTL 1h | 条件任一变化即失效；改价格（plan_revision 不变）不失效 hold |
| R-RT-24 | 优先级：**Escalation latch 与 CompetenceGuard 低分保护优先于 Sticky/ContextHold** | latch 生效时 sticky 让位，快照可解释 |

### 4.4 Escalation latch（§6.4）

| ID | 需求 | 验收 |
|---|---|---|
| R-RT-30 | 同 Category + 新证据确认 2 次 → Latched；证据指纹由调用点计算，不信任模型自报 | 伪造自报证据不触发 latch |
| R-RT-31 | latched 直达强档、不调判官（仅 ReleaseCheck 调） | latch 期间判官调用数不增 |
| R-RT-32 | 强档 ≥3 次且连续 decline → ReleasePending → 释放确认进 weak_cooldown（1 轮）；`strong_max_calls=min×4` 强制回落 | 硬上限用例：判官 off 时也能回落 |
| R-RT-33 | ReleasePending 超时/不可解析保持 latch（保守）；confidence<0.60 按 Uncertain | 保守方向有用例 |
| R-RT-34 | 作用域 `(session_key, task_id)`；流式默认下一轮生效 | 跨会话不串 latch |

### 4.5 Failover 与熔断（§6.6、§18.4）

| ID | 需求 | 验收 |
|---|---|---|
| R-RT-40 | Failover 只读健康做过滤与接力排序；写健康归 upstream；同一上游不原地重试 | 单请求 ≤6 attempts |
| R-RT-41 | 熔断双 key（binding/credential）；凭据/配额失败先换 key 再换候选 | 双账号场景先轮换 key |
| R-RT-42 | 冷却默认 binding 15s/credential 60s，退避 2/4/8/16/30s；到期半开探测（原子认领，30s 窗口），成功清零失败续冷 | 半开并发只放一个探测 |
| R-RT-43 | 软失败（5xx/超时）request-local 不跨请求污染；仅凭据/配额跨请求共享 | 单请求 5xx 不影响下一请求候选序 |
| R-RT-44 | 计数权重 RateLimit +0.5、Overloaded +2；Auth/Quota/InvalidRequest/ContextOverflow 不计数 | 权重表单测 |
| R-RT-45 | 全冷却 → 503 + Retry-After（`CANDIDATES_COOLING_DOWN`） | 响应头与 reason code 正确 |

### 4.6 决策流水线与扩展（§5、§6.3、§4.3）

| ID | 需求 | 验收 |
|---|---|---|
| R-RT-50 | 合并代数顺序固定：Filter 并集→Baseline→Classify 重排→Escalation Pin→Mode 仲裁（**Escalation > CompetenceGuard > Classify > Baseline**）→Failover→CacheLoss 取最大→Plan 校验 | 冲突场景仲裁结果有 golden 用例 |
| R-RT-51 | 策略 panic/非法 Plan 回退 Baseline 记 `CORE_FALLBACK` | 注入 panic 策略，请求不失败 |
| R-RT-52 | 新策略走 Strategy trait（draft/refine/on_outcome 全只读，状态经 StateUpdate 原子提交）；新信号走 SignalExtractor 写 SignalBag | 外部 crate 注册策略/信号不改 core（check-deps + diff=0 门） |
| R-RT-53 | 禁止虚拟→虚拟嵌套；Plan 必须非空、window 过滤、failover ⊆ preference | 非法配置 validate 拒绝 |

## 5. 非功能需求

- **确定性**：`core::decide(&snapshot)` 两次回放逐字段一致（G2）；core 纯函数零 IO。
- **延迟**：判官 deadline 内不阻塞主路径；路由自身开销不引入网络调用。
- **冻结**：本 PRD 全部需求对应已冻结内核；产品层迭代 `core/decide/proto` diff=0。语义变更必须先改设计文档。

## 6. 配置与界面入口

- 配置：`[profiles.*]`（mode/stream_escalation/strength_order/权重/unknown_price_policy）、`[[virtual_models]]`（profile/strategies/pool）、`[judge]`、`[cost_gate]`。见 [配置参考](../guide/configuration.zh.md)。
- 界面：Models/VirtualModels 页管理候选与池；判官/成本闸在 Settings。路由行为的可视化见 [可解释 PRD](./explainability.zh.md)。

## 7. 现状与依赖

- 全部需求已实现（S0–P3 交付，316+ 测试含回放一致性）；判官默认 off，`active` 需用户配置判官服务。
- 待兑现的体验依赖：P5-C 路由时间线（让仲裁过程可见）、P5-B 配额信号（reset-aware 策略经 R-RT-52 扩展点接入，见 [路线图](../roadmap.zh.md)）。