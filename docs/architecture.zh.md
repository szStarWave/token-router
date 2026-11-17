# Token Router v2 架构精编

> 状态：现行（2026-11-17）。本文是面向读者的精编版。
> **内核语义的权威来源**是 [v2 内核设计（归档）](./archive/rewrite-v2-design.zh.md)（下称"设计文档"，章节号 §x 均指它）；本文与其冲突时以设计文档为准。
> 管理面/桌面端（P4 增量）的事实基准是 [管理 API 契约](./contracts/admin-api.zh.md) 与 [P4 交付报告（归档）](./archive/p4-delivery-report.zh.md)。

## 1. 总览

v2 是一个本地 LLM 网关：三种入口协议归一化为 canonical 表示，经纯函数路由内核产出**物理候选计划**，由上游层执行并接力，全过程单事务写入 SQLite 账本。管理面与桌面端在内核外围，内核不知道它们的存在。

```
agent 请求 ──► proto（协议编解码）──► agent（编排/会话/账本）
                                        │  纯函数调用
                                        ▼
                                   core（信号+策略+Plan 合并）
                                        │  判官咨询（可选）
                                        ▼
                                   decide（DecisionModel）
                                        │
                                        ▼
                                   upstream（HTTP/流式/接力/熔断）
```

## 2. crate 边界与门禁

workspace 位于 `v2/`，12 个 crate（包名统一 `tr-` 前缀）：

| crate | 职责 | 关键约束 |
|---|---|---|
| `contracts` | 纯类型：Canonical*、Signals、Plan、Verdict、Cost、ReasonCode、admin DTO | 只依赖 serde/thiserror；ts-rs 导出 TS 绑定 |
| `proto` | 三协议编解码、compat 修复、流式状态机 | 只依赖 contracts |
| `core` | 信号抽取 + 策略 + Plan 合并 | **纯函数、零 async、零 IO**；禁 tokio/reqwest/rusqlite/axum |
| `decide` | DecisionModel trait + LLM/HTTP 判官 + 预算/deadline | 唯一允许调判官的 crate |
| `agent` | 会话状态、latch、账本、shadow、编排、事务提交 | 内核唯一编排点 |
| `upstream` | provider 客户端、流式、接力 | 唯一 HTTP 边界；ErrorClass 归一化只在此 |
| `server` | axum 组装、入站鉴权、模型端点、健康 | 不直调 core/decide，一律经 agent |
| `control` | `/v2/admin` 管理 API：配置/诊断/账本查询/token/审批 | 内核无感知（P4 增量） |
| `cli` | `tr-cli`：check-config/doctor/replay/eval/admin 等 | `replay` 是唯一直调 `core::decide` 的例外（只读） |
| `client` | 管理 API 的 Rust 客户端（P4） | 供 CLI/桌面壳使用 |
| `onboard` | agent 配置发现与精确合并、WSL（P4） | 幂等、保留用户配置 |
| `mock-upstream` | 测试用上游 | 仅测试 |

依赖单向，`cargo xtask check-deps` 按白名单强制。硬门禁（每次提交）：

- **`core` / `decide` / `proto` diff = 0**：产品层迭代不许动内核（P4 起执行）。
- `config-only-check`：新增候选/虚拟模型只改 TOML，内核与契约源码指纹不变。
- `contract-check`：ts-rs 生成物与 `bindings/` 无 diff。
- 其余：fmt、clippy -D warnings、全部测试、fixtures-check。见 [开发者文档](./dev/contributing.zh.md)。

## 3. 协议面（设计文档 §10）

三条 wire 入口，单漏斗：

| 协议 | 端点 |
|---|---|
| OpenAI Chat Completions | `POST /v1/chat/completions` |
| OpenAI Responses | `POST /v1/responses` |
| Anthropic Messages | `POST /v1/messages` |

另有 `GET /v1/models`、`GET /health`。

- `proto::to_canonical` 归一化 → agent 唯一决策点 → `from_canonical` 按来源协议回写。未知字段进 `passthrough`，不丢弃。
- 流式：各协议独立 SSE 状态机，统一 `CanonicalStreamEvent`；tool_call id/index 配对、reasoning/thinking/signature、采样参数显式保留。
- `previous_response_id` 存 `protocol_state` 表；缺失降级为"无历史"并记 `PROTOCOL_STATE_MISSING`，不 500。
- 请求 `model` 字段 = 虚拟模型 Selection；`auto` / `token-router` 为内置虚拟模型。
- **语义提交**（首个 content delta 或首个完整 tool_call 帧）后不切换候选。
- Gemini 原生协议、图片/视频生成明确不做（§14，P7 评估）。

