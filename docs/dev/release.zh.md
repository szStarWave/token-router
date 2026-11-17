# 打包与发布

> 事实基准：`v2/apps/desktop/src-tauri/tauri.conf.json`、`v2/apps/desktop/scripts/prepare-sidecar.mjs`、[P4 交付报告（归档）](../archive/p4-delivery-report.zh.md)（2026-11-17）。
> 状态：可打包产物与验收自检已完成；**GA 剩签名/公证与 24h soak**（需发布 CI 与真实环境）。

## 产物形态

- 产品名：`Token Router v2`；bundle 目标当前**仅 NSIS（Windows）**。
- 网关二进制 `token-router-v2` 以 Tauri sidecar 捆绑（`externalBin: ["sidecar/token-router-v2"]`），桌面壳 supervisor 托管。
- macOS（dmg）/Linux（AppImage）打包配置待平台演练。
- 仓库根 `packaging/`（electron-win32、flatpak）与 Makefile 的 tauri-build/OTA 目标属 **v1**，与 v2 无关。

## 构建流程

```bash
cd v2
cargo build -p tr-server --release
node apps/desktop/scripts/prepare-sidecar.mjs
# 默认 triple：x86_64-pc-windows-msvc
# 复制 target/{debug|release}/token-router-v2.exe → sidecar/token-router-v2-<triple>.exe
cd apps/desktop/src-tauri
cargo tauri build
```

## 发布检查单

- [ ] 全门禁通过（见 [开发者指南](./contributing.zh.md)）
- [ ] Windows 安装包签名（证书在发布 CI 提供；流程与配置已就绪，未执行）
- [ ] macOS 公证（待平台演练）
- [ ] 真实客户端回归：Hermes/Codex/Claude Code 经 UI 接入后的真实流式/非流式请求（需有上游的环境）
- [ ] 24h soak 与 10 万行账本 p95 性能基线（索引与 `window_too_large` 90 天限制已就位）
- [ ] 升级验证：安装新版不覆盖数据目录

## 更新方式

手动更新优先（MVP 裁剪）；OTA manifest 签名校验流程已文档化未启用。`docs/ota-release-notes.json` 属 v1 OTA 发布流程（`scripts/publish_ota/`），v2 不读取。

## 已知限制（交付基线）

1. 签名/公证未执行（依赖发布 CI 证书）。
2. 真实客户端回归、24h soak、性能基线待真实环境。
3. 打包以 Windows NSIS 为主。
4. 前端 UnoCSS 预设 vendored（上游未发布 npm），升级需手工同步，见 [前端设计系统](./frontend-design-system.zh.md) §2/§9。