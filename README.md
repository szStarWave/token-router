# Token Router

本地 LLM 网关：多个 coding agent 共用一个模型入口，每次请求由路由引擎决定用哪个模型，并且每个决定都留有事后可查的证据。

运行在你自己的机器上：启动网关，agent 接进来，模型候选配进去，之后每个 agent 只是"指到一个模型名"。用哪个模型、为什么用它、花了多少钱，都在本地留痕可查。

> 产品尚未公开发布（安装包签名未执行、24h 稳定性未验证）。能力现状的严格口径见 [docs/product.zh.md](docs/product.zh.md)。

## 定位

| 维度 | 说明 |
|------|------|
| **是什么** | 本地 LLM 网关：三协议入口归一化，纯函数路由内核产出物理候选计划，全程 SQLite 账本留痕 |
| **服务对象** | 本机同时跑多个 coding agent（Claude Code、Codex、OpenCode、Cursor 等）、手里有多家模型 API key、想要统一入口和可控成本的开发者 |
| **不是什么** | 不是 Agent 本体（不负责工具执行、记忆、消息渠道）；不做资源层账号托管（那是 magpie/OmniRoute 的事） |
| **差异化** | 别家管你的账号和额度（资源层），这里管你的每一次模型选择（决策层）：决策快照可确定性回放，路由是明文仲裁的决策语义 |

## 快速开始

依赖：Rust（含 cargo）。

```bash
# 构建网关与 CLI
cd v2
cargo build --release

# 准备最小配置（按需编辑：候选 base_url / model / credential）
cp example/config.minimal.toml /path/to/config.toml

# 启动独立服务端（默认监听 127.0.0.1:16621）
cargo run -p tr-server --release -- --config /path/to/config.toml
```

把 agent 的 OpenAI 兼容 `base_url` 指向网关：

```
http://127.0.0.1:16621/v1
```

请求体里的 `model` 字段填虚拟模型（如内置的 `auto`），路由引擎负责选实际物理候选。

日常使用推荐桌面端（Tauri，supervisor 自动托管网关，默认 `127.0.0.1:17621`）：13 个页面完成配置闭环，不必手改 TOML。配置文件的事实基准是 [docs/guide/configuration.zh.md](docs/guide/configuration.zh.md)，最小示例见 [v2/example/config.minimal.toml](v2/example/config.minimal.toml)（`schema_version` 严格等于 `1`）。

## 协议面

三条 wire 入口，单漏斗归一化，未知字段 passthrough 保真：

| 协议 | 端点 |
|---|---|
| OpenAI Chat Completions | `POST /v1/chat/completions` |
| OpenAI Responses | `POST /v1/responses` |
| Anthropic Messages | `POST /v1/messages` |

另有 `GET /v1/models`、`GET /health`。支持流式 + 工具 + reasoning + `previous_response_id` 续接；语义提交（首个 content delta 或完整 tool_call 帧）后不切换候选。

## 路由

路由是互相仲裁的决策语义，仲裁顺序明文；策略经统一 trait 插拔，新能力不改内核。

- **级联（cascade）**：按 tier/strength 顺序接力，端侧先行、云端兜底。
- **判官（judge）**：LLM/HTTP 决策模型咨询，`off` / `shadow` / `active` 三档，默认 `off`；预算、deadline、fail-open 语义齐全。
- **Sticky 会话保持 / ContextHold**：会话内粘住已验证的候选。
- **升级闩锁（Escalation latch）**：升级需证据确认，不静默换行为。
- **Failover + 熔断 + 冷却 + 半开**：上游健康与配额语义分离。
- **内置策略**：`quota-reset`（配额感知接力，默认启用）、`fast`（账本 TTFT 数据驱动的速度路由，可选）。

完整语义见 [docs/prd/routing.zh.md](docs/prd/routing.zh.md)；决策契约与回放语义见 [docs/prd/explainability.zh.md](docs/prd/explainability.zh.md)。

## 能力面

