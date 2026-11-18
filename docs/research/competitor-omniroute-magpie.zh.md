# 竞品深度调研：magpie 与 OmniRoute

> 调研日期：2026-11-17。方法：只认一手来源（GitHub 仓库本体、官方文档站、源码、release notes/CHANGELOG），逐条核实内部文档声称；每条事实附来源 URL。
> 用途：[产品文档](../product.zh.md) 与 [路线图](../roadmap.zh.md) 对标表的事实基准。内部声称与查证不符处，以本文为准。

---

## magpie 产品调研（一手来源核实）

- **调研日期**：2026-11-17
- **调研对象**：https://github.com/yetone/magpie
- **仓库实际状态**（以 GitHub 页面与仓库文件当日实际显示为准）：
  - 仓库**存在**，未改名、未归档。描述："Every agent's model. One place. Codex on DeepSeek, Claude Code on Kimi, from the menu bar."
  - **Star：37.9k**（GitHub 仓库页面侧栏实际显示；内部文档声称"约 2.9 万星"，**不符，实际更高**），Fork 3.4k，Watch 44，commit 总数 8,555，默认分支 `main`。来源：https://github.com/yetone/magpie
  - 语言 **Go**（仓库根有 `go.mod`、`main.go`），许可证 **MIT**。桌面端用系统 webview（Wails），单二进制约 30 MB。来源：https://github.com/yetone/magpie 、https://github.com/yetone/magpie/blob/main/docs/reference.md
  - **最新 release：v0.1.4358，发布于 2026-11-17 09:07**（调研当天）。注意：二进制 release 发在独立仓库 **yetone/magpie-releases**，主仓库不放 release。该版修复 Codex 账号 "fast" service tier 发送问题。来源：https://github.com/yetone/magpie-releases/releases/latest （tag 页 https://github.com/yetone/magpie-releases/releases/tag/v0.1.4358 ）
  - 官网/官方文档站：**https://usemagpie.ai**（Docs: https://usemagpie.ai/docs/start ）。
  - 社区规则：**主仓库不接受外部 PR**（仅维护者可开 PR），反馈走 Discord。来源：README "Community" 节。
  - 支持平台：macOS / Windows / Linux（菜单栏 app、窗口、TUI、Web UI、CLI）、Docker、Termux。来源：README 徽章与正文。

### 一手来源清单

| 来源 | URL | 用途 |
| :-- | :-- | :-- |
| GitHub 仓库主页 | https://github.com/yetone/magpie | star 数、目录结构、About |
| README（main 分支原文） | https://raw.githubusercontent.com/yetone/magpie/main/README.md | 功能总览、agent 列表、预设列表、路由模式表 |
| 完整参考手册 docs/reference.md（约 30 万字符） | https://github.com/yetone/magpie/blob/main/docs/reference.md | 每个 agent 的配置文件与改写字段、网关端点、路由/账号选择、gateway key 限额、CLI、Docker、同步、遥测 |
| docs/ 目录 | https://github.com/yetone/magpie/tree/main/docs | code-standards.md、contributors.md、integrating.md、reference.md、session-relocation.md、screenshots/、subsystems/ |
| 源码目录 internal/ | https://github.com/yetone/magpie/tree/main/internal | 模块划分佐证（见下文） |
| 预设定义源码 | https://raw.githubusercontent.com/yetone/magpie/main/internal/provider/presets.go | 精确预设数量与清单 |
| Release 仓库 | https://github.com/yetone/magpie-releases/releases/latest | 最新版本号与日期 |
| 官方文档：Intent routing | https://usemagpie.ai/docs/intent | 意图路由完整机制（官方一手文档） |
| 官方插件社区仓库 | https://github.com/magpie-community/plugins | 插件生态 |

官方文档站导航（usemagpie.ai/docs，一手）还包含：start、plugins、remote、docker、privacy、masking、mcp-router、library、claude-code-models、codex-models、gemini-cli-models、subscriptions、local-models、opencode-models、alternatives（magpie vs CC Switch vs CCR）、import、integrate。来源：https://usemagpie.ai/docs/intent 页面导航栏。

---

### 1. 产品形态与总体架构

- 定位：本地 LLM 网关 + 多 agent 模型管理桌面应用。"One local gateway serves them all, and when a quota runs out, it quietly moves on to your next account."（README）
- 网关默认监听 **`127.0.0.1:3425`**，agent 通过环境变量接入（`OPENAI_BASE_URL=http://127.0.0.1:3425/v1`、`ANTHROPIC_BASE_URL=http://127.0.0.1:3425`、`GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:3425`，key 均为 `magpie`）。（README "Supported agents" 节）
- 运行形态：`magpie`（窗口+菜单栏）、`magpie tray`、`magpie tui`（终端版，打开期间可兼做网关）、`magpie web`（浏览器版，适合 WSL/SSH 服务器）、`magpie serve`（纯网关）、`magpie daemon install`（systemd/launchd 服务）。（docs/reference.md § Use、§ As a background service）
- 安装：`curl -fsSL https://usemagpie.ai/install.sh | sh`、`go install github.com/yetone/magpie@latest`、Docker 镜像 `ghcr.io/yetone/magpie`；Mac 构建签名+公证，release 文件带 GitHub Actions build attestation，自动更新；卸载脚本会先还原 agent 配置。（README "Quick start"；reference § Install / § Signed releases / § Docker）
- 配置目录 `~/.config/magpie`（settings.json、providers.json、caller-keys.json 等）。（README；reference § Files）
- 源码模块佐证（internal/ 下目录，GitHub API 实际 listing）：`gateway`、`routes`、`provider`、`agent`、`catalog`、`usage`/`usageapi`、`stats`、`budget`/`gwbudget`（网关预算）、`masking`/`maskrec`/`redact`/`pii`（隐私遮蔽）、`plugin`/`pluginrt`/`middleware`/`gwmw`（插件与中间件）、`profile`、`library`、`sessions`、`wslrun`（WSL）、`claudebridge`（Claude 订阅桥接）、`otlp`、`davsync`（WebDAV 同步）、`mtls`/`netguard`、`tui`/`gui`/`cli` 等。来源：https://github.com/yetone/magpie/tree/main/internal ；另有顶层 `native/privacy-filter`（本地隐私过滤模型）、`plugins/`、`site/`（官网）。来源：https://github.com/yetone/magpie

### 2. 协议层（声称：4 协议互转）

**查证：属实。** README 原文："The gateway at `127.0.0.1:3425` speaks OpenAI Chat, OpenAI Responses, Anthropic Messages and Gemini, and translates between them, streaming, tool calls and reasoning included."（https://raw.githubusercontent.com/yetone/magpie/main/README.md § How it fits together）

reference.md 给出的端点表（§ Connecting anything else，https://github.com/yetone/magpie/blob/main/docs/reference.md ）：

| 路径 | API |
| :-- | :-- |
| `/v1/chat/completions` | OpenAI Chat Completions |
| `/v1/responses` | OpenAI Responses（HTTP SSE，**也支持 Codex使用的 WebSocket 形式**） |
| `/v1/messages` | Anthropic Messages |
| `/v1/messages/count_tokens` | Anthropic token 计数 |
| `/v1beta/models/{model}:generateContent` | Google Gemini（另有 `:streamGenerateContent`、`:countTokens`） |
| `/v1/audio/transcriptions`、`/v1/audio/translations`、`/v1/audio/speech` | OpenAI 语音转写/TTS（透传） |
| `/v1/models`、`/v1beta/models`、`/v1/codex/models` | 模型目录（含 Codex catalog 格式） |
| `/api/version`、`/api/tags`、`/api/show` | **以 Ollama 服务器形式暴露目录**（供只认 Ollama 端点的客户端） |
| `/mcp/magpie/hub`、`/mcp/magpie/hub/{agent}` | MCP Streamable HTTP 路由（Library 的 MCP server 聚合） |
| `/v1/magpie/native/{provider}/{path}` | 厂商原生 API 透传 |

即实际协议面**超出**内部声称的 4 协议：还有音频、Ollama 兼容目录、MCP hub、原生透传，以及管理 API（`/v1/magpie/admin/`，reference § Management API）、embeddings/rerank（§ Embeddings and rerank）、决策 API `POST /v1/systemone`（§ Decisions）。

### 3. Provider / 账号管理（声称：30+ 预置厂商）

**查证：属实且明显保守。** 源码 `internal/provider/presets.go` 的 `var presets = []PresetDef{…}` 共 **69 个预设**（逐条计数，main 分支，2026-11-17 抓取）：37 个 vendor、26 个 relay、5 个 local（Ollama、LM Studio、oMLX、GPUStack、MLX-Serve）、1 个 remote（Remote magpie）。来源：https://raw.githubusercontent.com/yetone/magpie/main/internal/provider/presets.go

- vendor 类（37）：Anthropic、OpenAI、Google Gemini、Google Vertex AI、DeepSeek、xAI、Kimi、Kimi (China)、Kimi Code、Kimi Code (China)、Zhipu GLM、Z.ai、MiniMax、MiniMax (China)、StepFun、StepFun (China)、Xiaomi MiMo、LongCat、Baidu Qianfan、Tencent Cloud、Huawei Cloud MaaS、Volcengine Ark、Qwen、Qwen (China)、Qwen Token Plan、Bailian Token Plan、Alibaba Cloud Token Plan、Qwen Cloud Token Plan、Qwen Cloud、Mistral、Voyage AI（仅 embedding/rerank）、Groq、Amazon Bedrock、Bedrock (Mantle)、Azure OpenAI、Ollama Cloud、Nous Portal。
- relay 类（26）：OpenRouter、Vercel AI Gateway、GMI Cloud、OpenCode Go、ClinePass、OpenCode Zen、Kilo Gateway、Command Code、Together AI、Fireworks、Fireworks Nexus、SiliconFlow、NVIDIA NIM、ModelScope、AiHubMix、PipeLLM、302.AI、CherryIn、YYLX、AIGoCode、TypeSafe、Vercel-Jev、Cloudflare-Jev、Bailian-Decision、Qwen-Decision、Volcengine-Decision（后几个为决策模型 API）。
- README 的 "Presets include …" 列表点名 32 个并以 "…" 结尾（非穷尽），与源码一致。来源：README § Providers。

配套能力（均有出处）：

