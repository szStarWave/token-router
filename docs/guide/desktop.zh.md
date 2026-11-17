# 桌面端页面指南

> 事实基准：`v2/apps/desktop/frontend/src/router.tsx` 与 `pages/`（2026-11-17）。
> 桌面端为 Tauri 壳 + React 前端；supervisor 托管网关（固定 `127.0.0.1:17621`，崩溃退避重启 1..16s ×5，孤儿进程清理）。
> 所有数据经 Rust 侧代理访问 `/v2/admin`（路径/方法白名单、8MiB 上限）；WebView 不持有 admin token，不直读 SQLite/日志文件。

| 页面 | 功能 |
|---|---|
| **Overview** | 网关状态总览；Agent 一键配置与 WSL 配置入口 |
| **Models** | 物理候选管理：增删改 base_url/model/凭据引用/tier/strength/价格 |
| **VirtualModels** | 虚拟模型管理：候选池、profile、strategies |
| **Requests** | 请求账本：按会话/模型/outcome/协议/原因码过滤；详情含 plan/judge/contributions/authority 决策过程 |
| **Stats** | 统计图表（与账本 SQL 同源）：请求量/成功率/成本/延迟序列 |
| **Logs** | 日志尾部查看、过滤、打开日志目录（敏感字段已脱敏） |
| **Tokens** | 管理面 token 创建/撤销/改 scope（inference/admin:read/admin:write/admin:approve），撤销立即生效 |
| **Approvals** | 工具调用审批：enforce 拦截的待审项批准/拒绝，账本留痕 |
| **Diagnostics** | 自检诊断（doctor） |
| **Onboarding** | Agent 接入向导：detect → plan（diff 预览）→ apply → verify，含 WSL |
| **Settings** | supervisor 与网关设置 |

## 配置闭环

Models/VirtualModels/Settings 页的修改走 validate → apply：

- 校验失败返回逐字段 issues，不落盘；
- apply 原子写 + 备份（≤10 份）+ 审计；并发修改 CAS 409 不覆盖；
- apply 后重启生效；可 rollback。

全程不需要手改 TOML。事件经 SSE（`/v2/admin/events`，Last-Event-ID 续传）桥接到 UI 实时刷新。

## 已知限制

- 打包仅 Windows NSIS；macOS/Linux 待演练。
- 路由时间线可视化、成本分解/热力图、Playground 在路线图 P5-C/P5-D，当前 Requests 详情为结构化数据展示。