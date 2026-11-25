# 模型路由行为（现行实现口径）

> 状态：现行（2026-11-24）。事实基准：`v2/` 代码。本文回答"一次请求进来，实际发生了什么"；
> 需求编号与验收标准见 [PRD：路由](../prd/routing.zh.md)，内核语义权威仍是
> [内核设计文档](../archive/rewrite-v2-design.zh.md)。三者冲突时以代码与内核文档为准，本文随之更新。

本文描述的是**已实现并默认生效**的行为，不包含路线图项。涉及冻结内核（`core/decide/proto`）的语义只做引用，不在此重述论证。

## 1. 全链路

```text
agent PTY ──POST──> /v1/chat/completions | /v1/responses | /v1/messages
                     │
                     ├─ 入站鉴权（inference key；auth_enabled=false 时跳过）
                     ├─ 网关 key 限额（token/成本，窗口 + 模型子集）→ 超限 429 + Retry-After
                     ├─ 协议解码 → CanonicalRequest；未知顶层字段进 passthrough，原样回传上游
                     ├─ 协议形状修复（工具调用配对、空消息清理），不改语义
                     ├─ 推理档位：x-token-router-thinking-level > reasoning_effort
                     └─ 请求原因：x-token-router-request-reason > 推断
                     │
        Pipeline::prepare（agent）
                     ├─ 会话键（x-token-router-session-id + 消息前缀；内容过稀时降级并告警一次）
                     ├─ 虚拟模型解析（见 §2）
                     ├─ 信任档位 → profile（未知档位回退默认 profile 并记 reason）
                     ├─ 组级关闭思考：disable_thinking → 档位钉 off（见 §7.3）
                     ├─ 候选池 + 过滤（enabled / 模态 / window / 健康；凭据在执行期校验）
                     ├─ 信号：难度/硬信号/上下文、配额窗口、近期 TTFT
                     ├─ core::decide：Baseline → 各策略 draft/refine → 仲裁 → Plan
                     ├─ 全局判官（[judge] off/shadow/active）
                     ├─ 组级判断（rules / 能力模式，llm / jev）
                     ├─ Sticky / ContextHold（组判断已决策时跳过）
                     ├─ 成本闸（off/shadow/active）
                     └─ 成员直选（group/member）把该候选提到队首
                     │
        执行（agent + upstream）
                     ├─ 按 Plan.preference 逐个 attempt；可重试错误换下一候选
                     ├─ 熔断检查 / 半开探测（probe_allowed）
                     ├─ 上游 wire 按候选 provider 选择（Chat / Responses / Anthropic）
                     ├─ 显式 off：剥离 reasoning + 已知厂商关闭开关（§7.3）
                     └─ 流式：首个事件到达才提交响应头，之后绝不换模型
                     │
        收尾
                     ├─ 账本（attempts / 策略贡献 / judge trace / 成本分账 / judge_calls）
                     ├─ 响应头（x-token-router-*，§8）
                     └─ 会话状态（上次成功派发、ContextHold、熔断健康、策略状态）
```

非流式与流式共用同一份 `prepare` 与 Plan，差别只在执行与提交边界。

## 2. 虚拟模型解析优先级

请求的 `model` 字段按以下顺序解析（`crates/agent/src/pipeline.rs`）：

| 优先级 | 条件 | 结果 |
|---|---|---|
| 1 | `model` 命中启用中的虚拟模型 id | 直接用 |
| 2 | `model` 形如 `组id/成员物理模型名` 或 `组id/候选id` | 用该组，并把该成员候选提到 Plan 队首 |
| 3 | 未指定 `model`，会话有上次选择且仍启用 | 沿上次选择 |
| 4 | 未指定 `model`，无历史或已失效 | `default_virtual_model`（配置缺省时取第一个启用组） |

组不存在或已停用 → 400 `MODEL_NOT_FOUND`，不会静默换一个模型顶上。会话上次选择失效时会回退默认组并记 `VIRTUAL_MODEL_MISSING`。