- **自定义 provider**：`url=`（OpenAI 兼容）、`anthropic=`（Anthropic 兼容）、`responses=`、`catalog=`（借用 models.dev 目录）、`models=`；`magpie provider add "My Relay" url=… key=… models=…`。（reference § Providers and the gateway）
- **每 key/每账号独立代理**；自定义 header（`header.<Name>=`）、`resolve.<host>=`（curl --resolve 式连接覆写）、User-Agent 透传策略（`user-agent=agent|magpie`）。（README § Providers；reference 同上节）
- **导入**：从 CC Switch、Claude Code、Codex、Alma 导入 provider；Providers 页自动发现本地可导入配置；另读 OpenCode `auth.json` 的 API key（只读不写）；"Add to magpie" 一键导入链接（`magpie://import?preset=…&key=…`，官方文档 https://usemagpie.ai/docs/import ）。（README § Providers；reference § Routing groups 末尾导入段、§ Import links）
- **订阅账号即 provider**（声称"订阅账号 failover"的核心，**查证属实**）：
  - 已登录的 Claude Code（Keychain / `~/.claude/.credentials.json`）、Codex/ChatGPT（`~/.codex/auth.json`）、Copilot（`~/.config/github-copilot/apps.json`）、Devin、Qoder / Qoder CN（magpie 内 OAuth device flow）自动成为 provider，模型以 `claude/…`、`codex/…`、`copilot/…` 拼写供其他 agent 使用；每次读 agent 自己的凭据并按其方式刷新 token，不另存密钥。（reference § Signed-in agents as providers）
  - **Claude 订阅特殊实现**：因 Anthropic 把第三方 system prompt 判为第三方流量，magpie **驱动本地真实 `claude` 二进制**完成每次生成，调用方工具经 **MCP 桥接**进该进程（`internal/claudebridge`）。要求本机装有并登录 Claude Code。（README § Subscriptions；reference 同上节）
  - Grok（SuperGrok）走 grok CLI 的 Responses API；Gemini CLI / Antigravity 走 Google Code Assist API（个人账号已不再被 Google 支持，需 Code Assist Standard/Enterprise + GCP 项目；Antigravity 账号有被 Google 封禁风险，magpie 添加前会警告）。（reference 同上节）
  - **同一订阅可加多个账号，配额用尽自动切换（failover）**；账号可从文件导入（Codex auth.json、codexbar、Cockpit Tools、CLIProxyAPI、Sub2API 导出；Claude 的 .credentials.json、claude-swap 导出等）。（README § Subscriptions；reference § Signed-in agents as providers）
  - **Daily warm-up**：按你选的时间（或上一个窗口重置时刻）提前触发每个 Claude/ChatGPT 账号的 5 小时窗口。来源：README § Subscriptions（"Daily warm-up starts each Claude and ChatGPT account's 5-hour window at the times you choose, or the moment the last one resets."）
  - 订阅自动发现可关：Settings → Privacy → Subscription accounts，或 `magpie accounts discovery off`，或单次 `MAGPIE_NO_ACCOUNTS=1`。（README § Quick start）
- **账号选择/failover 细节**：reference § How magpie picks an account 详述 429/5xx/配额耗尽后的休息（rest）与重试、`magpie provider quota-rest <id> 6h`、ChatGPT credits 策略（`magpie quota credits <acct> off|last|first`）等。

### 4. 模型目录

- **不内置模型表**：有 key 后向厂商实时拉模型列表；名称、reasoning 档位、上下文等元数据来自 **models.dev** 目录，后台自动刷新；离线时用上次缓存/随包副本；可逐 provider 选择暴露哪些模型（`magpie provider models deepseek +x -y`）、定时刷新（`refresh-every relay 6h`）、新模型自动纳入模式（`auto-include 'gpt-*' 're:claude-.*'`）。（reference 开头要点与 § Providers and the gateway）
- **每 agent 独立模型短名单**："Showing 5 / 32 models" 可勾选；"Only models I pick" 开关（`magpie visible <agent> --only-picked`）。（reference 开头要点）
- 厂商宣告 deprecated/retired 的模型自动下架，请求成功或一周后恢复。（reference § Providers and the gateway）
- 模型价格/能力/别名/ served-on API 均有专门章节（§ What a model costs、§ What a model takes、§ The name a vendor knows a model by、§ The API a model is asked on），支持 `magpie model spec` 一次看全。

### 5. 路由策略（声称：路由组 5 模式 smart/order/rotate/usage/pace）

**查证：5 个模式全部属实，但实际是 7 个模式——内部清单漏了 `weight` 和 `fast`。**

reference § Routing groups 原文定义（https://github.com/yetone/magpie/blob/main/docs/reference.md ）：

| 模式 | 行为（据原文） |
| :-- | :-- |
| `smart`（默认） | 在还有配额的订阅中，优先用**最快重置**的那个，减少重置时浪费 |
| `order` | 用第一个模型直到不能回答，再换下一个 |
| `rotate` | 每轮（turn）换下一个成员 |
| `usage` | 最少使用的优先 |
| `pace` | 周节奏：优先用"距重置前每小时剩余周配额最多"的账号 |
| `weight` | 按权重（1–1000）平滑分摊请求，嵌套组算一个成员 |
| `fast`（别名 `fastest`/`speed`） | 按近期首 token 时间与输出速度选最快成员；`magpie provider set <id> routing=fast` 可对单 provider 的多 key 生效 |

- 路由组以 `group/<id>` 作为一个"模型"供 agent 选择；同名模型跨 provider 自动成组；**组可嵌套组**；CLI：`magpie group add "Opus anywhere" models=claude/…,copilot/… routing=smart`、`group set/copy/rm/restore`。（README § Routing groups；reference 同节）
- **会话粘性 `stays=`**：`auto`（默认，厂商 prompt cache 还值得保留期间不换账号）/`session`/`turn`/`off`/`strict`（绝不换，错误直达 agent）。（reference 同节）
- **组规则（rules）**：条件可含 token 长度、是否带图、reasoning、agent、**intent（意图）**；首条命中规则的模型排前，其余成员做 failover；规则还可指定 `use-effort=`。（https://usemagpie.ai/docs/intent ；reference 同节）
- `listed=no` 可让组内模型从目录/各 agent 列表隐藏，只暴露组本身。（reference 同节）
- **纯决策模型组**（Jev、Bailian decision-model-preview 等）：不进 agent 选择器，经 `POST /v1/systemone` 调用，可作聊天组的分类器。（reference § Routing groups、§ Decisions）
- 单账号/单 key 且无组时，错误会直达 agent（README 明确说明 failover 的边界）。

#### 5.1 意图路由（声称：意图路由 —— 查证属实）

官方专页 https://usemagpie.ai/docs/intent （页面自述截至 v0.1.99 的行为）：

- **机制**：每个新 turn（最后一条 user 消息含文本/图片且无 tool result）决策一次。把候选规则的 intent 描述编号列表 + 用户最新消息发给组指定的**分类器小模型**（经 magpie 自己的网关，享受该 provider 的 failover），模型只回一个数字（1..n 或 0=都不是）。**无 embedding、无关键词表、无训练**。
- **配置**：Routing → 组编辑 → Add a rule（"asks for → 模型"）→ "Intent told by" 选分类器（可以是任意已有模型，含本地 Ollama/LM Studio 模型；不能是组）。CLI：`magpie group rule add coder use=deepseek/deepseek-v4-flash intent="a quick question" classifier=groq/llama-3.1-8b-instant`；`rule mv/rm/classifier`。
- **分类器请求细节**：只发最新 user 消息文本（剥离 `<system-reminder>`；>4000 字符截为首 3000+尾 1000）；temperature 0、max_tokens 2048、最低 reasoning 档、非流式、UA `magpie-router/1`。
- **关键参数**：超时默认 15 s（可设 1–60 s，Settings → Models → Replies → Intent classifier timeout / settings.json `classifyTimeout`）；失败后该分类器休息 30 s；同消息+同意图答案缓存 10 min（内存，仅存 hash）；turn 决策内存保留 24 h；intent 描述最长 200 字符。
- **失败兜底**：分类器超时/出错/答非数字 → 意图规则不命中，按组常规路由继续，**绝不使请求失败**；每种情况写入 Routing trace。
- **turn 内保持**：工具轮不再问分类器；failover 后粘住接替者；唯一例外是对话涨到当前模型上下文 95% 时可迁到更大上下文模型（复用先前分类答案）。子 agent 独立决策。
- **成本/隐私**：每 turn 一次短调用（示例 212 in / 1 out），计入 Usage 的 `magpie` agent；消息文本会发给分类器 provider，可用本地模型规避。
- 官方 FAQ 给出实测：20 条真实编码消息，DeepSeek V4 Flash / GLM-5.3 Flash / MiMo v2.6 Flash 全对，中位耗时 0.75/1.3/1.9 s。

### 6. 配额 / 限额（含声称：局域网共享 key 限额）

#### 6.1 订阅/Key 配额

- `magpie quota`：列出每个订阅、计划、key 的余额与配额窗口；Usage 页与菜单栏同显；窗口将重置且剩余较多时提醒。`quota refresh`（30 s 限频）、`quota wait codex`（阻塞到有额度）、`quota credits … off|last|first`（ChatGPT credits 策略）。API：`GET /v1/magpie/quotas`、`/quotas/cards`、`POST /quotas/refresh`、`/quotas/codex-reset`。（README § Usage and cost；reference § Use、§ Signed-in agents as providers 前后配额段）
- 按量 key 也可带窗口：sub2api 中转 key（Balance URL 指向 `/v1/usage`）与 Kimi Code/GLM Coding Plan/MiniMax Coding Plan/OpenCode Go/Command Code/Ollama Cloud 等"计划 key"按 5h/日/周窗口参与路由权衡。（reference § Routing groups）

#### 6.2 局域网共享与 gateway key 限额（**查证属实，且比声称更细**）

来源：README § Sharing；reference § Providers and the gateway（gateway keys 段，含 #882/#905/#1509/#2102/#2227 等 issue 号）。

- 开启 **Settings → Network and sharing → Share on local network**，然后 **Gateway → Gateway keys → Add gateway key**，为每个客户端发命名 key（与上游厂商 key 完全分离；可自带 key 值、改名、停用、轮换、删除）。
- **每 key 限额**：token 总量和/或估算成本（US$），窗口支持 **日/周/月（本地日历）、每 N 天（1–3650）、每 N 小时（1–8760，如 5h 对齐订阅节奏）、total（永不按时重置）、跟随某订阅账号的窗口（account）**；**一个 key 最多 5 条限额**，全部通过才放行；可按模型子集计限额（`--model openai/*`）；可选"Count cache reads too"。
- CLI 示例（reference 原文）：`magpie gateway-key limit <id> week --tokens 2m --cost 5`、`limit <id> add month --cost 20 --model openai/*`、`limit <id> account --account codex/me@example.com --window 5h --tokens 2m`、`limit <id> reset|off|list`。
- 超限在触达任何 provider 之前拒绝：按该 API 自身错误形状返回 **429**，指明 key、限额、重置时间并带 `Retry-After`；在途请求有预留额度；计数来自 usage log，重启不丢。
- key 还可限定**可用模型**（`gateway-key models`）、**可用账号/账号组**（`accounts`、`groups`，支持 `!` 排除）、**可用 MCP server/工具**（`mcp`）、是否可见配额（`quotas`）、是否可走管理 API（`admin local|any|off`）。
- 本机不带 gateway key 的请求不受限；本机带 key 的请求同样受限。key 自查 `GET /v1/magpie/limit`。
- 传输安全：局域网 HTTPS（`magpie https`，自签 CA 可导入）、**mTLS 客户端证书**（`magpie mtls issue/revoke`）、反代场景 `MAGPIE_TRUST_PROXY`、Host 校验。（reference § Sharing/HTTPS 段、§ Use）
- **Remote magpie**（https://usemagpie.ai/docs/remote ）：笔记本把台式机 magpie 当一个 provider，用其 providers/账号/组，本机 agent 仍由本机接线；远端 key 的限额会显示在本机 Usage 页（#1547）。
- **Docker**：`ghcr.io/yetone/magpie`，Web UI 为"gateway mode"（见 §8）；**Sync**：文件备份、WebDAV（坚果云/Nextcloud）、S3、加密 GitHub 仓库备份。（README § Sharing；reference § Docker、§ Keeping machines in sync）

