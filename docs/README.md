# Token Router 文档

Token Router 是一个本地 LLM 网关：把多个 coding agent 的模型请求收拢到一个入口，由路由引擎决定每次请求用哪个模型，并留下完整可解释的决策记录。

> 产品尚未发布。文档以 `v2/` 代码为事实基准；v1（仓库根 `src/`）为历史产品线。

## 文档地图

### 了解产品

| 文档 | 内容 | 状态 |
|---|---|---|
| [product.zh.md](./product.zh.md) | 产品定位、与 magpie/OmniRoute 对标、核心能力、已知限制 | 现行 |
| [roadmap.zh.md](./roadmap.zh.md) | v3 路线图：Provider 层、配额、分析、Playground（P5-A~D） | 草案 |
| [research/competitor-omniroute-magpie.zh.md](./research/competitor-omniroute-magpie.zh.md) | 竞品深度调研（2026-11-17 一手来源快照，逐模块逐功能带来源 URL） | 快照 |

### 使用指南（guide/）

| 文档 | 内容 |
|---|---|
| [guide/quickstart.zh.md](./guide/quickstart.zh.md) | 快速开始：启动网关 → 配候选 → 建虚拟模型 → 接入 agent |
| [guide/configuration.zh.md](./guide/configuration.zh.md) | config.toml 配置参考 |
| [guide/agents.zh.md](./guide/agents.zh.md) | Agent 接入与 WSL |
| [guide/desktop.zh.md](./guide/desktop.zh.md) | 桌面端页面说明 |

### 产品需求（prd/）

优势功能的详细需求（需求编号 + 验收标准）。已实现部分表述现行冻结语义，规划部分标注路线图期号。

| 文档 | 内容 | 状态 |
|---|---|---|
| [prd/routing.zh.md](./prd/routing.zh.md) | 路由决策引擎：级联/判官/sticky/latch/failover/仲裁 | 现行（内核冻结） |
| [prd/explainability.zh.md](./prd/explainability.zh.md) | 决策可解释与回放：快照/账本/分账 + P5-C 时间线 | A 现行 / B 规划 |
| [prd/security.zh.md](./prd/security.zh.md) | 密钥与信任边界：凭据引用/token/配置治理 | 现行 |
| [prd/onboarding.zh.md](./prd/onboarding.zh.md) | Agent 接入：精确合并/WSL + P5-D 扩列表/Profiles | 现行 + 规划 |

### 设计与契约

| 文档 | 内容 | 状态 |
|---|---|---|
| [architecture.zh.md](./architecture.zh.md) | v2 架构精编：crate 边界、协议漏斗、路由语义、安全模型 | 现行 |
| [contracts/admin-api.zh.md](./contracts/admin-api.zh.md) | 管理 API 契约 v1（`/v2/admin`） | 冻结，只增不改 |
| [s0-gap-report.md](./s0-gap-report.md) | 协议覆盖报告（`cargo xtask fixtures-check` 自动生成，勿手改） | 生成物 |

### 开发者（dev/）

| 文档 | 内容 |
|---|---|
| [dev/contributing.zh.md](./dev/contributing.zh.md) | 开发环境、门禁、xtask 命令 |
| [dev/release.zh.md](./dev/release.zh.md) | 打包、签名、发布流程与已知限制 |
| [dev/frontend-design-system.zh.md](./dev/frontend-design-system.zh.md) | 前端设计系统（UnoCSS + OpenChamber 规范） |

### 历史档案（archive/）

不再维护，仅供追溯决策过程。其中 [rewrite-v2-design.zh.md](./archive/rewrite-v2-design.zh.md) 仍是**内核语义的冻结契约**，架构精编版与其冲突时以它为准。

| 文档 | 原身份 |
|---|---|
| [archive/architecture.md](./archive/architecture.md) | v1 网关架构（0.19.11） |
| [archive/model-router.md](./archive/model-router.md) | v1 决策内核 |
| [archive/rewrite-v2-design.zh.md](./archive/rewrite-v2-design.zh.md) | v2 内核设计（语义权威） |
| [archive/rewrite-v2-implementation-plan.zh.md](./archive/rewrite-v2-implementation-plan.zh.md) | v2 S0–P3 实施计划（已执行） |
| [archive/p4-client-refactor-plan.zh.md](./archive/p4-client-refactor-plan.zh.md) | P4 客户端技术方案（已交付） |
| [archive/p4-client-development-plan.zh.md](./archive/p4-client-development-plan.zh.md) | P4 开发计划（已交付） |
| [archive/p4-delivery-report.zh.md](./archive/p4-delivery-report.zh.md) | P4 交付报告（M0–M7 基线） |
| [archive/p1-shadow-report.md](./archive/p1-shadow-report.md) | P1 shadow 评测快照 |
| [archive/p2-eval.md](./archive/p2-eval.md) | P2 评测对照快照 |
| [archive/s0-client-smoke.md](./archive/s0-client-smoke.md) | S0 真实客户端冒烟快照 |

另：`ota-release-notes.json` 为 v1 桌面端 OTA 更新说明数据，非文档。

## 维护规则

- 事实以代码为准；文档与代码冲突时改文档。
- 内核语义（路由/协议/配置契约）变更必须先改 [内核设计](./archive/rewrite-v2-design.zh.md) 或其后继契约，再改精编版。
- 管理 API 变更遵循 [契约](./contracts/admin-api.zh.md) 的只增不改规则，`contract-check` 门禁保证生成物一致。
- 过程性报告（评测/冒烟/交付）产出后归档，不回填修改。