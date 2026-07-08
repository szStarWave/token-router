# Token Router Desktop (Tauri)

基于 [Tauri v2](https://v2.tauri.app/start/) 的桌面壳：内嵌 Gateway（`embedded` 模式）、Web UI、系统托盘。

## 前置条件

- [Rust](https://rustup.rs/)（`cargo`）
- Node.js 18+ 与 [pnpm](https://pnpm.io/)
- **Windows**：WebView2（Win10/11 通常已自带）
- **macOS**：Xcode Command Line Tools（`xcode-select --install`）

## 网络代理（下载 crates 时）

PowerShell：

```powershell
$env:HTTPS_PROXY = "http://127.0.0.1:7890"
$env:HTTP_PROXY = "http://127.0.0.1:7890"
```

macOS / Linux：

```bash
export HTTPS_PROXY=http://127.0.0.1:7890
export HTTP_PROXY=http://127.0.0.1:7890
```

## 开发

```bash
cd desktop
pnpm install
pnpm --dir frontend install
pnpm run tauri:dev
```

或从仓库根目录：

```bash
make tauri-dev
```

- 前端：`desktop/frontend/`（Vite 开发服 `http://localhost:1420`）
- 后端：`src-tauri/`（Rust 命令 + 托盘）

## 构建安装包

### macOS

```bash
cd desktop
pnpm --dir frontend install
pnpm run tauri:build
# 或
make tauri-build-macos
```

产物：

| 类型 | 路径 |
|------|------|
| `.app` | `desktop/src-tauri/target/release/bundle/macos/Token Router.app` |
| `.dmg` | `desktop/src-tauri/target/release/bundle/dmg/Token Router_<version>_aarch64.dmg` |

数据目录：`~/.token-router-desktop/`（配置、日志、Gateway 状态）。

### Windows

```powershell
cd desktop
pnpm run tauri:build:win
# 或仓库根目录：make tauri-build
```

产物：`src-tauri/target/release/bundle/nsis/`（NSIS 安装包）及 `token-router-desktop.exe`。

Windows 打包 NSIS 时需要 Tauri 工具链（与 WiX/MSI 无关）。若 GitHub 下载超时，先运行：

```powershell
make setup-tauri-nsis
# 或
powershell -ExecutionPolicy Bypass -File scripts/setup-tauri-nsis.ps1
```

工具会安装到 `%LOCALAPPDATA%\tauri\NSIS\`（含 `makensis.exe` 与 `Plugins\x86-unicode\additional\nsis_tauri_utils.dll`）。MSI 才需要 WiX，解压到 `%LOCALAPPDATA%\tauri\WixTools314\`。

## 功能说明

| 能力 | 说明 |
|------|------|
| Gateway | 启动时自动 `embedded::start()`，无需单独 CLI 守护进程；数据目录 `~/.token-router-desktop/`（Windows：`%USERPROFILE%\.token-router-desktop\`） |
| Agent 快捷配置 | 一键写入 OpenClaw / Hermes / Claude Code / Codex / OpenCode 配置（macOS / Windows 桌面版） |
| Herdsman 集成 | Windows：命名管道 + HTTP；**macOS**：HTTP 探测 + `.app` 启动检测 |
| WSL Agent 配置 | 仅 Windows 桌面版 |
| UI | React + Vite 前端 |
| 托盘 | 右键菜单：Show / 显示、Quit / 退出；左键显示窗口 |
| 关闭窗口 | 隐藏到托盘，不退出进程（托盘 Quit 退出） |
| OTA 更新 | Windows / macOS 发布版（开发模式不启用后台检查） |

## IPC Status（Gateway 发现）

第三方客户端可查询 Gateway URL，协议与 [Herdsman](https://github.com/szStarWave/herdsman) 对称。

### Windows（命名管道）

| 项目 | 值 |
|------|-----|
| 管道路径 | `\\.\pipe\Token-Router-status` |
| 命令 | `/status` → JSON；`/exit` → 关闭连接 |

PowerShell 示例：

```powershell
$pipe = New-Object System.IO.Pipes.NamedPipeClientStream(".", "Token-Router-status", [System.IO.Pipes.PipeDirection]::InOut)
$pipe.Connect(3000)
$writer = New-Object System.IO.StreamWriter($pipe)
$reader = New-Object System.IO.StreamReader($pipe)
$writer.Write("/status")
$writer.Flush()
$reader.ReadToEnd()
$pipe.Close()
```

### macOS / Linux（Unix Domain Socket）

| 项目 | 值 |
|------|-----|
| Socket 路径 | `~/.token-router-desktop/Token-Router-status.sock` |
| 命令 | `/status` → JSON；`/exit` → 关闭连接 |

macOS 示例：

```bash
printf '/status' | nc -U ~/.token-router-desktop/Token-Router-status.sock
```

`/status` 响应示例：

```json
{
  "app_name": "Token Router",
  "running": true,
  "host": "127.0.0.1",
  "port": 11080,
  "endpoint": "http://127.0.0.1:11080",
  "webui_url": "http://127.0.0.1:11080/setup",
  "openai_endpoint": "http://127.0.0.1:11080/v1",
  "chat_endpoint": "http://127.0.0.1:11080/v1",
  "responses_endpoint": "http://127.0.0.1:11080/v1/responses",
  "anthropic_endpoint": "http://127.0.0.1:11080/anthropic",
  "timestamp": "..."
}
```

实现参考：`src-tauri/src/status_pipe.rs`（服务端）、`src-tauri/src/herdsman.rs`（Herdsman 客户端）。

## Tauri 命令（前端 `invoke`）

| 命令 | 说明 |
|------|------|
| `gateway_start` | 启动内嵌 Gateway，返回 base URL |
| `gateway_stop` | 停止内嵌 Gateway |
| `gateway_restart` | 重启内嵌 Gateway |
| `gateway_is_running` | 是否运行中 |
| `gateway_url` | 当前 `http://host:port` |
| `gateway_status` | `{ running, url, version }` |
| `configure_*_agent` | 写入各 Agent 配置文件 |
| `wsl_*` | WSL Agent 配置（Windows only） |
| `show_main_window` | 显示主窗口 |
| `feedback_app_version` / `feedback_submit` | 意见反馈 |
| `ota_*` | Windows / macOS 发布版 OTA（开发模式不启用后台检查） |

## OTA 发布

### Windows

见仓库根目录 `Makefile` 的 `build-ota` / `push` 目标。

Manifest：`{region}/{channel}/{with_account|without_account}/latest.json`

### macOS

1. 构建 DMG：`make tauri-build-macos`
2. 上传至 ModelScope（`macos` 子目录）：

```bash
export MODELSCOPE_TOKEN=<token>
uv run --with modelscope python scripts/publish_ota/publish.py \
  --platform macos \
  --channel flowy --region-scope CN --version v0.16.0 \
  --enable-account-system true \
  --setup-path "desktop/src-tauri/target/release/bundle/dmg/Token Router_0.16.0_aarch64.dmg"
```

Manifest：`{region}/{channel}/{with_account|without_account}/macos/latest.json`

更新说明维护在 [`docs/ota-release-notes.json`](../docs/ota-release-notes.json)。

```
desktop/
  frontend/            # React + Vite UI
  package.json
  src-tauri/           # Tauri Rust 后端
```