### 7. 用量 / 成本分析（声称：用量成本分析 —— 查证属实）

来源：README § Usage and cost；reference § Providers and the gateway（usage 段）、§ What a model costs、§ OTLP export。

- **Tokens 与成本**：输入/输出、cache 读写（**区分 Anthropic 5m/1h 两档 cache write**）、reasoning token、调用次数；按目录价以美元或人民币计成本，**可为任意模型自定义价格**；两个 id 记为同一模型（§ Counting two ids as one model）。
- **请求归因**：每请求记录实际服务的**上游 key 指纹与名称**（含 failover 后），CSV/JSON 带 `provider_key_id/name`；失败/重试口径明确（failover 成功不算失败，记 `retried`；每次 try 单独留痕 `handed_on`）；service tier 三列（asked/sent/served）；上游 stop reason。
- **输出速度 tok/s**：首 token 后计时，排除 reasoning、突发缓冲等失真场景（reference 有长段算法说明）。
- **Sessions**：每个 agent 会话带成本与标题，一键在终端重开（macOS 可选终端 app，含 Rex 集成）；Routing 页 Requests 列表可按 request/session/gateway key 分组。
- **Context**：每请求上下文占用、cache 命中占比、内容构成。
- **Bills**：按账号、按上游 key 出账；**OTLP 导出**到自己的可观测性栈（reference § OTLP export，含请求/响应体开关与遮蔽联动）。
- CLI：`magpie usage [today|7d|30d|all] [--last 24h|--days 1.5] [--csv|--json]`。
- 日界线可设（`magpie settings set dayStart 4`）。

### 8. UI 页面（含声称：Live Routing 实时视图 —— 查证属实）

- **菜单栏面板 + 窗口**：Agents 页（每 agent 一行，点值弹出可搜索模型列表，`provider/model`；配置**原子改写**，注释/顺序/格式保留）；Providers 页（key、暴露模型、Test、导入）；**Routing 页**；Usage 页（Requests、Sessions、按日柱状图点选）；**Insights › Masking**；Plugins（含 Discover）；Library；Gateway（Connect、Gateway keys）；Settings。（README § Any model, any agent 及各节截图 alt 文本；reference § Use）
- **Live Routing**：README 原文 "The Routing view shows every decision live."，截图说明 "The Routing view: four agents through magpie to seven providers, live"；意图路由页展示实时 trace（分类器答案、耗时、命中规则、failover 每次 try）。Routing 页 Requests 列表默认按时间，可切 By session / By key，显示每请求/会话估算成本。**"Live Routing 实时视图"这一声称成立**（官方名称是 Routing view/Routing tab，不叫 "Live Routing"）。（README § Routing groups；https://usemagpie.ai/docs/intent § Reading the trace；reference § Routing groups）
- **TUI**：`magpie tui`，方向键选 agent/字段、`s` 存 profile、`p` 应用 profile、`S` 同步目录。（reference 开头与 § Use 键位表）
- **Web UI**：`magpie web` / `serve --web`；Docker/NAS 上为 gateway mode：显示 Sessions、Library、Providers、Routing、Gateway、Usage、Masking、Plugins、Settings，**无 Agents 页**。（reference § Docker，原文 "Such a page is in gateway mode: it shows Sessions, Library, Providers, Routing, Gateway, Usage, Masking, Plugins and Settings, with no Agents tab."）
- 纯 HTML + 系统 webview，无前端框架；品牌图标来自 lobehub/lobe-icons。（reference 开头要点）

### 9. Agent 接入（声称：80+ agent 配置改写）

**查证：数量级属实——README "Supported agents" 逐一列举 88 个 agent**（本次逐个数得；含 Claude Code、Claude Desktop、Claude Science、Codex、Gemini CLI、Antigravity CLI、Dim、OpenCode、OpenChamber、MiMo Code、DevEco Code、Pi、oh-my-pi、StepCode、MothX、PenguinHarness、Aside、OmO、Prime Agent、mu、Goose、Cursor CLI、Cursor Private Inference、Zed、VS Code Chat、VS Code Insiders、VSCodium Chat、Zoo Code、JetBrains Air、Air (JetBrains IDEs)、Copilot (JetBrains)、Copilot CLI、Crush、DeepSeek Harness、Reasonix Studio、Command Code、fx、Devin、Hermes Agent、Mister Morph、Kimi Code、jcode、Mistral Vibe、Qwen Code、Muse Code、Empryo、nanobot、miniQ、Paimon、MiniMax Code、Droid、Cline、Qoder、Qoder CN、Grok Build、Grok App、ZCode、WorkBuddy、WorkBuddy AI、CodeBuddy、Pencil、Paseo、OpenPencil、OpenBot、Craft Agents、Tuanjie Cowork、NewMax、LiveAgent、Raycast、Kiro Crew、OpenMausBot、T3 Code、OpenHanako、AtomCode、Open Code Review、Snow CLI、Alma、Shelley、Cindy、Lody、AionUi、Cherry Studio、Ante、AstrBot、QwenPaw、OpenClaw、Avibe、Kun）。来源：https://raw.githubusercontent.com/yetone/magpie/main/README.md § Supported agents

- reference § Agents 用大表给出**每个 agent 的配置文件路径（含各 OS 差异与环境变量覆写）与被改写字段**，例如 Claude Code `~/.claude/settings.json`（provider/model/opus/sonnet/haiku/fable/auto mode/登录）、Codex `~/.codex/config.toml`（provider/model/effort）、Gemini CLI `~/.gemini/settings.json`+`.env`、OpenCode `opencode.json(c)`（model/small）、VS Code Chat `settings.json`+`chatLanguageModels.json`（1.122+，无需 Copilot 登录）等；reference 表还包含 README 未列的 Obsidian（Copilot for Obsidian 插件）。来源：https://github.com/yetone/magpie/blob/main/docs/reference.md § Agents
- **措辞需修正**："配置改写"是主路径但非全部：Dim/Antigravity 用 shell profile 里的标记函数+启动环境变量；Alma/Shelley/Kun/Avibe/OpenHanako 走应用本地 API/socket；Lody 只弹其导入对话框、"no settings edited by magpie"；NewMax 写 SQLite。断开（Disconnect）时还原用户原值。（reference § Agents、§ Connecting anything else）
- 只显示本机已安装/已配置的 agent；Agents 页显示各 CLI 版本、按原安装方式更新、给未安装者安装命令。（README § Supported agents、§ Also）
- **Profiles**（声称属实）：一个名字快照全部 agent 设置并一键切回；每 agent 的模型短名单也随 profile 保存；`magpie save work && magpie use work`、`magpie profiles`（当前 profile 打 ✓）、`magpie rm work`；托盘面板有 Profiles 按钮。（README § Any model, any agent；reference 开头与 § Use）
- **Library**：instructions、MCP servers、skills、subagents 写一次，按各 agent 格式写入其文件，只移除自己写的；MCP router 把 Library 的 MCP server 聚合成一个 HTTP MCP 端点（`/mcp/magpie/hub`）。（README § Also；reference 端点表；官方文档 https://usemagpie.ai/docs/library 、https://usemagpie.ai/docs/mcp-router ）

### 10. 隐私遮蔽（声称：隐私遮蔽 —— 查证属实）

来源：README § Privacy masking（含功能表）；reference § Gateway middleware（"Privacy masking is the built-in plugin `redact`"）、§ Files（`redact.key`）；源码目录 `internal/masking`、`internal/redact`、`internal/pii`、顶层 `native/privacy-filter`；官方专页 https://usemagpie.ai/docs/masking 。

- **默认关闭**，在 Settings → Privacy 打开。开启后，prompt 中的 key/密码/邮箱等在**本机**换成占位符再出网，回复中还原；agent 与本地文件始终持真值。
- **两种发现方式**：确定性规则（API key、私钥、token、邮箱、校验和通过的卡号/证件号）+ 可选**本地 CPU 模型**识别姓名/地址/密码（**1.6 GB 下载**）。
- 占位符形如 `{{API_KEY_k3v9x2mq}}`；**流式文本与 tool-call 参数同样还原**（agent 执行的命令拿到真 key）；同值同占位符，保住对话、prompt cache 与 thinking signature 有效性。
- **值只存内存**；记录仅留类型/长度/首字符；遮蔽开启但无法运行时**扣下请求（503）**，绝不原文出网（fail-closed）。
- **Insights › Masking** 逐请求对照展示厂商实际收到什么；Settings 有 **Try it** 预览；规则可细到护照/IBAN/MAC/公网 IP 等类别、自定义词与正则，动作三档 **Mask / Warn / Block**；**Never mask** 白名单；示例域名邮箱（example.com/.test/.invalid）不遮蔽。
- 代价明示：模型看不到真值，关于值本身的任务做不了；规则可能误判。
- 遥测（README § Privacy）：每日一次匿名使用上报（随机 id、版本、系统、所用 agent/provider/model 的 magpie 内部 id、插件安装计数、合作伙伴展示/点击计数），无名称/URL/账号/key/prompt；Settings → Privacy 可分项关，或 `DO_NOT_TRACK=1`；图标经 `usemagpie.ai/api/icon` 代理。reference § Counting users 有完整规则。

### 11. 插件系统（声称：插件系统 —— 查证属实）

来源：README § Plugins；reference § Plugins（含 For plugin authors / Web search APIs from a plugin / Gateway middleware / Agent plugins）；官方文档 https://usemagpie.ai/docs/plugins ；社区仓库 https://github.com/magpie-community/plugins 。

- **运行时为 Bun**（首次需要时自动下载）。两类：
  1. **Provider 插件**：任意 OpenCode auth 插件或 pi 包（npm），`magpie plugin add opencode-gemini-auth`、`magpie plugin login google-plugin`（插件自己的登录流在 magpie 内完成）；插件负责登录、列模型、发请求，其模型与其他 provider 无异。
  2. **Gateway middleware**：JavaScript 钩子 `onRequest`（改写/拒绝请求）、`onEvent`（每个流事件）、`onResponse`（完整回复）；钩子抛错或超时则请求按原样放行（fail-open）。可 `magpie plugin add ./alias.middleware.js` 装本地文件。
