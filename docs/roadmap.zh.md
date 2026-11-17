# 产品路线图（v3：对齐 OmniRoute / magpie）

> 状态：草案 v1（2026-11-17）
> 前置文档：[v2 内核设计（归档，语义权威）](./archive/rewrite-v2-design.zh.md)、[管理 API 契约 v1](./contracts/admin-api.zh.md)（将被 v2 契约增量扩展）、[P4 交付报告（归档，当前交付基线）](./archive/p4-delivery-report.zh.md)
> 定位变更：**从"路由策略研究项目"转向"对齐 OmniRoute / magpie 的完整本地 LLM 网关产品"，路由策略仍是差异化卖点，但不再是唯一卖点。**
> 文中 "v2 §x" 均指内核设计文档章节。

---

## 0. 一句话

v3 = v2 内核不动 + 补齐产品层：**Provider/账号管理、模型目录、配额跟踪、用量与成本分析、实时路由可视化、Playground、网关 key 限额** —— 对齐 magpie（2.9 万星）/ OmniRoute 的日常使用面，同时保留 v2 独有的级联+判官+虚拟模型路由。

### 0.1 与参考产品的对标结论

调研基准（2026-11-17）：

| 参考产品 | 定位 | 核心功能面 |
|---|---|---|
| [magpie](https://github.com/yetone/magpie)（Go，~2.9 万星） | 本地网关，"每个 agent 的模型，一处管理" | 4 协议互转、30+ 预置厂商、80+ agent 配置改写、路由组（smart/order/rotate/usage/pace）、订阅账号 failover、意图路由、隐私遮蔽、插件、Profiles、用量成本、局域网共享 key 限额、Web 搜索、WSL |
| [OmniRoute](https://github.com/diegosouzapw/OmniRoute)（TS，~9 万星） | OpenRouter 开源替代 | ~370 providers、14 种 combo 策略、`auto/*` 前缀路由、9 因子评分、热力图分析、Playground（翻译器+模型测试台）、配额跟踪、熔断、SSRF 防护、CLI 套件 |

### 0.2 差距矩阵（v2 现状 → 产品级）

| 能力域 | magpie/OmniRoute | v2 现状 | 判定 |
|---|---|---|---|
| 网关协议 | OpenAI Chat/Responses/Anthropic/Gemini 互转，流式+工具+reasoning | 3 协议单漏斗，流式+工具+reasoning+`previous_response_id` | ✅ 已对齐（Gemini 延后） |
| 路由策略 | 分组 5 模式 / 14 combo / 9 因子评分 | Strategy trait + 级联 + 判官 + sticky/ContextHold + Escalation latch | ✅ **更强**（差异化核心） |
| **Provider/账号管理** | 30+ 预置厂商模板、多账号、OAuth 订阅接入、key 健康度、每 key 代理 | 手填 base_url + `env:NAME` 的裸表格 | ❌ **最大差距 → P5-A** |
| **模型目录** | models.dev 元数据富化、模型可见性开关、目录刷新 | 手填 model 字符串 | ❌ → P5-A |
| **配额/余额** | 配额窗口跟踪、余额监控、续期提醒、每日 warm-up | 无（仅熔断计数） | ❌ → P5-B |
| **用量与成本** | 按 agent/账号/模型成本、热力图、缓存命中分解、OTLP 导出 | requests/stats/series 三个基础端点 + 价格模型（账本数据全有，呈现不足） | ⚠️ → P5-C |
| 实时路由视图 | magpie Live Routing、OmniRoute Live Monitor | attempts/contributions 数据在账本里，前端只有列表 | ⚠️ → P5-C |
| Playground | 模型测试台（Monaco/流式/计时）、协议转换器 | 无 | ❌ → P5-D |
| 网关 key 限额 | named key + 日/周/月 token 与成本限额 | token + scope，无限额 | ⚠️ → P5-B |
| Agent onboarding | 配置自动改写（80+ agent）、订阅发现、WSL | 8 agent 精确合并器 + WSL + verify | ✅ 基本对齐（扩充 agent 列表即可） |
| 隐私遮蔽 / 插件 / Web 搜索 / 媒体 | 有 | 明确不做 | P7 评估 |
| 断路器/健康 | breaker + cooldown + half-open | `upstream` 已有 + UI 呈现 | ✅ |

> 结论：v2 的"发动机"（内核）已经比两家参考产品强，缺的是"仪表盘和油箱"——Provider 层、配额层、分析层、测试层。这四层全部落在 `control`/`upstream` 外围，**不动 `core`/`decide`/`proto`**（diff=0 门继续有效）。

---

## 1. 目标与非目标

### 1.1 目标

1. **开箱即用**：新用户 5 分钟内完成「选厂商 → 贴 key → 选模型 → agent 接入」全流程，不需要手写 TOML。
2. **Provider 一等公民**：厂商模板（预置 base_url/协议/模型目录/价格）、多账号（同名厂商多把 key）、key 健康度与配额窗口。
3. **模型目录自动化**：从上游 `/v1/models`（及 models.dev 元数据）拉取候选模型，用户勾选即成候选，不再手填 model 字符串。
4. **配额感知路由**：429/配额头学习 → 账号配额窗口 → 通过既有 `SignalBag`/Strategy 扩展点参与路由（reset-aware 接力）。
5. **用量与成本分析**：按 agent/虚拟模型/物理候选/账号 维度的成本分解、时序热力图、缓存命中分解、OTLP 导出。
6. **实时路由可视化**：每次请求的选择→计划→派发→接力→判官全过程时间线（数据已有，补 UI 与事件流）。
7. **Playground**：在桌面端直接对任意候选/虚拟模型发起测试请求（流式、计时、原始事件查看）。
8. **网关 key 限额**：named key 绑定 日/周/月 token 与成本预算，超限 429。

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
Provider(厂商模板)  1──N  Account(账号=key+代理+备注)  1──N  ModelEntry(目录勾选)
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

### 2.2 配额进入路由的方式（唯一一处内核邻接改动）

- `upstream` 新增 `QuotaTracker`：从 429 响应体/`x-ratelimit-*` 头学习每个账号的窗口与剩余额度，持久化到 SQLite（新表 `quota_windows`）。
- 配额状态通过 **`SignalBag`** 注入（`SignalId("provider.quota_ratio")` 等）——这是 v2 设计里预留的插拔位（§4.3），`contracts/src/signals.rs` 的 extractor 接口不改，只加一个内置 extractor 实现（落在 `agent` 外的 `upstream`，注册进流水线）。
- 熔断语义不变；配额耗尽 ≠ 熔断（熔断是健康，配额是预算），reset-aware 策略作为新增内置 Strategy（`upstream` 或新 `strategy-quota` crate，走 `Strategy` trait 注册）。

---

## 3. 管理面 v2 契约（增量）

既有 `/v2/admin` 契约保持兼容，新增资源（全部 ts-rs 导出 + contract-check 门）：

| 资源 | 端点 | 说明 |
|---|---|---|
| Provider | `GET/POST/PATCH/DELETE /v2/admin/providers` | 厂商实例（模板实例化 + 自定义）；含模型目录同步状态 |
| Account | `GET/POST/PATCH/DELETE /v2/admin/providers/{id}/accounts` | key 写入后响应只含 `fingerprint`（后 4 位）+ `credRef`；`POST .../verify` 实测连通性 |
| Provider 模板 | `GET /v2/admin/provider-templates` | 内置 20+ 厂商模板（OpenAI/Anthropic/DeepSeek/Kimi/GLM/Qwen/OpenRouter/Groq/xAI/SiliconFlow/Ollama/LM Studio/任意 OpenAI 兼容…） |
| 模型目录 | `POST /v2/admin/providers/{id}/catalog/refresh`、`GET .../catalog` | 拉上游 `/v1/models` + models.dev 元数据（context/价格/能力标签） |
| 候选投影 | `GET /v2/admin/candidates`、`POST /v2/admin/candidates/compile` | 查看/触发 Provider→candidates 编译（写 config，走 CAS） |
| 配额 | `GET /v2/admin/quotas` | 各账号窗口/剩余/重置时间；`POST .../{id}/reset` 手动清 |
| 用量分析 | `GET /v2/admin/usage?by={agent,model,candidate,account}&window=...`、`GET /v2/admin/usage/heatmap` | 账本聚合（数据已有，加维度与热力图） |
| Playground | `POST /v2/admin/playground/execute` | 管理面直达某候选/虚拟模型的测试请求（绕过路由、不计账本路由决策，单独记 playground 标记） |
| 网关 key 限额 | `PATCH /v2/admin/tokens/{id}`（limits 字段） | 日/周/月 token 与成本预算；超限返回 429 + 重置时间 |

事件流新增 kinds：`provider.changed`、`account.quota_exhausted`、`catalog.refreshed`、`key.limit_hit`（SSE 既有机制，只加枚举）。

---

## 4. 客户端增量（桌面端）

| 页面 | 现状 | v3 |
|---|---|---|
| Providers（新） | 无 | 模板画廊 → 实例化 → 账号管理（添加 key/验证/代理）→ 模型目录勾选 → 一键生成候选并挂到虚拟模型 |
| Overview | 状态卡 | + 今日成本/请求数、配额临近耗尽的账号提醒条 |
| VirtualModels | 手填池 | 候选选择器按 Provider/账号分组树形展示 |
| Stats | 3 窗口序列 | + 成本分解（agent/模型/账号）、热力图、缓存命中分解 |
| Requests | 列表+详情 | 详情页加「路由时间线」可视化：Selection→Strategies→Plan→Dispatch→(Failover/Judge) 每步的 reason/耗时 |
| Playground（新） | 无 | 模型/参数/消息编辑器，流式输出，计时与原始 SSE 事件查看 |
| Onboarding | 8 agent | 扩到 magpie 同级的常用列表（~20 agent）；完成后直接推荐 Providers 页 |
| Settings | supervisor | + 限额默认、目录刷新周期、OTLP 端点 |

---

## 5. 分期

| 期 | 内容 | 验收 |
|---|---|---|
| **P5-A** Provider 层 | contracts DTO、模板库、providers/accounts CRUD + keyring、catalog 拉取、候选编译投影、Providers 页 | 全流程不写 TOML 完成「加厂商→加 key→勾模型→出候选」；contract-check、config CAS 审计完整 |
| **P5-B** 配额与限额 | QuotaTracker（429/头学习）、quota_windows 表、SignalBag extractor、reset-aware 策略、网关 key 限额、Quotas UI | 双账号 key 模拟 429 后自动接力且决策快照可解释；key 超限 429 |
| **P5-C** 分析与可视化 | usage 聚合端点、热力图、成本分解、路由时间线 UI、OTLP 导出 | 按 agent/模型/账号三维出账；时间线与账本 attempts 一致 |
| **P5-D** Playground 与打磨 | playground 执行端点、Playground 页、Onboarding 扩列表、Overview 提醒 | Playground 流式计时正确；agent 列表 ≥20 |

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