`/v1/models` 只列启用中的组；组配置 `list_members = true` 时额外列出 `组/成员模型`，客户端可直接选成员（仍按该组路由与账号）。

## 3. 候选池、过滤与信号

候选来自供应商编译（`[[candidates]]`），组只引用候选 id。进入决策前过滤：

- `enabled = false` 的候选不可选；
- 模态不匹配（请求含图片/音频而候选未声明）不可选；
- 估算输入 token 超过候选 `window` 不可选；
- 凭据缺失或不可解析的候选在执行期失败（`Auth` 类，不阻塞请求，换下一候选；同模型的另一个账号是另一个候选）；
- 冷却中的候选被内核 `Failover` 过滤；冷却到期后由执行层做半开探测（30s 窗口只放一个请求）；

信号（`SignalBag`，只读输入，不反向控制内核）：

| 信号 | 来源 | 用途 |
|---|---|---|
| 难度 / 硬信号 | 内核从请求内容与历史提取 | Classify / Escalation / cost gate |
| 上下文窗口提示 | 组 `context_window_mode`（min/max/custom） | 选路信号，不是硬限制 |
| 配额窗口 | 上游 429 / 限流头学习（无标准头时本地估算并标 `estimated`） | `quota-reset` 策略 |
| 近期 TTFT | 账本 attempts 平均值 | `fast` 策略（需显式启用） |

## 4. 计划生成与策略

`core::decide` 是纯函数：同一份快照两次回放逐字段一致。默认策略集（组未写 `strategies` 时）：

- `Baseline` → `Failover` → `Escalation` → `Classify`，外加 `quota-reset`（v3 P5-B，默认启用）；
- 组显式写出 `strategies` 时按列表注册；写空列表等价默认集；
- `fast` 不在默认集，必须在组里显式列出（"快"与"强/省"是竞争目标）。

仲裁顺序（冲突时）：**Escalation > CompetenceGuard > Classify > Baseline**，随后 Failover、CacheLoss、Plan 校验。判官结论在 `active` 模式下覆盖策略 draft；`shadow` 只记录不参与。

`quota-reset`：排除本窗口已耗尽的候选（`QUOTA_DEPLETED`）；可配置为按重置时间升序优先（先到期先用）。配额 ≠ 健康：冷却仍归 Failover。

成本闸（`[cost_gate]`）：`off` 不计算；`shadow` 只写 `expected_cache_loss` 与结论；`active` 在"切换收益不足以抵消缓存损失"时把上一成功候选放回队首。组判断已决策时成本闸不再回搬顺序。

## 5. 判官：全局与组级

### 5.1 全局判官（`[judge]`）

- 模式：`off`（默认）/ `shadow` / `active`；服务类型 `local` / `http` / `llm` / `jev`。
- 问题由内核策略产生（升级确认、降级释放、能力评分），文本固定在 `crates/core/src/strategy/*.rs`。
- 预算：每轮 / 每分钟 / 每会话 + deadline（默认 300ms），超预算不咨询。
- **fail-open**：超时、失败、不可解析都不进用户错误路径，请求按原 Plan 继续。
- LLM 服务用统一 system 提示 + 三选一结构化输出（`json_schema` / `json_mode`（默认）/ `tool_call`），答案必须匹配选项集合（`crates/decide/src/llm.rs`）。

### 5.2 组级判断（虚拟模型自己的判断模型）

组级判断在 Plan 生成之后、Sticky/ContextHold 之前执行，只回答"本组内优先谁"：

| 组配置 | 模式 | 问题 |
|---|---|---|
| 有 `rules` | 规则模式 | 按配置顺序列出规则，答案是首个命中的 rule id 或 `none` |
| 无 `rules` | 能力模式 | 列出启用候选的 tier/strength/window/模态/档位/tags/价格，答案是候选 id 或 `none` |

发送内容（隐私边界）：

