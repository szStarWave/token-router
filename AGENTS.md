# AGENTS.md

## 仓库结构：两条产品线

- **v1（遗留）**：根目录 `src/`、`desktop/`、`ffi/`、`example/`、`packaging/`、`scripts/publish_ota/`。只修明确的 v1 bug，不做新功能。
- **v2（现役）**：`v2/`。Rust workspace（`crates/*` 12 个 + `xtask`）+ Tauri 桌面端（`apps/desktop`，独立于 workspace）。新工作默认落 v2。

## 硬门（v2）

- **内核冻结**：产品层改动保持 `v2/crates/{core,decide,proto}` diff = 0（提交纪律，逐次提交自查；自动化的 `config-only-check` 只覆盖 core 指纹）。确需改内核语义：先改 [内核设计文档](docs/archive/rewrite-v2-design.zh.md)（语义权威），再动代码。
- **新路由能力走扩展点**：SignalBag extractor / Strategy trait 注册，不改内核。crate 边界：`core` 纯函数零 IO，`decide` 唯一判官入口，`upstream` 唯一 HTTP 边界，`server` 经 `agent` 调内核；`check-deps` 白名单强制。
- **新增候选/虚拟模型只改 TOML**（`config-only-check` 强制）。
- **管理 API 变更留痕**：DTO 改动跑 `cargo xtask contract-check` 更新 TS 绑定；删除/重命名端点或字段等破坏性变更记入契约 §5.1。契约：[docs/contracts/admin-api.zh.md](docs/contracts/admin-api.zh.md)。
- **密钥只存引用**（`env:`/`file:`/`keyring:`）：明文 key 只存在于 Rust 侧内存与凭据存储；配置、API 响应、日志、账本、前端一律只见引用。admin token 不进 WebView。

## 验证

提交前跑 `make v2-check` + `make v2-test`；改了 admin DTO 加跑 `contract-check`。全门禁清单与命令见 [docs/dev/contributing.zh.md](docs/dev/contributing.zh.md)。

## 易错点

- 配置文件 `schema_version` 严格等于 `1`（`tr-agent::config::SCHEMA_VERSION`）。
- 端口：独立服务端默认 `127.0.0.1:16621`；桌面 supervisor 与 `tr-cli` 默认 base-url 是 `17621`。
- 根目录 `example/`、`packaging/`、Makefile 的 tauri-build/OTA 目标属 v1；v2 示例在 `v2/example/config.minimal.toml`。
- `docs/s0-gap-report.md` 由 xtask 生成，勿手改。

## 界面文字纪律（v2 桌面端 `v2/apps/desktop/frontend/`）

界面文字默认按噪声处理：砍掉标语、提示段落和次要标签。**如果一个页面非得靠这些东西才看得懂，那是布局的问题——改布局，不要补文案。**

- **页头只放标题**：无副标题、无标语、无导语。**卡片不写解释性段落**：一个事实只写一次，不在每张卡上重复（93 个模型的列表不能有 93 段说明）。
- **说明只能出现在三处**：控件标签、空状态的下一步动作、真实警告/错误。口径类说明收进 `title` 悬停提示（设置搜索仍匹配提示文本）。
- **用布局承载语义，不用句子承载**：把当前值放进控件本身（如 `xxx（不可用）` 的禁用选项），用显式字段名与表头（"等同于/列为" → "模型名/上游模型"），删掉派生/重复字段与逐项计数胶囊。
- **保留**：破坏性确认、错误横幅、状态标记（默认/禁用/不可用）——它们是决策，不是标语。

每次界面改动都要同时满足三条，缺一条就不算完成：

- **UX**：人看得懂、少读字。
- **DX**：开发者读得懂、能定位问题。
- **AX**：智能体（Claude Code / Codex 等）读得懂——标识符、状态、错误码可被机器消费，不靠散文描述。

**不许弄坏任何东西**：删文案前确认没有唯一信息只存在于被删文案中；改完在真实 WebView 验证（每个 nav 路由能渲染、弹层能开关、控制台 0 错误、组 E2E 往返后配置回到原摘要），再跑 `lint` / `test` / `check:uno` / `build`。

细则与逐页对照见 [frontend-design-magpie.zh.md](docs/frontend-design-magpie.zh.md) §8。

## 文档

索引：[docs/README.md](docs/README.md)。事实以代码为准；文档与代码冲突时改文档。过程性报告产出后归档（`docs/archive/`），不回填。

## 工作方式

- 工作区常有并行未提交改动：只提交本任务相关文件，其余原样保留。
- commit 信息用英文 conventional 风格（`feat(v2):` / `fix(v2):` / `docs:`）。