# 管理 API 契约 v1

> 状态：**契约冻结 v1（2026-11-16）**。历史背景见 [P4 客户端技术方案（已归档）](../archive/p4-client-refactor-plan.zh.md) §4。
> v3 将新增 providers/accounts/catalog/quotas/usage/playground 等资源（见 [路线图](../roadmap.zh.md)），遵循同样的只增不改规则。
> 规则：契约冻结后端点/字段**只增不改**；删除/重命名需要 `api_version + 1` 与弃用期。
> 类型生成：Rust DTO 用 `ts-rs` 导出 TS 类型到 `apps/desktop/frontend/src/api/types.ts`；
> `cargo xtask contract-check` 校验生成物无 diff（P4-003）。

## 1. 基本约定

- 前缀 `/v2/admin`；JSON 编码；时间为 Unix 毫秒；金额为 micros；ID 为字符串。
- 响应头：`x-request-id`（每个请求）、`cache-control: no-store`。
- 鉴权：`Authorization: Bearer <token>`（兼容 `x-admin-token`）；scope 见 §3。
- 分页：`?limit=50&cursor=<opaque>` → `{"items":[...],"next_cursor":"..."}`；
  游标为 `(created_at, id)` 的 base64 编码；非法游标 → `400 invalid_request`。
- 时间窗：`from`/`to`（毫秒，左闭右开）；缺省最近 24h；最大 90 天（超过 → `413 window_too_large`）。
- 错误统一信封：

```json
{
  "error": {
    "type": "config_invalid",
    "message": "candidates[1].strength must be 0..=100",
    "request_id": "01J...",
    "issues": [
      {"path": "candidates[1].strength", "code": "out_of_range", "message": "got 120"}
    ]
  }
}
```

错误码表（集中定义于 `tr-control::error::ApiErrorType`，handler 禁止手写字符串）：

| HTTP | type | 场景 |
|---|---|---|
| 400 | `invalid_request` | 参数/游标非法 |
| 401 | `admin_auth_error` | 缺 token/错 token/已撤销 |
| 403 | `admin_scope_error` | scope 不足；远程管理未开启 |
| 404 | `not_found` | request/token/candidate 不存在 |
| 409 | `config_conflict` | `If-Match` 不匹配 |
| 413 | `window_too_large` | 统计窗口/分页超限 |
| 422 | `config_invalid` | 配置校验失败（带 `issues[]`） |
| 500 | `internal_error` | 未预期错误（带 request_id） |
| 503 | `server_unavailable` | 正在关闭/数据库忙 |

## 2. 端点总表（v1）

| 方法 | 路径 | 作用 | 作用域 |
|---|---|---|---|
| GET | `/v2/admin/status` | 版本、监听、运行时长、schema、digest、模式、计数 | read |
| GET | `/v2/admin/health` | 候选熔断状态 | read |
| GET | `/v2/admin/doctor` | 配置/DB/判官/磁盘/日志自检 | read |
| GET | `/v2/admin/config` | 可编辑配置文档（脱敏）+ revision + warnings | read |
| POST | `/v2/admin/config/validate` | 纯校验，返回规范化 TOML、digest、diff、issues | read |
| POST | `/v2/admin/config/apply` | CAS 写配置 + 备份，返回 `restart_required` | write |
| POST | `/v2/admin/config/rollback` | 回滚到上一份可用备份 | write |
| GET | `/v2/admin/config/audit` | 配置变更审计（脱敏） | read |
| GET | `/v2/admin/candidates` | 候选只读投影（**未实现**，见 §2.1） | read |
| GET | `/v2/admin/virtual-models` | 虚拟模型只读投影（**未实现**，见 §2.1） | read |
| GET | `/v2/admin/profiles` | profile 只读投影（**未实现**，见 §2.1） | read |
| GET | `/v2/admin/trust-tiers` | 别名 → profile（**未实现**，见 §2.1） | read |
| GET | `/v2/admin/requests` | 账本列表：时间/会话/模型/outcome/协议/原因码过滤 | read |
| GET | `/v2/admin/requests/{id}` | 单请求详情 | read |
| GET | `/v2/admin/stats/summary` | 时间窗汇总（`/v2/admin/stats` 为兼容别名） | read |
| GET | `/v2/admin/stats/series` | 时间序列 | read |
| GET | `/v2/admin/stats/models` | 模型维度汇总 | read |
| GET | `/v2/admin/shadow` | shadow 一致率/延迟/成本 | read |
| GET | `/v2/admin/judge/calls` | 判官调用审计（**未实现**，见 §2.1） | read |
| GET | `/v2/admin/tokens` | token 列表（脱敏） | read |
| POST | `/v2/admin/tokens` | 创建 token（明文只返回一次） | write |
| DELETE | `/v2/admin/tokens/{id}` | 撤销 token | write |
| GET | `/v2/admin/logs` | 服务端日志尾（ring buffer） | read |
| POST | `/v2/admin/shutdown` | 优雅关闭 | write |

（SSE `/v2/admin/events` 为 P6 任务，不在 v1 契约内。）

### 2.1 声明但未实现的端点（2026-11-18 核对）

`crates/control/src/lib.rs` 的路由表里没有下面五个端点，调用返回 404：

- `GET /v2/admin/candidates`
- `GET /v2/admin/virtual-models`
- `GET /v2/admin/profiles`
- `GET /v2/admin/trust-tiers`
- `GET /v2/admin/judge/calls`