- **Provider 层**：20 个内置厂商模板（DeepSeek/OpenAI/Anthropic/Kimi/GLM/Qwen/OpenRouter/Groq 等）→ 多账号（key 入凭据存储，界面只回显指纹后 4 位；每 key 代理与自定义 header）→ 上游 `/v1/models` 目录拉取勾选 → 一键编译为候选；已装 agent 配置可反向扫描导入。
- **虚拟模型**：候选组成虚拟模型（agent 实际指向的"模型名"），候选池按 Provider/账号分组；组级判官可选 LLM 或 JEV(SystemOne) 后端。
- **Agent 接入**：8 个 agent 自动精确合并（detect → diff 预览 → apply → verify，保留用户字段与注释、原子写 + 备份），14 个常用 agent 手动引导；WSL 同等对待。合计 22 条接入路径。
- **配额与限额**：候选配额窗口从 429/限流头学习（无标准头降级本地估算并标记）；网关 key 限额（token/成本预算，小时/日/周/月/每 N 小时窗口，可按模型子集限定，超限 429 + Retry-After）。
- **用量分析**：五维成本分解（agent / 虚拟模型 / 物理候选 / 账号 / 会话）、活动热力图、TTFT 均值与缓存命中率、成本 answer/routing 分账（micros 整数精度）。
- **诊断**：请求账本 + 路由时间线（每步 reason 与耗时）、逐次尝试记录、决策快照确定性回放（`tr-cli replay`）、doctor 自检、配置审计 + 备份 + 回滚。
- **Playground**：同 prompt 最多 4 个目标并行对比（TTFT / 总耗时 / TPS / token 数）。

## 架构

workspace 位于 `v2/`，12 个 crate，依赖单向，`cargo xtask check-deps` 白名单强制：

| crate | 职责 | 关键约束 |
|---|---|---|
| `contracts` | 纯类型：Canonical*、Signals、Plan、Verdict、Cost、admin DTO | ts-rs 导出 TS 绑定 |
| `proto` | 三协议编解码、流式状态机 | 只依赖 contracts |
| `core` | 信号抽取 + 策略 + Plan 合并 | **纯函数、零 async、零 IO** |
| `decide` | DecisionModel trait + 判官 + 预算/deadline | 唯一允许调判官的 crate |
| `agent` | 会话状态、latch、账本、编排 | 内核唯一编排点 |
| `upstream` | provider 客户端、流式、接力 | 唯一 HTTP 边界 |
| `server` | axum 组装、入站鉴权 | 不直调 core/decide，一律经 agent |
| `control` | `/v2/admin` 管理 API | 内核无感知 |
| `cli` | `tr-cli`：replay / check-config / doctor / eval / admin 等 | `replay` 是唯一直调 core 的只读例外 |
| `client` | 管理 API 的 Rust 客户端 | 供 CLI/桌面壳使用 |
| `onboard` | agent 配置发现与精确合并 | 幂等、保留用户配置 |
| `mock-upstream` | 测试用上游 | 仅测试 |

**内核冻结**：产品层迭代保持 `core` / `decide` / `proto` diff = 0（提交门禁）；新路由能力走 SignalBag extractor / Strategy trait 扩展点。桌面端为 Tauri 应用（`v2/apps/desktop`，独立于 workspace；前端 React 19 + TanStack Router + UnoCSS）。架构细节见 [docs/architecture.zh.md](docs/architecture.zh.md)。

## 安全边界

- **上游凭据只存引用**（`env:`/`file:`/`keyring:`）：明文只存在于 Rust 侧内存与凭据存储；配置文件、API 响应、日志、账本、前端一律只见引用。
- **网关访问 key**：创建时明文只显示一次，服务端仅存 SHA-256 摘要与前缀；scoped 撤销立即生效。
- **admin token**：管理面走 `/v2/admin`，token 由 Rust 侧注入代理，不进 WebView。
- **配置可追溯**：CAS 并发保护 + 审计 + 备份（≤10 份）+ 回滚；账本落 `config_digest`，任一请求可追溯当时对应的配置版本。

## 开发

```bash
cd v2
make v2-check   # fmt + clippy -D warnings + check-deps + config-only-check
make v2-test    # workspace 全部测试
```

改了 admin DTO 需加跑 `cargo xtask contract-check`。全门禁清单见 [docs/dev/contributing.zh.md](docs/dev/contributing.zh.md)；文档索引见 [docs/README.md](docs/README.md)。

## 已知限制与非目标

- 未发布：打包仅 Windows NSIS，macOS/Linux 待演练；24h soak 与性能基线待验证。
- Playground 非流式呈现（服务端内部流式计时，前端统一展示）。
- 无标准限流头的厂商只有估算配额窗口（明确标记）。
- 明确不做：token 压缩（违背 passthrough 保真契约）、隐私遮蔽、插件系统、Web 搜索、媒体生成、云备份、局域网共享。

完整口径见 [docs/product.zh.md](docs/product.zh.md) §8–§9。

## License

MIT
