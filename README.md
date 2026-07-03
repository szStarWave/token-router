# Flowy Router

端云 LLM 智能路由：**单一 `token-router` 可执行文件** — CLI 管理命令与 Gateway 守护进程合并在同一二进制中。Agent（OpenClaw、Hermes 等）将 OpenAI 兼容 `base_url` 指向 Gateway 即可，在 Agent 行为不变的前提下降低云端 **输入 Token** 成本。

---

## 产品概述

### 背景

自主 Agent（如 [OpenClaw](https://github.com/openclaw/openclaw)、[Hermes Agent](https://github.com/NousResearch/hermes-agent)）在一次用户意图下往往触发 **多轮 LLM 推理**：ReAct 循环中每一步都是独立的 Chat Completions 请求，且每步都会重发 **完整 system + 全历史 + 工具 schema**。

实测（OpenClaw 类负载）单次请求约 **52,830 输入 Token / 357 输出 Token**，输入占比 **99%+**。因此 Flowy 的首要优化目标是 **少把巨型 prompt 送进云端计价模型**，而非压缩输出。

端侧小模型 / MoE（如 Qwen3.5-35B-A3B）适合 Agent 循环中的轻推理步骤；云端旗舰模型保留给规划、复杂工具链、长文档理解等。

### 定位

| 维度 | 说明 |
|------|------|
| **是什么** | Agent 专用的 OpenAI 兼容模型代理：端云统一接入 + 按 **Inference Step** 粒度的路由 + 成本/质量可观测 |
| **主要服务对象** | OpenClaw、Hermes 及同类「自带 Gateway + Agentic Loop + 可配置 OpenAI-compatible endpoint」的运行时 |
| **不是什么** | 不是 Agent 本体（不负责工具执行、记忆、消息渠道） |
| **核心价值** | 在可控质量下减少云端输入 Token 次数与体量 |

### Agent 集成示意

```
┌─────────────────┐     ┌──────────────────┐     ┌─────────────────────┐
│ OpenClaw /      │     │  Flowy Router    │     │ Edge（Ollama 等）    │
│ Hermes Gateway  │────►│  逐请求路由       │────►│ 或                  │
│ Agentic Loop    │     │  base_url 替换    │     │ Cloud（DeepSeek 等） │
└─────────────────┘     └──────────────────┘     └─────────────────────┘
```

| 职责 | Agent | Flowy Router |
|------|-------|--------------|
| 工具执行、记忆、Skills | ✅ | ❌ |
| 单次 LLM 调用的端/云选择 | ❌ | ✅ |
| 端侧低质量时的 Cascade / Fallback | ❌ | ✅ |
| 日常/心跳标为 casual、路由 edge/cascade | ❌ | ✅ |
| 路由与 Token 统计（`token_router_meta`） | 可选 | ✅ |

---

## 架构

### 整体架构概览

```
┌──────────────────────────────────────────────────────────────────────┐
│  token-router（单一二进制 / 动态库）                                           │
│    CLI 层:  gateway start/stop/restart, setup, stats, env, status    │
│    HTTP 层: Axum 服务器, OpenAI 兼容 API, 管理端点, Web 配置页        │
└──────────────────────────────┬───────────────────────────────────────┘
                               │
                   Agent (OpenClaw/Hermes)
                   POST /v1/chat/completions
                               │
┌──────────────────────────────┼──────────────────────────────────────┐
│                              ▼                                       │
│  ┌───────────────── Routing Engine (decide()) ──────────────────┐   │
│  │                                                               │   │
│  │  [1] SignalExtractor   提取消息特征、token估算、步态线索       │   │
│  │  [2] StepKind          推断步态类型（direct_chat / tool_select │   │
│  │                            / initial_plan / ...）              │   │
│  │  [3] Hard Gates        硬约束门控（命中则跳过评分直接定路）    │   │
│  │  [4] DifficultyScore   加权难度评分 [0,1] + 经验偏置          │   │
│  │  [5] Classifier        朴素贝叶斯预测 edge_ok 概率 -> 融合难度 │   │
│  │  [6] Policy            根据 Profile + mode 映射为路由层        │   │
│  │  [7] WorkStrategy      Work 步态: CachedEdge / Verify / Plan  │   │
│  │  [8] StickyCascade     粘性期内 Work 执行步态走级联            │   │
│  │  [9] Multimodal        视觉模型能力探测与路由                   │   │
│  │  [10] EdgeBusy         端侧负载感知 -> 非 casual 临时升云      │   │
│  │  [11] UpstreamAvail    上游可用性检查 -> 单路强制               │   │
│  └──────────────────────────┬────────────────────────────────────┘   │
│                             ▼                                        │
│  ┌───────────────── Upstream Execution ─────────────────────────┐   │
│  │  Edge / Cloud / Cascade / Verify / Multimodal Probe          │   │
│  │  SSE 流式转发 + TTFT/TPS 指标 + TokenRouterMeta 注入               │   │
│  └──────────────────────────┬────────────────────────────────────┘   │
│                             ▼                                        │
│  ┌───────────────── Learning Feedback ──────────────────────────┐   │
│  │  ExperienceStore    按 step_kind 记录 outcome -> 偏置学习    │   │
│  │  ClassifierStore     特征向量 + edge_ok 标签 -> 贝叶斯更新   │   │
│  │  SessionStore        粘性状态 + tok_loop_delta               │   │
│  └──────────────────────────────────────────────────────────────┘   │
│                             ▼                                        │
│  ┌───────────────── Adaptive Tuner (定时刷新) ──────────────────┐   │
│  │  每 30s / 40 请求: 调整 verify_sample_rate, θ_edge, θ_cloud │   │
│  └──────────────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────────────┘
```

> **核心原则**：路由仅由请求体 `messages[]` / `tools[]` 与 `config.toml` 推断，不依赖自定义 Header。调试通过响应体内 `token_router_meta`（非流式）或 `X-Token-Router-*` 响应头（流式）查看每一步决策原因。

### 源码结构

```
src/
├── main.rs                 CLI 入口 + __serve 守护进程模式
├── lib.rs                 动态库入口（FFI 嵌入）
├── client.rs              Gateway HTTP 客户端（CLI → Gateway）
├── daemon_ctl.rs           进程生命周期管理（start/stop/restart)
├── env_cmd.rs             token-router env 命令
├── setup_cmd.rs           token-router setup 命令
├── stats_cmd.rs           token-router stats 命令
├── cli_settings.rs         CLI 设置
├── embedded.rs             FFI 嵌入式运行模式
├── ffi.rs                  C FFI 导出
├── config/
│   ├── mod.rs
│   ├── paths.rs            配置文件路径解析
│   ├── file.rs             ConfigFile (TOML 结构)
│   └── setup.rs            交互式配置向导
└── gateway/
    ├── mod.rs              模块声明与重导出
    ├── config.rs           AppConfig 运行时配置 + TOML 映射
    ├── config_manager.rs   运行时配置热更新（Arc<RwLock>）
    ├── server.rs           Axum HTTP 服务器启动 + 优雅关闭
    ├── daemon.rs           PID 文件管理 + 进程健康检查
    ├── edge_load.rs        端侧并发推理计数器（Atomic）
    ├── error.rs            AppError 枚举（400/401/500/502/503）
    ├── logging.rs          tracing 初始化（文件 + stderr）
    ├── routing/            路由决策引擎（核心）
    │   ├── mod.rs
    │   ├── decision.rs     decide() 主决策流水线
    │   ├── signals.rs      请求信号提取器
    │   ├── step_kind.rs    步态分类
    │   ├── gates.rs        硬约束门控
    │   ├── difficulty.rs   难度评分
    │   ├── policy.rs       Profile + routing_mode 策略映射
    │   ├── work.rs         Work 步态策略（CachedEdge / Verify）
    │   ├── adaptive.rs     自适应路由参数计算
    │   ├── adaptive_tuner.rs 定时刷新自适应参数
    │   ├── edge_busy.rs    端侧负载回退
    │   ├── upstream_availability.rs 上游可用性
    │   ├── conversation.rs 会话键生成（稳定哈希）
    │   └── tests.rs
    ├── classifier/         朴素贝叶斯分类器
    │   ├── mod.rs
    │   ├── data.rs         持久化数据（LabelCounts）
    │   ├── features.rs     特征工程（FeatureVector）
    │   ├── model.rs        朴素贝叶斯推理 + 训练
    │   └── store.rs        内存存储 + 定时刷盘 + 衰减
    ├── experience/         经验学习
    │   ├── mod.rs
    │   ├── data.rs         持久化数据（StepExperience）
    │   ├── outcome.rs      RequestOutcome 定义
    │   ├── store.rs        经验存储 + 偏置计算
    │   └── tests.rs
    ├── session/            会话状态
    │   ├── mod.rs
    │   ├── data.rs         会话数据（sticky_until, last_tok_in）
    │   └── store.rs        内存 + 按需懒加载 + 定时刷盘
    ├── stats/              统计系统
    │   ├── mod.rs          GatewayStats（双范围: 会话 + 全局）
    │   ├── data.rs         累加计数器 + 持久化
    │   └── metrics.rs      上游调用指标 + SSE 解析
    ├── multimodal/         多模态路由
    │   ├── mod.rs          策略枚举
    │   ├── data.rs         模型能力缓存
    │   ├── fingerprint.rs  上游指纹（缓存失效依据）
    │   └── store.rs        能力查询 + 探测记录
    ├── api/                HTTP 处理层
    │   ├── mod.rs          路由注册
    │   ├── routes.rs       路由表定义
    │   ├── chat.rs         聊天接口主处理
    │   ├── auth.rs         请求鉴权
    │   ├── admin.rs        管理端点
    │   ├── setup.rs        配置页 Web UI
    │   ├── meta.rs         TokenRouterMeta 构建 + 响应注入
    │   └── openai.rs       OpenAI 类型定义（请求/响应）
    ├── upstream/           上游 LLM 转发
    │   ├── mod.rs          转发入口
    │   ├── forward.rs      上游 HTTP 客户端
    │   ├── sse.rs          SSE 流式封装 + 指标采集
    │   └── verify.rs       云端校验边缘输出
    └── tests/
```

### 详细模块设计

#### 1. 路由决策引擎（`routing/decision.rs`）

`decide()` 函数是系统的核心，每次聊天请求都会执行一次完整的决策流水线：

1. **`SignalExtractor::extract()`** — 深入解析请求体，提取约 35 个信号字段
2. **`resolve_step_kind()`** — 根据信号推断步态类型
3. **`FeatureVector::from_signals()`** — 构建分类器特征向量
4. **固定路由检查** — 若 `config.route` 为 `edge`/`cloud`/`cascade` 则直接使用
5. **`check_hard_gates()`** — 遍历 8 条门控规则，命中则跳过评分
6. **`DifficultyScore::compute()`** — 加权公式 + 经验偏置 → `[0, 1]`
7. **分类器预测** — `predict_and_fuse()` 融合经验与贝叶斯
8. **`map_policy_with_thresholds()`** — Profile 阈值映射为 `RouteTier`
9. **`apply_work_route()`** — Work 步态特殊策略（Plan→Cloud / CachedEdge / Verify）
10. **Cloud Sticky 覆盖** — 粘性期内 Work 执行步态 → Cascade
11. **多模态策略** — `route_hint()` 查询视觉模型能力 → 修正路由
12. **`apply_edge_busy_fallback()`** — 端侧繁忙时非 casual 步态升云
13. **`finalize_route()`** — 检查上游可用性，仅一路可用时强制定向

##### 1.1 信号提取器（`routing/signals.rs`）

`SignalExtractor` 通过静态分析请求体计算：

| 信号类别 | 具体字段 |
|---------|---------|
| **Token 估算** | `tok_system`、`tok_tools_schema`、`tok_rest`（transcript）、`tok_total_in`、`tok_loop_delta`（本轮新增）、`tok_out_estimate` |
| **轮次分析** | `n_tool_defs`、`n_turns`、`last_user_tok`、`loop_steps` |
| **状态标记** | `pending_tool_calls`、`tool_arg_ready`、`last_role_tool`、`assistant_failed_recent`、`had_tool_roundtrip` |
| **步态线索** | `is_heartbeat_poll`（正则匹配 `[OpenClaw heartbeat poll]`）、`subagent_spawn_hint`、`memory_compact_hint`、`cron_background` |
| **意图识别** | `intent_hard` / `intent_easy` / `intent_plan`（**仅最新 user 消息**，关键词见 `routing/keywords.rs`） |
| **词汇稀有度** | `rare_lexical`（统计：[tokenizers](https://github.com/huggingface/tokenizers) WordLevel 分词 + `wordfreq` 查频，词表存 SQLite `wordfreq.db`；OOV/低频 → 稀有）、`special_lexical`（领域专名关键词：GDPR/K8s/CVE 等）、`rare_token_ratio` |
| **工具分析** | `risky_tool_tier1`（exec/write/browser/sessions_spawn 等）、`consecutive_tool_error_streak`（尾部连续含错误关键词的 tool result 条数；≥1 重分类为 `RecoveryAfterFailure` 并渐进升难，≥2 触发硬门控）、`tool_invocations_since_last_user`（自上次 user 以来的 tool result 条数；≥5 渐进升难） |
| **多模态探测** | 检查 `content` 是否含 `image_url` 或 `data:image` |

**关键词模块**（`routing/keywords.rs`）：集中管理 `tool_error`、`hard_intent`、`plan_intent`、`easy_intent`、`reject_intent`、`uncertainty`（cascade/verify 质量门）、`special_lexical` 七组词表；ASCII 词大小写不敏感，短词（≤4 字符）使用词边界匹配避免误伤。

**统计稀有词**（`routing/lexical.rs` + `routing/lexical_tokenizer.rs` + `routing/wordfreq_store.rs`）：对最新 user 消息做 `whatlang` 语言检测（粤语 fallback 到 zh 词频表）→ 分词（英文用 [huggingface/tokenizers](https://github.com/huggingface/tokenizers) WordLevel + Whitespace；中日韩用词表最长匹配）→ 查 `{data_dir}/wordfreq.db` 词频。首次启动写入默认常见词（`wordfreq_seed.rs`，由 `scripts/gen_wordfreq_tables.js` 生成）；运行时可在 `easy_intent` 对话及 edge 成功的 DirectChat/HeartbeatAck 下持续学习并落盘（`wordfreq_learning_enabled`，默认 true）。满足任一即 `rare_lexical=true`：token 频率 < 1e-7、稀有 token ≥2、或稀有占比 ≥25%（总 token < 3 时不看占比）。命中 `easy_intent` 且无 `special_lexical` 时跳过稀有统计。

**关键设计**：`tok_loop_delta` 通过 `SessionStore::get_last_tok_in()` 计算 `tok_total_in - last_tok_in`，用于检测 Agent 循环中是否出现大量新增 token（如从外部注入长上下文）。

##### 1.2 步态分类（`routing/step_kind.rs`）

`resolve_step_kind()` 通过信号组合推断步态类型：

| 步态 | 判定逻辑 | 难度偏置 | 典型路由 |
|------|---------|---------|---------|
| `HeartbeatAck` | `is_heartbeat_poll` 匹配 | -0.60 | casual |
| `DirectChat` | `is_casual_chat()` 通过（关键实现见下文） | -0.55 | casual |
| `ToolSelect` | `pending_tool_calls == false && intent_hard == false` 且 `tools` 存在 | -0.10 | Work |
| `ToolArgFill` | `pending_tool_calls == true && tool_arg_ready == true` | -0.25 | Work |
| `ToolResultDigest` | `last_role_tool == true` | -0.45 | Work |
| `InitialPlan` | `n_turns == 0 && !is_casual` | +0.35 | **云端** |
| `FinalReply` | `had_tool_roundtrip == true && n_tool_calls == 0` | +0.05 | Work |
| `RecoveryAfterFailure` | `assistant_failed_recent == true` 或 `consecutive_tool_error_streak >= 1` | +0.55 | **云端 / cascade** |
| `SubagentSpawn` | `subagent_spawn_hint == true` | +0.50 | **云端** |
| `MemoryCompact` | `memory_compact_hint == true` | +0.20 | Work |
| `CronBackground` | `cron_background == true` | -0.15 | Work |

`is_casual_chat()` 的实现要点：
- **体量**：`tok_rest`（仅 transcript，不含 system/tools schema）< 8192
- **轮次**：用户消息数 ≤ 8
- **排除**：有 tool 往返、`pending_tool_calls`、`intent_hard`、assistant 失败恢复、图+tools 并存
- **多轮 + tools**：最新 user 须命中 easy 意图关键词
- **多轮无 tools**：最新 user 很短（`last_user_tok ≤ 512`）也可视为 casual 跟帖

> **重要**：`DirectChat` / `HeartbeatAck` 在 `d < θ_cloud` 时决策为 **edge**（不再产出 `cascade`）；双上游可用时执行层经 `cascade_gate_pass()` 质量门，不合格自动升云（`fallback=true`）。仅高难度或硬门控直接走云。

##### 1.3 硬约束门控（`routing/gates.rs`）

每条门控规则返回 `Option<HardGate>`，命中后直接定路，不执行后续评分：

| 门控 | 条件 | 路由结果 |
|------|------|---------|
| `GATE_EDGE_DOWN` | 端侧未配置或不可用 | **cloud**（或 503） |
| `GATE_USER_REJECT` | 最新 user 否定上一轮 assistant（中/英/日/韩/粤关键词，见 `routing/keywords.rs` → `contains_reject_intent`） | **cloud** |
| `GATE_CTX_OVERFLOW` | `tok_total_in > 80% × ctx_edge_max_tokens`；**casual 仅按 `tok_rest`（transcript）计算** | **cloud** |
| `GATE_ASSISTANT_FAILURE` | 最近 assistant 含失败标记（`RecoveryAfterFailure`） | **cloud** |
| `GATE_TOOL_ERROR_STREAK` | 连续 2+ 条 `role=tool` 含错误关键词 → 当次升云并 `force_cloud_sticky` | **cloud** + 粘性 |
| `GATE_RISKY_TOOL` | Tier-1 工具（`exec`/`write`/`edit`/`browser`/`sessions_spawn`/`message`） | **cloud** |
| `GATE_OPENCLAW_COMPACT` | `MEMORY_COMPACT` 提示且上下文 > 12K token | **cloud** |
| `GATE_EDGE_BUSY` | 端侧已有推理进行中 + 云端可用 + **非 casual** 步态 | **cloud**（临时） |
| `MULTIMODAL_COMPLEX_CLOUD` | 非 `DirectChat` 的多模态（含图片 + 内容或 tools） | **cloud** |

##### 1.4 难度评分（`routing/difficulty.rs`）

`DifficultyScore::compute()` 使用 sigmoid 转换的加权线性公式：

```
raw = 0.20 × ctx_ratio + 0.25 × user_ctx_ratio + 0.10 × tool_ratio
      + 0.30 × intent_hard - 0.40 × intent_easy + 0.12 × user_multimodal
      + step_kind.bias() + experience_bias
      + assistant_failed_recent_bonus + tool_error_streak_bias + tool_loop_bias + lexical_rarity_bias

d = 1.0 / (1.0 + e^(-raw))
```

其中：
- `ctx_ratio = tok_loop_delta / ctx_edge_max_tokens`（本轮上下文增量；**DirectChat/HeartbeatAck 用 `tok_rest` 对话 transcript**，不计 OpenClaw system/tools 静态开销）
- `user_ctx_ratio = last_user_tok / ctx_edge_max_tokens`（**最新 user 消息**体量，权重最高之一）
- `tool_ratio = n_tool_defs / 20`（工具定义密度，上限 1.0）
- `intent_hard` / `intent_easy` / `intent_plan` 均来自**最新 user 消息**（0 或 1）
- `user_multimodal`：仅当最新 user 消息含图片时为 1（全请求 `multimodal` 仍用于步态/门控）
- `step_kind.bias()` 见上表
- `experience_bias` 来自 `ExperienceStore::bias_for(step_kind)`
- `assistant_failed_recent_bonus`：assistant turn 失败时 +0.15
- `tool_error_streak_bias`：连续 tool 失败渐进加成 — streak 1 → +0.15，2 → +0.30，3+ → +0.40（封顶）
- `tool_loop_bias`：自上次 user 以来 tool result 条数渐进加成 — 0–4 → 0，5–6 → +0.10，7 → +0.18，8+ → +0.25（封顶）
- `lexical_rarity_bias`：词汇稀有度渐进加成（非硬门控）— 仅 rare → +0.08，仅 special → +0.12，两者皆有 → +0.18（封顶）；reason code：`LEXICAL_RARE` / `LEXICAL_SPECIAL` / `LEXICAL_BOTH`；分类器特征 bucket：`lexical:none|rare|special|both`

##### 1.5 策略映射（`routing/policy.rs`）

四个内置 Profile，各定义了基于阈值的路由策略：

| Profile | θ_edge | θ_cloud | 默认 Mode | 说明 |
|---------|--------|---------|-----------|------|
| `economy` | 0.40 | 0.60 | Cascade | 更多走端侧 |
| `balanced` | 0.35 | 0.55 | Cascade | 默认均衡 |
| `premium` | 0.25 | 0.45 | Single | 更多走云端 |
| `privacy` | 0.45 | 0.65 | Edge | 尽量端侧（Recovery 除外） |

`map_policy_with_thresholds(difficulty, profile, mode, theta_edge, theta_cloud)`：
- `d < θ_edge` → Edge
- `θ_edge ≤ d < θ_cloud` → 根据 mode 决定：`single` 走 Cloud，`cascade` 走 Cascade，`split` 走 Cloud
- `d ≥ θ_cloud` → Cloud

##### 1.6 Work 步态策略（`routing/work.rs`）

Agent 循环中的执行步态（`ToolSelect`/`ToolResultDigest`/`FinalReply` 等）采用特殊策略：

| 策略 | 触发条件 | 行为 |
|------|---------|------|
| `InitialPlan` → Cloud | `is_plan_step()` 判断: 步态为 `InitialPlan` 或 `intent_plan == true` | 强制云端 |
| `WorkExecEdge` | `is_work_step()` 且难度未超 θ_edge | 默认走端侧 |
| `CachedEdge` | `experience.edge_trusted(step_kind)` 返回 true | 直接走端，不校验 |
| `Verify` | `should_work_verify_sample()` 命中抽样 | Cascade + 云端校验（验证 tool 名称或文本兼容性） |
| `StickyCascadeRetry` | `sticky_cascade_applies()` 粘性期内 | Cascade 重试端侧 |

`should_work_verify_sample()` 使用确定性哈希做抽样，确保同一会话+步态+token 数的请求抽样结果一致（避免连续入样）。

##### 1.7 Cloud Sticky 会话粘性（`routing/work.rs` + `session/store.rs`）

粘性以 **Unix 时间戳 TTL** 实现，保存于 `sessions/<conv>.json` 的 `cloud_sticky_until_unix`。

| 操作 | 触发条件 |
|------|---------|
| **开启/续期** | cascade_fallback、upstream_error（非心跳）、GATE_TOOL_ERROR_STREAK |
| **清除** | 任一路径端侧成功（edge_ok && !fallback && !upstream_error） |
| **有效期行为** | Work 执行步态 → `STICKY_CASCADE_RETRY`；casual 步态不受影响；InitialPlan/Recovery 仍走云 |

##### 1.8 多模态路由（`routing/decision.rs` + `multimodal/`）

视觉模型能力通过主动探测学习缓存：

1. 首次遇到某模型的图片请求时，`route_hint()` 返回 `Probe`
2. 先尝试端侧，记录结果到 `MultimodalStore`（`record_edge()`/`record_cloud()`）
3. 下次同模型请求 → `CachedEdge` 或 `CachedCloud`
4. 若上游 URL/Key 变化（指纹变更），清空缓存重新探测

简单多模态日常（DirectChat + 仅 1-2 张图片）可走端侧；复杂多模态（非 DirectChat 或含工具）走云端。

#### 2. 朴素贝叶斯分类器（`classifier/`）

作为经验学习的补充，分类器从另一个维度学习路由模式。

**特征工程**（`features.rs`）：
- `step_kind:<type>`、`ctx_bucket:<low|mid|high>`、`tool_bucket:<none|few|many>`
- `loop_bucket:<none|short|long>`、`turn_bucket:<short|long>`、`intent:<easy|hard|plan|neutral>`
- 布尔标记：`multimodal`、`risky_tool_tier1`、`pending_tool_calls`、`assistant_failed_recent`、`is_heartbeat_poll`、`had_tool_roundtrip`、`tools_enabled`

**推理**（`model.rs`）：
- 对数空间朴素贝叶斯 + Laplace 平滑：`P(edge_ok | features) ∝ log prior + Σ log P(f_i | edge_ok)`
- 融合难度：`d_final = (1-w) × d_heuristic + w × (1 - p_edge)`，权重 `w` 随样本数线性增长至 `min_samples`
- 冷启动时通过 `seed_heuristic_priors()` 注入领域先验

**训练**：
- `label_from_outcome(outcome)` → `EdgeOk`（端侧成功）或 `CloudNeeded`（需云）
- `should_record_outcome()` 确保仅在端侧确实被尝试时才记录
- 指数衰减（默认 72 小时半衰期）老化旧观测，适应环境变化

**持久化**：
- `classifier.json` 存储先验与特征计数，版本化 + 原子写入
- 每隔 5 秒自动刷盘
- 每小时执行一次衰减

#### 3. 经验学习（`experience/`）

按 `step_kind` 维度记录路由结果，产出两种能力：

| 能力 | 算法 |
|------|------|
| **难度偏置** | `bias = learning_rate × (fallback_rate - target_fallback)`，截断在 `[-max_bias, +max_bias]` |
| **边缘可信** | `edge_trusted()` 在 verified_samples ≥ 3 且 fallback_rate ≤ target_fallback 时返回 true |

`RequestOutcome` 三元组：`{edge_ok, cascade_fallback, upstream_error}`。

持久化：`experience.json`，记录每个 `step_kind` 的 edge_ok / cascade_fallback / upstream_error 计数。

#### 4. 自适应路由（`routing/adaptive.rs` + `adaptive_tuner.rs`）

`AdaptiveTuner` 每 30 秒或 40 请求执行 `compute_effective_routing()`：

- **预热期**：云端验证样本 < `adaptive_min_verified_samples` → 保持配置基线
- **健康状态**（回退率 ≤ 目标 + 信任步态充足）：
  - 降低 `work_verify_sample_rate`
  - 放宽 `θ_edge`（最多 `adaptive_max_theta_shift`）
  - 提高端侧使用率
- **吃力状态**（回退率偏高）：
  - 提高校验抽样率
  - 收紧阈值
  - 保证复杂任务正确率
- **级联统计覆盖**：`stats.json` 中等级级联回退率 > 28% 时进一步收紧

安全边界：InitialPlan、复杂多模态、Hard Gates、高难度走云/级联不会被自适应放宽。

#### 5. 上游执行（`upstream/forward.rs` + `sse.rs` + `verify.rs`）

**执行模式**：

| 模式 | 流程 |
|------|------|
| **Edge** | 调用端侧 → 返回结果 |
| **Cloud** | 调用云端 → 返回结果 |
| **Cascade** | 调用端侧 → `cascade_gate_pass()` 检查 → 失败则升云重答（`fallback=true`） |
| **Verify** | 调用端侧 + 调用云端 → `cloud_verifies_edge()` 对比 → 信任端侧或采纳云端 |
| **Probe**（多模态） | 调用端侧 → 记录能力 → 失败则调云端 → 记录能力 |

**质量门**（`cascade_gate_pass`）：端侧结果满足其一即过关：
- 非空 `content`，长度 > 8，不含「不确定」
- 合法 `tool_calls`：至少一条，且每条 `function.name` / `arguments` 非空

**云端校验**（`cloud_verifies_edge`）：
- Tool calls：名称精确匹配且顺序一致
- 文本：Jaccard 相似度 ≥ 12%，双方 ≥ 8 字符，云端不说不确定

**流式处理**（`sse.rs`）：封装 `reqwest` SSE 流，逐块解析 `data:` 行，采集 `TTFT`、`usage`、吞吐量等指标，按需注入 `X-Token-Router-*` 头。

#### 6. 统计系统（`stats/`）

双范围设计：

| 范围 | 生命周期 | 持久化 |
|------|---------|-------|
| **Session（会话）** | 进程级，进程退出即丢失 | 内存 |
| **Global（全局）** | 跨进程重启 | `stats.json`（version 2） |

`StatsData` 计数器覆盖：请求量、路由分布、上游 Token（输入/输出/缓存）、Cascade 结果、延迟（TTFT/TPS）、Cloud Input Saved、错误分布、步态分布。

#### 7. 配置管理（`config_manager.rs`）

`ConfigManager` 以 `Arc<RwLock<AppConfig>>` 持有运行中配置：

- **热更新**：`POST /v1/admin/setup` → `apply_setup()` 写回 `config.toml` 并重载内存
- **恢复默认**：`POST /v1/admin/setup/init` → `write_default_setup()`
- **文件重载**：`reload_from_file()` 重新解析 TOML

#### 8. 守护进程生命周期（`daemon_ctl.rs` + `daemon.rs`）

```
token-router gateway start
  → daemon_ctl::start_daemon()
    → 生成 `token-router __serve` 子进程（setsid 分离会话）
    → wait_until_healthy() 轮询 /health
    → 返回 PID

token-router gateway stop
  → daemon_ctl::stop_daemon()
    → POST /v1/admin/shutdown（带 token）
    → 超时后 SIGTERM / SIGKILL

token-router gateway restart
  → POST /v1/admin/restart
    → schedule_daemon_restart() 生成 __restart-wait 辅助进程
    → 辅助进程等待旧 PID 退出后执行 `__serve`
    → 最多重试 20 次等待健康
```

#### 9. 端侧负载感知（`edge_load.rs`）

`EdgeInferenceTracker` 使用 `AtomicU32` 追踪并发推理数。通过 RAII 守卫 `EdgeInferenceGuard`（`begin()` 返回，`drop()` 时递减），确保不会漏减计数。

#### 10. 会话管理（`session/`）

每会话独立 JSON 文件 `sessions/<conv>.json`，内容：

| 字段 | 说明 |
|------|------|
| `last_tok_in` | 上次请求总 token 数，用于计算 `tok_loop_delta` |
| `cloud_sticky_until_unix` | 粘性到期时间戳 |
| `last_assistant_failed` | 上次 assistant 是否失败 |

实现特点：
- 按需懒加载（首次访问时读盘）
- 脏标记追踪，每 5 秒批量刷盘
- 会话键（`conversation_key()`）仅哈希 anchor messages（第一条 system + 第一条 user + tool 名称），在 Agent 循环中保持稳定

#### 11. 任务成功判定与准确率保障

Flowy 通过 **多层级质量评估系统** 判断端侧任务是否成功、是否需要升云，并在执行后持续学习优化。整个过程不依赖用户反馈，完全通过执行结果本身进行信号提取。

##### 11.1 任务结果的定义

核心数据结构 `RequestOutcome`（`experience/outcome.rs:5`）包含三个布尔字段，组合表示每次请求的结果：

| 组合 | `edge_ok` | `cascade_fallback` | `upstream_error` | 含义 |
|------|-----------|-------------------|-----------------|------|
| **端侧成功** | `true` | `false` | `false` | Edge 返回合格结果，直接采纳 |
| **级联回退** | `false` | `true` | `false` | Edge 结果不合格，升云重答后成功 |
| **上游错误** | `false` | `false` | `true` | Edge/Cloud 请求本身失败（网络/HTTP错误） |
| **纯云端** | `false` | `false` | `false` | 决策层直接走了 Cloud，端侧未尝试 |

`RequestOutcome::success(decision, fallback)` 构造函数根据路由类型自动映射：
- `RouteTier::Edge` + `casual_quality_fallback` + `fallback` → `{ cascade_fallback: true, ... }`（端侧质量门未过，升云重答）
- `RouteTier::Edge`（其他） → `{ edge_ok: true, ... }`
- `RouteTier::Cloud` → `{ edge_ok: false, ... }`（端侧未尝试，不算成功也不算失败）
- `RouteTier::Cascade` → `edge_ok = !fallback, cascade_fallback = fallback`

##### 11.2 级联质量门（Cascade Gate）

当路由决策为 `Cascade`，或 `DirectChat` / `HeartbeatAck` 决策为 **edge** 且 `casual_quality_fallback=true`（双上游可用）时，系统先请求端侧，然后通过 `cascade_gate_pass()` 判断端侧结果是否合格：

**文本回复的质量门**（`cascade_text_pass`, `forward.rs:594`）：
```
通过条件: content ≠ "" && content.len() > 8 && content 不含 "不确定"
```
- 空回复 → 不通过（端侧模型未输出或输出被截断）
- 极短回复 ≤ 8 字符 → 不通过（模型未认真回答）
- 包含中文「不确定」→ 不通过（端侧缺乏足够知识）

**Tool calls 的质量门**（`cascade_tool_calls_pass`, `forward.rs:601`）：
```
通过条件: tool_calls 非空 && 每条 call 的 name 和 arguments 均非空
```
- 覆盖 `TOOL_RESULT_DIGEST` 后模型只返回 tool call、无长文本的情况
- 空 tool_calls → 不通过（Agent 循环中期望工具调用但端侧未产出）

**两个条件满足其一即过关**，不要求同时满足。

##### 11.3 云端校验（Work Verify）

当路由启用了 `WorkStrategy::Verify` 时（通过 `should_work_verify_sample()` 确定性抽样），系统会请求端侧和云端，然后执行 `cloud_verifies_edge()`（`verify.rs:6`）做输出对比验证：

**Tool calls 校验**（优先级高于文本）：
```
edge_tool_names == cloud_tool_names（精确匹配且顺序一致）
```
- 端侧选择了与云端完全一致的工具序列 → 信任端侧
- 任一缺失或顺序不同 → 不信任，使用云端结果

**文本回复校验**（`text_responses_compatible`, `verify.rs:38`）：
```
条件: edge.len() ≥ 8 && cloud.len() ≥ 8 && cloud 不含 "不确定"
       && Jaccard(edge_words, cloud_words) ≥ 12%
```
- Jaccard 相似度 = |edge_words ∩ cloud_words| / |edge_words ∪ cloud_words|
- 低阈值（12%）设计是因为端侧模型可能用不同措辞表达同一意思
- 云端自己说不确定 → 不校验（云端也没把握，信任端侧）
- 任一方文本太短 → 不校验（无意义的简短回复不做判断）

**验证通过 → 采纳端侧结果并标记 `edge_ok=true`**，同时`cloud_input_saved`记录节省的输入Token。

**验证不通过 → 丢弃端侧结果，返回云端结果**，标记 `cascade_fallback=true`。

##### 11.4 多模态能力探测（Probe）

对多模态请求，`MultimodalStrategy::Probe`（`forward.rs:178`）采用类似 Cascade 的试错：
1. 尝试端侧 → 若 `cascade_gate_pass()` 通过 → 记录 `MultimodalStore` 为 `edge_capable`
2. 端侧失败或结果不合格 → 尝试云端 → 记录 `MultimodalStore` 为 `cloud_capable`
3. 两者都失败 → 兜底走端侧（尽力而为）
4. 下次同模型请求 → 直接从 `MultimodalStore` 取缓存能力，跳过探测

##### 11.5 学习反馈闭环

每次请求完成后，`record_learning()`（`chat.rs:114`）将结果反馈到三个学习系统：

**经验学习**（`experience/store.rs:104`）：
```
每 step_kind 维护: edge_ok, cascade_fallback, upstream_error 三个计数器
→ 计算 fallback_rate = cascade_fallback / (edge_ok + cascade_fallback)
→ 计算 bias = learning_rate × (fallback_rate - target_fallback)
→ 判断 edge_trusted: verified ≥ 3 且 fallback_rate ≤ target_fallback
```

**朴素贝叶斯分类器**（`classifier/model.rs:9`）：
```
记录条件: should_record_outcome() 确保仅端侧真正被尝试时记录
→ label_from_outcome() 映射: edge_ok → EdgeOk, fallback/error → CloudNeeded
→ 更新特征计数（step_kind, ctx_bucket, intent, tool_bucket 等 ~15 维）
→ 指数衰减老化旧观测（默认 72h 半衰期）
→ 冷启动注入领域先验（如 hard_intent → cloud: 0.75）
```

**会话状态**（`session/store.rs`）：
```
cloud_sticky_until_unix 更新:
  cascade_fallback → 续期 TTL
  upstream_error（非心跳） → 续期 TTL
  GATE_TOOL_ERROR_STREAK → force_cloud_sticky
  edge_ok && !fallback && !error → 清除粘性
```

##### 11.6 自适应调优

`compute_effective_routing()`（`adaptive.rs:37`）每 30s/40 请求根据历史统计微调关键参数：

| 系统状态 | verify_sample_rate 调整 | θ_edge 调整 | 效果 |
|---------|----------------------|------------|------|
| **健康**: 回退率 ≤ 目标×0.75 且信任步态 ≥ 20% | 降低至 base×0.65~1.0（根据信任比率缩放） | 放宽 -max_shift（最多 0.05） | 减少不必要的校验，提高端侧使用率 |
| **略吃力**: 回退率 > 目标但 ≤ 目标×1.35 | 升高至 base×1.15 | 不变 | 轻微加强校验 |
| **吃力**: 回退率 > 目标×1.35 | 升高至 base×1.45 | 收紧 +max_shift×0.6 | 强化校验，保障准确 |
| **级联回退率 > 28%** | 额外 ×1.12 | 不变 | 从 Cascade 维度额外收紧 |

**安全边界**：InitialPlan、复杂多模态、Hard Gates 命中、高难度走云/级联 — 不被自适应放宽。

##### 11.7 准确率保障全景图

```
┌─────────────────────────────────────────────────────────────────────────┐
│                     准确率保障体系                                         │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  ┌─ 前置路由层 ─────────────────────────────────────────────────────┐  │
│  │  Hard Gates: 预设高风险场景直接走云（不经过端侧质量评估）           │  │
│  │  难度评分: 高难度请求倾向 Cloud/Cascade                           │  │
│  │  Profile 策略: θ_edge / θ_cloud 阈值控制什么时候信端侧             │  │
│  └────────────────────────────────────────────────────────────────────┘  │
│                                ▼                                         │
│  ┌─ 执行质量层 ─────────────────────────────────────────────────────┐  │
│  │  Cascade Gate: 端侧产出必须≥8字/非空tool_calls || 不含"不确定"     │  │
│  │  Cloud Verify: 端+云双执行，Jaccard≥12% 或 tool 名精确匹配才信任   │  │
│  │  Multimodal Probe: 未知模型通过实际试错学习视觉能力               │  │
│  └────────────────────────────────────────────────────────────────────┘  │
│                                ▼                                         │
│  ┌─ 学习反馈层 ─────────────────────────────────────────────────────┐  │
│  │  Experience: 按步态统计 fallback_rate，产出难度偏置和边缘信任      │  │
│  │  Classifier: 多维特征贝叶斯模型，冷启动有领域先验，72h半衰期衰减   │  │
│  │  Session: 粘性追踪防止连续错误                                      │  │
│  └────────────────────────────────────────────────────────────────────┘  │
│                                ▼                                         │
│  ┌─ 自适应调优层 ──────────────────────────────────────────────────┐  │
│  │  健康→降低校验率/放宽阈值，吃力→提高校验率/收紧阈值               │  │
│  │  级联回退>28%时额外收紧，InitialPlan等安全边界不受影响             │  │
│  └────────────────────────────────────────────────────────────────────┘  │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

**核心逻辑总结**：

| 问题 | 答案 |
|------|------|
| **怎么判定端侧任务成功？** | `cascade_gate_pass()` — 非空内容≥8字且不含"不确定"，或合法 tool_calls |
| **什么时候需要升云？** | Cascade: 质量门不通过；Verify: 云端校验不信任端侧输出；Probe: 端侧不支持多模态；EdgeBusy/复杂多模态: 决策层直接走云 |
| **怎么保证最终准确率？** | 四层保障：① Hard Gates 前置过滤 ② Cascade Gate + Cloud Verify 执行质量把关 ③ Experience + Classifier 持续学习 ④ Adaptive Tuner 实时调参 |

---

## 1. 安装

需要 [Rust](https://rustup.rs/)（`cargo` 可用）。

```bash
git clone <your-repo-url> token-router
cd token-router
cargo build --release
# 或
make release
```

| 二进制 | 路径 |
|--------|------|
| token-router | `target/release/token-router` |

**加入 PATH**

```bash
export PATH="$PWD/target/release:$PATH"
# 或
make install
cp target/release/token-router ~/.local/bin/
```

**Windows（PowerShell）**：`$env:Path += ";$PWD\target\release"`

### Electron / 动态库

可将 Gateway **嵌入 Electron 主进程**（无需单独启动 CLI 守护进程）：

```bash
make release-dylib
# Windows: target/release/token_router.dll
# macOS:   target/release/libtoken_router.dylib
# Linux:   target/release/libtoken_router.so
```

C 头文件：`ffi/token_router.h`。Node/Electron 可通过 [koffi](https://github.com/Koromix/koffi) 等加载 DLL 并调用：

| 函数 | 说明 |
|------|------|
| `token_router_start(config_path, err, err_len)` | 后台启动 Gateway；`config_path` 可为 `NULL`（默认 `~/.token-router/config.toml`） |
| `token_router_stop(err, err_len)` | 停止并等待线程退出 |
| `token_router_is_running()` | 是否运行中 |
| `token_router_gateway_url(buf, len)` | 写入 `http://host:port` |
| `token_router_version()` | 库版本 |

最小示例见 `example/electron/`（`npm install && node main.mjs [config.toml]`）。

---

开发调试可用 `cargo run -- gateway start` 或 `make start`：

```bash
make start
# 等价于 token-router gateway start
```

---

## 2. 配置文件

所有业务配置写在 **TOML** 中，不使用 `TOKEN_*` 环境变量（日志级别除外：`RUST_LOG=token_router=debug`）。

| 系统 | 配置路径 |
|------|----------|
| Linux / macOS | `~/.token-router/config.toml` |
| Windows | `%USERPROFILE%\.token-router\config.toml` |

```
~/.token-router/
  config.toml       # 主配置
  gateway.pid       # 守护进程 PID
  stats.json        # 路由/流量累计统计（持久化）
  experience.json   # 按 step_kind 的隐式路由经验
  sessions/         # 每会话状态（含 cloud_sticky）
  logs/gateway.log  # Gateway 日志
```

**示例配置**

| 文件 | 用途 |
|------|------|
| [example/config.toml](./example/config.toml) | 推荐：Ollama + DeepSeek，`route = auto` |
| [example/config.economy.toml](./example/config.economy.toml) | 提高端侧占比：`economy` + 自适应 |
| [example/config.edge-only.toml](./example/config.edge-only.toml) | 固定端侧 |
| [example/config.minimal.toml](./example/config.minimal.toml) | 最小模板 |

```bash
mkdir -p ~/.token-router
token-router setup                    # 交互式填写云端/端侧配置
# 或复制示例后编辑
cp example/config.toml ~/.token-router/config.toml

# 或指定路径
token-router --config example/config.toml setup
token-router --config example/config.toml gateway start
make setup CONFIG=example/config.toml
make start CONFIG=example/config.toml
```

首次 `token-router gateway start` 时若 `config.toml` 不存在，会自动写入默认模板。

---

## 3. 使用流程

### 3.1 启动 Gateway

```bash
token-router gateway start
# 或 make start
```

首次启动示例输出：

```text
Created config at /home/you/.token-router/config.toml — edit upstream sections, then restart if needed.
gateway started (pid 12345, listen 127.0.0.1:11080, profile balanced)
```

### 3.2 初始化上游（setup）

```bash
token-router setup                                          # 交互式向导（默认）
token-router setup --cloud-url https://api.deepseek.com/v1 --cloud-key sk-...  # 非交互
token-router setup --edge-url http://127.0.0.1:11434/v1 --edge-model qwen3
token-router setup --remote                                 # 交互式，热更新运行中的 Gateway
token-router setup --non-interactive                        # 仅写入默认模板（cloud model=auto，edge 空）
token-router setup --reset                                    # 恢复默认（cloud model=auto，edge 清空）
token-router setup --json                                     # JSON 输出（跳过交互）
token-router setup --agent-id hermes --cloud-token-budget 500000  # 为指定 agent 设置云端 token 预算
```

**Web 配置页**：浏览器打开 `http://127.0.0.1:11080/setup`（地址与 `gateway.listen` 一致）。页面可查看/保存 **上游**（edge/cloud URL、模型、API Key）、**Agent 专属配置**（agent_id + cloud_token_budget）与 **Gateway**（`route`、`ctx_edge_max_tokens`、`cloud_sticky_ttl_secs`、经验/自适应/校验率等）；若配置了 `admin_token`，保存与「恢复默认」需在页面填写 Admin Token（等同请求头 `X-Token-Router-Admin-Token`）。

默认值：**云端** `model = auto`（转发时保留客户端请求的 model，由 Flowy 路由）；**端侧** 未配置（`edge` 段为空）。

### 3.3 编辑配置并重启

至少配置一侧上游的 `base_url`（可用 `token-router setup` 或 Web 页，或手改 `config.toml`）。本地改文件后：

```bash
token-router gateway restart
token-router env
```

**上游可用性**：`[upstream.edge]` 与 `[upstream.cloud]` 至少配置一侧，否则聊天接口返回 **503**。

### 3.4 查看状态

```bash
token-router gateway status
make gateway-status
```

**停止 / 重启**：`token-router gateway stop [--force]`、`token-router gateway restart`

日志写入 `~/.token-router/logs/gateway.log`；调试时可 `tail -f` 该文件。

### 3.5 curl 验证

```bash
curl -s http://127.0.0.1:11080/health

curl -s http://127.0.0.1:11080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "flowy-auto",
    "messages": [{"role": "user", "content": "[OpenClaw heartbeat poll]"}]
  }' | jq '{route:.token_router_meta.route,step_kind:.token_router_meta.step_kind,reason_codes:.token_router_meta.reason_codes}'
```

流式：`"stream": true` 时，响应头含 `X-Token-Router-Route`、`X-Token-Router-Step-Kind` 等（与非流式 `token_router_meta` 同源）。

若配置了 `gateway.api_key`，须加 `-H "Authorization: Bearer <key>"`。

### 3.6 接入 OpenClaw

编辑 `~/.openclaw/openclaw.json`：

```json
{
  "models": {
    "providers": {
      "flowy": {
        "baseUrl": "http://127.0.0.1:11080/v1",
        "apiKey": "",
        "models": [{ "id": "flowy-auto", "name": "Flowy Auto Route" }]
      }
    }
  }
}
```

- `baseUrl` 须与 `gateway.listen` 一致
- `apiKey` 选填：仅当配置了 `gateway.api_key` 时须一致

### 3.7 接入 Hermes

```bash
# hermes setup model → Custom OpenAI-compatible endpoint
# base_url: http://127.0.0.1:11080/v1
```

---

## 4. CLI 命令

| 命令 | 说明 |
|------|------|
| `token-router setup [--remote] [--non-interactive] [--reset] [--json] [--agent-id <id>] [--cloud-token-budget <n>]` | 交互式配置上游与 Agent 预算（或 CLI 参数非交互） |
| `token-router gateway start [--wait N]` | 后台启动 |
| `token-router gateway stop [--force]` | 停止 |
| `token-router gateway status [--json]` | 运行状态 |
| `token-router gateway restart [--wait N]` | 重启 |
| `token-router env [--json]` | 路径与解析后的配置 |
| `token-router stats [--json] [--lang en\|zh]` | **当前进程**会话统计 |
| `token-router stats --global [--json] [--lang en\|zh]` | **全部历史**（`stats.json`，gateway 未运行也可读盘） |

全局参数：`--config <path>`

**Makefile 快捷目标**：`make help`、`make test`、`make setup`、`make stats`、`make stats-zh`、`make stats-global-zh`

```bash
token-router stats --lang zh          # 中文格式化输出
token-router stats --global --lang zh # 全局累计 + 中文
```

`token-router stats` 输出包含：请求量、路由分布、上游 Token（输入/输出/缓存）、Cloud Input Saved、延迟（TTFT/TPS）、经验学习、**自适应路由（运行时）**、**Agent 云端 Token 预算** 等分区。

---

## 5. HTTP 端点

| 方法 | 路径 | 说明 |
|------|------|------|
| `GET` | `/setup` | 上游配置 Web 页（浏览器） |
| `GET` | `/v1/admin/setup` | 配置 JSON：`gateway`（路由/经验/自适应）、`edge`、`cloud` |
| `POST` | `/v1/admin/setup` | 热更新 `gateway` / `edge` / `cloud`（写回 `config.toml`）；可选 `X-Token-Router-Admin-Token` |
| `POST` | `/v1/admin/setup/init` | 恢复默认（cloud model=auto，edge 空）；可选 Admin Token |
| `GET` | `/health` | 存活与上游是否已配置 |
| `GET` | `/v1/admin/status` | 守护进程详情 |
| `GET` | `/v1/admin/stats` | 统计；`?scope=global` 为全部历史 |
| `POST` | `/v1/admin/shutdown` | 优雅关闭；可选 `X-Token-Router-Admin-Token` |
| `POST` | `/v1/admin/restart` | 优雅关闭并由独立 `__restart-wait` 进程拉起新实例（同 `token-router gateway restart`）；可选 Admin Token |
| `POST` | `/v1/chat/completions` | OpenAI Chat Completions 兼容（Agent 主入口） |
| `POST` | `/v1/responses` | OpenAI Responses API 兼容 |
| `POST` | `/v1/messages` | Anthropic Messages API 兼容 |

三种 LLM API 入口共享同一路由引擎与 edge/cloud 上游配置；请求在网关内归一化为 OpenAI Chat Completions 格式后转发至 `{base_url}/chat/completions`。Agent 可按 SDK 习惯选择入口（OpenAI SDK → `/v1/chat/completions`，Codex/Responses 客户端 → `/v1/responses`，Anthropic SDK → `/v1/messages`）。`token_router_meta` 扩展字段仅出现在 OpenAI Chat Completions 响应中。

**响应扩展**（非流式 JSON 根上的 `token_router_meta`，Agent 可忽略；仅 `/v1/chat/completions`）：

| 字段 | 含义 |
|------|------|
| `route` | 实际服务路径：`edge` / `cloud` / `cascade` |
| `step_kind` | 推断步态，如 `direct_chat`、`tool_select`、`heartbeat_ack` |
| `reason_codes` | 路由原因列表（门控、难度、Work、粘性、多模态等） |
| `fallback` | 是否发生过级联升云 |
| `difficulty_score` | 难度分 \(d \in [0,1]\) |
| `profile` | 生效 profile |
| `tokens_in` / `tokens_out` | 本响应 token 统计 |
| `cloud_input_saved` | 若走端侧，估算少送进云的输入 token |

```json
{
  "token_router_meta": {
    "route": "cascade",
    "fallback": false,
    "difficulty_score": 0.35,
    "step_kind": "direct_chat",
    "reason_codes": ["STEP_DIRECT_CHAT", "DIFFICULTY_0.35", "TOK_IN_120"],
    "tokens_in": 120,
    "tokens_out": 48,
    "cloud_input_saved": 60,
    "profile": "balanced"
  }
}
```

### 5.1 Setup API 调用示例

**GET 查询当前配置**

```bash
# 全局配置
curl -s http://127.0.0.1:11080/v1/admin/setup | jq .

# 指定 agent 配置
curl -s 'http://127.0.0.1:11080/v1/admin/setup?agent_id=hermes' | jq .
```

**POST 热更新 Gateway 参数**（立即生效，无需重启）

```bash
# 修改路由模式 + 端侧上下文上限 + 经验学习开关
curl -s http://127.0.0.1:11080/v1/admin/setup \
  -H "Content-Type: application/json" \
  -d '{
    "gateway": {
      "route": "auto",
      "routing_mode": "cascade",
      "default_profile": "economy",
      "ctx_edge_max_tokens": 131072,
      "experience_enabled": true,
      "experience_learning_rate": 0.1,
      "work_verify_sample_rate": 0.15,
      "adaptive_routing_enabled": true,
      "classifier_enabled": true
    }
  }' | jq .

# 切换到全部端侧
curl -s http://127.0.0.1:11080/v1/admin/setup \
  -H "Content-Type: application/json" \
  -d '{"gateway": {"route": "edge"}}' | jq .
```

**POST 配置全局上游**

```bash
# 只配置云端
curl -s http://127.0.0.1:11080/v1/admin/setup \
  -H "Content-Type: application/json" \
  -d '{
    "cloud": {
      "base_url": "https://api.deepseek.com/v1",
      "api_key": "sk-xxx",
      "model": "deepseek-chat"
    }
  }' | jq .

# 只配置端侧
curl -s http://127.0.0.1:11080/v1/admin/setup \
  -H "Content-Type: application/json" \
  -d '{
    "edge": {
      "base_url": "http://127.0.0.1:11434/v1",
      "model": "qwen3:8b"
    }
  }' | jq .

# 同时修改 gateway + 双上游
curl -s http://127.0.0.1:11080/v1/admin/setup \
  -H "Content-Type: application/json" \
  -d '{
    "gateway": {"default_profile": "economy"},
    "cloud": {"base_url": "https://api.deepseek.com/v1", "api_key": "sk-xxx"},
    "edge": {"base_url": "http://127.0.0.1:11434/v1", "model": "qwen3:8b"}
  }' | jq .
```

**POST 配置 Agent 专属上游 + Token 预算**

```bash
# 为 agent 配置完整专属上游 + 预算
curl -s http://127.0.0.1:11080/v1/admin/setup \
  -H "Content-Type: application/json" \
  -d '{
    "agent_id": "hermes",
    "cloud": {
      "base_url": "https://api.anthropic.com/v1",
      "api_key": "sk-ant-xxx",
      "model": "claude-sonnet",
      "token_budget": 500000
    },
    "edge": {
      "base_url": "http://127.0.0.1:11435/v1",
      "model": "qwen3:8b"
    }
  }' | jq .

# 仅设预算，不改上游
curl -s http://127.0.0.1:11080/v1/admin/setup \
  -H "Content-Type: application/json" \
  -d '{"agent_id": "hermes", "cloud": {"token_budget": 500000}}' | jq .

# 取消预算（0 = 不限）
curl -s http://127.0.0.1:11080/v1/admin/setup \
  -H "Content-Type: application/json" \
  -d '{"agent_id": "hermes", "cloud": {"token_budget": 0}}' | jq .

# 清除 agent 全部自定义配置（预算 + 专属上游）
curl -s http://127.0.0.1:11080/v1/admin/setup \
  -H "Content-Type: application/json" \
  -d '{"agent_id": "hermes", "cloud": {"token_budget": null, "clear": true}, "edge": {"clear": true}}' | jq .
```

**POST 恢复默认配置**

```bash
curl -s -X POST http://127.0.0.1:11080/v1/admin/setup/init | jq .
```

> 若 `config.toml` 中配置了 `gateway.admin_token`，以上 POST 请求须加 `-H "X-Token-Router-Admin-Token: <token>"`。

---

## 6. 配置字段说明

完整示例见 [example/config.toml](./example/config.toml)。

### 6.1 `[gateway]`

| 字段 | 默认值 | 说明 |
|------|--------|------|
| `listen` | `127.0.0.1:11080` | 监听地址；Agent `baseUrl` = `http://{listen}/v1` |
| `route` | `auto` | `auto` / `edge` / `cloud` / `cascade` |
| `routing_mode` | `cascade` | 仅 `route=auto`：`single` / `cascade` / `split` |
| `default_profile` | `balanced` | `economy` / `balanced` / `premium` / `privacy` |
| `ctx_edge_max_tokens` | `65536` | 端侧上下文上限；超过约 80% 触发 `GATE_CTX_OVERFLOW`；**可热更新** |
| `cloud_sticky_ttl_secs` | `600` | 见 §7.3；**可热更新** |
| `route` / `routing_mode` / `default_profile` | 见上 | **可热更新** |
| `experience_*` / `work_verify_sample_rate` / `adaptive_*` | 见示例 | **可热更新**（`POST /v1/admin/setup` 的 `gateway` 对象） |
| `session_persist_enabled` | `true` | 会话写入 `sessions/`；**改后需重启 Gateway** |
| `session_retention_days` | `7` | 过期 session 保留天数（`cloud_sticky` 失效且超期后删除）；`0` = 不删过期项；**改后需重启** |
| `session_cleanup_interval_secs` | `3600` | `sessions/` 扫描间隔（秒）；**改后需重启 Gateway** |
| `api_key` | — | 选填；入站鉴权 |
| `admin_token` | — | 选填；保护 shutdown、restart 与 setup 写操作 |
| `experience_enabled` | `true` | 按 `step_kind` 隐式学习 |
| `experience_learning_rate` | `0.08` | 经验偏置学习强度 |
| `experience_max_bias` | `0.12` | 单步态难度偏置上限 |
| `experience_target_fallback` | `0.15` | 级联升云目标比例（自适应路由参考） |
| `session_persist_enabled` | `true` | 会话写入 `sessions/`（含 `cloud_sticky_until_unix`） |
| `work_verify_sample_rate` | `0.1` | Work 步态云端校验抽样率（0–1） |
| `adaptive_routing_enabled` | `true` | 运行时自适应微调（见 §7.4） |
| `adaptive_min_verified_samples` | `20` | 预热期：云验证样本不足时用配置基线 |
| `adaptive_verify_rate_floor` | `0.05` | 校验率下限 |
| `adaptive_verify_rate_ceiling` | `0.45` | 校验率上限 |
| `adaptive_max_theta_shift` | `0.05` | 健康时 θ 最大放宽幅度 |

#### `gateway.route`

| 值 | 行为 |
|----|------|
| `auto` | 按难度 + profile + routing_mode 选择 |
| `edge` | 全部端侧（仅 `[upstream.edge]`，失败即报错，不升云） |
| `cloud` | 全部云端（仅 `[upstream.cloud]`，失败即报错，不降端） |
| `cascade` | 每请求先 edge，质量不过关再升 cloud |

#### `gateway.routing_mode`（仅 `route = auto`）

结合 `balanced` profile（约 θ_edge=0.35、θ_cloud=0.55）：

| 值 | 难度低 | 难度中 | 难度高 |
|----|--------|--------|--------|
| `single` | edge | cloud | cloud |
| `cascade` | edge | **先 edge，可能升 cloud** | cloud |
| `split` | cloud | cloud | cloud |

#### `default_profile`

| Profile | θ_edge | θ_cloud | 说明 |
|---------|--------|---------|------|
| `economy` | 0.40 | 0.60 | 更多走端 |
| `balanced` | 0.35 | 0.55 | 默认 |
| `premium` | 0.25 | 0.45 | 更多走云 |
| `privacy` | — | — | 尽量 edge |

### 6.2 `[upstream.edge]` / `[upstream.cloud]`

| 字段 | 说明 |
|------|------|
| `base_url` | OpenAI 兼容 API 根路径，**须含 `/v1`**；空表示未配置 |
| `api_key` | 选填；转发时附带 `Authorization: Bearer` |
| `model` | 选填；上游模型 id。云端默认 `auto` = 不覆盖客户端 model；端侧通常填具体模型名 |

至少配置一侧。只配 edge 时全部走端侧；只配 cloud 时全部走云端。

### 6.3 `[agent.<id>]`

为特定 Agent 设置专属上游和云端 token 预算。客户端在请求头中设置 `X-Agent-Id` 来标识 agent，Agent 配置为部分覆盖：未设置的字段回退到默认 `[upstream.*]`。

| 字段 | 说明 |
|------|------|
| `cloud_token_budget` | 5 小时滑动窗口内该 agent 的云端累计 token 预算；超过预算时 Cloud 路由降级为 Cascade（优先 edge）。`0` = 不限，`None` = 使用全局默认（无预算限制） |
| `upstream.edge.base_url` | 该 agent 专属端侧 API |
| `upstream.edge.api_key` | 专属端侧 key |
| `upstream.edge.model` | 专属端侧模型 |
| `upstream.cloud.base_url` | 该 agent 专属云端 API |
| `upstream.cloud.api_key` | 专属云端 key |
| `upstream.cloud.model` | 专属云端模型 |

```toml
[agent.hermes]
cloud_token_budget = 500000   # 5 小时窗口内云端累计 token 预算

[agent.hermes.upstream.cloud]
base_url = "https://api.anthropic.com/v1"
api_key = "sk-ant-..."
model = "claude-sonnet"
```

**预算超限行为**：当 agent 在当前 5 小时窗口内的云端 token 用量（估计值）接近或超过 `cloud_token_budget` 时，路由引擎将原本的 `Cloud` 决策降为 `Cascade`（先走 edge，质量不过关再升云），从而控制云端费用。Setup API 调用示例见 [§5.1](#51-setup-api-调用示例)。

### 6.4 `[cli]`

| 字段 | 说明 |
|------|------|
| `gateway_url` | CLI 访问 Gateway 的 URL，默认 `http://{gateway.listen}` |

### 6.5 常用组合

```toml
# 智能路由 + 级联（OpenClaw 推荐）
route = "auto"
routing_mode = "cascade"
default_profile = "balanced"

# 全部本地
route = "edge"

# 全部云端
route = "cloud"

# 提高端侧比例：economy + 开启自适应（默认已开）
default_profile = "economy"
adaptive_routing_enabled = true

# 日常尽量固定端侧（仍标 direct_chat，但 route 恒为 edge）
# routing_mode = "single"
# default_profile = "economy"
```

### 6.6 日常 / 心跳 vs Agent 任务（速查）

| 目标 | 建议 |
|------|------|
| 日常、心跳走 **casual 步态** + 允许 **cascade** | 默认即可：`route = auto`，`routing_mode = cascade` |
| 日常尽量少升云、常走端 | `default_profile = "economy"`；或 `routing_mode = "single"` |
| 确认路由是否符合预期 | 看 `token_router_meta.step_kind` 与 `reason_codes`（[§5](#5-http-端点)、[架构-步态分类](#12-步态分类routingstep_kindrs)） |
| OpenClaw 大包仍被判成 `tool_select` | 检查 `tok_rest`、多轮是否缺 easy 关键词、是否在 tool 循环 |


---

## 7. 常见问题

**`token-router` not found** — `cargo build --release` 或将 `target/release` 加入 PATH。

**`gateway did not become healthy within 30s`** — 检查端口占用；查看 `~/.token-router/logs/gateway.log`；确认 `listen` 与 `cli.gateway_url` 一致。

**Agent 无真实回复** — 确认上游 `base_url` 可达；未配置任何上游时返回 503。

**OpenClaw 日常「看起来」仍走云**（排障）— 正常标为 casual 时 `step_kind` 应为 `direct_chat` 或 `heartbeat_ack`，决策上多为 **edge**（见 [架构-步态分类](#12-步态分类routingstep_kindrs)）。用 `token_router_meta` 区分：

1. **`step_kind` 不是 casual** → 未进日常步态，常直接云或走 Work。查：`tool_select` / `initial_plan`（tool 循环中且最新 user 无 easy 关键词）、`tok_rest` ≥ 8192、`n_turns` > 8、含规划意图、assistant 失败恢复。
2. **`step_kind` 对，但 `route` = `cloud`** → 查 `reason_codes` 里 `GATE_*`（含 `GATE_USER_REJECT` 用户否定纠错）、`CONFIG_ROUTE_*`，或 `premium` + 高难度分。
3. **`route` = `edge` 且 `fallback` = true** → 决策走端侧，**质量门未过**升云重答（`CASUAL_EDGE_FALLBACK`）；下一轮仍会重新路由判断，DirectChat 不受 sticky 钉云。

用户用中/英/日/韩/粤说「不对/错了/wrong/違う/틀렸어」等纠正上一轮 assistant 时，当轮 `GATE_USER_REJECT` 升云；下一轮正常跟帖仍按 DirectChat 规则重判。

Work 步态出现 `GATE_CTX_OVERFLOW` 时可调大 `ctx_edge_max_tokens`（casual 溢出门控只计 `tok_rest`）。

**日常 meta 显示 edge 但答案来自云** — 见上第 3 点：`route=edge` + `fallback=true` 表示端侧质量门失败后升云，非决策错误。

**粘性期内全是云** — 已移除 `GATE_STICKY_CLOUD`；**日常/心跳** 不受粘性改路；**Work 执行** 在粘性期走 `STICKY_CASCADE_RETRY`（先端后云）。端侧成功会清除 sticky。

**停止无效** — `token-router gateway stop --force`

**stats 里「已持久化 false」** — 表示当前查看的是 **会话（session）** 范围，非「未写入磁盘」；`--global` 查看跨重启累计。

**Agent Token 预算超限** — 在 `token-router stats` 输出的「Agent 云端 Token 预算」分区查看当前用量。当某个 agent 的 Cloud 路由决策被预算拦截时，会降级为 Cascade。可通过 `token-router setup --agent-id <id> --cloud-token-budget <n>` 调整，或设为 `0` 取消限制。

---

## 8. 开发与测试

```bash
make test          # 或 cargo test
cargo test routing # 只测路由
make check
```

源码结构见 [架构-源码结构](#源码结构)。配置示例见 `example/`。

---

## 9. 路线图

| 阶段 | 内容 | 状态 |
|------|------|------|
| MVP | OpenAI Gateway、Profile、Single/Cascade、OpenClaw 步态、Hard Gates | ✅ |
| 可观测 | `stats.json`、`token-router stats`、Token 分解、TTFT/TPS、Cloud Input Saved | ✅ |
| 经验学习 | `experience.json`、按 step_kind 偏置与 `edge_trusted` | ✅ |
| 自适应路由 | 运行时微调校验率与 θ（experience + stats） | ✅ |
| 端侧利用率 | casual 用 `tok_rest`、不强制 edge、casual 溢出门控、粘性 Cascade、tool_calls 过关、setup 热更新 | ✅ |
| 增强 | 轻量难度分类器、Split 模式、流式 Cascade 早停、Bandit | 规划中 |
| 企业 | SSO、审计、多租户预算熔断 | 规划中 |

---

## 10. 附录

### OpenClaw system 分段标记（实现参考）

```
STATIC_END_MARKER = "# Dynamic Project Context"
INBOUND_MARKER    = "## Inbound Context"
RUNTIME_MARKER    = "## Runtime"
HEARTBEAT_USER    = /^\[OpenClaw heartbeat poll\]/
ASSISTANT_FAILED  = /\[assistant turn failed/
```

### 工具风险分级（摘要）

| 层级 | 示例工具 | 策略 |
|------|----------|------|
| Tier-1 | `exec`、`write`、`browser`、`sessions_spawn` | 强制云 |
| Tier-2 | `read`、`process`、`web_fetch` | Cascade |
| Tier-3 | `memory_get`、`session_status` | 端侧优先 |

### 指定其它配置文件

```bash
token-router --config /path/to/dev.toml gateway start
token-router --config /path/to/dev.toml stats --lang zh
```

CLI 与 Gateway 守护进程须使用 **同一份** `config.toml`。

---

**文档维护**：产品与实现变更请同步更新本 README。