- **现成中间件**（官方称多数复刻 New API 的渠道能力、同 JSON）：`param-override`、`model-map`、`system-prompt`、`word-guard`（敏感词）、`think-tags`；`magpie plugin add @magpie-community/middleware-model-map` + `magpie plugin options model-map '{"mapping":…}'`；在 Plugins › Discover › Gateway middleware 浏览。
- 插件还可注册 **Web 搜索 API**（进 Settings › Models › Web search，与内置 Tavily/Brave 并列）与 **agent 插件**、工具（经 MCP 暴露）。
- 隐私遮蔽本身就是内置插件 `redact`（见 §10）。

### 12. Web 搜索（声称：Web 搜索 —— 查证属实）

来源：README § Also（"Web search for any model"）；reference § Web search APIs from a plugin、§ Use（`magpie tool web_search`）。

- 不能搜索的模型获得 magpie 的搜索：用**能搜索的 provider 模型**，或搜索 API：**Tavily、Brave、Exa、Firecrawl、You.com、DeepSeek、Doubao Search（火山引擎）、Yuanbao Search（腾讯云）、TinyFish、SearXNG**（共 10 个，README 逐一列出）。
- 按拖拽顺序依次尝试；每个可单独停用而不丢 key；一个 API 可放多把 key 轮询（或 `pick=order` 顺序用），近期被拒的 key 排最后；Tavily key 显示额度用量（Settings 与 `magpie search`）。
- 任意 agent 可把同一搜索挂成 **MCP server**：`web_search` 工具 + `web_fetch`（按 URL 读页面）。
- CLI 直调：`magpie tool web_search "go 1.27 release notes"`。

### 13. 其他工具能力（README § Also，一手）

- **图像/视频**：任意 agent 经 MCP 用图像生成工具（模型可选）；视频用 Grok 订阅或 MiniMax API key（MiniMax H3）；`magpie tool generate_image "…" -o hero.png`、`magpie tool analyze_image ./shot.png`。（README § Also、Quick start；reference § Use）
- **Keep awake**、明暗主题、Omarchy 主题适配。（README § Also）

### 14. WSL 支持（声称：WSL 支持 —— 查证属实）

- README § Also 原文："**WSL**: on Windows, agents inside WSL distros are wired too, and their sessions counted."
- reference 细节：WSL 内装有 OpenCode 的发行版会多出 `Magpie (WSL <distro>)` ACP agent（JetBrains Air 行）；WSL 内的 Antigravity CLI 列为 `Antigravity CLI · WSL <distro>`，在其 shell profile 写标记函数，网关地址按 NAT 下 Windows 地址自动改写；WSL NAT 地址访问共享网关被视为"另一台电脑"，需 gateway key；`magpie web` 明确面向 WSL/SSH 场景；WSL 内 `magpie daemon` 可用（需先开 systemd）；会话统计含 WSL mirrored 网络与 Docker Desktop 场景。源码有 `internal/wslrun` 模块。（https://github.com/yetone/magpie/blob/main/docs/reference.md § Agents、§ Connecting anything else、§ Sharing、§ Use、§ As a background service、§ OTLP export；https://github.com/yetone/magpie/tree/main/internal ）

### 15. 版本与发布节奏

- 最新：**v0.1.4358**（2026-11-17，github-actions 自动发布；release notes 中英双语；19 个 assets：macOS dmg×2、Windows exe×2（含 ARM64）、Linux×2、CLI-only 各平台）。来源：https://github.com/yetone/magpie-releases/releases/tag/v0.1.4358
- 版本号 0.1.4xxx 与主仓 8,555 commits 表明**近乎每日多次的持续发布**；已安装实例自动更新；有更新通道设置（reference § Update channel）。
- 主仓 commit 快照：`cd89ccd`（main，2026-11-17 抓取时）。来源：https://github.com/yetone/magpie

---

### 内部声称 vs 查证结果对照表

| # | 内部声称 | 查证结果 | 依据（一手来源） |
| :-- | :-- | :-- | :-- |
| 1 | 仓库 yetone/magpie 存在，Go 语言 | **属实**。Go（go.mod/main.go），MIT，Wails 桌面端 | https://github.com/yetone/magpie |
| 2 | 约 2.9 万星 | **不符（偏低）**。GitHub 页面实际显示 **37.9k** stars（2026-11-17），另有 3.4k forks | https://github.com/yetone/magpie （侧栏 Stars） |
| 3 | 4 协议互转（OpenAI Chat/Responses/Anthropic/Gemini） | **属实**。含流式、tool calls、reasoning 互转；实际还多：Responses WebSocket、音频 API、Ollama 兼容目录、MCP hub、原生透传、embeddings/rerank、决策 API | README § How it fits together；reference 端点表 |
| 4 | 30+ 预置厂商 | **属实但保守**。源码 presets.go 共 **69 个预设**（37 vendor + 26 relay + 5 local + 1 remote），另支持任意 OpenAI/Anthropic 兼容自定义 URL | https://raw.githubusercontent.com/yetone/magpie/main/internal/provider/presets.go ；README § Providers |
| 5 | 80+ agent 配置改写 | **基本属实**。README 列 **88 个 agent**，reference 逐一给出配置文件与改写字段；但并非全部走"改配置文件"：部分经 shell 函数/环境变量（Dim、Antigravity）、应用本地 API（Alma、Shelley、Kun 等）、导入对话框（Lody）或 SQLite（NewMax） | README § Supported agents；reference § Agents、§ Connecting anything else |
| 6 | 路由组 5 模式（smart/order/rotate/usage/pace） | **属实但不完整**。5 模式定义与声称一致，实际共 **7 模式**：另有 `weight`（加权）与 `fast`/`fastest`/`speed`（速度优先）；还有 stays 粘性、嵌套组、规则系统 | reference § Routing groups；README § Routing groups |
| 7 | 订阅账号 failover | **属实**。同订阅多账号自动切换；Claude 订阅经本地真实 claude 二进制 + MCP 工具桥接；另有 Daily warm-up、账号文件导入 | README § Subscriptions；reference § Signed-in agents as providers、§ How magpie picks an account |
| 8 | 意图路由 | **属实**。组规则 intent 条件 + 分类器小模型每 turn 选号；超时 15 s 默认、失败兜底、10 min 答案缓存、Routing trace 可审计 | https://usemagpie.ai/docs/intent ；README § Routing groups |
| 9 | 隐私遮蔽 | **属实**。默认关；规则 + 可选 1.6 GB 本地 CPU 模型；占位符在流式/工具参数中还原；仅内存；fail-closed 503；Mask/Warn/Block；Insights › Masking 审计 | README § Privacy masking；reference § Gateway middleware、§ Files |
| 10 | 插件系统 | **属实**。Bun 运行时；npm OpenCode auth 插件/pi 包；gateway middleware（onRequest/onEvent/onResponse）；官方中间件 5 款；社区仓库 magpie-community/plugins | README § Plugins；reference § Plugins；https://usemagpie.ai/docs/plugins |
| 11 | Profiles | **属实**。全 agent 设置快照/一键切换，含每 agent 模型短名单；`magpie save/use/profiles/rm` | README § Any model, any agent；reference § Use |
| 12 | 用量成本分析 | **属实**。token/cache(5m/1h)/reasoning/成本（$/¥、自定义价）、按账号与上游 key 出账、Sessions、Context、OTLP 导出、CSV/JSON | README § Usage and cost；reference usage 段、§ What a model costs、§ OTLP export |
| 13 | 局域网共享 key 限额 | **属实且更细**。命名 gateway key；token+成本限额，日/周/月/每N天/每N小时/total/跟随账号窗口，最多 5 条并存，可按模型子集；另可限模型/账号/MCP；429+Retry-After；HTTPS/mTLS | README § Sharing；reference gateway keys 段（#882/#905/#1509/#2102/#2227） |
| 14 | Web 搜索 | **属实**。10 个搜索 API（Tavily/Brave/Exa/Firecrawl/You.com/DeepSeek/豆包/元宝/TinyFish/SearXNG）+ 可搜索模型；多 key 轮询；MCP `web_search`/`web_fetch` | README § Also；reference § Web search APIs from a plugin |
| 15 | WSL 支持 | **属实**。WSL 内 agent 接线并计入会话；WSL 版 Antigravity/Air ACP agent；NAT 地址按共享网络处理；`magpie web` 面向 WSL | README § Also；reference § Agents、§ Connecting anything else、§ Use |
| 16 | Live Routing 实时视图 | **属实**（官方名为 Routing view/tab）。实时展示每个路由决策、意图 trace、每次 failover try；By request/session/key 视图 | README § Routing groups（"shows every decision live"）；https://usemagpie.ai/docs/intent § Reading the trace；reference § Routing groups |
| 17 | （补充）最新版本 | 内部文档未记录。查证：**v0.1.4358，2026-11-17 发布**，release 在独立仓库 yetone/magpie-releases | https://github.com/yetone/magpie-releases/releases/latest |

#### 总体结论

内部文档对 magpie 的功能声称**方向全部成立，无虚构功能**；需要修正的是三处数字/口径：star 数（2.9 万 → 实际 37.9k）、路由模式数（5 → 实际 7）、预设厂商数（30+ → 实际 69 个预设）；"80+ agent 配置改写"宜改为"88 个 agent 接入（以原子配置改写为主，辅以 shell 函数/本地 API 等方式）"。magpie 的实际能力面（音频透传、Ollama 兼容、MCP router、决策模型组、mTLS、OTLP、Library、Remote magpie、多平台同步等）显著宽于内部清单。
---

## OmniRoute 产品调研（一手来源核实）

