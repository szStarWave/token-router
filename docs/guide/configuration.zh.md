# 配置参考（config.toml）

> 事实基准：`v2/crates/agent/src/config.rs`（2026-11-17）。最小示例：`v2/example/config.minimal.toml`。
> 所有段 `deny_unknown_fields`（未知字段直接报错）；`schema_version` 必须为 `1`（严格相等校验）。
> 桌面端 UI 可完成全部配置闭环（validate → apply → 重启生效，CAS 并发保护 + 审计 + 回滚），不必手改 TOML。

## 顶层

```toml
schema_version = 1                  # 必填，当前只接受 1
default_virtual_model = "auto"      # 可选；请求未指定 model 时的缺省虚拟模型

[server]
[profiles.<name>]
[[candidates]]
[[virtual_models]]
[judge]
[cost_gate]
[trust_tiers]
[recording]
```

## [server]

| 字段 | 默认 | 说明 |
|---|---|---|
| `listen` | `127.0.0.1:16621` | 网关监听地址（桌面端 supervisor 固定用 17621） |
| `auth_enabled` | `false` | 入站请求鉴权开关 |
| `auth_mode` | `inbound_key` | 鉴权模式 |
| `inbound_key` | 空 | `auth_enabled = true` 时必填的入站 key |
| `admin_token` | 空 | 管理面 token（一般不用手填：首启自动生成 bootstrap token 写 `<config目录>/admin.token`） |

环境变量只允许覆盖 secrets 与 listen。

## [profiles.\<name\>]

路由档位，被虚拟模型引用。

| 字段 | 默认 | 说明 |
|---|---|---|
| `mode` | `single` | `single` / `cascade` / `verify_after`，语义见 [架构 §4.3](../architecture.zh.md) |
| `stream_escalation` | `next_turn` | `next_turn` / `gate_before_tool` / `off` |
| `strength_order` | `[]` | tier 标签的强弱顺序，如 `["local", "cloud"]` |
| `difficulty_weight_micros` | 0 | 难度信号权重（micros） |
| `hard_signal_weight_micros` | 0 | 硬信号权重（micros） |
| `unknown_price_policy` | `quality_allow` | 缺价格时：`quality_allow`（质量升级放行、成本降级拒绝）/ `deny_all` |

## [[candidates]]

物理候选 = 一个上游模型入口。

| 字段 | 必填 | 说明 |
|---|---|---|
| `id` | ✅ | 候选唯一 id |
| `base_url` | ✅ | 上游 OpenAI 兼容 base URL |
| `model` | ✅ | 上游物理模型名 |
| `tier` | | 字符串标签（如 `local`/`cloud`），供 `strength_order` 排序，不是结论 |
| `strength` | | 0–100，profile 语境下的强度声明 |
| `provider` | | 上游 wire 协议，默认 `openai`；见下 |
| `credential` | | 凭据**引用**：`env:NAME` / `file:path`；明文不落配置。`keyring:` 引用会被配置层原样保留，但上游运行时尚未实现（请求会失败），候选请只用 `env:` / `file:` |
| `proxy` | | 每候选代理 URL（仅非敏感地址；凭据仍走 `credential`） |
| `extra_headers` | | 静态非敏感 header 表；流式与非流式请求都会带上 |
| `window` | | 上下文窗口（token） |
| `input` | | 模态列表：`text` / `image` / `audio` |
| `thinking_levels` | | 支持的 thinking 档位白名单；见下 |
| `disable_thinking` | | `true` 时该候选一律按 `off` 处理（剥离 reasoning + 已知厂商关闭开关）；与组级 `disable_thinking` 取并集。桌面端在模型设置「关闭思考」 |
| `max_tokens` | | 候选级输出上限：请求 `max_tokens` 取 min(客户端值, 上限)，未给时填上限；被截断时账本记 `OUTPUT_CLAMPED`。编译时取目录「最大输出」。注意：硅基流动等厂商在思考开启时会忽略 `max_tokens`（推理 token 不计入上限），要真正限制需同时 `disable_thinking` |
| `enabled` | | 默认 `true` |
| `price` | | 价格表，见下 |

`provider` 决定发往上游的 wire 协议（不是展示标签）：

