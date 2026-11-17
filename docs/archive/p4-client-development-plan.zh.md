> **已归档（2026-11-17）**：本文档为历史档案，不再维护。现行文档见 [docs/README.md](../README.md)。

# P4 客户端开发计划

> 状态：**开发计划 v1（2026-11-16），待评审，未开工**。
> 配套：[`p4-client-refactor-plan.zh.md`](./p4-client-refactor-plan.zh.md)（技术方案）。
>
> 前提：v2 是全新产品线。P4 客户端从零配置，**不做 v1 配置/数据迁移、不做双跑/回退、不追求 v1 功能对齐**。
> P4 不在 S0–P3 验收范围内；本计划是 P4 的独立交付计划，不改变 S0–P3 已完成的结论。

---

## 0. 计划摘要

| 项 | 结论 |
|---|---|
| 交付物 | v2 原生桌面客户端 + 管理 API + `tr-client` + `tr-onboard` + 打包/更新 |
| 范围 | 9 个页面、8 个 Agent 接入、WSL、token/config/日志/统计/审批/诊断 |
| 估算 | **约 165 人日**（不含 15% 缓冲）；约 **33 人周**；含缓冲约 **38 人周** |
| 工期 | 2 人约 **18–20 周**；3 人约 **13–14 周**；单人全职约 **8–9 个月** |
| 里程碑 | M0 契约 → M1 只读 API → M2 鉴权/配置 → M3 壳 → M4 页面 → M5 onboarding → M6 打包 → M7 GA |
| 首个可演示切片 | M3 结束：真实 sidecar + 真实上游 + Overview/Settings/Candidates + 账本 |
| 关键路径 | 契约 → 只读 API → 配置写入 → 壳 → 页面 → onboarding → 打包 → GA |
| 硬门禁 | `core`/`decide`/`proto` diff = 0；所有管理面经 HTTP；密钥不进前端 |

> 估算按“熟悉 Rust/Tauri、有 v1 代码可参考”的团队。若团队不熟悉 Tauri 或首次做签名/公证，M6 增加 20–30%。

---

## 1. 交付物

| 编号 | 交付物 | 形态 |
|---|---|---|
| D1 | 管理 API 契约 | `docs/p4-admin-api-contract.zh.md` + JSON Schema + 生成 TS 类型 |
| D2 | 服务端管理面 | `tr-control`/`tr-server`/`tr-agent` 扩展 + M5 迁移 |
| D3 | `tr-client` | Rust HTTP 客户端 crate，CLI/测试/桌面共用 |
| D4 | `tr-onboard` | Agent/WSL detect/plan/apply/verify crate |
| D5 | 桌面客户端 | `v2/apps/desktop`（Tauri 2 + React） |
| D6 | 打包与更新 | Windows/macOS 安装包、sidecar、签名、更新 manifest |
| D7 | 测试与报告 | 契约/服务端/UI/E2E/真实客户端/性能/安全报告 |
| D8 | 文档 | 用户指南、配置参考、故障排查、发布说明 |

---

## 2. 团队与角色

| 角色 | 职责 | 建议投入 |
|---|---|---|
| BE（Rust 后端） | `control`/`server`/`agent` 管理面、token、配置、统计、日志、事件 | 1.0 FTE |
| FE（前端） | React 页面、配置编辑器、图表、onboarding UI、i18n | 1.0 FTE |
| FS（Tauri/全栈） | 壳、supervisor、admin 代理、host 命令、打包 | 0.5–1.0 FTE |
| QA/DevOps | 契约测试、E2E、真实客户端、CI、签名/公证、发布 | 0.5 FTE |
| 设计（兼职） | 信息架构、页面线框、空态/错误态、视觉评审 | 0.2 FTE |

最小可用组合：BE + FE + FS（0.5）+ QA（0.5）。没有 QA/DevOps 时，BE/FE 需各承担一半测试与打包，工期加 15–20%。

---

## 3. 工作分解（WBS）

估算单位：人日（1 人日 = 6 小时有效工作）。依赖列只列强依赖。

### 3.1 Phase 0：契约与脚手架（10 人日）