- **调研日期**：2026-11-17
- **调研对象**：https://github.com/diegosouzapw/OmniRoute
- **仓库实际状态**（以 GitHub 页面与仓库文件当日实际显示为准）：
  - 仓库**存在**。原地址 `diegosouzapw/OmniRoute` 已**转移到组织 `cheaper-inference/OmniRoute`**（GitHub 自动重定向，页面标题与导航均显示 cheaper-inference/OmniRoute；README 内链接仍写 diegosouzapw）。作者 Diego Souza，MIT 许可证。来源：https://github.com/diegosouzapw/OmniRoute （重定向至 https://github.com/cheaper-inference/OmniRoute ）
  - **Star：88.2k**（GitHub 页面侧栏实际显示；内部文档声称"约 9 万星"，**基本接近、略低**），Fork 13.5k，Watch 499，commit 总数 13,518。来源：https://github.com/cheaper-inference/OmniRoute
  - **默认分支是 `release/v3.8.55`，不是 `main`**（release rail 发布模型，PR 进 `release/*`，见 docs/ops/BRANCHING_MODEL.md）。**最新已发布 release：v3.8.53，2026-11-06**（release 页显示 "released this 06 Nov"，tag commit `191dce8`；CHANGELOG 记 `[3.8.53] — 2026-11-06`）。默认分支 v3.8.55 尚未打 release，CHANGELOG 顶部有 `[Unreleased]` 段。**因此 main 分支 README 与默认分支 README 数字不一致是正常现象，本文两处都记录。**来源：https://github.com/cheaper-inference/OmniRoute/releases 、https://github.com/cheaper-inference/OmniRoute/blob/main/CHANGELOG.md
  - 语言：**100% TypeScript**（README Tech Stack 表："TypeScript 6.0 — 100% TypeScript across src/ and open-sse/"），Next.js 16 + React 19 + Tailwind 4，Node 22/24，SQLite（better-sqlite3，WAL），默认端口 **20128**，数据目录 `~/.omniroute/`。内部声称 TypeScript **属实**。来源：https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/README.md （Tech Stack 节）
  - 自我定位是 **"The Free AI Gateway"**（免费 AI 网关）：一个端点聚合大量第三方免费额度 + 订阅账号，服务 Claude Code / Codex / Cursor 等编码 CLI，主打配额感知自动回退与 token 压缩。README 有与 OpenRouter / 9router / CLIProxyAPI / LiteLLM 的对比表（docs/comparison/OMNIROUTE_VS_ALTERNATIVES.md）。内部"OpenRouter 开源替代"的定位**大体成立但不够准确**——它更像"免费额度聚合 + 编码 CLI 网关"，且 README 自述数字是营销口径（如约 1.62B 免费 tokens/月为第三方免费额度估算聚合，非官方授予）。来源：README 标题、About、comparison 节。
  - 官网：https://omniroute.online ；文档在仓库 `docs/`（约 150+ 篇，67 语言镜像），dashboard 内嵌 `/docs` UI（Fumadocs）。来源：https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/docs/README.md

### 一手来源清单