- 只发**最后一条 user 消息的文本块**（截断 4000 字符）；system / tool 内容、历史消息一律不进判官。
- LLM（Chat Completions）：system 提示固定为"routing judge + 不可信数据 + 不得改选项 + 无法判断答 uncertain + 只回 JSON"；user 消息由 `TASK` + `QUESTION` + `ALLOWED VALUES` 组成。
- JEV（SystemOne `/v1/systemone`，endpoint 必须是完整路径；组工作台选中决策模型时自动归一化）：没有 messages；`state.task` 是最后一条 user 文本，`questions.route` 是 `type = choice`，`instructions` 是同一个问题文本，`criteria` 是 `{选项id: 标签}`。
- LLM 结构化输出：`json_schema`（严格 schema）/ `json_mode`（`json_object`，默认）/ `tool_call`（强制 `answer_judge_question` 工具），三者共用一个 JSON 形状：`{"value": …, "confidence": 0..1, "reason": "…"}`，`value` 必须是允许集合内的值或保留答案 `uncertain`。

判定与失败语义：

- `none` 是合法答案，表示"没有匹配项"，不改顺序；
- 超时 / 失败 / 非法答案 / 选择不存在或已停用的候选 → fail-open：保持原 Plan，写 `judge_calls` 审计行，不向客户端报错；
- `active` 一旦决策，后续 Sticky / ContextHold / 成本闸不再把顺序拉回旧候选；
- 判官成本计入账本 `routing_cost`，与回答成本分账；
- 组判断不支持 `shadow`（配置校验拒绝）。

## 6. 会话保持

### 6.1 Sticky（硬复用）

| 请求原因 | 行为 |
|---|---|
| `Retry` | 优先复用上次失败的候选（`StickyFailed`）；`Overloaded` / `ContextOverflow` 不复用，避免必然失败 |
| `Continuation` | 优先复用上次成功的候选（`StickyPrevious`） |
| `Direct` | 默认不粘（`direct_opts_into_sticky = false`） |
| 其他（User 轮） | 走 ContextHold 软亲和 |

复用前做 usable 检查（仍在池内、启用、模态匹配、window 够、健康可用）；任一不满足不阻塞请求，回退 ContextHold / Plan 并记对应 `STICKY_*` reason（`STICKY_TARGET_UNAVAILABLE` / `STICKY_INPUT_UNSUPPORTED` / `STICKY_WINDOW_EXCEEDED`）。

### 6.2 ContextHold（软亲和）

User 轮把上次成功候选排到 preference 首位，可被 Plan 改选。生效条件全部满足：

- 同一虚拟模型、同一组摘要（`group_digest`：候选池 + 判官配置 + 关闭思考）、同一 `plan_revision`、同一 profile、同一入口协议、同一身份（会话键）；
- 未超过 TTL（1 小时）。

任一条件变化即失效（记 `CONTEXT_HOLD_INVALIDATED`）。Escalation latch 与 CompetenceGuard 低分保护优先于 Sticky/ContextHold；组级判断已决策时同样跳过。

## 7. 执行：failover、熔断与上游 wire

### 7.1 尝试循环

按 `Plan.preference` 顺序逐个尝试，每个候选最多一次（不会对同一候选原地重试；同模型不同账号是两个候选）。

| 错误类 | 是否换候选 | 熔断计数（0.5 单位） |
|---|---|---|
| Transport / Timeout / Protocol / Unknown | 是 | 2（即 +1） |
| RateLimit | 是 | 1（即 +0.5） |
| Overloaded | 是 | 4（即 +2） |
| Auth / Quota | 是 | 0 |
| InvalidRequest / ContextOverflow | 否，直接返回 | 0 |

### 7.2 熔断

- 状态按候选持久化；候选 id 含账号指纹，因此同一模型的不同账号各自计数、互不牵连。
- 连续失败累计 ≥ 3 → 打开；冷却时长优先取上游 `Retry-After`，否则按类别：Overloaded 30s、RateLimit 60s、其余 15s。
- 冷却结束后的 30s 窗口内只放行一个半开探测（原子认领）；探测成功清零，失败继续冷却。
- 冷却中的候选被 `Failover` 过滤（`CANDIDATES_COOLING_DOWN`），失败后按 preference 接力。
- 全部候选不可用 → 503 `no_eligible_candidate`，错误体带完整 reason codes（含 `CANDIDATES_COOLING_DOWN`）；当前不写 `Retry-After`（见 §10）。