| ID | 任务 | 角色 | 估算 | 依赖 | 产物/验收 |
|---|---|---|---|---|---|
| P4-001 | 管理 API 契约 v1：端点、DTO、错误、分页、scope | BE | 4 | — | 契约文档评审通过；JSON Schema 可生成 |
| P4-002 | 脚手架：`v2/apps/desktop`、`tr-client`、`tr-onboard`、CI 作业 | FS | 3 | — | `cargo check`/前端 build 绿；CI 跑通空壳 |
| P4-003 | 类型生成与契约门禁：`ts-rs` + `xtask contract-check` | BE/FE | 3 | P4-001/002 | 生成物 diff 即失败；TS 类型可用 |

**退出标准**：契约冻结；任何人改 DTO 必须先改契约并过生成门禁。

### 3.2 Phase 1：只读管理面与鉴权（32 人日）

| ID | 任务 | 角色 | 估算 | 依赖 | 产物/验收 |
|---|---|---|---|---|---|
| P4-011 | `/status` | BE | 2 | P4-001 | 版本/uptime/schema/digest/modes/计数；测试 |
| P4-012 | `/health` 熔断状态 | BE | 2 | P4-001 | 候选状态/计数；测试 |
| P4-013 | `/requests` 列表 + 游标 + 过滤 | BE | 4 | P4-001 | 时间/会话/模型/outcome/协议/原因码；并发游标测试 |
| P4-014 | `/requests/{id}` 详情 | BE | 3 | P4-013 | plan/judge/contributions/authority/attempts；404 |
| P4-015 | `/stats/summary` | BE | 3 | P4-013 | 与 SQL 聚合一致；时间窗边界 |
| P4-016 | `/stats/series` + `/stats/models` | BE | 4 | P4-015 | bucket/group_by/metric；性能达标 |
| P4-017 | 日志滚动 + ring + `/logs` | BE | 3 | P4-002 | 滚动/脱敏/上限；测试 |
| P4-018 | `/doctor` | BE | 2 | P4-011 | 配置/DB/判官/磁盘/日志检查 |
| P4-019 | token store + scope + 管理端点 | BE | 4 | M5 迁移 | 鉴权矩阵；撤销即时生效；bootstrap |
| P4-020 | 契约/服务端测试 + 性能基线 | QA/BE | 3 | P4-011..019 | golden；10 万行 p95 达标 |
| P4-021 | 优雅关闭 `/shutdown` | BE | 2 | P4-011 | 停止接新请求；5s 等待；测试 |

**退出标准**：只读管理面可用；`tr-cli` 能用 HTTP 读状态/账本/统计；鉴权矩阵全绿。

### 3.3 Phase 2：配置生命周期（18 人日）

| ID | 任务 | 角色 | 估算 | 依赖 | 产物/验收 |
|---|---|---|---|---|---|
| P4-031 | 配置文档 GET + 脱敏 + warnings | BE | 3 | P4-001 | 不返回密钥；revision 同源 |
| P4-032 | validate + 规范化 TOML + diff | BE | 4 | P4-031 | 校验规则表全覆盖；字段路径可映射 |
| P4-033 | apply + rollback + CAS + 原子写 + 审计 | BE | 4 | P4-032 | 409/422；备份上限；并发 apply |
| P4-034 | 配置 schema v2 + 1→2 迁移 | BE | 3 | P4-031 | 旧配置可加载；新字段默认值；测试 |
| P4-035 | token 管理端点（CRUD/撤销） | BE | 2 | P4-019 | 明文只返回一次；列表脱敏 |
| P4-036 | 配置 DTO → TS 类型 + 前端表单模型 | BE/FE | 2 | P4-003/032 | 生成物无 diff；字段一一对应 |

**退出标准**：UI 可以通过 API 完成“新增候选 → 新增虚拟模型 → apply → 重启 → 生效”，无需手改 TOML。

### 3.4 Phase 3：桌面壳与客户端 SDK（22 人日）