| 来源 | URL | 用途 |
| :-- | :-- | :-- |
| GitHub 仓库主页（默认分支 release/v3.8.55） | https://github.com/cheaper-inference/OmniRoute | star 数、目录结构、About、默认分支 README（394 providers / 22 策略口径） |
| README（main 分支原文） | https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/README.md | 功能总览、380 providers / 20 策略口径、auto/* 变体表、CLI、部署、Tech Stack |
| Releases 页 | https://github.com/cheaper-inference/OmniRoute/releases | 最新版本 v3.8.53 与日期、release notes |
| CHANGELOG（main，约 3.9 MB） | https://github.com/cheaper-inference/OmniRoute/blob/main/CHANGELOG.md | 各功能引入版本、OTLP/热力图/Live Monitor/9 因子纠错的一手记录 |
| docs/ 索引 | https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/docs/README.md | 文档全集导航 |
| Auto-Combo 引擎 | https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/docs/routing/AUTO-COMBO.md | 16 因子评分表、20 策略、auto/* 机制、mode packs、请求头控制 |
| 韧性指南 | https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/docs/architecture/RESILIENCE_GUIDE.md | 熔断三层阈值、冷却、模型锁定、配额共享并发、队列准入 |
| 功能画廊 | https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/docs/guides/FEATURES.md | dashboard 各页面、Translator Playground 四模式、热力图、SSRF Guard |
| Playground Studio | https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/docs/frameworks/PLAYGROUND_STUDIO.md | /dashboard/playground 四 tab、API、预设、指标 |
| 用量/配额指南 | https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/docs/guides/USAGE_QUOTA_GUIDE.md | 用量记录字段、成本计算、热力图 widget、per-key 配额、REST API |
| 监控指南 | https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/docs/ops/MONITORING_GUIDE.md | 健康页、quota monitor、autopilot、OTel 导出"planned for v3.9"的滞后表述 |
| 环境变量参考 | https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/docs/reference/ENVIRONMENT.md | SSRF 三变量及默认值、存储/加密/队列变量 |
| CLI 参考 | https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/docs/reference/CLI-TOOLS.md | CLI 命令面、setup-*、run/configure、容器守卫、key policy |

> 说明：raw.githubusercontent.com 的 `diegosouzapw/OmniRoute/main/...` 路径当日均可正常访问（重定向后的仓库同一内容）。以下各节引用均回到仓库/官方文档原文，未使用第三方博客。

---

### 1. 协议层

- 统一端点 `http://localhost:20128/v1`，OpenAI 兼容：chat completions、`/v1/responses`（Responses API）、completions、embeddings、images、audio（speech / transcriptions / translations）、moderations、rerank、search，另有 `/v1/ocr`（Mistral OCR）。来源：README "What's New"、CLI-TOOLS.md "Available API Endpoints"。
- **协议翻译是核心子系统**：OpenAI ↔ Claude（Anthropic Messages）↔ Gemini/Antigravity 等格式互转，含流式、tool calls、reasoning 的翻译；CHANGELOG 中大量 `fix(translator)` 条目（如 v3.8.53 的 OpenAI→Claude prompt caching、Claude/Gemini→OpenAI max_tokens 保持等）。来源：https://github.com/cheaper-inference/OmniRoute/blob/main/CHANGELOG.md （[3.8.53] 段）。
- Anthropic 兼容 `/v1/messages`（Claude Code 直连，base URL 不带 /v1）、Gemini 原生 `/v1beta/*`（API-only 端口也中继，v3.8.53 #16745）、Ollama 兼容别名（`/vscode/{key}/api/chat`、`/api/tags`，供不能发 Authorization 头的客户端）、WebSocket 桥 `/v1/ws`（v3.6.6+）。来源：CLI-TOOLS.md、FEATURES.md "V1 WebSocket Bridge"、CHANGELOG [3.8.53]。
- Agent 协议：**MCP**（stdio `omniroute --mcp` / HTTP `/api/mcp/stream` / SSE `/api/mcp/sse`，**110 tools、33 scopes**）+ **A2A v0.3**（`/.well-known/agent.json`，JSON-RPC 2.0 + SSE）+ ACP（反向 spawn CLI 作为后端）。来源：README "Full CLI + A2A & MCP" 表。
- 模型发现：`/v1/models` 按 capability/context/kind/text 过滤，`POST /v1/models/query` 返回确定性短名单（v3.8.53+）。来源：README "What's New"。

### 2. Provider 管理（声称：约 370 个 providers）

**查证：数量级属实，但精确数字随分支/口径漂移，内部"约 370"偏旧。**

- GitHub About（当日显示）："one endpoint, **375 providers** (150+ free), 1,400+ models"。来源：https://github.com/cheaper-inference/OmniRoute
- main 分支 README："**380 registered providers** … including **153 carrying `hasFree: true`**"；并明确区分口径：chat 模型注册表 **239 providers / 3,059 provider-model 对 / 1,475 原始 model ID**；免费预算目录 **471 per-model 行 / 34 个循环 pool / 54 个 recurring/keyless 永久免费 provider**。来源：https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/README.md （"380 AI Providers" 节）
- 默认分支 release/v3.8.55 的 README hero/对比表已写 **394 providers**。来源：https://github.com/cheaper-inference/OmniRoute （README 渲染内容）
- README 自述这些数字**每两周重审、双向变动**（provider 取消免费额度就降）。机器可读目录在 `docs/reference/PROVIDER_REFERENCE.md`（由 `src/shared/constants/providers.ts` 自动生成）。
- 接入方式：OAuth 2.0（PKCE，Claude Code/Codex/Copilot/Cursor/Antigravity 等）、API key、web-cookie（ChatGPT Web/Claude Web/Gemini Web，Playwright）、keyless 免费 provider；自定义 OpenAI/Anthropic 兼容节点走 `omniroute nodes add` + `keys add`。连接级配额抓取器、`tos: avoid` 标记的 provider（如 Kiro）默认排除出 auto 路由。来源：README first-run 注记、FEATURES.md "Providers"、CLI-TOOLS.md。
- v3.8.53 新增：per-API-key `preferredConnections` 有序优先账号（#13102）、GitHub Copilot 连接记录 plan/配额状态（#12833）、Entra ID SSO 认证 /v1/*。来源：CHANGELOG [3.8.53]、releases 页。

### 3. 路由策略 / Combo（声称：14 种 combo 路由策略）

**查证：不符。main 分支为 20 种，默认分支 release/v3.8.55 为 22 种。未找到任何"14 种"的现行口径。**

- combo = 一条 `(provider, model, connection)` 步骤链，配额耗尽/故障/成本飙升时自动移到下一个健康目标；支持嵌套 combo-ref、结构化 builder、逐步骤 Test 按钮（v3.8.53 #16748）、combo 导入导出 JSON（#16747）。来源：README "Combos — The Flagship"、FEATURES.md "Combos"、CHANGELOG [3.8.53]。
- **main 分支 20 策略**（`src/shared/constants/routingStrategies.ts` → `ROUTING_STRATEGY_VALUES`，AUTO-COMBO.md 全表）：`priority`、`fill-first`、`weighted`、`round-robin`、`p2c`、`least-used`、`random`、`strict-random`、`cost-optimized`、`headroom`、`quota-weighted`、`reset-window`、`reset-aware`、`context-relay`、`context-optimized`、`cache-optimized`、`lkgp`、`auto`、`fusion`、`pipeline`。来源：https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/docs/routing/AUTO-COMBO.md （"All Routing Strategies"）
- **release/v3.8.55 新增 2 种至 22**：`expiry-first`（先花掉即将过期作废的配额：剩余最多且重置最近者胜）、`jev`（用户排序列表 + TypeSafe Jev 决定每请求执行哪些条目，opt-in flag）。来源：https://github.com/cheaper-inference/OmniRoute （README "Or build your own — 22 routing strategies" 表，第 21/22 行）
- 值得注意的语义细节（AUTO-COMBO.md）：`weighted` 是按权重比例随机抽取而非均衡器；`round-robin` 默认粘性批量 3 次成功再轮换（`stickyRoundRobinLimit`）；`fusion` 并行扇出模型面板 + judge 合成单一答案（带 tool 请求绕过面板，`minPanel`/`stragglerGraceMs`/`panelHardTimeoutMs` 可调）；`pipeline` 支持 planner/executor 的 agentic orchestration（`maxToolRounds` 默认 8）。
- 每个响应带 `X-OmniRoute-Decision` 头（策略/provider/延迟），路由透明化。来源：README "What's New"。

### 4. auto/* 前缀路由（声称：具备）

**查证：属实，且比内部描述更丰富。**来源：https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/docs/routing/AUTO-COMBO.md （"Zero-Config Auto-Routing"）、README auto 表。

- 零配置：客户端 `model: "auto"` 即可，无需建 combo。网关检测 `auto/` 前缀（`src/sse/handlers/chat.ts`）→ 拉取全部活跃连接 → 过滤有效凭据 → 每请求构建**内存虚拟 combo**（不落库）→ 按变体权重 + LKGP 策略路由。新增 provider 自动扩池。
- README（main）列出 10 个变体：`auto`（平衡/LKGP）、`auto/coding`、`auto/fast`、`auto/cheap`、`auto/subscription`（只走已付费订阅配额，**fail-closed 绝不回退到计费**）、`auto/thrifty`（订阅优先、逐级下探最便宜付费、重置后回到订阅）、`auto/offline`、`auto/smart`（质量优先 + 10% 探索）、`auto/lkgp`、`auto/chaos`（并行扇出面板，默认每 provider 一个模型共 5 个，非故障注入）。
- ** category × tier 组合**：`auto/<category>:<tier>`，category ∈ coding/reasoning/vision/chat/multimodal，tier ∈ fast/cheap(floor)/reliable/free/pro，如 `auto/coding:cheap`、`auto/multimodal:free`；过滤 fail-open（无匹配则用全池）。
- 候选池控制：`GET /v1/auto-combo/{channel}/candidates` 只读暴露实时候选（熔断/冷却/锁定状态）；per-API-key 排除候选（`auto_candidate_overrides` 表，migration 128）。
- 注意：`auto` **不使用**用户持久化 combo（除非 combo 恰好叫 auto）；`openrouter/auto` 是 OpenRouter 的真实付费产品，不是 OmniRoute 别名。

### 5. 评分机制（声称：9 因子评分）

**查证：不符。现行为 16 因子；官方 CHANGELOG 明确把"9 因子"作为过时错误表述纠正过。**

- AUTO-COMBO.md："a **16-factor scoring function**（`open-sse/services/autoCombo/scoring.ts` → `DEFAULT_WEIGHTS`）"，默认权重和为 1.0。16 因子及默认权重：`quota` 0.1429、`health` 0.1605、`costInv` 0.1429（60% 输入 + 40% 输出混合价）、`latencyInv` 0.1143（p95）、`taskFit` 0.0762、`stability` 0.0476、`tierPriority` 0.0476、`tierAffinity` 0.0476、`specificityMatch` 0.0476、`contextAffinity` 0.0476、`sessionAvailability` 0.0476、`connectionDensity` 0.0476、`cacheAffinity` 0（默认禁用）、`resetWindowAffinity` 0（默认禁用）、`quality` 0.03、`reliability` 0（默认禁用）。来源：https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/docs/routing/AUTO-COMBO.md （因子表）
- **一手纠错证据**：CHANGELOG [3.8.51]（2026-09-29）PR #12316："Six more documents that quote the scoring factor count joined the `check:docs-counts` gate, which caught five stale claims — **including one naming nine factors that do not exist** … The dashboard routing panel … labelled the default strategy '6-Factor Scoring'"。即官方文档门禁曾抓出"9 因子"与"6 因子"的过时说法。来源：https://github.com/cheaper-inference/OmniRoute/blob/main/CHANGELOG.md （[3.8.51] 段，docs(auto-combo) 条）
- 6 个 mode pack（ship-fast / cost-saver / quality-first / offline-friendly / reliability-first / chaos-mode）整体替换权重；每请求头 `X-OmniRoute-Mode`、`X-OmniRoute-Budget`（USD 上限）、`X-OmniRoute-Budget-Fallback`（cheapest/strict，strict 超预算返回 402）、`X-OmniRoute-Effort`（自适应 thinking 预算）。
- 持久 auto combo 另有 7 种 routerStrategy：rules（默认 16 因子）、score、cost/eco、latency/fast、sla-aware/sla、lkgp、nadir（外部 Nadir 决策 API，opt-in fail-open）。
- 自愈：得分 < 0.2 临时排除 5 分钟（渐进退避至 30 分钟）；>50% 熔断 OPEN 进入 incident mode 关闭探索；5% bandit 探索（可配）。配额不可读时候选 quota 轴记 0 并乘 0.5 软惩罚（#15347）。来源：AUTO-COMBO.md。

### 6. 配额跟踪与熔断（声称：具备）

**查证：均属实，且体系化程度高于内部描述。**

#### 6.1 配额/用量跟踪

- 每请求生成 usage 记录（apiKeyId/provider/model/comboId、prompt/completion/cached tokens、costUsd、latencyMs、status、errorClass）；无 usage 返回的 web-cookie provider 按 ~4 字符/token 估算。成本按 LiteLLM 同步价目计算（cached token 单独计价），`POST /api/pricing/sync` 手动触发。来源：https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/docs/guides/USAGE_QUOTA_GUIDE.md
- **per-API-key 预算**：`quotaWarnAt`（软警告）/ `quotaLimit`（硬限，超限 429 + Retry-After）/ `quotaWindow`（day/week/month/all）；`quotaSnapshots` 每请求快照，驱动进度条与 30 天趋势。v3.8.53 另有 per-key 日/周 USD 上限与无价目用量策略 `UNPRICED_USAGE_BUDGET_POLICY`（默认 fail_closed）。
- **订阅配额监控**：Claude Code / Codex / Copilot 的 per-session quota monitor（starting/idle/healthy/warning/exhausted/error，80% 告警）；`/dashboard/free-tiers` 实时显示免费额度 used/remaining；`/dashboard/quota` 每 key 用量 vs 限额、重置时间、告警历史。来源：MONITORING_GUIDE.md "Quota Monitors"、README free-tiers 节。
- **Quota-Share**（v3.8.50+）：多个 API key 公平共享一个上游账号配额，work-conserving（空闲份额可借出）；配套 per-connection `max_concurrent` 信号量串行化与 per-model 并发上限（v3.8.36+）。来源：README "What's New"、RESILIENCE_GUIDE.md §4、docs/routing/QUOTA_SHARE.md（索引）。
- REST：`GET /api/usage`、`/api/usage/analytics`、`/api/usage/quota`、`/api/usage/history`；MCP 工具 `omniroute_cost_report`、`omniroute_check_quota`；CLI `omniroute quota` / `usage` / `cost`。订阅/编码计划 provider 在成本分析中读 $0（诚实平价口径，v3.8.53）。

#### 6.2 熔断 / 韧性（3 独立层 + 可选层）

来源：https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/docs/architecture/RESILIENCE_GUIDE.md （v3.8.52，2026-10-18）、README resilience 图注。

| 层 | 作用域 | 关键默认值 |
| :-- | :-- | :-- |
| 1. Provider 熔断器 | 整个 provider | 仅 408/500/502/503/504 触发；OAuth 5 次 DEGRADED / 8 次 OPEN / 60s 重置，API-key 7/12/30s，本地 2 次/15s；OPEN 到期进 HALF_OPEN 探测；惰性恢复（无后台定时器）；状态 CLOSED/DEGRADED/OPEN/HALF_OPEN |
| 2. 连接冷却 | 单个 key/账号 | OAuth 基础 5s、API-key 3s，指数 ×2 退避 + 防惊群；429 优先上游 Retry-After/reset 头；并发型 429 固定 1s 冷却不爬退避梯；成功即清全部错误态 |
| 3. 模型锁定 | provider+connection+model | **v3.8.23 起 opt-in，默认关闭**；默认错误码 [403,404,429,502,503,504]，基础 120s、上限 30min、最多 10 级退避；成功响应使失败计数减半（success-decay）；状态在内存，重启即失 |

- 终态（banned/expired/credits_exhausted）不走冷却，需运营者处理。可选第 4 层全局 provider cooldown 窗口门（`PROVIDER_COOLDOWN_ENABLED`，默认关）。
- 配置面：Dashboard → Settings → Resilience / `PATCH /api/resilience`（**持久化设置优先于 env 默认值**）；`POST /api/resilience/reset` 重置熔断；Cooldown Manager 页（`/dashboard/resilience/cooldowns`）统一查看/清除冷却与锁定；`GET/DELETE /api/resilience/model-cooldowns`。
- 请求队列准入（v3.8.49，Bottleneck）：`maxWaitMs` 默认 30s（排队预算）、`executionMaxWaitMs` 默认 10min、`maxQueueDepth` 默认 0=不限；API-key 连接默认自动启用限流保护（`RATE_LIMIT_AUTO_ENABLE`）。
- 其他：combo 冷却感知重试（短 429 等待后重发而非直接回 429）、Claude OAuth 用量墙低优先通道/周重置（opt-in）、会话亲和（`sessionAffinityTtlMs`）、独占连接租约（`lease:exclusive`）、慢流吞吐看门狗（opt-in）。

### 7. 用量分析 / 热力图（声称：热力图分析）

**查证：属实。**

- Analytics dashboard："token consumption, cost estimates, **activity heatmaps**, weekly distribution charts, and per-provider breakdowns"。来源：https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/docs/guides/FEATURES.md （"Analytics" 节）
- 热力图精确定义：**小时 × 星期网格，颜色 = 请求数**（"Activity heatmap — Hour-of-day × day-of-week grid, color = request count"），由 `src/lib/usageAnalytics.ts` 计算，支持 1d/7d/30d/90d/ytd/all/custom 七档时间范围；同页还有日趋势、模型/ provider 分解、Top API keys、错误分析。来源：USAGE_QUOTA_GUIDE.md "Dashboard Widgets Computed"。
- 另有两处热力图（CHANGELOG 一手）：**小时级故障热力图** + 成本-延迟散点（v3.8.4，AI 路由分析，combo_metrics 查询）；**压缩工作室 token 显著性热力图**（v3.8.40，ultra 引擎逐 token scoreToken 着色）。 ActivityHeatmap 组件早在 v3.6.8 已有修复记录（#1309 自动滚动到当前日期）。来源：https://github.com/cheaper-inference/OmniRoute/blob/main/CHANGELOG.md （[3.8.4]、[3.8.40]、[3.6.8] 段）

### 8. Playground（声称：协议翻译器 + 模型测试台）

**查证：属实。实际有三个相关 surface，内部描述对应 Translator Playground + Model Playground。**

#### 8.1 Translator Playground（协议翻译调试）

FEATURES.md 原文："Four modes for debugging API translations: **Playground** (format converter), **Chat Tester** (live requests), **Test Bench** (batch tests), and **Live Monitor** (real-time stream)."——即格式转换器、实时请求测试、批量测试台、实时流监控四模式，与内部"协议翻译器 + 模型测试台"声称吻合。来源：https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/docs/guides/FEATURES.md （"Translator Playground" 节，截图 05-translator.png）

#### 8.2 Playground Studio（/dashboard/playground，v3.8.6 发布）

来源：https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/docs/frameworks/PLAYGROUND_STUDIO.md

- 四 tab：**Chat**（多轮流式 + markdown + 每消息 token/成本 + 重生成）、**Compare**（同一 prompt 最多 **4 模型并行**，每列独立 SSE，实时 TTFT/TPS/tokens/估算成本，标注"client-side estimate"）、**API**（Monaco 编辑器，10 端点：chat/completions/embeddings/images/audio/speech/transcriptions/moderations/rerank/search，多模态上传）、**Build**（tools[] JSON schema 编辑器 + 结构化输出校验，Zod）。
- 共享 Config 面板（endpoint/model/system/参数滑条/预设）、Prompt Improver（`POST /api/playground/improve-prompt`，消耗配额有警告弹窗）、代码导出（curl/Python/TS，key 占位 `$OMNIROUTE_API_KEY`）、预设 CRUD（`/api/playground/presets/*`，SQLite `playground_presets` 表，migration 084）。
- 流指标：ttftMs/totalMs/tps/tokensIn/tokensOut/costUsd（静态价目表估算）。

#### 8.3 Model Playground（v2.0.9+）

dashboard 内直接选 provider/model/endpoint 测试任意模型，Monaco 编辑、实时流、中途 abort、时序指标。来源：FEATURES.md "Model Playground"。

### 9. UI 页面（dashboard）

来源：FEATURES.md、MONITORING_GUIDE.md、CLI-TOOLS.md、CHANGELOG。

- 主要页面：Providers、Combos、Analytics（含热力图）、System Health（`/dashboard/health`：uptime/内存/p50/p95/p99/缓存/熔断态/quota 会话）、Quota（`/dashboard/quota`）、Free Tiers（`/dashboard/free-tiers`）、Playground、Translator、Request Logs（实时、按 provider/model/account/key 过滤）、Log Export（`/dashboard/log-export`，v3.8.52+）、API Endpoint / API Key 管理（`/dashboard/api-manager`）、Audit Log、Media（图/视频/音乐生成）、CLI Code's（`/dashboard/cli-code`，26-28 工具）、CLI Agents（`/dashboard/cli-agents`，11）、ACP Agents（`/dashboard/acp-agents`）、Settings（7 tab：General/Appearance/AI/Security/Routing/Resilience/Advanced）、Cooldown Manager。
- 主题 7 预设色 + 自定义 hex，light/dark/system；**67 语言** i18n；v3.8.53 起 dashboard 换用 vendored Keak CI UI Kit（`packages/ui-kit`）。
- 健康 API 双表面：`GET /healthz`（轻量生命周期，K8s readiness 用）与 `GET /api/monitoring/health`（深度，含 credentialHealth 探针缓存）；`/livez` 仅进程存活。provider 健康矩阵/autopilot/quota monitor 无专用 REST，走 MCP `observability_snapshot` 或 dashboard。来源：MONITORING_GUIDE.md。

### 10. CLI 套件（声称：具备）

**查证：属实。README 称 "80+ commands"，CLI 演示图注写 "89-command top-level surface"。**

- 顶层命令组（README cli-terminal.svg 图注）：providers · oauth · keys · combo · nodes · models · cache · compression · cost · usage · quota · health · resilience · telemetry · logs · audit · mcp · a2a · cloud · memory · skills · eval · tunnel · backup · sync · webhooks · policy · pricing · translator · simulate …。来源：https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/README.md （"Full CLI" 节）
- 常用：`omniroute`（serve，20128）、`omniroute chat`（TUI，/model /combo /skill /memory）、`setup`（向导，支持 --non-interactive）、`doctor`（8 项检查，--json）、`serve --headless` / `OMNIROUTE_HEADLESS=1`（v3.8.53+，仅 /v1/* 代理引擎，无 dashboard）。
- **15 个 `setup-*`** 一键配置命令（codex/claude/opencode/cline/kilo/continue/cursor/roo/crush/goose/qwen/aider/5dive/zed/whycodes），读实时模型目录写目标工具配置，支持 `--remote --api-key --dry-run`；容器内写入有守卫（exit 2 / API 422 `containerEphemeralTarget`）。`omniroute run <target>` 注入 env 直接启动 CLI（claude/codex/aider/goose/opencode/qwen/gemini/whycodes）；`omniroute configure` 11 目标交互式选择器。来源：https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/docs/reference/CLI-TOOLS.md 、README "What's New"。
- **Remote mode**：`omniroute connect <host>`（密码→scoped token 存为 context）、`contexts use`、`tokens create --scope read|write|admin`；进程 spawn 类路由仅 loopback。来源：README "Remote mode"、docs/guides/REMOTE-MODE.md（索引）。
- 其他：`keys policy show/set`（模型 allowlist、RPM、日/周 USD 上限）、`auth export --force`（本地明文导出凭据，0600 文件）、`backup/restore`、`sync bundle`、`tunnel`（cloudflare/tailscale/ngrok）、`providers import --dry-run`、shell completion。来源：CLI-TOOLS.md。

### 11. 安全 / SSRF 防护（声称：具备）

**查证：属实。v3.6.6+ 提供两层出站守卫。**来源：https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/docs/guides/FEATURES.md （"Safe Outbound Fetch & SSRF Guard" 节）、https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/docs/reference/ENVIRONMENT.md （§4 表，第 252-256 行附近）。

- 层 1 **URL guard**（`src/shared/network/outboundUrlGuard.ts`）：先按字面 hostname/IP 检查；public-only 模式下 safe-fetch 还会 DNS 解析并拒绝私有答案。**默认封禁云元数据端点**（整个 169.254.0.0/16 + 已知元数据主机名）。
- 层 2 **safe fetch wrapper**（`src/shared/network/safeOutboundFetch.ts`）：套用 URL guard、超时归一化、瞬态错误指数退避重试。provider 校验与模型发现全部走此守卫。
- 变量与默认值（ENVIRONMENT.md 原文）：`OUTBOUND_SSRF_GUARD_ENABLED` 默认 `true`；`OMNIROUTE_ALLOW_LOCAL_PROVIDER_URLS` 默认 **`true`**（允许本地/LAN URL——本地网关产品定位使然）；`OMNIROUTE_ALLOW_PRIVATE_PROVIDER_URLS` 默认 `false`（置 true 关闭检查）。dashboard 开关保存值优先于 env。内置本地 provider（LM Studio/Ollama/vLLM/Llamafile/Triton）跳过主机检查。
- 违规表现：`URL_GUARD_BLOCKED`（校验路径 HTTP 503，模型发现路径 400）；真实 SSRF 拦截写入审计日志 `provider.validation.ssrf_blocked` 事件。
- 周边安全（README/FEATURES/ENVIRONMENT）：凭据 AES-256-GCM 静态加密（可选整库加密 `STORAGE_ENCRYPTION_KEY`）、prompt-injection guard（每条 LLM 路由，红队套件）、opt-in PII/凭据遮蔽（v3.8.53 扩至 GDPR/POPIA 标识符）、错误净化 `sanitizeErrorMessage`、IP 过滤、CORS 策略、可选 OIDC 登录门、TLS 指纹伪装（wreq-js JA3/JA4）与 MITM/TPROXY 透明解密（per-SNI CA）。README 安全库清单含 ssrf-req-filter、Helmet.js、DOMPurify、safe-regex、Google Tink。

### 12. Live Monitor 实时视图（声称：具备）

**查证：属实，有两个实体。**

1. **Translator Playground 的 Live Monitor 模式**：实时翻译流监控（四模式之一）。来源：FEATURES.md "Translator Playground"。
2. **实时 WebSocket live monitoring daemon**（v3.8.3，2026-05-24，PR #2668）："runs a Node.js WebSocket daemon sidecar on port **20129** to emit real-time events for request starts/completes/fails, combo attempts, and credential status in the dashboard logs." 来源：https://github.com/cheaper-inference/OmniRoute/blob/main/CHANGELOG.md （[3.8.3] 段）
- 配套实时面：Request Logs 实时过滤、System Health 实时指标、quota telemetry live（v3.8.50）、`/dashboard/health` 可配 `refresh_interval_ms`（`~/.omniroute/dashboard.json`）。来源：FEATURES.md、README 增长表、MONITORING_GUIDE.md。

### 13. OTLP 导出（声称：具备）

**查证：属实，但是很新的能力（v3.8.53，2026-11-06），且 main 分支监控文档尚未同步。**

- **v3.8.52**（2026-10-19，PR #11945）先落地**可插拔日志导出目的地框架**：按小时 JobRegistry cron 持续导出调用日志，per-destination 游标、批量插入、`/api/log-export/*` CRUD/连接测试/手动运行、`/dashboard/log-export` 配置 UI；首个目的地 **Google BigQuery**（服务账号密钥加密存储、按 timestamp 日分区）；`includeBodies` opt-in 携带请求/响应载荷（继承 PII 净化与 secret redaction）。来源：https://github.com/cheaper-inference/OmniRoute/blob/main/CHANGELOG.md （[3.8.52] 段）
- **v3.8.53**（2026-11-06，PR #16664）新增 **OTLP/HTTP JSON traces 目的地**："Export persisted call logs as OTLP/HTTP JSON traces, with optional **Langfuse** generation mapping, encrypted headers, per-destination pseudonymous identities, bounded requests and retries"；可选 `requestHeaderAllowlist` 把保存的客户端请求头作为 OTLP string-array 属性（凭据头永远排除）；导出记录含可空 TTFT。同版 #14736 丰富 **OTel spans**：携带 `omniroute.api_key.id` 与 `gen_ai.usage.cost`/`.currency`。来源：CHANGELOG [3.8.53] 段（第 174、209 行附近）。
- **文档滞后提示**：main 分支 MONITORING_GUIDE.md（frontmatter v3.8.50，2026-10-07）仍写 "Prometheus / OpenTelemetry Export (Phase 2) — **Planned for v3.9**"，并说延迟百分位暂无 REST。这与 CHANGELOG 的已发布 OTLP 日志导出并不矛盾（前者指原生指标/tracing 导出管线），但引用时须以 CHANGELOG/release notes 为准。main 分支 ENVIRONMENT.md（v3.8.52）中**未检索到任何 OTEL_/OTLP 变量**——OTLP 目的地经 dashboard/`/api/log-export` 配置而非 env。来源：https://raw.githubusercontent.com/diegosouzapw/OmniRoute/main/docs/ops/MONITORING_GUIDE.md 、ENVIRONMENT.md 全文检索。

### 14. 部署形态

来源：README "Where OmniRoute Runs" / "More install methods"、docs 索引。

- npm 全局（`npm install -g omniroute`，npm 11+ 需 allow-scripts 白名单）、Bun 全局（bun:sqlite 自动探测）、Docker（`diegosouzapw/omniroute`，AMD64+ARM64；`:latest` 跟最高已发布稳定 SemVer，`:next` 跟 release/* 分支仅供测试；**coding agent 负载需 8-12 GB V8 heap**，镜像默认 1024 MB 仅够 dashboard）、Dockerfile.bun、Compose profiles（base/selfhost/prod/ollama-local）、Podman（Quadlet/systemd）、Electron 桌面（Win/macOS/Linux，Electron 44）、Termux（Android 手机）、PWA、AUR（omniroute-bin）、Nix flake、Fly.io/VM 部署指南、headless 模式。
- 周边：OmniRouteTray（社区 macOS 菜单栏监管器，Tauri v2，非官方）、OmniCopilot VS Code 扩展（模型进原生 Copilot Chat 选择器）、`@omniroute/opencode-provider` / opencode-plugin-v2。
- 远程访问：Cloudflare Quick Tunnel / Tailscale Funnel / ngrok、reverse-proxy basePath、scoped sync tokens + config bundle（ETag）。

### 15. 内部声称之外的显著能力（供 Token Router 参照）

- **12 引擎 token 压缩管线**（Session-Dedup/CCR/Lite/RTK/Responses Tool Output/Headroom(GCF)/Relevance/Caveman/Aggressive/LLMLingua-2(MobileBERT ONNX)/Ultra/OmniGlyph），默认 RTK→Caveman 叠加宣称 78-95%（均值 ~89%）节省；代码块/URL/JSON 字节级保护；per-combo 管线、每请求 `x-omniroute-compression` 头、自适应 context-budget 拨盘、离线 eval harness。来源：README 压缩节、docs/compression/*。
- MCP 110 工具 + A2A + OmniConductor（入站 A2A 委派 + Faro 语音）、持久记忆（FTS5 + int8 向量，默认关）、skills/plugin 框架 + marketplace、guardrails/evals、语义响应缓存、Batch & Files API、gamification、Telegram bot 桥、Radar 免费目录 overlay（opt-in、GET-only、不上传遥测）、teams 成本中心（v3.8.53）、嵌入式 sidecar（Bifrost/Mux/CLIProxyAPI/sing-box）。来源：README "What's New"、docs 索引、CHANGELOG [3.8.53]。

---

### 内部声称 vs 查证结果对照表

| # | 内部声称 | 查证结果 | 依据（一手来源） |
| :-- | :-- | :-- | :-- |
| 1 | 仓库 diegosouzapw/OmniRoute 存在 | **存在**，已转移至 cheaper-inference/OmniRoute（原 URL 重定向有效） | https://github.com/cheaper-inference/OmniRoute |
| 2 | 约 9 万星 | **88.2k**（当日页面显示），基本接近、略低 | 同上 |
| 3 | TypeScript | **属实**，100% TS（src/ + open-sse/），Next.js 16 + React 19 | main README Tech Stack |
| 4 | 定位"OpenRouter 开源替代" | **大体成立但不精确**：自述 "The Free AI Gateway"，主打免费额度聚合 + 编码 CLI 网关；README 有 vs OpenRouter 对比表 | main README 标题/对比节 |
| 5 | 约 370 个 providers | **数量级属实、数字偏旧且口径漂移**：About 375 / main README 380（153 标免费）/ 默认分支 release/v3.8.55 已写 394；chat 注册表口径 239 providers | GitHub About；main README Providers 节；默认分支 README |
| 6 | 14 种 combo 路由策略 | **不符**：main 分支 **20** 种；release/v3.8.55 **22** 种（新增 expiry-first、jev）。未见 14 种口径 | AUTO-COMBO.md 策略表；默认分支 README 22 策略表 |
| 7 | `auto/*` 前缀路由 | **属实且更丰富**：10 变体（含 subscription/thrifty/chaos）+ `auto/<category>:<tier>` 组合 + 候选池只读 API + per-key 排除 | AUTO-COMBO.md；main README auto 表 |
| 8 | 9 因子评分 | **不符**：现行 **16 因子**（DEFAULT_WEIGHTS）；CHANGELOG v3.8.51 #12316 明确纠正过"nine factors that do not exist"的过时文档 | AUTO-COMBO.md 因子表；CHANGELOG [3.8.51] |
| 9 | 热力图分析 | **属实**：Analytics 活动热力图（小时×星期，色=请求数）；另有小时级故障热力图（v3.8.4）与压缩显著性热力图（v3.8.40） | USAGE_QUOTA_GUIDE.md；FEATURES.md；CHANGELOG |
| 10 | Playground（协议翻译器+模型测试台） | **属实**：Translator Playground 四模式（格式转换/Chat Tester/Test Bench/Live Monitor）+ Playground Studio（Chat/Compare/API/Build，v3.8.6）+ Model Playground（v2.0.9+） | FEATURES.md；PLAYGROUND_STUDIO.md |
| 11 | 配额跟踪 | **属实**：per-key 软硬限额+窗口、quotaSnapshots、订阅会话 quota monitor、/dashboard/free-tiers 实时余量、Quota-Share 公平分配 | USAGE_QUOTA_GUIDE.md；MONITORING_GUIDE.md；README |
| 12 | 熔断 | **属实**：3 独立层（provider 熔断 8/12/2 次阈值、连接冷却 5s/3s 指数退避、模型锁定 opt-in）+ 队列准入 + 可选全局冷却；Settings/API 可调 | RESILIENCE_GUIDE.md；README resilience 图注 |
| 13 | SSRF 防护 | **属实**（v3.6.6+）：两层出站守卫，默认封 169.254.0.0/16 元数据；`OMNIROUTE_ALLOW_LOCAL_PROVIDER_URLS` 默认 true（放行本地/LAN）；拦截写审计 `provider.validation.ssrf_blocked` | FEATURES.md SSRF 节；ENVIRONMENT.md §4 |
| 14 | CLI 套件 | **属实**：80+（图注 89）顶层命令，15 个 setup-*、run/configure、remote mode scoped tokens、headless | main README CLI 节；CLI-TOOLS.md |
| 15 | Live Monitor 实时视图 | **属实**：Translator Playground 的 Live Monitor 模式 + WebSocket 实时监控 daemon（端口 20129，v3.8.3 #2668） | FEATURES.md；CHANGELOG [3.8.3] |
| 16 | OTLP 导出 | **属实（v3.8.53 新增）**：log-export 框架（v3.8.52，BigQuery 首发）+ OTLP/HTTP JSON traces 目的地（Langfuse 映射、加密头、假名化）；OTel span 带成本属性。注意 main 监控文档仍写"planned for v3.9"（滞后），配置走 dashboard/API 而非 env | CHANGELOG [3.8.52] #11945、[3.8.53] #16664/#14736；MONITORING_GUIDE.md |
| 17 | （隐含）最新版本 | **v3.8.53，2026-11-06**；默认分支 release/v3.8.55 未发布；main 与默认分支内容存在版本差 | https://github.com/cheaper-inference/OmniRoute/releases ；CHANGELOG |

### 引用注意事项

1. **分支口径**：该仓库默认分支是 `release/v3.8.55` 而非 main，README/文档数字（providers、策略数、免费额度条目）两分支不一致。本文对每个数字标注了所属分支；后续引用建议锁定 tag（如 `blob/v3.8.53/...`）以免漂移。
2. **文档滞后**：多篇 docs frontmatter 停在 v3.8.40/3.8.50/3.8.52，CHANGELOG/release notes 才是最新事实（OTLP 即一例）。官方甚至设有 `check:docs-counts` CI 门禁抓文档数字漂移——侧面说明其文档数字曾长期不可靠。
3. **营销数字**：~1.62B 免费 tokens/月、15-95% 压缩节省、provider 总数均为项目自述口径（README 注明两周重审、双向变动、pool 去重），本调研只核实"官方如此声称"，未独立验证其计算。
---

## 横向对比与对 Token Router 的启示

### 两家产品的共同走向（验证 v3 方向）

1. **配额感知路由已成标配**：magpie 的 smart/pace/reset 窗口策略 + Daily warm-up，OmniRoute 的 quota-weighted/reset-aware/reset-window/expiry-first 策略与 Quota-Share——对应我们 P5-B 的 QuotaTracker + reset-aware 策略，方向正确且两家都在加码。
2. **实时决策视图是门面**：magpie Routing view（每决策 live + intent trace）、OmniRoute Live Monitor daemon + Request Logs——对应 P5-C 路由时间线。注意两家都做到了"每次 failover try 可见"，我们的 attempts 账本数据粒度足够，缺的只是呈现。
3. **成本分析维度趋同**：按账号/key/模型出账、cache 读写分解（magpie 区分 Anthropic 5m/1h 两档）、OTLP 导出（两家都有，OmniRoute 还带 Langfuse 映射）——P5-C 的分解维度可对齐。
4. **Playground 形态**：OmniRoute 的 Compare（4 模型并行 + TTFT/TPS 实时指标）值得 P5-D 借鉴；magpie 无独立 Playground（用真实 agent 测试）。
5. **网关 key 限额**：magpie 的每 key 最多 5 条限额（日/周/月/每N小时/跟随账号窗口）+ 模型/账号/MCP 范围限定，比 P5-B 当前设计（日/周/月 token+成本）更细，可作二期参考。

### Token Router 的差异化仍然成立

- **决策确定性回放**：两家都没有"决策快照逐字段回放一致"的契约（magpie 有 trace 展示，OmniRoute 有 X-OmniRoute-Decision 头，但都不保证可回放）。我们的 DecisionSnapshot + core 纯函数是独有的可解释性深度。
- **判官参与路由**：magpie 意图路由用分类器小模型选规则（每 turn 一次、fail-open），OmniRoute fusion 用 judge 合成面板答案、jev 用外部决策 API——思路与我们的判官同族，但我们的判官有预算/deadline/确认闩锁的完整语义，且与级联/sticky 的仲裁顺序是明文契约。
- **内核冻结的工程纪律**：两家都是快速迭代的大泥球（OmniRoute 13.5k commits、文档数字长期漂移需 CI 门禁抓；magpie 8.5k commits、日更 release）。我们的 diff=0 门 + 契约冻结是长期可维护性卖点，但对外宣传要转译成用户价值（"行为可预期、升级不破坏"）。

### 需要警惕的差距

- **订阅账号驱动**是 magpie 的核心护城河（Claude 订阅经真实 claude 二进制 + MCP 桥接、多账号 failover、warm-up），我们 P7 才评估——对"手里是订阅不是 API key"的用户群，短期无竞争力。
- **免费额度聚合**是 OmniRoute 的定位核心（153 个免费 provider、keyless 接入），我们完全不做——对标宣传时避免在"省钱"维度正面交锋，主打"决策质量与可解释"。
- **agent 接入数量**：magpie 88 / OmniRoute 15 个 setup-* + 11 configure 目标，我们 8 个；P5-D 扩到 ~20 仍落后，但精确合并（保留注释/幂等/golden 测试）质量口径领先。
- **数字时效**：两家迭代极快（magpie 日更、OmniRoute 周更），对标数字会持续漂移；本文锁定 2026-11-17 快照，对外引用注明日期，OmniRoute 建议锁 tag 引用。