### 7.3 上游 wire

每个候选的 `provider` 决定 wire 协议与端点（`crates/upstream/src/http.rs`）：

| `provider` | 请求路径 | 鉴权 |
|---|---|---|
| `openai`（OpenAI 兼容） | `{base_url}/chat/completions` | `Authorization: Bearer` |
| `responses` | `{base_url}/responses` | `Authorization: Bearer` |
| `anthropic` | `{base_url}/v1/messages`（base 已带 `/v1` 只补 `/messages`） | `x-api-key` + `anthropic-version` |

其他值显式失败（`UnsupportedProvider`），不猜协议。每个账号可带代理与静态自定义 header；凭据在 Rust 侧解析，配置与日志只见引用。

推理档位在上游收口：

- 候选声明了 `thinking_levels` 且请求档位不在其中 → dispatch 档位记 `off`，不发送 reasoning 参数；
- 声明为空 = 未声明，原样透传（不猜能力）；
- 显式 `off` / `none` / `disabled`（组级 `disable_thinking`、候选级 `disable_thinking`、header 或 `reasoning_effort`）→ 无条件剥离 reasoning，并对已知厂商附加关闭开关：

| 厂商 host | 附加字段 |
|---|---|
| 硅基流动、DashScope | `enable_thinking: false` |
| 智谱 GLM / Z.ai | `thinking: {"type": "disabled"}` |
| OpenRouter | `reasoning: {"enabled": false}` |

未知厂商只剥离参数，不发送未确认字段；Anthropic / Responses 不携带 thinking 即为关闭。

输出上限：候选级 `max_tokens`（编译自目录「最大输出」）在发往上游前做截断——客户端值更小则保留，未给则填上限；触发截断时账本与响应头的 reason codes 带 `OUTPUT_CLAMPED`。

### 7.4 流式提交边界

- 上游连接失败或首个事件到达前出错 → 不算提交，可换下一候选（响应头尚未发出）；
- 首个事件到达 → 立即提交响应头，此后任何错误都不换模型，只把错误作为流结束；
- 配置里的 `stream_escalation`：`next_turn`（默认）与 `off` 行为一致（升级只在下一轮生效）；`gate_before_tool` 目前只有内核判定函数 `may_switch_after_commit`，执行层未接线，实际等同 `next_turn`（见 §10）。

## 8. 账本与响应头

每次请求落一条账本：`request_id`、`plan_id`、`session_key`、Selection 与 Dispatch（含 clamp 后档位）、`dispatch_source`、reason codes、逐次 attempt（错误类/耗时/是否提交/用量）、逐策略贡献（`contributions`）、judge trace、`preference` / `failover` 列表、答案成本与路由成本分账、`config_digest`、授权身份。判官每次调用另落 `judge_calls`。

账本**不保存完整的 `DecisionSnapshot`**；离线确定性回放用 `tr-cli replay <snapshot.json>` 读取独立快照文件（`v2/crates/cli/src/lib.rs`）。"当时用的是哪份配置"由 `config_digest` + 配置审计链回答。

响应头：

| header | 含义 |
|---|---|
| `x-token-router-selected-model` | 客户端选择的虚拟模型 |
| `x-token-router-served-model` | 实际服务的上游物理模型 |
| `x-token-router-thinking-level` | clamp 后的有效档位（`off` 表示未启用推理） |
| `x-token-router-dispatch-source` | `strategy` / `sticky_previous` / `sticky_failed` / `context_hold` / `failover` / `fallback` |
| `x-token-router-plan-id` | 决策 id，可查账本 |
| `x-token-router-served-slot` | `候选id#序号` |
| `x-token-router-mode` | `single` / `cascade` / `verify_after` |
| `x-token-router-reason-codes` | 逗号分隔的理由码 |
| `x-token-router-request-reason` | 请求原因 |
| `x-token-router-judge` | 目前恒为 `off`（见 §10） |

