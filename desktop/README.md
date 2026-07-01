# Token Router Desktop (Tauri)

基于 [Tauri v2](https://v2.tauri.app/start/) 的桌面壳：内嵌 Gateway（`embedded` 模式）、Web UI、系统托盘。

## 前置条件

- [Rust](https://rustup.rs/)（`cargo`）
- Node.js 18+
- Windows：WebView2（Win10/11 通常已自带）

## 网络代理（下载 crates 时）

PowerShell：

```powershell
$env:HTTPS_PROXY = "http://127.0.0.1:7890"
$env:HTTP_PROXY = "http://127.0.0.1:7890"
```

## 开发

```powershell
cd desktop
npm install
npm run tauri:dev
```

或从仓库根目录：

```powershell
make tauri-dev
```

- 前端：`index.html`（Vite 开发服 `http://localhost:1420`）
- 浏览器预览：`demo.html` 或 `npm run dev` 后打开 `http://localhost:1420`
- 后端：`src-tauri/`（Rust 命令 + 托盘）

## 构建安装包

```powershell
cd desktop
npm run tauri:build
```

产物：`src-tauri/target/release/bundle/`（安装包）及 `token-router-desktop.exe`。

## 功能说明

| 能力 | 说明 |
|------|------|
| Gateway | 启动时自动 `embedded::start()`，无需单独 CLI 守护进程 |
| UI | `index.html`（Tauri）；`demo.html`（纯浏览器演示） |
| 托盘 | 右键菜单：Show / 显示、Quit / 退出；左键显示窗口 |
| 关闭窗口 | 隐藏到托盘，不退出进程（托盘 Quit 退出） |

## Tauri 命令（前端 `invoke`）

| 命令 | 说明 |
|------|------|
| `gateway_start` | 启动内嵌 Gateway，返回 base URL |
| `gateway_stop` | 停止内嵌 Gateway |
| `gateway_restart` | 重启内嵌 Gateway |
| `gateway_is_running` | 是否运行中 |
| `gateway_url` | 当前 `http://host:port` |
| `gateway_status` | `{ running, url, version }` |
| `show_main_window` | 显示主窗口 |
| `feedback_app_version` / `feedback_submit` | 意见反馈（企业微信 Webhook + Gateway 日志） |
| `ota_*` | Windows 发布版 OTA 检查、下载、安装（开发模式不启用后台检查） |

## OTA 发布（Windows）

1. 创建 ModelScope 数据集（一次性）：

```powershell
$env:MODELSCOPE_TOKEN = "<your-token>"
uv run --with modelscope python desktop/cmd/publish_ota/init_dataset.py
```

2. 构建 release 并复制为版本化 exe 名称，例如 `Token-Router-v0.1.0-flowy-CN-with_account.exe`。

3. 上传 exe 与 `latest.json`：

```powershell
uv run --with modelscope python desktop/cmd/publish_ota/publish.py `
  --channel flowy --region-scope CN --version v0.1.0 `
  --enable-account-system true `
  --exe-path "path\to\Token-Router-v0.1.0-flowy-CN-with_account.exe"
```

更新说明维护在仓库根目录 [`docs/ota-release-notes.json`](../docs/ota-release-notes.json)。

Manifest URL 示例：

`https://modelscope.cn/datasets/flowy2025/token_router_versions/resolve/master/CN/flowy/with_account/latest.json`

```
desktop/
  index.html           # Tauri 主界面
  demo.html            # 浏览器演示（无 Tauri）
  package.json
  vite.config.ts
  dist/                # Vite 构建产物
  src-tauri/           # Tauri Rust 后端
```