| ID | 任务 | 角色 | 估算 | 依赖 | 产物/验收 |
|---|---|---|---|---|---|
| P4-041 | Tauri 脚手架 + 复制 v1 前端 + 移除 v1 依赖 | FS | 4 | P4-002 | 应用可启动；无 v1 crate 依赖 |
| P4-042 | supervisor：spawn/健康/重启/孤儿/端口 | FS | 5 | P4-011/021 | 崩溃 10s 恢复；端口冲突；单实例 |
| P4-043 | admin 代理 + token 注入 + SSE 桥 | FS | 4 | P4-019/042 | token 不进 JS；路径白名单；错误映射 |
| P4-044 | host 命令：ui-state/打开目录/窗口/版本 | FS | 2 | P4-041 | 白名单；原子写 |
| P4-045 | `tr-client` Rust SDK | BE | 4 | P4-001 | 覆盖只读 + 配置 + token；错误类型 |
| P4-046 | 壳布局/路由/主题/i18n/错误边界 | FE | 3 | P4-041 | 首屏可导航；tour 可跑 |

**退出标准**：M3 演示——真实 sidecar + 真实上游 + Overview/Settings/Candidates + 账本可查。

### 3.5 Phase 4：页面（34 人日）

| ID | 任务 | 角色 | 估算 | 依赖 | 产物/验收 |
|---|---|---|---|---|---|
| P4-051 | Overview | FE | 3 | P4-043/046 | 状态/端点/最近请求/健康/接入入口 |
| P4-052 | Settings：服务/鉴权/日志/保留期/更新 | FE | 3 | P4-043 | 启停/重启；apply 后重启进度 |
| P4-053 | Candidates 编辑器 | FE | 4 | P4-036/043 | 增删改/禁用/引用检查/validate/apply |
| P4-054 | Virtual Models/Profiles/Trust Tiers | FE | 4 | P4-036/043 | 候选池/策略/默认模型；只改配置完成新增 |
| P4-055 | Requests 列表 + 详情 | FE | 4 | P4-013/014 | 过滤/分页/详情/复制 ID；空态说明 |
| P4-056 | Stats 页面 + 图表适配 | FE | 5 | P4-015/016 | summary/series/models；时间窗/分组 |
| P4-057 | Logs 页面 | FE | 2 | P4-017 | level/target/尾随/打开目录 |
| P4-058 | Tokens 页面 | FE | 2 | P4-035 | 创建/撤销/一次性明文/scope |
| P4-059 | Approvals 页面 | FE | 2 | P4-019 | pending/批准/拒绝/倒计时 |
| P4-060 | Diagnostics + 配置审计/回滚 | FE | 3 | P4-018/033 | doctor/health/audit/rollback |
| P4-061 | 加载/空/错/无权限状态 + a11y | FE | 2 | P4-051..060 | 所有页面有明确状态；键盘可用 |

**退出标准**：MVP 页面全部可用；配置编辑闭环；统计与账本对得上。

### 3.6 Phase 5：Agent onboarding（17 人日）

| ID | 任务 | 角色 | 估算 | 依赖 | 产物/验收 |
|---|---|---|---|---|---|
| P4-071 | `tr-onboard` 核心：detect/plan/apply/verify | BE | 5 | P4-002 | 计划/备份/原子写/回滚；临时 HOME 测试 |
| P4-072 | 8 个 Agent 适配器 + golden | BE | 6 | P4-071 | 每个 Agent 有 before/after fixture；幂等 |
| P4-073 | WSL 检测/配置 | BE/FS | 3 | P4-071 | UNC home；可达性探测；参数数组 |
| P4-074 | Onboarding UI + cc-switch 导出 | FE | 3 | P4-043/072 | diff 预览/应用/验证；链接生成 |

**退出标准**：Hermes/Codex/Claude Code 真实接入并完成请求；8 个 Agent 单测全绿。

### 3.7 Phase 6：事件流（6 人日）

| ID | 任务 | 角色 | 估算 | 依赖 | 产物/验收 |
|---|---|---|---|---|---|
| P4-081 | SSE 事件总线 + 续传 | BE | 4 | P4-013/033 | `Last-Event-ID`；心跳；溢出提示 |
| P4-082 | UI 实时刷新（请求/状态/审批） | FE | 2 | P4-081 | 页面隐藏暂停；断线回退轮询 |

### 3.8 Phase 7：打包与更新（13 人日）

| ID | 任务 | 角色 | 估算 | 依赖 | 产物/验收 |
|---|---|---|---|---|---|
| P4-091 | sidecar 捆绑（per-triple） | FS | 3 | P4-042 | 安装后能找到并启动 sidecar |
| P4-092 | 安装器/签名/公证 | FS/DevOps | 4 | P4-091 | Windows 安装；macOS 签名/公证 |
| P4-093 | 更新检查/下载/校验/替换 | FS | 3 | P4-091 | manifest 签名；失败保留旧版 |
| P4-094 | 发布 CI + 版本/通道 | DevOps | 3 | P4-092/093 | alpha/beta/stable 产物 |

