# AGENTS.md

## 仓库结构：两条产品线

- **v1（遗留）**：根目录 `src/`、`desktop/`、`ffi/`、`example/`、`packaging/`、`scripts/publish_ota/`。只修明确的 v1 bug，不做新功能。
- **v2（现役）**：`v2/`。Rust workspace（`crates/*` 12 个 + `xtask`）+ Tauri 桌面端（`apps/desktop`，独立于 workspace）。新工作默认落 v2。

## 硬门（v2）

- **内核冻结**：产品层改动保持 `v2/crates/{core,decide,proto}` diff = 0（提交纪律，逐次提交自查；自动化的 `config-only-check` 只覆盖 core 指纹）。确需改内核语义：先改 [内核设计文档](docs/archive/rewrite-v2-design.zh.md)（语义权威），再动代码。
- **新路由能力走扩展点**：SignalBag extractor / Strategy trait 注册，不改内核。crate 边界：`core` 纯函数零 IO，`decide` 唯一判官入口，`upstream` 唯一 HTTP 边界，`server` 经 `agent` 调内核；`check-deps` 白名单强制。
- **新增候选/虚拟模型只改 TOML**（`config-only-check` 强制）。
- **管理 API 只增不改**：删除/重命名需 `api_version + 1` 与弃用期；DTO 改动跑 `cargo xtask contract-check` 更新 TS 绑定。契约：[docs/contracts/admin-api.zh.md](docs/contracts/admin-api.zh.md)。
- **密钥只存引用**（`env:`/`file:`/`keyring:`）：明文 key 只存在于 Rust 侧内存与凭据存储；配置、API 响应、日志、账本、前端一律只见引用。admin token 不进 WebView。

## 验证

提交前跑 `make v2-check` + `make v2-test`；改了 admin DTO 加跑 `contract-check`。全门禁清单与命令见 [docs/dev/contributing.zh.md](docs/dev/contributing.zh.md)。

## 易错点

- 配置文件 `schema_version` 严格等于 `1`（`tr-agent::config::SCHEMA_VERSION`）。
- 端口：独立服务端默认 `127.0.0.1:16621`；桌面 supervisor 与 `tr-cli` 默认 base-url 是 `17621`。
- 根目录 `example/`、`packaging/`、Makefile 的 tauri-build/OTA 目标属 v1；v2 示例在 `v2/example/config.minimal.toml`。
- `docs/s0-gap-report.md` 由 xtask 生成，勿手改。

## 文档

索引：[docs/README.md](docs/README.md)。事实以代码为准；文档与代码冲突时改文档。过程性报告产出后归档（`docs/archive/`），不回填。

## 工作方式

- 工作区常有并行未提交改动：只提交本任务相关文件，其余原样保留。
- commit 信息用英文 conventional 风格（`feat(v2):` / `fix(v2):` / `docs:`）。