# Token Router 产品文档

> 状态：现行（2026-11-18）。事实基准：`v2/` 代码与 [路线图 §8 实施状态](./roadmap.zh.md)。
> 产品尚未公开发布：安装包签名未执行、24h 稳定性未验证。本文严格区分**已实现**（今天可用）、**待实施**（设计已产出或方向已定，未动工）与**目标**（验收标准，非既成事实），不描述虚构能力。

## 1. 定位

**Token Router 是一个本地 LLM 网关：多个 coding agent 共用一个模型入口，每次请求由路由引擎决定用哪个模型，并且每个决定都留有事后可查的证据。**

它运行在你自己的机器上：桌面端启动，agent 接进来，模型候选配进去，之后每个 agent 只是"指到一个模型名"。用哪个模型、为什么用它、花了多少钱，都在本地留痕可查。

## 2. 目标用户

在本机同时跑多个 coding agent（Claude Code、Codex、OpenCode、Cursor 等）、手里有多家模型 API key、想要统一入口和可控成本的开发者。典型诉求按时间顺序：

1. **先接通**：不想手写配置，不想弄坏 agent 原有配置；
2. **日常用**：请求稳定走通，坏了能看出来；
3. **出问题时**：能查清"刚才那次为什么失败、为什么选了这个模型"；
4. **长期用**：成本看得见、花销可设上限、配额快耗尽有提醒。

## 3. 产品原则

产品按用户任务组织：先接通/使用，需要时再展开诊断；不要求理解路由语义也能完成主路径。这是设计目标——当前桌面端 13 个页面（Overview/Providers/Models/VirtualModels/Requests/Stats/Playground/Quotas/Tokens/Logs/Diagnostics/Onboarding/Settings）按工程对象平铺，对专业用户友好，但对新用户暴露了过多内部结构；按任务重组主导航与页面目标（见 [product-design.zh.md](./product-design.zh.md)）是桌面端改版的动因。专业内容不因此消失：时间线、策略贡献、审计回滚、配置文件等始终可达。

1. **默认简洁，按需深入（目标）**：主路径不挡路，诊断面按需展开。
2. **不弄坏你已有的东西**：agent 配置精确合并、保留注释、原子写 + 备份；配置改动走审计。
3. **留证据，而不是给保证**：每个决定留可回放的快照；"选得好不好"由账本与评测证据说话，产品不预先宣称。
4. **密钥只存引用**：见 §7 安全边界。
5. **不静默换行为**：流式绝不流到一半换模型；升级需证据确认；failover 用确定的物理候选列表。

## 4. 核心价值

与 magpie / OmniRoute 等本地网关的差异一句话：**别家管你的账号和额度（资源层），我们管你的每一次模型选择（决策层）。**

- **决策可解释是契约，不是事后展示**：每个请求留存完整决策快照，逐字段可确定性回放；任一请求可追溯"当时是哪份配置"（见 §7 对 `config_digest` 边界的说明）。决策契约、回放语义与证据口径见 [PRD：可解释性](./prd/explainability.zh.md)。
- **路由是互相仲裁的决策语义**：级联、判官（默认关闭）、sticky 会话保持、升级闩锁、熔断 + 冷却 + 半开等机制协同工作，仲裁顺序明文，策略经统一 trait 插拔、新能力不改内核。级联/判官是否"选得对"，需以账本数据与评测证据衡量，本文不做绝对宣称。完整语义见 [PRD：路由](./prd/routing.zh.md)。
- **行为可预期**：内核冻结（core/decide/proto diff=0 提交门禁）、契约绑定由 `contract-check` 固定、合并语义由 golden 测试固定。这些是升级兼容性的机制保障，不是"永不变化"的承诺。
- **资源层是使用前提**：Provider/配额/分析/Playground 不是差异点，而是让用户先"接得上、看得懂"的必要条件。

## 5. 用户任务与旅程

本节按用户任务描述现有产品的使用旅程：先接通使用，需要时才展开诊断。每个任务对应已实现的产品面，并注明当前所在页面；任务化的信息架构目标见 [product-design.zh.md](./product-design.zh.md)。

### 任务 1：把它跑起来（服务状态）