**退出标准**：干净机器安装→首启→配置→请求成功；升级不丢配置/DB。

### 3.9 Phase 8：硬化与 GA（13 人日）

| ID | 任务 | 角色 | 估算 | 依赖 | 产物/验收 |
|---|---|---|---|---|---|
| P4-101 | Playwright E2E 主路径 | QA/FE | 4 | M4/M5 | 启动/配置/请求/统计/重启/onboarding |
| P4-102 | 真实客户端矩阵回归 | QA | 3 | P4-072 | 3 客户端 × 流式/非流式；账本校验 |
| P4-103 | 性能/24h soak | QA/BE | 3 | M4 | 内存/日志/孤儿/延迟达标 |
| P4-104 | 安全评审 | BE/QA | 2 | M5 | token/路径/脱敏/权限清单 |
| P4-105 | 文档/发布说明 | FE/BE | 3 | M6 | 用户指南/配置参考/故障排查 |

**退出标准**：§7 验收清单全绿；RC 签收。

---

## 4. 里程碑与排期

### 4.1 单人全职（约 33 周 + 15% 缓冲）

| 里程碑 | 周 | 内容 | 退出标志 |
|---|---|---|---|
| M0 契约 | W1–W2 | P4-001..003 | 契约冻结；生成门禁绿 |
| M1 只读 API | W3–W8 | P4-011..021 | 状态/账本/统计/日志/鉴权可用 |
| M2 配置 | W9–W11 | P4-031..036 | validate/apply/restart 闭环 |
| M3 壳 | W12–W15 | P4-041..046 | 真实 sidecar + 首屏 + 配置页 |
| M4 页面 | W16–W22 | P4-051..061 | MVP 页面全可用 |
| M5 onboarding/事件 | W23–W26 | P4-071..082 | 3 客户端接入；实时刷新 |
| M6 打包 | W27–W29 | P4-091..094 | 安装包/更新可用 |
| M7 GA | W30–W33 | P4-101..105 | 验收全绿；RC |

> 单人全职按 33 周基础排期；含 15% 缓冲约 38 周。上表是顺序排期，不按人日逐项累加。

### 4.2 两人并行（约 18–20 周）

- BE：M0→M1→M2→M5（后端）→M7 支持。
- FE/FS：M0 脚手架→M3 壳→M4 页面→M5 onboarding UI→M7。
- QA/DevOps 0.5：M1 起介入契约测试，M4 起 E2E，M6 打包。

关键重叠：M1 与 M3 可并行（FE 先做壳，BE 做 API），但 **P4-001 必须单人收口**，否则契约分裂。

### 4.3 三人并行（约 13–14 周）

- BE1：管理 API + 配置/鉴权。
- BE2/FS：`tr-onboard` + supervisor + 打包。
- FE：页面 + onboarding UI。
- QA 0.5：E2E/真实客户端。
- 风险：契约变更沟通成本上升；需要每日 15 分钟同步 + 契约 owner。

### 4.4 MVP 裁剪（如果只有 8–10 周）

按顺序砍：

1. 不做 Logs/Approvals/Diagnostics 页面（保留 API）。
2. onboarding 只做 Claude Code/Codex/Hermes 三个。
3. 不做 WSL/cc-switch。
4. 不做 SSE（只轮询）。
5. 不做 macOS/Linux 包（只 Windows）。
6. 不做 OTA（手动更新）。

保留：契约、只读 API、token、配置 apply、壳、Overview/Settings/Candidates/Requests/Stats。这样仍可日常自用，但**不能宣称替代 v1**。

---

## 5. 关键路径与依赖

```text
P4-001 契约
  → P4-011..018 只读 API
  → P4-031..034 配置
  → P4-041..043 壳/代理
  → P4-051..061 页面
  → P4-071..074 onboarding
  → P4-091..094 打包
  → P4-101..105 GA
```

可并行支线：