前端目前通过 `GET /v2/admin/config` 的 `document` 读候选/虚拟模型/profile，功能上不受影响。
保留在表里是为了记下「契约承诺 vs 实现」的差距；补齐或删除都要走 `api_version` 规则。
`GET /v2/admin/shadow` 已实现，返回 `{total, agreed, disagreed, unknown, avg_latency_ms,
p95_latency_ms, total_cost_micros}`。

## 3. 作用域

| 作用域 | 允许 |
|---|---|
| `inference` | 数据面三协议、`/v1/models` |
| `admin:read` | 所有 GET 管理端点 |
| `admin:write` | `config/apply`、`config/rollback`、token 管理、shutdown |

- token 随机 32 字节；库中只存 `sha256`；列表只返回 `prefix`（前 8 字符）与 `last_used_at`。
- 首次启动且 `tokens` 表为空时，生成全 scope admin token，明文写入 `<data_dir>/admin.token`（0600）。
- 撤销立即生效；比较常数时间；`last_used_at` 每分钟最多回写一次。

## 4. DTO（`tr-contracts::admin` 生成 TS）

命名与字段以 `v2/crates/contracts/src/admin.rs` 的 `#[derive(TS)]` 为准；
本节给出关键形状供评审。生成门禁保证两者不漂移。

### StatusResponse

```json
{
  "api_version": 1,
  "version": "0.20.0",
  "started_at": 1763000000000,
  "uptime_ms": 12345,
  "listen": "127.0.0.1:17621",
  "schema_version": 1,
  "db_schema_version": 9,
  "config_digest": "6f2c...",
  "default_virtual_model": "auto",
  "modes": {"judge": "off", "cost_gate": "shadow"},
  "requests_total": 128,
  "breakers": {"closed": 2, "half_open": 0, "open": 1}
}
```

### HealthResponse

`items[]`：`candidate_id/status(closed|half_open|open)/consecutive_failures/opened_count/open_until/half_open_since/last_error_class/last_failure_at/updated_at` + `counts{closed,half_open,open}`。

### RequestSummary（列表项）

`request_id/created_at/session_key/branch_id/ingress_protocol/selection_model/dispatch_model/dispatch_source/outcome/http_status/ttft_ms/duration_ms/tokens{input,cache_read,output}/cost_micros{answer,routing}/reason_codes[]/plan_id`。

### RequestDetail

在列表字段外增加：`task_id/session_key_source/collision_suspect/selection_level/dispatch_provider/dispatch_level/plan_mode/preference/failover/attempts/switches/cache_write_tokens/facts_completeness/profile_revision/config_digest/judge/contributions/authority`。
不返回会话内容/消息正文。

### StatsSummary

`requests/successes/success_rate/upstream_errors/quality_fails/cancelled/rejected/switches/tokens{input,output,cache_read,cache_write}/cost_micros{answer,routing}/latency{ttft_p50,ttft_p95,duration_p95}/judge{calls,failed,cost_micros}`。

### SeriesResponse

`bucket(hour|day)/from/to/group_by(virtual_model|dispatch_model|outcome|protocol)/metric(requests|successes|cost|input_tokens|output_tokens)/series[{key,points[{t,value}]}]`。

### ConfigDocument

见技术方案 §4.5；`credential` 只返回引用（`env:`/`file:`），字面量一律 `redacted:literal`；
`revision` = 规范化 JSON SHA-256（裸 hex，与账本 `config_digest` 同源）。

### ValidateResponse

`{valid, revision, normalized_toml, issues[{path,code,message}], diff[{path,from,to}]}`；
`validate` 对可解析但无效的文档返回 `200 + valid:false`；`apply` 返回 `422 config_invalid`。

### ApplyRequest / ApplyResponse

`apply` 必须带 `If-Match: <revision>` 头；body `{document_toml: string, restart: "manual"|"auto"}`；
响应 `{revision, restart_required: true, backup_path}`。不匹配 → `409 config_conflict`。

### TokenInfo / CreatedToken

列表项：`id/name/prefix/scopes/created_at/last_used_at/expires_at/revoked_at`；
创建响应额外含一次性 `token` 明文。

### LogEntry

`{seq, at, level, target, message}`；查询 `?cursor&limit&level&target`；脱敏见技术方案 §4.10。

## 5. 兼容规则

- `GET /v2/admin/status` 的 `api_version: 1`；新字段只增不改；客户端忽略未知字段。
- 配置 schema 独立演进：DB `schema_version`（当前迁移 10）与配置文件 `schema_version` 分开计数。配置文件当前为 `1`（代码严格校验，见 `tr-agent::config::SCHEMA_VERSION`；勘误 2026-11-17：原文误写"P4 起 2"）。

### 5.1 不兼容变更记录

产品尚未发布，「只增不改」的舍入在 2026-11-18 由维护者放弃，允许删除端点/字段。

| 日期 | 变更 | 影响 |
|---|---|---|
| 2026-11-18 | 删除 `/v2/admin/approvals`（GET/POST） | 审批模块整体移除 |
| 2026-11-18 | `ModeSummary` 去掉 `approvals` 字段 | `GET /v2/admin/status` 响应少一个键 |
| 2026-11-18 | `AdminScope` 去掉 `admin_approve` | 已存 token 里的该 scope 在读取时被静默丢弃 |
| 2026-11-18 | DB migration 10 `DROP TABLE approvals` | 审批审计记录永久删除 |

`api_version` 保持 `1`：没有外部消费者，不引入版本协商成本。