- 启动桌面端（Tauri，supervisor 自动托管网关，默认 `127.0.0.1:17621`），或独立服务端（默认 `127.0.0.1:16621`）。
- **Overview 页**：服务状态、最近 24 小时统计、候选健康。
- 三协议单漏斗：OpenAI Chat（`/v1/chat/completions`）、OpenAI Responses（`/v1/responses`）、Anthropic（`/v1/messages`），流式 + 工具 + reasoning + `previous_response_id` 续接；未知字段 passthrough 保真。

### 任务 2：把模型配进去（模型来源与模型入口）

- **模型来源**：**Providers 页**从 20 个内置厂商模板（DeepSeek/OpenAI/Anthropic/Kimi/GLM/Qwen/OpenRouter/Groq 等）实例化厂商 → 添加账号（key 写入凭据存储，界面回显只有指纹后 4 位；支持每 key 代理与自定义 header）→ 拉取上游 `/v1/models` 目录并勾选模型 → 一键编译为候选；**Models 页**只读展示供应商编译出的物理候选（base_url/model/凭据引用/tier/strength），不接受手写候选。
- **反向导入**：已装 agent 的配置可一键扫描出 Provider/账号草稿（只读扫描，明文 key 永不导入）。
- **模型入口**：**VirtualModels 页**把候选组成虚拟模型（候选池按 Provider/账号分组），这是 agent 实际指向的"模型名"，也是路由语义的作用对象。

### 任务 3：把 agent 接进来（应用接入）

- **Onboarding 页**：8 个 agent（OpenClaw、Hermes、Hermes Flash、Claude Code、Codex、OpenCode、CodeBuddy、WorkBuddy）走 detect → plan（diff 预览）→ apply → verify 全流程：幂等精确合并、保留用户字段与注释、原子写 + 时间戳备份、接入后真实请求验证、WSL 同等对待。
- 另有 14 个常用 agent（Cursor、Cline、Continue、Zed、Goose、Aider、Gemini CLI 等）提供**手动接入引导**：页面给出网关地址与该 agent 需要改的环境变量，照着填即可。合计 22 条接入路径（8 条自动合并 + 14 条手动引导），并非 22 个自动集成。

### 任务 4：日常使用与看护（请求活动）

- **Overview**：配额耗尽的候选顶部提醒条，一键跳转处理。
- **熔断健康**：上游熔断 + 冷却 + 半开，Diagnostics 页可见；配额是预算、熔断是健康，两者语义分离。
- **余额页（供应商 → 余额）**：查询厂商公开的 usage 接口，按账号展示可用余额、赠送/充值拆分、计划（如 OpenRouter 免费层/按量、OpenCode Go）与键限额。厂商专用适配：DeepSeek / Moonshot(Kimi) / OpenRouter / OpenCode Zen-Go（`/zen/go/v1/usage` 的 rolling/weekly/monthly 订阅窗口）；未知厂商再探测 OpenAI 旧版计费接口（`/dashboard/billing/subscription` + `/usage`，new-api/one-api 类中转站普遍实现），404 或仅接受管理员 key 时明确显示「未提供余额接口」。不猜测、不用本地账本冒充余额；金额保留接口原始字符串，不跨币种求和。
- **候选限流窗口**：仍由路由侧从 429 / 限流头学习（OpenAI/Anthropic/Retry-After），无标准头时降级为本地估算并标记 `estimated`；供概览、账号行与请求页提示，不再作为余额页主视图。

### 任务 5：看花了多少、管住花销（用量分析）

- **Stats 页**：汇总、时间序列、五维成本分解（agent / 虚拟模型 / 物理候选 / 账号 / 会话）、活动热力图（UTC 天 × 小时）、TTFT 均值与缓存命中率。成本 answer 与 routing（判官开销）分账，micros 整数精度。
- **Tokens 页**：网关 key 限额——token 与成本预算，窗口 小时/日/周/月/每 N 小时（覆盖计划 key 的 5h 节奏），可按模型子集限定；超限返回 429 + Retry-After + 重置时间。

### 任务 6：选模型（模型测试）

- **Playground 页**：同 prompt 最多 4 个目标（候选或虚拟模型）并行对比，每列给出 TTFT / 总耗时 / TPS / token 数与错误信息。
- **执行语义**：`virtual:` 目标走该虚拟模型的正常路由，计入账本（产生路由决策记录与实际成本/用量）；`candidate:` 目标绕过路由直达上游，**不记路由决策账本**，但调用上游 API 同样产生真实费用。对比测试时请注意两者对账本与成本统计的影响不同。
- 当前为**非流式呈现**：服务端内部以流式计时取真实 TTFT，但前端等全部请求结束后统一展示各列完整结果，不支持逐 token 输出与原始 SSE 事件查看（待实施，见 §10）。`finish_reason` 在响应 DTO 中存在，当前 UI 不展示。