| 值 | 端点 | 鉴权 |
|---|---|---|
| `openai` / `openai_compatible` | `base_url` + `/chat/completions` | `Authorization: Bearer` |
| `responses` | `base_url` + `/responses` | `Authorization: Bearer` |
| `anthropic` | `base_url` + `/v1/messages`（base 已带 `/v1` 时只补 `/messages`） | `x-api-key` + `anthropic-version` |

三种协议的请求、响应与流式事件都由 `tr_proto` 双向转换；未知 provider 的候选直接返回 `UnsupportedProvider`，不静默降级成错误的上游调用。供应商类型 `responses` / `anthropic` 编译出的候选会写入对应的 `provider` 值。

`thinking_levels` 是候选能力声明（白名单）。请求档位来自 `x-token-router-thinking-level` header，缺省取客户端的 `reasoning_effort`（Chat / Responses；Anthropic 的 thinking budget 不是档位）。声明非空且请求档位不在列表里时，dispatch 档位记 `off`，且不把 reasoning 参数发给该候选；声明为空表示「未声明」，保持透传（不猜测能力）。

供应商目录里的 embedding / rerank 模型（按模型 id 的通行命名识别）不会进入对话模型目录，也不会编译成候选。

`[candidates.price]`（金额一律 micros 整数）：

```toml
[candidates.price]
currency = "USD"
input_micros_per_mtok = 3000000      # 每百万输入 token
output_micros_per_mtok = 15000000
cache_read_micros_per_mtok = 300000  # 可选
cache_write_micros_per_mtok = 3750000 # 可选
request_micros = 0                   # 每请求固定成本
```

缺价格时按 profile 的 `unknown_price_policy` 处理。

## [[virtual_models]]

agent 面向的命名入口。

| 字段 | 必填 | 说明 |
|---|---|---|
| `id` | ✅ | 请求 `model` 字段填它；`auto`/`token-router` 为内置 |
| `profile` | ✅ | 引用 `[profiles.<name>]` |
| `candidate_pool` | | 物理候选 id 列表（禁止嵌套虚拟模型） |
| `provider` | | 示例用 `router` |
| `name` | | 展示名 |
| `strategies` | | 附加策略 id 列表 |
| `thinking_levels` | | 对外暴露的 thinking 级别（桌面端「成员共有」= 空，跟随成员候选） |
| `disable_thinking` | | `true` 时本组请求一律按 `off` 处理：剥离 reasoning 参数，并对已知厂商附带关闭开关（桌面端「关闭思考」） |
| `context_window` | | 显式窗口提示（配合 `context_window_mode = "custom"`） |
| `context_window_mode` | | `min`（默认，取池内最小）/ `max`（取池内最大）/ `custom`（用 `context_window`）；只作路由信号 |
| `max_tokens` | | 可选上限 |
| `list_members` | | `true` 时 `/v1/models` 额外列出 `组/成员模型`；客户端可直接用该 id 选成员（仍按本组路由与账号） |
| `input` | | 模态列表 |

> `context_window_mode` / `list_members` 不写时行为与旧配置一致（不改变 `config_digest`）。
> 成员直选的 id 形如 `auto/Qwen/Qwen3.6-27B`：首个 `/` 前是组 id，其余是成员候选的上游 model 名（或候选 id）；
> 显式选择会在决策后把该候选提到首位，失败仍可在同组内接力。

`disable_thinking = true` 是组级策略：决策档位固定为 `off`（客户端原始 `reasoning_effort` 仍留在决策快照里可审计），上游请求不携带 reasoning 参数。对已知厂商再附加各自的关闭开关，未知厂商只剥离参数、不发送未确认字段：

| 厂商 | 附加字段 |
|---|---|
| 硅基流动 / DashScope（阿里百炼） | `enable_thinking: false` |
| 智谱 GLM / Z.ai | `thinking: { "type": "disabled" }` |
| OpenRouter | `reasoning: { "enabled": false }` |

Anthropic / Responses wire 不携带 thinking 即为关闭，无需附加；客户端显式传 `x-token-router-thinking-level: off`（或 `reasoning_effort` 为 `off`/`none`/`disabled`）等价生效。

### 组级判断与规则（`virtual_models.judge` / `virtual_models.rules`）

