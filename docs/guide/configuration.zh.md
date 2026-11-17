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
[approvals]
[trust_tiers]
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
| `provider` | | 默认 `openai` |
| `credential` | | 凭据**引用**：`env:NAME` / `file:path` / `keyring:...`；明文不落配置 |
| `window` | | 上下文窗口（token） |
| `input` | | 模态列表：`text` / `image` / `audio` |
| `thinking_levels` | | 支持的 thinking 级别（字符串标签列表） |
| `enabled` | | 默认 `true` |
| `price` | | 价格表，见下 |

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
| `thinking_levels` | | 对外暴露的 thinking 级别 |
| `context_window` / `max_tokens` | | 可选上限 |
| `input` | | 模态列表 |

## [judge]

判官（独立决策模型），默认关闭。

| 字段 | 默认 | 说明 |
|---|---|---|
| `mode` | `off` | `off` / `shadow`（只记录不参与）/ `active` |
| `max_calls_per_turn` | 1 | 每轮预算 |
| `max_calls_per_minute` | 30 | 每分钟预算 |
| `max_calls_per_session` | 200 | 每会话预算 |
| `deadline_ms` | 300 | 判官时限；超时/失败 fail-open |

`[judge.service]`：`kind`（`local`/`http`/`llm`）、`endpoint`、`model`、`credential`（引用）、`timeout_ms`、`structured_output`（`json_schema`/`json_mode`/`tool_call`）、`temperature`、`max_output_tokens`，以及判官自身计价（micros，计入 routing_cost）。

## [cost_gate]

成本闸：`mode = "off" | "shadow" | "active"`，默认 `shadow`。

## [approvals]

工具调用审批，默认关闭。

| 字段 | 默认 | 说明 |
|---|---|---|
| `mode` | `off` | `off` / `observe` / `enforce` |
| `min_risk` | — | `low` / `medium` / `high`；达到才拦截（未知工具按 high） |
| `ttl_secs` | — | 审批结论有效期 |
| `fail_closed` | `true` | enforce 下审批不可用时拒绝（428/403） |

## [trust_tiers]

信任档位别名，只映射到 profile：`trial = "some-profile-id"`。

## 变更与审计

- 手改 TOML 后重启生效；UI 走 validate/apply（原子写 + 备份 ≤10 + CAS 409 + config_audit 审计 + rollback）。
- `config_digest`（规范化形态的 sha256）随决策快照与账本落库，可追溯"这个决定是在哪份配置下做出的"。
- 管理 API 的配置端点见 [契约](../contracts/admin-api.zh.md)。