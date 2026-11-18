# Token Router 产品文档

> 状态：现行（2026-11-17）。事实基准：`v2/` 代码与 [P4 交付基线](./archive/p4-delivery-report.zh.md)。
> 产品尚未发布；本文描述已实现能力，未实现能力一律标注路线图。

## 1. 一句话定位

**Token Router 是一个本地 LLM 网关：多个 coding agent 共用一个模型入口，每次请求由路由引擎决定用哪个模型，并且每个决定都能事后解释。**

目标用户：在本机同时跑多个 coding agent（Claude Code、Codex、Hermes 等）、手里有多家模型 API key、想要统一入口和可控成本的人。

与同类产品的差异一句话：magpie / OmniRoute 管的是**账号和额度**（资源层），Token Router 管的是**每一次请求选哪个模型、为什么选它**（决策层）。对外口径：**"别家管你的账号和额度，我们管你的每一次模型选择——选得对、说得清、靠得住。"** 资源层能力正在按 [路线图](./roadmap.zh.md) 补齐——差异化不能豁免资源层欠账，用户先要"5 分钟接上"，才轮得到体会"路由更聪明"。

## 2. 对标 magpie / OmniRoute

参考产品：[magpie](https://github.com/yetone/magpie)（Go，37.9k 星，本地网关，"每个 agent 的模型，一处管理"）、[OmniRoute](https://github.com/cheaper-inference/OmniRoute)（TS，88.2k 星，"免费 AI 网关"，免费额度聚合 + 编码 CLI 网关）。数字为 2026-11-17 一手来源查证快照，逐模块调研与来源 URL 见 [竞品深度调研](./research/competitor-omniroute-magpie.zh.md)。

| 能力 | 参考产品 | Token Router 现状 |
|---|---|---|
| 路由决策 | 路由组 7 模式 / 20+ combo 策略 / 16 因子评分 | ✅ **更强**：级联 + 判官 + 虚拟模型 + sticky 会话保持 + 升级闩锁，策略可插拔，决策快照可确定性回放 |
| 决策可解释 | 请求日志 | ✅ **更深**：每个请求留存完整决策快照（选择→策略→计划→派发→接力→判官），逐步可查 |
| 网关协议 | OpenAI/Anthropic/Gemini 互转 | ✅ OpenAI Chat / OpenAI Responses / Anthropic 三协议单漏斗，流式 + 工具 + reasoning（Gemini 🗺 评估中） |
| Agent 接入 | magpie 88 个 agent / OmniRoute 15 个 setup-* | ⚠️ 质量口径领先（8 agent 幂等精确合并、保留配置与注释、接入后验证 + WSL），数量落后；扩充 🗺 P5-D |
| 熔断与健康 | breaker / cooldown | ✅ 上游熔断 + 冷却 + 半开，桌面端可见 |
| 桌面管理 | Web UI | ✅ 原生桌面端（Tauri），13 个页面，配置改动全程 UI 闭环 |
| Provider/账号管理 | magpie 69 预设厂商、订阅账号 failover / OmniRoute 375+ providers | ⚠️ 手填 base_url + 凭据引用；厂商模板/多账号 🗺 P5-A |
| 模型目录 | models.dev 元数据、目录刷新 | ⚠️ 手填模型名；自动目录 🗺 P5-A |
| 配额/余额 | 配额窗口、余额监控 | ❌ 🗺 P5-B |
| 用量与成本分析 | 成本分解、热力图 | ⚠️ 账本数据齐全，呈现只有基础统计；分析视图 🗺 P5-C |
| Playground | 模型测试台 | ❌ 🗺 P5-D |
| 网关 key 限额 | 日/周/月 token 与成本预算 | ⚠️ key 有 scope 无限额；限额 🗺 P5-B |
| 隐私遮蔽 / 插件 / Web 搜索 | 有 | 明确不做（P7 评估） |

图例：✅ 已对齐或更强 · ⚠️ 部分具备 · ❌ 暂无 · 🗺 已排期。

## 3. 核心优势

按兑现程度排列。每条标注状态：**已兑现**（用户今天可感知）、**结构优势**（契约/架构独有，但需呈现层落地才可感知）、**潜在优势**（数据基础已有，呈现待建）。各优势功能的详细需求见对应 PRD。

### 3.1 路由决策语义的完整度（结构优势，独有）→ [PRD](./prd/routing.zh.md)

竞品的路由是策略列表（配额/健康/成本/速度加权），模型被视为可互换的供给；我们的路由是一套**互相仲裁的决策契约**：

- **级联（cascade）**：弱档先答，质量门不满足才升级强档（非流式）；流式只在语义提交前因传输错误接力，**绝不流到一半换模型**。
- **判官（judge）**：独立决策模型，有预算（每轮/每分钟/每会话）与 deadline，证据确认才升级；失败 fail-open 绝不阻断用户请求。
- **sticky / ContextHold**：硬复用保 prompt cache 与 thinking signature；软亲和可被更优策略改选。
- **Escalation latch**：升级需 2 次证据确认，闩锁后不震荡降级，释放有硬上限兜底。
- **仲裁顺序明文**：Escalation > CompetenceGuard > Classify > Baseline，冲突行为可预期。
- **failover + 熔断**：双 key 熔断、半开探测、不原地重试，计划是确定的物理候选列表。

策略经统一 trait 插拔，新能力不改内核。语义权威：[内核设计](./archive/rewrite-v2-design.zh.md)；精编：[架构文档](./architecture.zh.md)。
**兑现条件**：判官默认 off，普通用户开箱感知有限；需 P5-C 时间线 UI 与评测证据把"选得对"变成可见收益。

### 3.2 确定性可解释（结构优势，独有）→ [PRD](./prd/explainability.zh.md)

竞品的 trace 是事后展示；我们的可解释是**契约**：

- **DecisionSnapshot 逐字段回放一致**——"这个决定为什么这样做"可以重演验证，不是"看起来合理"。
- 账本全留痕：attempts（逐次尝试/错误类/耗时/是否提交）、contributions（各策略贡献）、judge、authority。
- **config_digest 落库**：任一请求可追溯"当时是哪份配置"。
- **成本分账**：answer_cost 与 routing_cost 分离（判官开销单独可见），micros 整数精度。
- 统计与账本 SQL 同源，不是另一套口径。

**兑现条件**：数据与回放 CLI 已有；时间线可视化、热力图、四维成本分解在 P5-C。

### 3.3 行为可预期（已兑现）

内核冻结（core/decide/proto diff=0 提交门禁）、管理 API 只增不改、316+ 测试含 16 个 golden 合并器测试、决策回放一致性测试。竞品日更/周更、文档数字长期漂移；我们的升级不破坏既有行为。对"把网关当基础设施长期用"的用户，这是真实卖点：**出问题能定位，升级不出意外**。

### 3.4 安全边界干净（已兑现）→ [PRD](./prd/security.zh.md)

- 明文 key 只存在于 Rust 侧内存与凭据存储；配置/API/日志/账本/前端一律只见引用 + 指纹后 4 位，0 密钥泄漏是每次提交的门禁断言。
- admin token 不进 WebView（Rust 侧注入，代理白名单）；scoped token 撤销立即生效。
- 配置 CAS 并发保护 + 审计 + 备份 + 回滚；日志脱敏。

### 3.5 Agent 接入质量（已兑现）→ [PRD](./prd/onboarding.zh.md)

数量落后（8 vs magpie 88），但"接进去不弄坏你原有配置"做得最认真：detect→plan（diff 预览）→apply→verify 全流程，幂等精确合并、保留用户字段与 TOML 注释（golden 测试固定语义）、原子写 + 时间戳备份、接入后真实请求验证、WSL 同等对待。

### 3.6 轻量运行时（对 OmniRoute 成立）

Rust 单二进制 sidecar，桌面壳 supervisor 托管（崩溃退避重启、孤儿进程清理）。调研实测口径：OmniRoute 跑 coding agent 负载需 8–12 GB V8 heap。注意：magpie 同为轻量单二进制（Go，~30MB），此条不对它成立。

### 3.7 基础能力（优势的地基）

- **三协议单漏斗**：OpenAI Chat / Responses / Anthropic Messages 归一化路由，流式 + 工具 + reasoning + `previous_response_id` 续接；未知字段 passthrough 保真。覆盖以 [fixtures 报告](./s0-gap-report.md) 为准。
- **桌面闭环**：Tauri 原生壳，13 页面，配置改动全程 UI 闭环（validate→apply→重启生效），不手改 TOML。见 [桌面指南](./guide/desktop.zh.md)。
- **熔断健康**：上游 breaker + 冷却 + 半开，桌面可见。

> 诚实的总结：3.1/3.2 是护城河但目前是"结构优势"，3.3–3.5 是信任基本盘，成本分账等"潜在优势"待 P5-C 兑现。短期胜负手在资源层补齐速度（P5-A~D），让差异化有机会被体验到。

## 4. 快速开始

见 [快速开始指南](./guide/quickstart.zh.md)：启动网关 → 配置候选 → 建虚拟模型 → 接入 agent。配置字段见 [配置参考](./guide/configuration.zh.md)。

## 5. 已知限制

1. 安装包签名/公证流程就绪但未执行（需发布 CI 证书）；打包以 Windows NSIS 为主，macOS/Linux 待演练。
2. 长期稳定性（24h soak）与大规模性能基线待真实环境验证。
3. Provider/账号/配额/Playground 等资源层能力未实现（见 §2 对标表与路线图）。
4. 更新方式为手动更新；OTA 签名校验流程已文档化未启用。

## 6. 接下来

[路线图](./roadmap.zh.md)：P5-A Provider 层 → P5-B 配额与限额 → P5-C 分析与可视化 → P5-D Playground 与打磨。内核路由语义保持不变。