路由组就是虚拟模型本身：不新增 group 实体，也不允许嵌套。可选地为某个虚拟模型配置判断模型与规则；判断模型可以不在组内，也不会成为回答模型，只用于挑选组内优先候选。语义见 [设计文档 §7.7](../archive/rewrite-v2-design.zh.md#77-组级判断与规则agent-层扩展)。

```toml
[[virtual_models]]
id = "auto"
profile = "default"
candidate_pool = ["qwen-local", "deepseek-chat", "claude-sonnet"]

[virtual_models.judge]
mode = "active"                 # off | active（不接受 shadow）
max_calls_per_turn = 1
max_calls_per_minute = 30
max_calls_per_session = 200
deadline_ms = 300

[virtual_models.judge.service]
kind = "llm"                    # llm | jev
endpoint = "http://127.0.0.1:11435/v1"
model = "gemma-2b"
credential = "env:GROUP_JUDGE_TOKEN"   # 只存引用；off 时不解析
structured_output = "json_mode"        # 仅 llm：json_schema | json_mode | tool_call
temperature = 0.0                      # 仅 llm
max_output_tokens = 256                # 仅 llm
timeout_ms = 300

# JEV（SystemOne）示例，与上面的 llm 二选一：
# [virtual_models.judge.service]
# kind = "jev"
# endpoint = "https://api.typesafe.ai/v1/systemone"   # 完整 SystemOne 路径
# model = "jev-1.13"
# credential = "env:JEV_API_KEY"
# timeout_ms = 300

[[virtual_models.rules]]
id = "coding"
intent = "代码实现、调试或重构"
prefer = "deepseek-chat"
```

字段：

| 字段 | 默认 | 说明 |
|---|---|---|
| `judge.mode` | `off` | `off` / `active`；有规则时必须 `active` |
| `judge.max_calls_per_turn` | 1 | 每轮判断预算 |
| `judge.max_calls_per_minute` | 30 | 每分钟判断预算 |
| `judge.max_calls_per_session` | 200 | 每会话判断预算 |
| `judge.deadline_ms` | 300 | 判断时限；超时 fail-open。网络型判官（尤其 SystemOne）常需 300–500ms，实测不够时与 `judge.service.timeout_ms` 一起调到 1000 |
| `judge.service` | 同 `[judge.service]` | 复用判官服务字段；`active` 时 `kind` 允许 `llm` / `jev`。JEV 使用 SystemOne，`endpoint` 需为完整 `/v1/systemone` |
| `rules[].id` | 必填 | 规则 id，非空且组内唯一 |
| `rules[].intent` | 必填 | 规则意图描述，非空；按配置顺序发给判断模型 |
| `rules[].prefer` | 必填 | 命中后的优先候选，必须在本 VM 的 `candidate_pool` 中 |

输入与行为：

- 判断模型在组工作台直接选：下拉列出所有已编译的启用候选（显示模型名 + 供应商，值即候选 id），不要求供应商页预设 `judge_llm` / `jev` 用途；选中后自动写入本段 `service` 的 `kind` / `endpoint` / `model` / `credential` / `service_id`。普通候选写 `llm`；目录模型带 `judge:jev` 标签（供应商页「决策模型（SystemOne）」）的写 `jev`，并把 `endpoint` 归一化为 `{base_url}/v1/systemone`。判断模型不必是本组成员。
- **有规则**：判断模型输入 = 规则列表（按配置顺序）+ 最后一条 user 消息；输出命中的规则 `id` 或 `none`。
- **无规则 + `active`**：判断模型输入 = 组内回答模型能力信息 + 最后一条 user 消息；输出优先候选 `id` 或 `none`。
- LLM 与 JEV 输出**同一个契约**：一个选项 id + 可选置信度。JEV 通过 SystemOne 的 `choice` 问题返回该 id；LLM 通过结构化 JSON 返回该 id。
- 只发送最后一条 user 消息，不发送 system / tool 内容。
- 判断失败、超时、非法答案、未知规则/候选 → fail-open 到默认顺序，HTTP 仍成功。
- 组判断生效时跳过全局 `[judge]`、Sticky/ContextHold 与成本闸回搬；优先候选仍要过 Plan 校验。
- 组判断不可被 `cli replay` / `core::decide` 回放；审计落 contributions 与账本。

校验（配置错误直接拒绝启动/apply）：

- 有规则但 `mode != "active"`：``virtual model `X` 配置了 N 条规则，必须启用 judge``。
- `mode` 只允许 `off` / `active`（`shadow` 拒绝）。
- `active` 时 `service.kind` 只允许 `llm` / `jev`；`endpoint` / `model` 必填；凭据解析失败是配置错误。
  - `file:` 相对路径按**配置文件所在目录**解析（不是进程工作目录）；桌面壳 / CLI 从任意目录启动 sidecar 都能读到 `file:credentials/<account>.key`。绝对路径与 `env:` 保持原样。
  - `llm`：`structured_output` 只允许 `json_schema` / `json_mode` / `tool_call`。
  - `jev`：`endpoint` 必须是完整 SystemOne 路径（通常以 `/v1/systemone` 结尾）；`structured_output` / `temperature` / `max_output_tokens` 被忽略。
- `off` 时不解析凭据、不要求 `endpoint` / `model`。
- 规则 id 非空且组内唯一；intent 非空；`prefer` 必须在 `candidate_pool` 中。
- 无规则 + `active` 合法（能力选择模式）。

## [judge]

判官（独立决策模型），默认关闭。

| 字段 | 默认 | 说明 |
|---|---|---|
| `mode` | `off` | `off` / `shadow`（只记录不参与）/ `active` |
| `max_calls_per_turn` | 1 | 每轮预算 |
| `max_calls_per_minute` | 30 | 每分钟预算 |
| `max_calls_per_session` | 200 | 每会话预算 |
| `deadline_ms` | 300 | 判官时限；超时/失败 fail-open |

`[judge.service]`：`kind`（`local`/`http`/`llm`/`jev`）、`endpoint`、`model`、`credential`（引用，当前只解析 `env:` / `file:`；`keyring:` 会被当作字面量，请勿使用）、`timeout_ms`、`structured_output`（`json_schema`/`json_mode`/`tool_call`，仅 `llm`）、`temperature` / `max_output_tokens`（仅 `llm`）、`disable_thinking`（默认 `true`），以及判官自身计价（micros，计入 routing_cost）。`kind = "jev"` 时 `endpoint` 为完整 `/v1/systemone` 路径；SystemOne 同样返回一个 choice 答案。

`disable_thinking` 只对 `kind = "llm"` 有意义：默认关闭思考，按 endpoint 主机注入厂商开关
（硅基流动/DashScope `enable_thinking:false`、智谱/Z.ai `thinking:{type:"disabled"}`、
OpenRouter `reasoning:{enabled:false}`）。它**不受候选/提供商配置与客户端思考档位影响**——
判官服务只读自己的配置；关掉它对会思考的模型能把判官延迟从几十秒压到几秒。

## [cost_gate]

成本闸：`mode = "off" | "shadow" | "active"`，默认 `shadow`。

> `[approvals]` 段已于 2026-11-18 随审批模块整体移除。旧配置里出现该段会因
> `deny_unknown_fields` 报错，删掉即可。

## [trust_tiers]

信任档位别名，只映射到 profile：`trial = "some-profile-id"`。

## [recording]

本地记录策略：`transcript = true | false`（默认 `true`）。

- `true`：每轮请求的**消息正文**（发给上游的 messages + 模型回复）写入本地库
  `request_transcripts`，供「活动 → 会话」按轮查看；账本本身仍不存正文。
- 只存展平文本：图片/音频变占位符（不存 base64），工具调用/结果展开为文本；
  单条 20,000 字符、单轮 200,000 字符，超出截断并标记。
- 关闭后新请求不再记录，历史记录保留。消息只在本机数据库中，不随管理 API 外发。
- 桌面端入口：设置 → 存储与安全 → 「记录消息正文」（改这一项并自动保存）。

## 变更与审计

- 手改 TOML 后重启生效；UI 走 validate/apply（原子写 + 备份 ≤10 + CAS 409 + config_audit 审计 + rollback）。
- `config_digest`（规范化形态的 sha256）随决策快照与账本落库，可追溯"这个决定是在哪份配置下做出的"。
- 管理 API 的配置端点见 [契约](../contracts/admin-api.zh.md)。