- P4-019/035 token 与鉴权（依赖 M5 迁移，可与只读 API 并行）。
- P4-045 `tr-client`（契约冻结后即可开始）。
- P4-071/072 onboarding（不依赖 UI，可与页面并行）。
- P4-081 SSE（不阻塞页面，M5 前完成即可）。

阻塞点：

- 没有 M5 迁移，token store 不能落库 → P4-019 必须早于 P4-043。
- 没有配置 apply，Candidates/VM 页面只能读 → P4-031..034 必须先于 P4-053/054。
- 没有 supervisor，UI 无法验证重启 → P4-042 必须先于 P4-052。

---

## 6. 测试与质量计划

| 阶段 | 测试 | 门禁 |
|---|---|---|
| M0 | 契约 golden、类型生成 diff | `contract-check` |
| M1 | 鉴权矩阵、分页、统计对账、日志脱敏 | `cargo test`；性能基线 |
| M2 | CAS、原子写、备份、schema 1→2 | 配置 golden；并发 apply |
| M3 | supervisor 崩溃/端口/孤儿；代理路径白名单 | 手工 + 自动 |
| M4 | 页面组件/单测；Playwright 主路径 | E2E 绿；视觉评审 |
| M5 | Agent golden（临时 HOME）；真实客户端 | 3 客户端账本校验 |
| M6 | 安装/升级/回滚；签名校验 | 干净机器演练 |
| M7 | 24h soak、安全评审、性能 | 验收清单 |

质量预算：

- 管理 API P95：list < 150ms、summary < 300ms（10 万行）。
- 冷启动 < 2.5s；配置 apply + 重启 < 5s。
- 崩溃恢复 < 10s；连续 5 次失败进入诊断态。
- 0 密钥出现在响应/日志/前端内存（自动化断言）。

---

## 7. 验收清单（GA）

### 7.1 功能

- [ ] 干净机器安装 → 首启 → 配置一个候选 → 创建虚拟模型 `auto` → 聊天请求成功。
- [ ] 8 个 Agent 中至少 Hermes/Codex/Claude Code 通过 UI 接入并完成流式 + 非流式请求。
- [ ] 候选/虚拟模型/profile/trust tier 的新增与修改只通过 UI 完成，不需手改 TOML。
- [ ] 请求列表可按会话/模型/outcome/协议/原因码过滤；详情含 plan/judge/contributions/authority。
- [ ] 统计的请求数/成功率/成本/token 与直接 SQL 聚合一致。
- [ ] 审批在 `enforce` 下拦截高风险工具；UI 批准/拒绝；账本 `rejected`。
- [ ] token 创建/撤销/改 scope；撤销后下一次请求 401。
- [ ] 配置 apply 后 5s 内重启生效；并发修改 409 且不覆盖。
- [ ] 日志可查、可过滤、可打开目录；敏感字段脱敏。

### 7.2 非功能

- [ ] UI 进程内没有 SQLite/日志文件直读（权限与代码审计）。
- [ ] admin token 不在 WebView 内存/响应/日志/错误中。
- [ ] sidecar 崩溃 10s 内恢复；5 次失败进入诊断态。
- [ ] 10 万账本下 list/summary 达标。
- [ ] 24h soak 内存稳定；无孤儿进程；日志滚动正常。
- [ ] Windows/macOS 安装包可用；升级不丢配置/DB。
- [ ] `core`/`decide`/`proto` diff = 0；全部门禁绿。

### 7.3 文档

- [ ] API 契约与生成物一致。
- [ ] 配置参考（含 P4 新字段）。
- [ ] 用户指南：安装、配置、接入、统计、故障排查。
- [ ] 发布说明与已知限制。

---

## 8. 发布计划

| 阶段 | 时间 | 范围 | 出口 |
|---|---|---|---|
| Alpha | M3 后 | 内部；Windows；核心页面 | 真实上游 + 真实账本；无阻断 bug |
| Beta | M5 后 | 受邀用户；Windows/macOS | 3 客户端接入；配置闭环；已知问题清单 |
| RC | M7 前 | 冻结功能；只修 bug | 验收清单 95%；安全评审通过 |
| GA | M7 | 公开；Windows/macOS；Linux beta | 验收全绿；发布说明 |

特性开关：SSE、Approvals、Linux 包、OTA 可分别灰度；默认关闭不影响核心路径。

