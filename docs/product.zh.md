# Token Router 产品文档

> 状态：现行（2026-11-17）。事实基准：`v2/` 代码与 [P4 交付基线](./archive/p4-delivery-report.zh.md)。
> 产品尚未发布；本文描述已实现能力，未实现能力一律标注路线图。

## 1. 一句话定位

**Token Router 是一个本地 LLM 网关：多个 coding agent 共用一个模型入口，每次请求由路由引擎决定用哪个模型，并且每个决定都能事后解释。**

目标用户：在本机同时跑多个 coding agent（Claude Code、Codex、Hermes 等）、手里有多家模型 API key、想要统一入口和可控成本的人。

与同类产品的差异一句话：magpie / OmniRoute 管的是**账号和模型**（资源层），Token Router 管的是**每一次请求选哪个模型、为什么选它**（决策层）。资源层能力正在按 [路线图](./roadmap.zh.md) 补齐。

## 2. 对标 magpie / OmniRoute

参考产品：[magpie](https://github.com/yetone/magpie)（Go，本地网关，"每个 agent 的模型，一处管理"）、[OmniRoute](https://github.com/diegosouzapw/OmniRoute)（TS，OpenRouter 开源替代）。调研细节与差距矩阵见 [路线图 §0](./roadmap.zh.md)。

| 能力 | 参考产品 | Token Router 现状 |
|---|---|---|
| 路由决策 | 路由组 5 模式 / 14 种 combo 策略 / 评分排序 | ✅ **更强**：级联 + 判官 + 虚拟模型 + sticky 会话保持 + 升级闩锁，策略可插拔 |
| 决策可解释 | 请求日志 | ✅ **更深**：每个请求留存完整决策快照（选择→策略→计划→派发→接力→判官），逐步可查 |
| 网关协议 | OpenAI/Anthropic/Gemini 互转 | ✅ OpenAI Chat / OpenAI Responses / Anthropic 三协议单漏斗，流式 + 工具 + reasoning（Gemini 🗺 评估中） |
| Agent 接入 | 80+ agent 配置改写 | ✅ 8 个 agent 精确合并（幂等、保留用户配置与注释）+ WSL + 接入后验证；列表扩充 🗺 P5-D |
| 熔断与健康 | breaker / cooldown | ✅ 上游熔断 + 冷却 + 半开，桌面端可见 |
| 桌面管理 | Web UI | ✅ 原生桌面端（Tauri），11 个页面，配置改动全程 UI 闭环 |
| Provider/账号管理 | 30+ 预置厂商、多账号、key 健康度 | ⚠️ 手填 base_url + 凭据引用；厂商模板/多账号 🗺 P5-A |
| 模型目录 | models.dev 元数据、目录刷新 | ⚠️ 手填模型名；自动目录 🗺 P5-A |
| 配额/余额 | 配额窗口、余额监控 | ❌ 🗺 P5-B |
| 用量与成本分析 | 成本分解、热力图 | ⚠️ 账本数据齐全，呈现只有基础统计；分析视图 🗺 P5-C |
| Playground | 模型测试台 | ❌ 🗺 P5-D |
| 网关 key 限额 | 日/周/月 token 与成本预算 | ⚠️ key 有 scope 无限额；限额 🗺 P5-B |
| 隐私遮蔽 / 插件 / Web 搜索 | 有 | 明确不做（P7 评估） |

图例：✅ 已对齐或更强 · ⚠️ 部分具备 · ❌ 暂无 · 🗺 已排期。

## 3. 核心能力

### 3.1 路由引擎：虚拟模型 + 级联 + 判官

用户面向的不是物理模型，而是**虚拟模型**——一个命名的候选池。agent 请求 `smart`（举例），网关在池内按策略编排：

- **级联（cascade）**：先执行弱档候选，质量门不满足再换强档（非流式）；流式请求只在语义提交前因传输错误接力，**绝不流到一半换模型**。
- **判官（judge）**：独立的决策模型，在预算与时限内为路由提供质量/难度评判，参与升级确认；判官失败绝不阻断用户请求（fail-open），评判记录进账本。
- **sticky / ContextHold**：同一会话粘住同一候选，避免上下文在模型间来回漂移。
- **Escalation latch**：一旦升级发生，会话内保持升级状态，不来回降级。
- **failover**：候选失败（含熔断）自动接力下一个，计划是确定的物理候选列表。

策略通过统一 trait 注册，可插拔；路由语义的权威定义见 [内核设计](./archive/rewrite-v2-design.zh.md)，精编见 [架构文档](./architecture.zh.md)。

### 3.2 每个决定都可解释

每个请求在账本（SQLite）中留存：选择了哪些候选、各策略给出了什么理由、最终计划、每次派发尝试（attempts）、各候选贡献（contributions）、判官结论。桌面端 Requests 页可按会话/模型/结果/协议/原因码过滤，详情页展示完整决策过程。统计与账本同源，不是另一套口径。

### 3.3 三协议单漏斗

OpenAI Chat Completions、OpenAI Responses、Anthropic Messages 三种入口协议归一化为内部 canonical 表示再路由，支持流式、工具调用、reasoning、`previous_response_id` 会话续接。协议覆盖以 [fixtures 覆盖报告](./s0-gap-report.md) 为准（自动生成，非兼容承诺）。

### 3.4 Agent 接入：detect → plan → apply → verify

桌面端 Onboarding 页自动发现本机 agent，生成配置改写计划，预览确认后精确合并——幂等、保留用户已有配置与 TOML 注释，接入后自动发真实请求验证。支持 WSL（枚举、UNC 路径、网关探测、白名单）。当前覆盖 8 个 agent，见 [Agent 接入指南](./guide/agents.zh.md)。

### 3.5 安全模型

- 明文 key 只存在于 Rust 侧内存与凭据存储；配置与 API 响应只出现 `env:`/`file:` 引用。
- 管理面走本机 `/v2/admin` HTTP；桌面端 WebView 不持有 admin token（Rust 侧注入）。
- 日志脱敏；配置变更原子写 + 备份 + CAS 并发保护 + 审计 + 回滚。
- 审批机制：enforce 模式下敏感操作需 UI 批准，账本留痕。

### 3.6 桌面端闭环

Tauri 原生壳 + 本地服务 supervisor（崩溃退避重启、孤儿进程清理）。11 个页面覆盖状态总览、候选/虚拟模型管理、请求审计、统计、日志、token、审批、诊断、接入、设置。配置改动不需要手改 TOML。

## 4. 快速开始

见 [快速开始指南](./guide/quickstart.zh.md)：启动网关 → 配置候选 → 建虚拟模型 → 接入 agent。配置字段见 [配置参考](./guide/configuration.zh.md)。

## 5. 已知限制

1. 安装包签名/公证流程就绪但未执行（需发布 CI 证书）；打包以 Windows NSIS 为主，macOS/Linux 待演练。
2. 长期稳定性（24h soak）与大规模性能基线待真实环境验证。
3. Provider/账号/配额/Playground 等资源层能力未实现（见 §2 对标表与路线图）。
4. 更新方式为手动更新；OTA 签名校验流程已文档化未启用。

## 6. 接下来

[路线图](./roadmap.zh.md)：P5-A Provider 层 → P5-B 配额与限额 → P5-C 分析与可视化 → P5-D Playground 与打磨。内核路由语义保持不变。