# 产品路线图（v3：对齐 OmniRoute / magpie）

> 状态：草案 v2（2026-11-17，并入竞品调研吸收决策 §0.3）
> 前置文档：[v2 内核设计（归档，语义权威）](./archive/rewrite-v2-design.zh.md)、[管理 API 契约 v1](./contracts/admin-api.zh.md)（将被 v2 契约增量扩展）、[P4 交付报告（归档，当前交付基线）](./archive/p4-delivery-report.zh.md)
> 定位变更：**从"路由策略研究项目"转向"对齐 OmniRoute / magpie 的完整本地 LLM 网关产品"，路由策略仍是差异化卖点，但不再是唯一卖点。**
> 文中 "v2 §x" 均指内核设计文档章节。

---

## 0. 一句话

v3 = v2 内核不动 + 补齐产品层：**Provider/账号管理、模型目录、配额跟踪、用量与成本分析、实时路由可视化、Playground、网关 key 限额** —— 对齐 magpie / OmniRoute 的日常使用面，同时保留 v2 独有的级联+判官+虚拟模型路由。

### 0.1 与参考产品的对标结论

调研基准（2026-11-17，一手来源逐条查证，详见 [竞品深度调研](./research/competitor-omniroute-magpie.zh.md)）：

| 参考产品 | 定位 | 核心功能面 |
|---|---|---|
| [magpie](https://github.com/yetone/magpie)（Go，37.9k 星） | 本地网关，"每个 agent 的模型，一处管理" | 4 协议互转（另有音频/Ollama 兼容/MCP hub）、69 预置厂商、88 agent 接入、路由组 7 模式（smart/order/rotate/usage/pace/weight/fast）、订阅账号 failover、意图路由、隐私遮蔽、插件、Profiles、用量成本、局域网共享 key 限额、Web 搜索、WSL |
| [OmniRoute](https://github.com/cheaper-inference/OmniRoute)（TS，88.2k 星） | "免费 AI 网关"：免费额度聚合 + 编码 CLI 网关 | 375+ providers（口径随分支漂移）、20+ combo 策略、`auto/*` 前缀路由（10 变体 + category:tier）、16 因子评分、热力图分析、Playground（Translator 四模式 + Studio）、配额跟踪、三层熔断、SSRF 防护、CLI 套件（80+ 命令）、OTLP 导出 |

### 0.2 差距矩阵（v2 现状 → 产品级）

| 能力域 | magpie/OmniRoute | v2 现状 | 判定 |
|---|---|---|---|
| 网关协议 | OpenAI Chat/Responses/Anthropic/Gemini 互转，流式+工具+reasoning | 3 协议单漏斗，流式+工具+reasoning+`previous_response_id` | ✅ 已对齐（Gemini 延后） |
| 路由策略 | 分组 7 模式 / 20+ combo / 16 因子评分 | Strategy trait + 级联 + 判官 + sticky/ContextHold + Escalation latch | ✅ **更强**（差异化核心） |
| **Provider/账号管理** | 69 预置厂商模板（magpie）、多账号、OAuth 订阅接入、key 健康度、每 key 代理 | 手填 base_url + `env:NAME` 的裸表格 | ❌ **最大差距 → P5-A** |
| **模型目录** | models.dev 元数据富化、模型可见性开关、目录刷新 | 手填 model 字符串 | ❌ → P5-A |
| **配额/余额** | 配额窗口跟踪、余额监控、续期提醒、每日 warm-up | 无（仅熔断计数） | ❌ → P5-B |
| **用量与成本** | 按 agent/账号/模型成本、热力图、缓存命中分解、OTLP 导出 | requests/stats/series 三个基础端点 + 价格模型（账本数据全有，呈现不足） | ⚠️ → P5-C |
| 实时路由视图 | magpie Live Routing、OmniRoute Live Monitor | attempts/contributions 数据在账本里，前端只有列表 | ⚠️ → P5-C |
| Playground | 模型测试台（Monaco/流式/计时）、协议转换器 | 无 | ❌ → P5-D |
| 网关 key 限额 | named key + 日/周/月 token 与成本限额 | token + scope，无限额 | ⚠️ → P5-B |
| Agent onboarding | 88 agent 接入（magpie，配置改写为主）、订阅发现、WSL | 8 agent 精确合并器 + WSL + verify | ⚠️ 质量口径领先、数量落后（P5-D 扩到 ~20） |
| 隐私遮蔽 / 插件 / Web 搜索 / 媒体 | 有 | 明确不做 | P7 评估 |
| 断路器/健康 | breaker + cooldown + half-open | `upstream` 已有 + UI 呈现 | ✅ |

> 结论：v2 的"发动机"（内核）已经比两家参考产品强，缺的是"仪表盘和油箱"——Provider 层、配额层、分析层、测试层。这四层全部落在 `control`/`upstream` 外围，**不动 `core`/`decide`/`proto`**（diff=0 门继续有效）。

### 0.3 竞品吸收决策（2026-11-17 调研）

判断标准：落在许可扩展面内（control/upstream/onboard/catalog/SignalBag/Strategy，diff=0 不破）、价值/成本比、强化而非稀释"决策质量与可解释"定位。吸收项的实施细节见 §3–§5 各期，此处只记决策与理由。

| 竞品能力 | 决策 | 去向与理由 |
|---|---|---|
| 自定义 header / 每 key 代理（magpie） | 吸收 | P5-A Account 字段；中转场景刚需，成本低 |
| 目录 auto-include 通配符 + deprecated 自动下架（magpie） | 吸收 | P5-A catalog；目录自动化的一部分 |
| 从现有 agent 配置反向导入 provider（magpie） | 吸收 | P5-A；onboard crate 已有全部 agent 配置路径知识，开箱即用捷径 |
| 限额每 N 小时窗口 + 按模型子集（magpie） | 吸收 | P5-B；Kimi Code/GLM Coding Plan 等计划 key 是 5h 窗口，日/周/月不够用 |
| cache write 5m/1h 两档计价（magpie） | 吸收 | P5-C；价格表已有 cache 字段，加一档 |
| 按会话聚合成本（magpie Sessions） | 吸收 | P5-C；账本已有 session 维度，纯聚合 |
| OTLP Langfuse 映射（OmniRoute） | 吸收（可选） | P5-C；生态钩子，不阻塞主验收 |
| Playground Compare 并行对比（OmniRoute） | 吸收 | P5-D；同 prompt 多候选并行 + 实时 TTFT/TPS，比单模型测试台更贴合"帮你选模型"定位 |
| fast 速度路由（magpie 7 模式之一） | **新增** | P5-D 可选；账本已逐 attempt 记 ttft/duration，只差 extractor + Strategy，性价比最高的新增 |
| Profiles 配置快照（magpie） | **新增** | P5-D 可选；落在 onboard crate，与"精确合并、可还原"能力同源 |
| 意图路由（magpie 分类器选规则） | 缓议 | P6 评估；可用既有判官设施实现为 Strategy，但每 turn 一次模型调用的延迟/成本需实测 |
| Quota-Share 公平共享（OmniRoute） | 缓议 | P5-B 落地后的自然延伸，现在做为时过早 |
| 订阅账号驱动（magpie 护城河） | 维持 P7 | **战略缺口而非功能缺口**：只有订阅没有 API key 的用户短期覆盖不了，是否提前需单独决策 |
| 免费额度聚合（OmniRoute 定位核心） | **不做** | provider 维护跑步机 + ToS 风险，与产品身份冲突（见 §7） |
| token 压缩（OmniRoute 12 引擎） | **不做** | 改写请求内容违背 passthrough 保真契约，coding agent 风险高（见 §7） |
| 局域网共享/mTLS、MCP Library、隐私遮蔽 | 维持非目标 | 安全面与范围蔓延代价大于收益 |

---

## 1. 目标与非目标

### 1.1 目标

1. **开箱即用**：新用户 5 分钟内完成「选厂商 → 贴 key → 选模型 → agent 接入」全流程，不需要手写 TOML。
2. **Provider 一等公民**：厂商模板（预置 base_url/协议/模型目录/价格）、多账号（同名厂商多把 key）、key 健康度与配额窗口。
3. **模型目录自动化**：从上游 `/v1/models`（及 models.dev 元数据）拉取候选模型，用户勾选即成候选，不再手填 model 字符串。
4. **配额感知路由**：429/配额头学习 → 账号配额窗口 → 通过既有 `SignalBag`/Strategy 扩展点参与路由（reset-aware 接力）。
5. **用量与成本分析**：按 agent/虚拟模型/物理候选/账号/会话 维度的成本分解、时序热力图、缓存命中分解（含 Anthropic cache write 5m/1h 两档）、OTLP 导出（可选 Langfuse 映射）。
6. **实时路由可视化**：每次请求的选择→计划→派发→接力→判官全过程时间线（数据已有，补 UI 与事件流）。
7. **Playground**：在桌面端直接对任意候选/虚拟模型发起测试请求（流式、计时、原始事件查看）；Compare 模式同 prompt 最多 4 候选并行对比。
8. **网关 key 限额**：named key 绑定 token 与成本预算，窗口支持 日/周/月/每 N 小时（对齐计划 key 的 5h 节奏），可按模型子集计限额，超限 429 + Retry-After + 重置时间。

### 1.2 非目标（延续 v2 §14，新增）

- 不做隐私遮蔽、插件系统、Web 搜索、媒体生成（P7 单独评估）。
- 不做云同步/WebDAV/S3 备份。
- 不做 Gemini 原生协议（`ProviderKind::Other("gemini")` 透传不够时再评估）。
- 不做 OAuth 订阅账号驱动（Claude/Codex 订阅登录态发现）——v3 只管理 **API key 账号**；订阅发现列 P7。
- 不改内核路由语义：级联/判官/sticky/Escalation 的行为与 v2 完全一致。

### 1.3 硬门（不变）

- `core` / `decide` / `proto` diff = 0。
- 明文 key 只存在于 Rust 侧内存与凭据存储；管理面只返回 `credRef` + 指纹后 4 位。
- 管理面走 `/v2/admin` HTTP，桌面端 WebView 无 token。

---

## 2. 总体架构增量

v2 的 crate 边界不变，新增/扩展：

```
v2/crates/
  contracts      # 扩展: Provider、Account、ModelCatalog、QuotaWindow、GatewayKey DTO（ts-rs 继续导出）
  upstream       # 扩展: AccountResolver（多账号轮换）、QuotaTracker（429/头学习）、CatalogFetcher
  control        # 扩展: /v2/admin/providers|accounts|catalog|quotas|usage|playground|keys 端点
  server         # 不动（鉴权中间件接 GatewayKey 限额检查）
  agent          # 不动（通过 SignalBag 读配额信号）
新 crate:
  catalog        # models.dev 元数据 + 上游 /v1/models 拉取 + 价格表; 产出 Candidate 草稿
```

依赖方向仍是单向：`catalog → contracts`；`upstream → contracts + catalog`；`control → upstream/catalog`。内核 `agent` 只通过 `SignalBag` 与 `Candidate` 消费外围（`core` 现有扩展点，零改动）。

### 2.1 关键设计决定：Provider/账号如何进入既有路由

v2 内核只认 `Candidate { upstream: UpstreamRef { base_url, model, credential: CredentialRef }, ... }`。v3 的映射：

```
Provider(厂商模板)  1──N  Account(账号=key+代理+自定义header+备注)  1──N  ModelEntry(目录勾选)
                                        │
                                        ▼ 组装/投影
                            Candidate（物理候选，tier/strength/价格由用户或模板给默认）
                                        │
                                        ▼
                        VirtualModel.candidate_pool（既有机制）
```

- **投影而非替换**：Provider/Account/ModelEntry 是管理面的"活数据"，保存时**编译**成 `config.toml` 的 `candidates` 段（既有 schema），内核完全无感知。好处：零内核改动、配置审计/回滚/CAS 全部复用既有 config_service。
- **账号轮换**：同一 `base_url+model` 有 N 个账号时，编译期展开为 N 个候选（`id = {model}@{account}`），用既有 failover + 新的 quota 信号决定顺序。**不做运行时 key 池**——那会破坏"Plan 是物理候选列表"的契约与决策快照确定性（v2 §4.7）。
- **凭据**：`Account.key` 存入 OS keyring / 加密文件（`keyring:tr2/{account_id}`），`config.toml` 只落 `CredentialRef` 引用。既有 `env:`/`file:` 继续支持。
- **反向导入**：从本机已有 agent 配置（Claude Code/Codex/OpenCode 等，复用 `onboard` 的路径与解析知识）读取 base_url/key 引用，一键生成 Provider+Account 草稿；key 只转成引用，不复制明文。
- **目录维护**：auto-include 通配符（如 `gpt-*`、正则）自动纳入新模型；上游宣告 deprecated 的模型自动下架（请求成功或一周后恢复，参考 magpie 口径）。

### 2.2 配额进入路由的方式（唯一一处内核邻接改动）

- `upstream` 新增 `QuotaTracker`：从 429 响应体/`x-ratelimit-*` 头学习每个账号的窗口与剩余额度，持久化到 SQLite（新表 `quota_windows`）。
- 配额状态通过 **`SignalBag`** 注入（`SignalId("provider.quota_ratio")` 等）——这是 v2 设计里预留的插拔位（§4.3），`contracts/src/signals.rs` 的 extractor 接口不改，只加一个内置 extractor 实现（落在 `agent` 外的 `upstream`，注册进流水线）。
- 熔断语义不变；配额耗尽 ≠ 熔断（熔断是健康，配额是预算），reset-aware 策略作为新增内置 Strategy（`upstream` 或新 `strategy-quota` crate，走 `Strategy` trait 注册）。
- **fast 速度策略（P5-D 可选）**：同走 SignalBag/Strategy 扩展点——extractor 从账本读各候选近期 TTFT/输出速度（数据已逐 attempt 留存），Strategy 据此调整 preference；零内核改动。

---

## 3. 管理面 v2 契约（增量）

既有 `/v2/admin` 契约保持兼容，新增资源（全部 ts-rs 导出 + contract-check 门）：

| 资源 | 端点 | 说明 |
|---|---|---|
| Provider | `GET/POST/PATCH/DELETE /v2/admin/providers` | 厂商实例（模板实例化 + 自定义）；含模型目录同步状态；`POST /v2/admin/providers/import` 从本机 agent 配置反向导入草稿 |
| Account | `GET/POST/PATCH/DELETE /v2/admin/providers/{id}/accounts` | key 写入后响应只含 `fingerprint`（后 4 位）+ `credRef`；含代理与自定义 header；`POST .../verify` 实测连通性 |
| Provider 模板 | `GET /v2/admin/provider-templates` | 内置 20+ 厂商模板（OpenAI/Anthropic/DeepSeek/Kimi/GLM/Qwen/OpenRouter/Groq/xAI/SiliconFlow/Ollama/LM Studio/任意 OpenAI 兼容…） |
| 模型目录 | `POST /v2/admin/providers/{id}/catalog/refresh`、`GET .../catalog` | 拉上游 `/v1/models` + models.dev 元数据（context/价格/能力标签） |
| 候选投影 | `GET /v2/admin/candidates`、`POST /v2/admin/candidates/compile` | 查看/触发 Provider→candidates 编译（写 config，走 CAS） |
| 配额 | `GET /v2/admin/quotas` | 各账号窗口/剩余/重置时间；`POST .../{id}/reset` 手动清 |
| 用量分析 | `GET /v2/admin/usage?by={agent,model,candidate,account,session}&window=...`、`GET /v2/admin/usage/heatmap` | 账本聚合（数据已有，加维度与热力图）；cache write 按 5m/1h 两档分解 |
| Playground | `POST /v2/admin/playground/execute` | 管理面直达某候选/虚拟模型的测试请求（绕过路由、不计账本路由决策，单独记 playground 标记）；Compare 由前端并行调用复用此端点 |
| 网关 key 限额 | `PATCH /v2/admin/tokens/{id}`（limits 字段） | token 与成本预算；窗口 日/周/月/每 N 小时，可按模型子集；超限返回 429 + Retry-After + 重置时间 |
| Profiles（P5-D 可选） | `GET/POST/DELETE /v2/admin/profiles`、`POST /v2/admin/profiles/{id}/apply` | agent 配置快照命名保存/一键切换；复用 onboard 合并器，幂等可还原 |

事件流新增 kinds：`provider.changed`、`account.quota_exhausted`、`catalog.refreshed`、`key.limit_hit`（SSE 既有机制，只加枚举）。

---

## 4. 客户端增量（桌面端）

| 页面 | 现状 | v3 |
|---|---|---|
| Providers（新） | 无 | 模板画廊 → 实例化 → 账号管理（添加 key/验证/代理/header）→ 模型目录勾选（auto-include/deprecated 下架）→ 一键生成候选并挂到虚拟模型；支持从本机 agent 配置反向导入 |
| Overview | 状态卡 | + 今日成本/请求数、配额临近耗尽的账号提醒条 |
| VirtualModels | 手填池 | 候选选择器按 Provider/账号分组树形展示 |
| Stats | 3 窗口序列 | + 成本分解（agent/模型/账号/会话）、热力图、缓存命中分解（含 5m/1h 两档） |
| Requests | 列表+详情 | 详情页加「路由时间线」可视化：Selection→Strategies→Plan→Dispatch→(Failover/Judge) 每步的 reason/耗时 |
| Playground（新） | 无 | 模型/参数/消息编辑器，流式输出，计时与原始 SSE 事件查看；Compare 模式最多 4 候选并行、每列实时 TTFT/TPS/成本 |
| Onboarding | 8 agent | 扩到常用列表（~20 agent）；完成后直接推荐 Providers 页；可选 Profiles（agent 配置快照命名保存/一键切换） |
| Settings | supervisor | + 限额默认、目录刷新周期、OTLP 端点 |

---

## 5. 分期

| 期 | 内容 | 验收 |
|---|---|---|
| **P5-A** Provider 层 | contracts DTO、模板库、providers/accounts CRUD + keyring（含代理/自定义 header）、catalog 拉取（auto-include/deprecated 下架）、agent 配置反向导入、候选编译投影、Providers 页 | 全流程不写 TOML 完成「加厂商→加 key→勾模型→出候选」；从已装 agent 配置一键导入出候选；contract-check、config CAS 审计完整 |
| **P5-B** 配额与限额 | QuotaTracker（429/头学习）、quota_windows 表、SignalBag extractor、reset-aware 策略、网关 key 限额（日/周/月/每 N 小时 + 模型子集）、Quotas UI | 双账号 key 模拟 429 后自动接力且决策快照可解释；5h 窗口计划 key 限额生效；key 超限 429 + Retry-After |
| **P5-C** 分析与可视化 | usage 聚合端点（含 session 维度）、热力图、成本分解（cache write 5m/1h 两档）、路由时间线 UI、OTLP 导出（可选 Langfuse 映射） | 按 agent/模型/账号/会话四维出账；时间线与账本 attempts 一致 |
| **P5-D** Playground 与打磨 | playground 执行端点、Playground 页（含 Compare 4 候选并行）、Onboarding 扩列表、Overview 提醒；**可选**：fast 速度策略（账本 TTFT extractor + Strategy）、Profiles（onboard 配置快照） | Playground 流式计时正确、Compare 各列指标独立；agent 列表 ≥20；可选项若做：fast 策略决策快照可回放、Profiles 切换幂等可还原 |

每期结束跑全门：fmt/clippy -D warnings/全部测试/check-deps/fixtures-check/config-only-check/contract-check/core-decide-proto diff=0。

## 6. 风险

| 风险 | 缓解 |
|---|---|
| 候选编译投影与手写 TOML 冲突（用户手改 candidates 段） | config 文档标记 `compiled_from: providers`；手改走审计，下次编译前 diff 提示 |
| models.dev 不可达/被墙 | 目录元数据是增强不是依赖；拉不到就用上游 `/v1/models` + 手填价格 |
| 配额学习对无标准头的厂商失效 | 回退到「计数式窗口」（本地滑动窗口估算），标记 `estimated: true` |
| keyring 在 Linux 无桌面环境不可用 | 回退加密文件（机器绑定 key），文档说明 |
| 范围蔓延回内核 | 每期 PR 强制 diff=0 检查；SignalBag/Strategy 是唯一许可的内核邻接面 |

---

## 7. 明确不做（P5–P6）

订阅账号发现与驱动（Claude/Codex 登录态）、隐私遮蔽、插件中间件、Web 搜索、图片/视频、局域网共享、远程实例、云备份 —— 全部记入 backlog，待 P5 落地后按用户反馈排序。

竞品有但我们**主动拒绝**的（2026-11-17 调研决策，见 §0.3）：

- **免费额度聚合**（OmniRoute 的定位核心）：375+ provider 的持续维护跑步机、web-cookie/keyless 接入的 ToS 风险，与"决策质量与可解释"的产品身份冲突；不在"省钱"维度与其正面交锋。
- **token 压缩**（OmniRoute 12 引擎）：改写请求内容违背"未知字段 passthrough 保真"的协议契约，对 coding agent 的工具参数/代码块风险高；其节省数字为营销口径，未经独立验证。

另：**订阅账号驱动是战略缺口**（magpie 护城河，覆盖"只有订阅没有 API key"的用户群），维持 P7 但值得单独决策是否提前。

---

## 8. 实施状态（2026-11-17）

P5-A ~ P5-D 已全部落地（分支 `v2`），每期均过全门（clippy 0 / 全测试 / contract-check / check-deps / config-only-check / fixtures-check / `core`+`decide`+`proto` diff = 0）。

| 期 | 提交 | 交付要点 |
|---|---|---|
| P5-A | `25b95ee` + `4f17601` + `d6eeef1` | 20 厂商模板、providers/accounts/catalog CRUD（migration 6/7）、凭据文件 + 指纹后 4 位、目录拉取、候选编译投影（CAS/备份/审计）、`file:` 相对路径按数据目录解析、每 key 代理与自定义 header（经 capability.tags 到 upstream）、auto-include 通配符 + deprecated 下架、agent 配置反向导入（`tr-onboard::import`，明文永不导入）、Providers 页（模板/账号/目录/导入/编译） |
| P5-B | `89386bf` | 限流头解析（OpenAI/Anthropic/Retry-After，6 测试）、`quota_windows`（migration 8）、429 与成功观测落库、`SignalBag` 注入、`quota-reset` 策略（默认启用，耗尽过滤 + 可选 reset-aware 排序）、数据面 token 鉴权 + 网关 key 限额（日/周/月/小时/N 小时 + 模型子集，429 + Retry-After，含集成测试）、`/v2/admin/quotas`、Tokens 页限额编辑器、Quotas 页 |
| P5-C | `b39d1eb` | 五维用量聚合（agent/虚拟模型/物理候选/账号/会话）+ 合计行 + TTFT 均值 + 缓存命中率、UTC 天×小时热力图、Stats 页成本分解与热力图、Requests 详情路由时间线（selection→strategies→plan→dispatch→failover→judge→outcome） |
| P5-D | `6de34a8` | `POST /v2/admin/playground/execute`（candidate 绕过路由直达上游 / virtual 走正常路由，服务端聚合流式以取真实 TTFT）、Playground 页（≤4 目标并行 Compare，TTFT/耗时/TPS/错误列）、Onboarding 手动 agent 矩阵（8 精确合并 + 14 手动 = 22 条接入路径）、Overview 配额耗尽提醒、`fast` 速度策略（账本 TTFT 信号 + 可选启用） |

### 本期未做 / 部分做（如实记录）

- **cache write 5m/1h 两档分解**：`tr-proto` 的 `Usage` 只有单一 `cache_write_tokens`，拆分需改内核（`proto` diff=0 门禁）；当前按 read/write 两桶在五维聚合中呈现。若放开内核，代价是 proto 契约与 codec 测试更新。
- **OTLP 导出 / Langfuse 映射**（§0.3 标记"可选"）：未实现；`GET /v2/admin/usage` 已提供结构化数据，可后接 exporter。
- **Profiles 配置快照**（§0.3 标记"可选"）：未实现；`tr-onboard` 的精确合并/幂等能力已具备，快照层可直接复用。
- 订阅账号驱动、隐私遮蔽、插件、Web 搜索、媒体、局域网共享：维持 §1.2 非目标（P7 评估）。