### 任务 7：出问题时，按需深入（诊断）

默认界面不阻挡主路径；以下诊断能力始终可达：

- **Requests 页**：账本按结果/协议/模型过滤；详情含**路由时间线**（从选择到结果的每一步 reason 与耗时）、逐次尝试（错误类/耗时/是否提交）、各策略贡献。
- **Diagnostics 页**：doctor 自检、熔断状态、配置审计与回滚。
- **Logs 页**：日志尾部查看与过滤，敏感字段已脱敏。
- **决策回放**：`tr-cli` 可对账本中的决策快照做确定性回放——"这个决定为什么这样做"可以重演验证。回放语义见 [PRD：可解释性](./prd/explainability.zh.md)。
- 配置变更的生效语义按资源区分：candidate/虚拟模型/监听地址等 TOML 配置类改动走 validate → apply → 重启生效；Provider/账号等管理面资源即时生效（详见各页面）。高级用户仍可直接读 `config.toml`（schema_version = 1）。

## 6. 任务与页面对照

| 用户任务 | 当前页面 | 备注 |
|---|---|---|
| 服务状态 | Overview、Settings | 服务健康与网关设置 |
| 模型来源 | Providers、Models | 模板/账号/目录/候选 |
| 模型入口 | VirtualModels | agent 指向的"模型名" |
| 应用接入 | Onboarding | 8 自动 + 14 手动引导 |
| 请求活动 | Overview、Quotas、Diagnostics、Logs | 看护与健康 |
| 用量分析 | Stats、Tokens | 成本分解与限额 |
| 模型测试 | Playground | ≤4 目标并行 |
| 按需诊断 | Requests、Diagnostics、Logs | 时间线/审计/回放 CLI |

按任务重组主导航（合并入口、隐藏诊断深度）是待实施的体验目标（见 [product-design.zh.md](./product-design.zh.md)）；当前一个页面可服务多个任务，上表为对照关系，非一一映射。

## 7. 安全边界与配置可追溯

- **上游凭据（provider API key）**：用户输入时仅表单瞬时持有，提交后管理面不回传明文（回显只有指纹后 4 位）；持久化配置只存引用（`env:`/`file:`/`keyring:`），运行时由 Rust 侧解析为明文并保持在内存/凭据存储中——配置文件、API 响应、日志、账本一律不见明文，0 泄漏是每次提交的门禁断言。
- **网关访问 key（inference key）**：创建时明文只显示一次，服务端仅存 SHA-256 摘要与前缀；scoped 撤销立即生效。
- **admin token**：管理面走 `/v2/admin`，admin token 由 Rust 侧注入代理，不进 WebView。
- **配置可追溯与回滚边界**：配置 CAS 并发保护 + 审计记录 + 备份（≤10 份）+ 回滚；账本落 `config_digest`，任一请求可追溯"当时对应哪份配置"。注意：`config_digest` 是**可追溯的标识**，不等于能从摘要还原配置内容；实际恢复依赖审计链与本地备份，超出备份保留窗口或审计记录的变更不可还原。

## 8. 能力现状

### 已实现（当前可用）

- 网关：三协议单漏斗、`/v1/models`、流式 + 工具 + reasoning、入站鉴权（inference key）。
- 路由内核：级联、判官（默认 off，off/shadow/active 三档）、sticky 会话保持、升级闩锁、failover + 熔断 + 冷却 + 半开；内置策略含 `quota-reset`（配额感知接力，默认启用）与 `fast`（可选启用）；策略经 trait/扩展点插拔。语义详见 [PRD：路由](./prd/routing.zh.md)。
- Provider 层：20 厂商模板、多账号、凭据文件 + 指纹、每 key 代理与自定义 header、上游目录拉取 + 勾选 + auto-include/deprecated 处理、候选编译投影（CAS/备份/审计）、agent 配置反向导入；虚拟模型组可「关闭思考」（决策固定 `off`，对硅基流动/DashScope/智谱/OpenRouter 附加厂商关闭开关）。
- 配额与限额：候选配额窗口（429/头学习 + 估算降级）、Overview 耗尽提醒、网关 key 限额（多窗口 + 模型子集 + 429/Retry-After）。
- 分析与诊断：五维用量聚合、热力图、成本 answer/routing 分账、路由时间线、账本全留痕、决策回放 CLI、doctor、审计 + 备份回滚。
- 桌面端：13 页面配置改动全 UI 闭环，SSE 实时刷新，admin token 不进 WebView。
- Agent 接入：8 精确合并 + 14 手动引导 + WSL + verify。

