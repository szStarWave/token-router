# PRD：密钥与信任边界

> 状态：现行需求（已实现，2026-11-17）；P5-A/P5-B 增量指针见 §7。
> 语义权威：[内核设计文档](../archive/rewrite-v2-design.zh.md) §8.5、§11–12、§18.4.4；管理面事实见 [契约](../contracts/admin-api.zh.md) 与 [P4 交付报告](../archive/p4-delivery-report.zh.md)。
> 产品定位：用户把全部模型 key 交给本地网关，信任边界是产品的基本盘。原则一句话：**明文 key 只存在于 Rust 侧内存与凭据存储，其余一切界面只见引用。**

## 1. 背景与目标

本地网关集中持有用户最敏感的资产（多家厂商 API key）。威胁模型不是外部攻击者，而是**意外泄漏**：日志、配置备份、API 响应、前端代码、账本导出任何一处出现明文 key 都是事故。

目标：
1. 明文 key 的存留面收敛到单点（Rust 内存 + 凭据存储）。
2. 管理面与数据面权限分离，WebView 零 token。
3. 配置变更可审计、可回滚、并发不覆盖。

## 2. 功能需求

### 2.1 凭据边界

| ID | 需求 | 验收 |
|---|---|---|
| R-SEC-01 | 配置/账本/快照/日志/API 响应中 credential 只出现引用（`env:NAME`/`file:path`/`keyring:...`） | config 脱敏 + LogRing sanitize 单测断言 0 明文 |
| R-SEC-02 | 明文 key 只在 Rust 侧内存与凭据存储；管理面写 key 后响应只含 `credRef` + 指纹后 4 位 | 写入→读取往返测试无明文 |
| R-SEC-03 | env 只许覆盖 secrets 与 listen，不许覆盖路由语义字段 | 配置加载用例 |
| R-SEC-04 | onboarding 改写 agent 配置时 token 只传引用，不复制明文 | golden 合并器测试 |

### 2.2 管理面鉴权

| ID | 需求 | 验收 |
|---|---|---|
| R-SEC-10 | scoped token：inference / admin:read / admin:write；SHA-256 存库，展示仅 prefix 前 8 位 | scope 越权 403 用例 |
| R-SEC-11 | 首启 tokens 表为空时生成全 scope bootstrap token，写 `<config目录>/admin.token` | 首启用例；文件权限说明 |
| R-SEC-12 | token 撤销立即生效 | tokens store 单测 |
| R-SEC-13 | **admin token 不进 WebView**：Rust 侧内存注入 Bearer；UI 全部经代理（路径白名单 `/v2/admin`、方法白名单、8MiB 上限） | 壳测试 + 前端无 token 静态检查 |
| R-SEC-14 | 入站数据面鉴权可选：`auth_enabled` + `inbound_key`（默认关，本机回环场景） | 开启后无 key 请求 401 |

### 2.3 审批闸（已移除，2026-11-18）

原 `[approvals]` off/observe/enforce 三档人工审批已整体删除：内核 `approval.rs`、`approvals` 表、
`/v2/admin/approvals`、`admin:approve` scope、`APPROVAL_REQUIRED`/`APPROVAL_DENIED` reason code
与 Approvals 页均不复存在（migration 10 删表）。

理由：审批闸判断的是**请求内容里有没有高危工具调用**，属于内容级拦截；本产品的边界是路由决策
（选哪个模型、为什么），不是替用户执行安全策略。保留它会让"未知字段 passthrough 保真"的协议
契约与"网关只做路由"的定位自相矛盾。

R-SEC-20 ~ R-SEC-24 全部作废。

### 2.4 配置治理（P4）

| ID | 需求 | 验收 |
|---|---|---|
| R-SEC-30 | validate 返回逐字段 issues（200+issues 不截断语义）；apply 校验失败 422 不落盘 | issues 路径精确到字段 |
| R-SEC-31 | apply 原子写 + 备份 ≤10 份 + config_audit 审计；并发修改 CAS 409 不覆盖 | 并发 apply 用例 |
| R-SEC-32 | rollback 可回退；apply 后重启生效 | 回滚 e2e |
| R-SEC-33 | 配置文档读取脱敏（plan 预览只有引用） | 脱敏用例 |

### 2.5 日志与观测脱敏

| ID | 需求 | 验收 |
|---|---|---|
| R-SEC-40 | 日志默认脱敏正文与工具参数；LogRing sanitize | 单测断言 |
| R-SEC-41 | shadow 记录 30 天脱敏；capture `--raw --encrypt` 本地加密默认关，`--redacted` 不可逐字段回放 | 文档明示代价 |
| R-SEC-42 | OTLP/导出（P5-C）凭据头永远排除 | 见 [可解释 PRD](./explainability.zh.md) R-EX-34 |

## 3. 非功能需求

- 崩溃恢复不泄漏：supervisor stderr 尾部展示经脱敏；孤儿进程清理（runtime.json）。
- 升级不覆盖数据目录（含 admin.token 与凭据存储）。
- 0 密钥出现在响应/日志是**每次提交的门禁断言**，不是抽查。

## 4. 边界

- 不做：局域网共享/mTLS/远程实例（攻击面扩大，维持非目标，见 [路线图 §7](../roadmap.zh.md)）；云备份；隐私遮蔽（P7）。
- 本机回环 + 单用户是信任假设前提；暴露到非回环地址必须开 inbound_key（文档警示）。

## 5. 用户场景

| 场景 | 期望 |
|---|---|
| 贴 key 进 UI | 响应/日志/配置备份永远只见引用+后 4 位 |
| agent 要执行危险工具 | 网关不介入内容级判断（审批模块已移除）；由 agent 自身的权限机制处理 |
| 两人同时改配置 | 后提交者 409，看到 diff，不静默覆盖 |
| 换机器 | 数据目录整体迁移，token/凭据引用关系不破坏 |

## 6. 现状与依赖

- 全部现行需求已实现（P4 交付，含脱敏单测与壳测试）。
- P5-A：Account.key 入 OS keyring/加密文件（`keyring:tr2/{account_id}`），Linux 无桌面回退加密文件——见 [路线图 §2.1/§6](../roadmap.zh.md)。
- P5-B：网关 key 限额（预算维度，429 + Retry-After）——见路线图；限额是预算不是鉴权，不改变本 PRD 的 scope 模型。