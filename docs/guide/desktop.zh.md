# 桌面端页面指南

> 事实基准：`v2/apps/desktop/frontend/src/router.tsx` 与 `pages/`（2026-11-18）。
> 桌面端为 Tauri 壳 + React 前端；supervisor 托管网关（默认 `127.0.0.1:17621`，占用则递增到 17640；崩溃退避重启 1..16s ×5，孤儿进程清理）。
> 所有数据经 Rust 侧代理访问 `/v2/admin`（路径/方法白名单、8MiB 上限）；WebView 不持有 admin token，不直读 SQLite/日志文件。

| 页面 | 功能 |
|---|---|
| **Overview** | 网关状态总览；配额耗尽提醒；Agent 一键配置与 WSL 配置入口 |
| **Providers** | 厂商模板实例化、账号（key/代理/自定义 header）、模型目录勾选、编译为候选、从 agent 配置反向导入 |
| **Quotas** | 上游配额窗口（剩余/重置/是否估算）与手动重置 |
| **Models** | 物理候选管理：增删改 base_url/model/凭据引用/tier/strength；行内高级面板含代理、header、价格、模态、思考档 |
| **VirtualModels** | 虚拟模型管理：候选池（按 Provider/账号分组）、profile、strategies |
| **Requests** | 请求账本：按结果/协议/模型过滤；详情含路由时间线、attempts、策略贡献 |
| **Stats** | 汇总、时间序列、模型维度、五维成本分解、活动热力图 |
| **Logs** | 日志尾部查看、过滤、打开日志目录（敏感字段已脱敏） |
| **Playground** | 同 prompt 最多 4 个目标并行对比（TTFT/耗时/TPS） |
| **Tokens** | 管理面 token 创建/撤销/改 scope（inference/admin:read/admin:write）与网关 key 限额 |
| **Diagnostics** | 自检诊断（doctor）、熔断状态、配置审计与回滚 |
| **Onboarding** | Agent 接入向导：detect → plan（diff 预览）→ apply → verify，含 WSL |
| **Settings** | supervisor 与网关设置、主题 |

## 配置闭环

Models/VirtualModels/Settings 页的修改走 validate → apply：

- 校验失败返回逐字段 issues，不落盘；
- apply 原子写 + 备份（≤10 份）+ 审计；并发修改 CAS 409 不覆盖；
- apply 后重启生效；可 rollback。

全程不需要手改 TOML。事件经 SSE（`/v2/admin/events`，Last-Event-ID 续传）桥接到 UI 实时刷新。

## 已知限制

- 打包仅 Windows NSIS；macOS/Linux 待演练。
- 路由时间线可视化、成本分解/热力图、Playground 在路线图 P5-C/P5-D，当前 Requests 详情为结构化数据展示。