### 未实现（明确记录，不虚构）

- **models.dev 元数据富化**：目录仅拉上游 `/v1/models`；模型价格/context 需手填或模板给默认。DTO 已预留 `source` 字段，富化属待实施项。
- **cache write 5m/1h 两档计价**：协议层只有单一 `cache_write_tokens`，拆分需动内核（diff=0 门禁）；当前按 read/write 两桶呈现。
- **OTLP 导出 / Langfuse 映射**：未实现；`/v2/admin/usage` 已提供结构化数据，可后接 exporter。
- **Profiles 配置快照**：未实现；onboard 合并器能力已具备，快照层可直接复用。
- **订阅账号驱动**（Claude/Codex 登录态）：暂不实现，列入评估池——只有订阅没有 API key 的用户短期覆盖不了，这是战略缺口而非功能缺口。
- **余额查询范围**：只调用厂商公开的 usage/余额接口（目前 DeepSeek / Moonshot / OpenRouter）；没有公开接口的厂商（如 OpenAI、Anthropic、SiliconFlow）不猜、不爬控制台，明确显示「未提供余额接口」。不驱动订阅登录态（Claude/Codex），那是另一件事。
- **明确不做**：免费额度聚合、token 压缩（违背 passthrough 保真契约）、隐私遮蔽、插件系统、Web 搜索、媒体生成、云备份、局域网共享。

## 9. 已知限制

1. **未发布**：安装包签名/公证流程就绪但未执行（需发布 CI 证书）；打包仅 Windows NSIS，macOS/Linux 待演练。
2. **稳定性**：24h soak 与大规模性能基线待真实环境验证。
3. **Playground 非流式**：全部请求完成后统一展示，无逐 token 输出与原始事件流查看（原始 SSE 查看为待实施项）；UI 不展示 finish_reason。
4. **配额依赖上游头**：无标准限流头的厂商只有估算窗口（明确标记）。
5. **配置恢复有边界**：回滚依赖审计链与本地备份（≤10 份），见 §7。
6. 手动更新，OTA 签名校验流程已文档化未启用。
7. Gemini 原生协议未支持（透传不够时再评估）。

## 10. 产品方向

以下分三层，避免把"已排期"与"评估中"混为一谈：

### 已完成的路线图期次（P5-A~D）

- **P5-A Provider 层**：模板/账号/目录/反向导入/编译投影 —— 已落地。
- **P5-B 配额与限额**：窗口学习 + reset-aware 接力 + key 限额 —— 已落地。
- **P5-C 分析与可视化**：五维分解/热力图/时间线 —— 已落地。
- **P5-D Playground 与打磨**：4 目标并行 Compare + 手动 agent 矩阵 + fast 策略 —— 已落地。

各期提交与验收记录见 [路线图 §8](./roadmap.zh.md)。

### 待实施的体验目标（设计已产出，见 [product-design.zh.md](./product-design.zh.md)）

- 桌面端信息架构按用户任务重组（§6 对照表为目标形态的过渡）。
- Playground 逐 token 输出与原始 SSE 事件查看。
- models.dev 元数据富化。
- OTLP 导出 / Langfuse 映射。
- Profiles 配置快照。
- cache write 5m/1h 两档计价（需评估内核 proto 契约变更）。

### 评估池（需要单独决策，未排期）

- **订阅账号驱动**（magpie 护城河，覆盖"只有订阅没有 API key"的用户群）。
- **意图路由**（每 turn 一次模型调用的延迟/成本需实测）。
- **Quota-Share 公平共享**（配额层落地后的自然延伸）。
- **Gemini 原生协议**、隐私遮蔽、插件、Web 搜索、媒体、局域网共享（延续非目标清单，按用户反馈重估）。

本轮桌面体验改版（信息架构重组、Playground 呈现等）不动内核：core/decide/proto diff=0 门禁继续有效。未来若涉及 proto 契约变更（如 cache write 两档拆分），按变更冻结流程单独走内核设计文档修订，不随产品迭代隐式发生。
