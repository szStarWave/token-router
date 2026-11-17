# 开发者指南

> 事实基准：根 `Makefile`、`v2/xtask/`、`v2/Cargo.toml`（2026-11-17）。

## 环境

- Rust 1.85（`v2/rust-toolchain.toml`），edition 2024；workspace version 0.20.0。
- workspace 成员：`v2/crates/*`（12 个 crate）+ `v2/xtask`；桌面端（`v2/apps/desktop`）独立于 workspace。
- 前端：Node.js + pnpm/npm，React 19 + TanStack Router + UnoCSS（见 [前端设计系统](./frontend-design-system.zh.md)）。
- 桌面壳开发需要 tauri-cli；sidecar 准备见 [发布文档](./release.zh.md)。

## 常用命令

```bash
# Rust（在仓库根目录）
make v2-check      # fmt --check + clippy -D warnings + check-deps + config-only-check
make v2-test       # workspace 全部测试
make v2-fix        # fmt 自动修复
make v2-fixtures   # fixtures-check（协议覆盖，报告写 docs/s0-gap-report.md）

# xtask（在 v2/ 下）
cargo xtask check-deps         # crate 依赖白名单
cargo xtask fixtures-check     # 协议 fixture 自检 + 覆盖报告（--report 可改输出路径）
cargo xtask config-only-check  # 加候选/虚拟模型只改 TOML，内核指纹不变
cargo xtask contract-check     # ts-rs 重新生成并与 bindings/ 比对，无 diff 才过

# 前端（v2/apps/desktop/frontend/）
npm run dev        # vite 开发
npm run build      # tsc -b + vite build
npm run lint       # oxlint
npm test           # config-edit 语义测试
npm run check:uno  # UnoCSS token/类名命中率检查
```

## 提交门禁（全量）

1. `fmt` / `clippy -D warnings`
2. workspace 全部测试（316+，含 16 golden 合并器测试、SSE 集成、22 onboard 单测）
3. `check-deps`（依赖单向白名单）
4. `fixtures-check`
5. `config-only-check`
6. `contract-check`（管理 API DTO ↔ TS 绑定一致）
7. **`core` / `decide` / `proto` diff = 0**（产品层迭代不许动内核）
8. 桌面壳测试（7）+ 前端测试（6）+ `check:uno`

## 改东西的规矩

- **内核语义**（路由/协议/配置契约）：先改 [内核设计文档](../archive/rewrite-v2-design.zh.md) 或其后继契约，再动代码；`core` 纯函数零 IO，`decide` 是唯一判官入口，`upstream` 是唯一 HTTP 边界。
- **管理 API**：只增不改；删除/重命名需 `api_version + 1` 与弃用期（[契约](../contracts/admin-api.zh.md)）。DTO 改完跑 `contract-check` 更新绑定。
- **新增候选/虚拟模型**：只改 TOML（`config-only-check` 强制）。
- **新增路由能力**：走 SignalBag extractor / Strategy trait 注册（[架构 §5](../architecture.zh.md)），不改内核。
- **前端**：不写手写 CSS，一律 UnoCSS token；规范见 [前端设计系统](./frontend-design-system.zh.md)。

## 测试与评测工具

`tr-cli` 子命令：`replay`（决策快照回放，唯一直调 core 的只读例外）、`check-config`、`shadow-report`、`judge-test`、`doctor`、`eval`（含 `eval calibrate`；任务集 `v2/tests/eval/tasks.json`，33 任务自带 checker）、`admin`（17 个管理子命令）。