## 4. 路由语义

### 4.1 结构（§3.6、§4.4–4.9）

- `Candidate { id, tier, strength, upstream: UpstreamRef { base_url, model, credential }, window, capability, price, enabled }`。`tier` 是字符串标签不是结论；`strength` 由 profile 语境声明；`credential` 只是 `env:`/`file:`/`keyring:` 引用。
- `VirtualModel` 绑定 profile + strategies + candidate_pool。**禁止虚拟→虚拟嵌套**。
- `Selection`（虚拟模型 + thinking level）与 `Dispatch`（物理模型 + clamp 后 level）分离，都进账本与响应头。

### 4.2 决策流水线（§5、§6.3）

```
SessionFacts/StrategyState/Health 加载
  → RequestReason 推断 {User, Continuation, Retry, Direct}
  → core::draft（纯函数）→ PlanDraft（contributions + judge_requests）
  → decide::consult（按预算，可选）→ Verdict
  → core::refine 消费 Verdict
  → Plan 合并代数：
      Filter 并集 → Baseline 顺序 → Classify 重排 → Escalation Pin
      → Mode 冲突仲裁（Escalation > CompetenceGuard > Classify > Baseline）
      → Failover 剔除熔断生成接力序 → CacheLoss 取最大
      → Plan 校验（非空、window 过滤、failover ⊆ preference）
  → upstream 执行 → Outcome 单事务写账本 + 状态
```

策略 panic 或产出非法 Plan 时回退 Baseline 并记 `CORE_FALLBACK`；判官失败绝不进用户错误路径（fail-open）。

### 4.3 PlanMode（§4.4、§10.6）

| 模式 | 非流式 | 流式 |
|---|---|---|
| `single` | 只执行首选，错误按 failover 接力 | 同左 |
| `cascade` | 质量门失败可换候选 | 仅提交前传输错误接力，不做内容质量门 |
| `verify_after` | 可同轮改强档重试一次 | 只影响下一轮 |

流式升级时机 `stream_escalation`：`next_turn`（默认）/ `gate_before_tool`（tool_call 前缓冲可换候选）/ `off`。

### 4.4 Sticky 与 ContextHold（§4.9.1）

- **Sticky 是硬复用**：Retry 复用 failed 候选、Continuation 复用 previous 候选（保 prompt cache 与 thinking signature）。`ErrorClass ∈ {Overloaded, ContextOverflow}` 时跳过 StickyFailed 回退 previous（纯 enum 判断，不匹配错误文本）。usable 要求候选在池/enabled/凭据/模态/window/健康全满足；任一不满足不阻塞，回退并记 `STICKY_*` reason。
- **ContextHold 是 User 轮软亲和**：上一成功候选置 preference 首位，可被改选；生效需同虚拟模型/候选组/plan_revision/profile/入口/身份 + TTL 1h。
- 两者都不是 latch；**Escalation latch 与 CompetenceGuard 低分保护优先于 Sticky/ContextHold**。

### 4.5 Escalation latch（§6.4）

同 Category + 新证据（调用点计算 `EvidenceRef.fingerprint`，不信任模型自报）确认 2 次 → Latched。latched 后直达强档、不调判官（仅 ReleaseCheck 调）；强档 ≥3 次且连续 decline → ReleasePending → 释放确认进 weak_cooldown（1 轮）；`strong_max_calls = min×4` 强制回落；ReleasePending 超时/不可解析保持 latch（保守）。confidence < 0.60 按 Uncertain。作用域 `(session_key, task_id)`。判官 off 时释放只靠硬上限。

### 4.6 Failover 与熔断（§6.6、§18.4）

- Failover 策略只读 CandidateHealth 做过滤与接力排序；写健康是 upstream 职责。同一上游不原地重试；已提交字节后不换候选。
- 熔断双 key：binding 与 credential 两级；凭据/配额失败先换 key 再换候选。
- 状态 `Active | CoolingDown{until} | Disabled`；到期允许一个半开探测（probe lease 原子认领，30s 窗口），成功回 Active 清零，失败继续冷却。默认 binding 冷却 15s、credential 60s，退避 2/4/8/16/30s；单请求 ≤6 attempts。
- 软失败（5xx/超时）request-local 不跨请求污染；仅凭据/配额错误跨请求共享。全冷却 → 503 + Retry-After（`CANDIDATES_COOLING_DOWN`）。
- 计数权重：RateLimit +0.5、Overloaded +2；Auth/Quota/InvalidRequest/ContextOverflow 不计数。

