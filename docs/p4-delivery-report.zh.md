# P4 交付报告（M0–M7）

> 状态：**M0–M6 已实施并提交；M7 完成可打包产物与验收自检；GA 剩签名/公证/24h soak（需发布 CI 与真实环境）**。
> 配套：[`p4-client-refactor-plan.zh.md`](./p4-client-refactor-plan.zh.md)（技术方案）、
> [`p4-client-development-plan.zh.md`](./p4-client-development-plan.zh.md)（开发计划）、
> [`p4-admin-api-contract.zh.md`](./p4-admin-api-contract.zh.md)（契约冻结 v1）。

## 1. 里程碑落地

| 里程碑 | 提交 | 内容 |
|---|---|---|
| M0 契约 | 641bedf | 契约冻结 v1、`tr-contracts::admin` DTO + ts-rs 生成、`xtask contract-check` 门禁、`tr-client`/`tr-onboard` 脚手架 |
| M1 只读 API + token | 641bedf | migration 5（tokens/config_audit）、scoped token store、AdminAuth（Bearer/scope/bootstrap）、status/health/doctor/requests/stats/logs/shutdown 端点 |
| M2 配置 | 641bedf | 脱敏文档、validate(200+issues)/apply(422+CAS 409)、原子写+备份(≤10)、rollback、审计 |
| M3 壳 | b0fd706 | `tr-cli admin` 16 子命令（真实服务端 E2E 验证）、Tauri 壳（supervisor/admin 代理/host 命令）、前端脚手架 |
| M4 页面 | f11acab | 数据层拆除 v1 镜像、11 页面（Overview/Models/VirtualModels/Requests/Stats/Logs/Tokens/Approvals/Diagnostics/Onboarding/Settings） |
| M5 onboarding | 9084ad3 | 8 Agent 精确合并器 + 16 golden 测试（幂等/用户配置保留/toml 注释保留）、WSL（枚举/UNC/网关探测/白名单）、onboarding UI（detect→plan→apply→verify） |
| M6 事件 | 558d9e4 | SSE `/v2/admin/events`（1024 ring/Last-Event-ID 续传/心跳/overflow）、ledger/熔断事件同步、Rust→Tauri 事件桥、UI 实时失效 |
| M7 打包 | 本提交 | sidecar 捆绑配置（externalBin per-triple）、prepare-sidecar 脚本、升级不覆盖数据目录、发布说明 |

## 2. 验收清单状态（开发计划 §7）

### 7.1 功能

- [x] 配置闭环：UI 完成「新增候选 → 新增虚拟模型 → apply → 重启生效」，无需手改 TOML（Models/VirtualModels/Settings 页）。
- [x] 8 个 Agent 接入能力齐备：onboarding UI 全流程（detect/plan/apply/verify）；Hermes/Codex/Claude Code 的合并语义由 golden 测试固定（真实客户端回归见已知限制）。
- [x] 候选/虚拟模型/profile 修改只通过 UI。
- [x] 请求列表按会话/模型/outcome/协议/原因码过滤；详情含 plan/judge/contributions/authority。
- [x] 统计与直接 SQL 同源（只聚合 ledger；admin_query 单测对照）。
- [x] 审批：enforce 拦截（S0–P3 已有）+ UI 批准/拒绝 + 账本记录（Approvals 页）。
- [x] token 创建/撤销/改 scope；撤销立即生效（tokens store 单测）。
- [x] 配置 apply 后重启生效；并发 CAS 409 不覆盖。
- [x] 日志可查/过滤/打开目录；敏感字段脱敏（LogRing sanitize + 单测）。

### 7.2 非功能

- [x] UI 进程无 SQLite/日志文件直读：全部经 `admin_request` 代理（路径白名单 `/v2/admin`、方法白名单、8MiB 上限）。
- [x] admin token 不进 WebView：Rust 侧内存注入 Bearer；plan 预览只有 `env:`/`file:` 引用。
- [x] 崩溃恢复：supervisor 退避重启（1..16s × 5）→ Failed 态 + stderr 尾部；孤儿进程清理（runtime.json）。
- [x] 0 密钥出现在响应/日志（config 脱敏 + LogRing sanitize 单测断言）。
- [ ] 10 万行 p95 性能基线：`window_too_large` 90 天限制与索引已就位；性能压测待真实环境。
- [ ] 24h soak：待真实环境长期运行。
- [x] `core`/`decide`/`proto` diff = 0（每次提交门禁）。
- [ ] Windows/macOS 安装包签名/公证：打包配置与流程文档就绪；签名证书在发布 CI 提供。

### 7.3 文档

- [x] API 契约与生成物一致（contract-check 门禁保证）。
- [x] 配置字段与 P4 新字段（契约文档 §4.5、schema_version=2）。
- [x] 用户指南：onboarding 页内嵌接入示例 + CLI 帮助；本报告汇总。
- [x] 发布说明与已知限制（本报告 §3）。

## 3. 已知限制

1. **签名/公证**：安装包签名需要发布 CI 的证书；本仓库内提供流程与配置，未执行。
2. **真实客户端回归**：Hermes/Codex/Claude Code 通过 UI 接入后的真实流式/非流式请求需在有上游的环境演练（开发计划 §11.4 门禁之一；产品已豁免开工门槛，保留为 GA 前验证项）。
3. **24h soak / 性能基线**：需要在真实环境长期运行；代码侧已具备指标路径（/stats/summary p95、账本索引）。
4. **OTA**：手动更新优先（MVP 裁剪 §4.4 第 6 项）；manifest 签名校验流程已文档化。
5. **macOS/Linux**：打包配置以 NSIS（Windows）为主；dmg/AppImage 待平台演练（开发计划 §8 支持矩阵）。
6. **设计系统预设 vendored**：前端 UnoCSS 预设取自 OpenChamber 提取版（MIT），未发布到 npm 故随仓 vendored；上游升级需手工同步（见 [`frontend-design-system.zh.md`](./frontend-design-system.zh.md) §2/§9）。

## 4. 门禁与测试总量

- workspace：316 tests（含 16 golden 合并器测试、SSE 集成测试、22 onboard 单测）
- 桌面壳：7 tests（supervisor/admin proxy/host/onboarding 事件桥）
- 前端：6 tests（config-edit 语义）+ tsc/vite 构建 + `check:uno`（类名命中率 + 字面量检查）
- 门禁：fmt、clippy -D warnings、check-deps（13 crates）、fixtures-check、config-only-check、contract-check、**core/decide/proto diff = 0**