## 9. 失败语义

| 场景 | 客户端看到 |
|---|---|
| 虚拟模型/成员不存在或停用 | 400 `MODEL_NOT_FOUND`（message 里带请求的模型名） |
| 网关 key 限额超限 | 429 + `Retry-After`（秒），错误体带窗口与已用/上限 |
| 全部候选不可用 | 503 `no_eligible_candidate` + reason codes（`CANDIDATES_COOLING_DOWN` 等） |
| 上游错误不可重试（如 InvalidRequest） | 该上游的状态码与错误体，不继续换候选 |
| 全部候选失败 | 最后一次失败的状态码（缺省 502）+ 账本可查完整 attempt 链 |
| 判官失败/超时 | 不影响响应，只影响路由质量 |

## 10. 已知偏差与边界

- `x-token-router-judge` 响应头目前固定为 `off`（字段未接判官模式）；判官实际行为看账本 `judge` 字段与 `judge_calls`。
- 全候选不可用（503 `no_eligible_candidate`）时不携带 `Retry-After`；目前只有网关 key 限额的 429 会带。上游 429 的 `Retry-After` 也不转发给客户端。
- [PRD：路由](../prd/routing.zh.md) 中 R-RT-41/43 描述的"binding/credential 双 key、退避 2/4/8/16/30s"与现行实现不同：熔断状态按**候选**持久化（候选 id 已含账号指纹），冷却固定为 Retry-After 优先、否则 15/30/60s，没有指数退避。本文以代码为准。
- 账本不保存完整 `DecisionSnapshot`（见 §8）；"完整快照逐字段回放"目前依赖单独导出的快照文件，不是账本自带能力。
- `stream_escalation = gate_before_tool` 的执行层接线未完成：当前所有模式都只在首个事件前换候选（§7.4）。
- LLM 判官的 `structured_output = "json_schema"` 目前把 `value` 写成 type 数组（如 `["boolean","string"]`），不符合 OpenAI strict 的"单 type / anyOf"要求；默认的 `json_mode` 不受影响。需要严格模式时应先改 `crates/decide/src/llm.rs` 的 `answer_schema`。
- 组判断不支持 `shadow`；组判断与全局判官不会同时在同一请求里各跑一次（组判断 `active` 时跳过全局判官阶段）。
- `list_members = true` 时成员直选会把该候选提到队首，但仍受组内 failover 与熔断约束：它不是一个独立入口。
- 余额/订阅查询（`/v2/admin/provider-usage`）与路由无关，不参与选路；配额窗口只来自 429 / 限流头或本地估算。
- 本文不含未实现项（如意图路由、订阅账号驱动）；这些见 [产品文档 §10](../product.zh.md)。

## 11. 代码索引

| 行为 | 位置 |
|---|---|
| 入口、鉴权、限额、协议编解码 | `v2/crates/server/src/lib.rs` |
| 虚拟模型解析、判官编排、Sticky、成本闸、尝试循环 | `v2/crates/agent/src/pipeline.rs` |
| 组级判断问题与失败语义 | `v2/crates/agent/src/group_judge.rs` |
| 全局判官预算与后端 | `v2/crates/agent/src/judge.rs` |
| LLM 判官提示词与结构化输出 | `v2/crates/decide/src/llm.rs` |
| JEV（SystemOne）判官请求形状 | `v2/crates/decide/src/system_one.rs` |
| 策略与仲裁 | `v2/crates/core/src/`、`v2/crates/agent/src/strategy_*.rs` |
| failover / 熔断 | `v2/crates/agent/src/dispatch.rs`、`store/health.rs` |
| 上游 wire / 端点 / 鉴权 / 关闭思考 | `v2/crates/upstream/src/http.rs`、`stream.rs` |
| 账本与响应头 | `v2/crates/agent/src/pipeline.rs`、`v2/crates/server/src/lib.rs` |
| 配置字段 | `v2/crates/agent/src/config.rs`、[配置参考](../guide/configuration.zh.md) |