### 4.7 决策快照确定性（§4.7）

`DecisionSnapshot` 含全部决策输入（selection/reason/previous/failed/context_hold/router_state/时钟/健康/verdicts）；BTreeMap 稳定排序、固定哈希与浮点精度；投影排除 latency_ms/request_id/created_at。`core::decide(&snapshot)` 两次回放逐字段一致——这是"每个决定可解释"的技术基础。

## 5. 扩展点（§4.3、§6.1、§3.5）

- **SignalBag**：`Signals = CoreSignals + BTreeMap<SignalId, SignalValue>`。新信号源实现 `SignalExtractor` 注册写 extras，不改 core。v3 配额信号即走此插拔位（见 [路线图](./roadmap.zh.md)）。
- **Strategy trait**：三段式 `draft/refine/on_outcome` 全只读，状态经 `StateUpdate` 由 agent 原子提交；外部 crate 可注册。
- 五个许可扩展面：candidate / profile / strategy / judge service / protocol adapter。只有 contracts 语义变更才许动内核。

## 6. 账本与统计（§9.2、§8.3、§11.4）

SQLite，金额整数 micros（禁 REAL），时间 Unix millis。`ledger` 逐请求记录：selection/dispatch、plan_mode、preference/failover、**attempts_json**（逐 attempt 候选/ErrorClass/耗时/是否提交）、switches、reason_codes、outcome、tokens、**answer_cost / routing_cost 分账**（判官成本入 routing）、ttft/duration、config_digest/profile_revision、judge/contributions/authority。

其余表：sessions、strategy_states、virtual_model_states、shadow_records、circuit_breakers、protocol_state、judge_calls、approvals、tokens、config_audit（后两者 P4）。保留期：账本 30 天、breaker 24h、protocol_state 7 天、approvals 90 天。

统计只聚合账本（与直接 SQL 同源）；评测用自带 checker 判成功，不用 LLM 判。

## 7. 配置模型

### 7.1 内核配置（§18.2）

`config.toml`，`schema_version = 1`（代码严格校验）。段：`[server]`、`[profiles.*]`、`[[candidates]]`、`[[virtual_models]]`、`[judge]`、`[cost_gate]`、`[approvals]`、`[trust_tiers]`。字段参考见 [配置指南](./guide/configuration.zh.md)。

- credential 只存引用（`env:`/`file:`/`keyring:`）；env 只许覆盖 secrets 与 listen。
- `deny_unknown_fields`；版本超前拒启动。
- `config_digest = sha256(规范化形态)` 随快照/账本落库；profile_revision 与 plan_revision 两级分开（改价格不失效 hold）。

### 7.2 管理面配置闭环（P4 增量，内核无感知）

`control` 提供脱敏文档读取、validate（200+issues）、apply（422+CAS 409）、原子写 + 备份（≤10）、rollback、审计（config_audit 表）。apply 后重启生效；并发修改 CAS 保护不覆盖。契约见 [管理 API](./contracts/admin-api.zh.md)。

## 8. 安全模型

内核侧（§8.5、§11–12）：

- 明文 key 不落库、不回显、不进日志/账本/Plan；API 只返回引用字符串。
- 日志默认脱敏正文与工具参数；shadow 记录 30 天脱敏。
- **审批** `[approvals]` off/observe/enforce：访问上游前扫描 ToolUse 按工具名分级（未知=high），`risk >= min_risk` 才拦截；本地 approvals 表 + TTL；enforce + fail_closed 返回 428/403。方向刻意不对称：**路由判官 fail-open，审批 fail-closed**。

P4 桌面增量：

- 管理面走本机 `/v2/admin`；admin token 不进 WebView（Rust 侧内存注入 Bearer），UI 全部经代理（路径/方法白名单、8MiB 上限）。
- scoped token（inference/admin:read/admin:write/admin:approve），SHA-256 存库；首启生成 bootstrap token 写 `<config目录>/admin.token`。
- supervisor 崩溃退避重启（1..16s ×5）、孤儿进程清理。

## 9. 明确不做（§14、§2.2）

多租户/HA/云端控制面；流式中途换模型；内置模型能力权威库；词表/关键词进内核；判官自然语言理由参与算术（只进审计）；上游主动健康探测；用 LLM 判任务成功；v1 配置/API/桌面端/数据不兼容不迁移。隐私遮蔽/插件/Web 搜索/媒体/OAuth 订阅见 [路线图 §7](./roadmap.zh.md)。