回滚：安装包保留上一版本；数据目录与 DB 向前兼容；不依赖 v1。

---

## 9. 风险与应对

| 风险 | 概率 | 影响 | 应对 |  Owner |
|---|---|---|---|---|
| 契约反复导致 UI 返工 | 中 | 高 | M0 冻结；只允许追加字段；每日同步 | BE |
| 配置 apply/重启体验差 | 中 | 中 | “应用并重启”进度态；合并多次编辑；失败回滚 | FE/BE |
| supervisor 跨平台差异 | 中 | 高 | Windows 先 GA；macOS/Linux 分阶段；Job Object | FS |
| Agent 配置格式漂移 | 中 | 中 | 每个 Agent golden fixture；版本探测；失败给出手动指引 | BE |
| 签名/公证/安装包延期 | 中 | 高 | M6 前完成 CI 演练；预留 20% 缓冲 | DevOps |
| 密钥泄漏 | 低 | 高 | Rust 代理；脱敏测试；安全评审 | BE/QA |
| 账本增长导致统计慢 | 中 | 中 | 保留期 + 索引 + 窗口限制 | BE |
| 范围蔓延到 v1 功能复刻 | 高 | 高 | MVP/GA 清单签字；砍掉项明确 | 产品/tech lead |
| 单人投入不足 | 中 | 高 | 按 33 周排；少于 1.5 FTE 不承诺 8 周 | 产品 |

---

## 10. 变更控制

- **契约冻结**：M0 后端点/字段只增不改；删除/重命名需 `api_version + 1` + 弃用期。
- **配置 schema**：P4 起 `schema_version = 2`；后续破坏性变更逐版递增，加载器保留上一版迁移。
- **范围变更**：任何新增页面/Agent/平台必须从 MVP/GA 清单中换出等量工作，不直接加人。
- **门禁**：`core`/`decide`/`proto` diff = 0；管理面不得直读 SQLite；密钥不得进前端。
- **每周演示**：每个里程碑末必须能演示“真实进程 + 真实上游 + 真实账本”；演示不了先缩范围。

---

## 11. 开工前检查清单

- [ ] 技术方案与开发计划评审通过，明确 MVP/GA 范围。
- [ ] 指定契约 owner（BE）与 UI owner（FE）。
- [ ] 确认支持矩阵（Windows/macOS/Linux beta）与签名账号。
- [ ] 确认默认端口 `17621`、数据目录 `~/.token-router-v2`。
- [ ] 确认不迁移 v1 配置/数据，不承诺 v1 功能对齐。
- [ ] 确认投入与范围匹配：不足 1.5 FTE 时按 §4.4 裁剪 MVP，不把团队规模作为不开工的门槛。
- [ ] CI 具备 Rust/前端/打包三套作业。

> 本计划不把“4 周 dogfood”“真实客户端无回归”等列为开工前置门槛；这些改为 M4/M5 的持续验证项，由阶段验收控制风险。

---

## 附录 A：任务汇总

| Phase | 任务数 | 人日 |
|---|---|---|
| P0 契约/脚手架 | 3 | 10 |
| P1 只读 API/鉴权 | 11 | 32 |
| P2 配置 | 6 | 18 |
| P3 壳/SDK | 6 | 22 |
| P4 页面 | 11 | 34 |
| P5 onboarding | 4 | 17 |
| P6 事件 | 2 | 6 |
| P7 打包 | 4 | 13 |
| P8 硬化 | 5 | 13 |
| **合计** | **52** | **165** |

> 165 人日为净估算；加 15% 缓冲约 190 人日（38 人周）。2 人并行约 19–20 周；3 人约 13–14 周。

## 附录 B：里程碑交付对照

| 里程碑 | 可演示内容 | 不可演示/已知缺口 |
|---|---|---|
| M3 | 启动/停止 sidecar；Overview；Candidates 读写；真实请求入账 | Stats/Logs/Onboarding 未完成 |
| M4 | 9 个页面中的 MVP 页面；配置闭环；统计对账 | onboarding、SSE、打包未完成 |
| M5 | 3 客户端接入；WSL；实时刷新 | 安装包/更新未完成 |
| M6 | 干净机器安装/升级 | GA 硬化未完成 |
| M7 | 全部验收 | — |
