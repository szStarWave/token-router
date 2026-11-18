# 快速开始

> 以 `v2/` 代码为准（2026-11-17）。平台：Windows 为主（打包仅 NSIS），macOS/Linux 可从源码运行但未演练。

## 1. 两种运行方式

| 方式 | 适合 | 监听地址 |
|---|---|---|
| 桌面端（Tauri） | 日常使用，supervisor 自动拉起网关 | 固定 `127.0.0.1:17621` |
| 独立服务端 | 开发/无桌面环境 | 配置决定，默认 `127.0.0.1:16621` |

两者是同一个网关二进制（`token-router-v2`），桌面端只是把它作为 sidecar 托管。

## 2. 从源码构建

需要 Rust 1.85（`v2/rust-toolchain.toml` 会自动选择）：

```bash
cd v2
cargo build -p tr-server --release
# 产物：v2/target/release/token-router-v2(.exe)
```

## 3. 准备配置

复制最小示例并修改：

```bash
cp v2/example/config.minimal.toml my-config.toml
```

```toml
schema_version = 1
default_virtual_model = "auto"

[server]
listen = "127.0.0.1:16621"

[profiles.default]
mode = "cascade"
strength_order = ["local", "cloud"]

[[candidates]]
id = "local-fast"
tier = "local"
strength = 20
provider = "openai"
base_url = "http://127.0.0.1:11434/v1"   # 例：本机 Ollama
model = "qwen2.5:14b"
window = 32768
input = ["text"]

[[candidates]]
id = "cloud-strong"
tier = "cloud"
strength = 80
provider = "openai"
base_url = "https://api.example.com/v1"
model = "example-strong"
credential = "env:EXAMPLE_API_KEY"        # 只写引用，不写明文 key
window = 128000
input = ["text"]

[[virtual_models]]
provider = "router"
id = "auto"
name = "Auto"
profile = "default"
candidate_pool = ["local-fast", "cloud-strong"]
```

字段详解见 [配置参考](./configuration.zh.md)。key 通过环境变量注入（`env:NAME`）或文件引用（`file:path`），明文不落配置。

## 4. 启动与验证

```bash
export EXAMPLE_API_KEY=sk-...
./v2/target/release/token-router-v2 --config my-config.toml
# 可选：--db <path>（默认与配置同目录 token-router-v2.db）、--listen <addr>
```

首次启动会在配置同目录生成 `admin.token`（全 scope bootstrap token，管理面鉴权用）。

```bash
curl http://127.0.0.1:16621/health
curl http://127.0.0.1:16621/v1/models

# 发一个走路由的请求（model 填虚拟模型 id）
curl http://127.0.0.1:16621/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{"model":"auto","messages":[{"role":"user","content":"hello"}]}'
```

CLI 管理（注意 CLI 默认 base-url 是桌面端的 17621，独立服务端要显式指定）：

```bash
tr-cli admin status --base-url http://127.0.0.1:16621
tr-cli admin requests --base-url http://127.0.0.1:16621
```

token 来源：`--token` / 环境变量 `TR_ADMIN_TOKEN` / `~/.token-router-v2/admin.token`。

## 5. 接入 agent

把 agent 的模型入口指向网关（`http://127.0.0.1:16621/v1`，模型名填虚拟模型 id 如 `auto`）。桌面端 Onboarding 页可自动完成发现→预览→合并→验证，支持 8 个 agent 与 WSL，见 [Agent 接入](./agents.zh.md)。

## 6. 桌面端

安装包发布后可直接使用（签名/公证为 GA 前置项，见 [发布文档](../dev/release.zh.md)）。从源码运行：

```bash
cd v2
cargo build -p tr-server                      # sidecar 需要二进制
node apps/desktop/scripts/prepare-sidecar.mjs # 复制为 per-triple sidecar
cd apps/desktop/src-tauri && cargo tauri dev  # 需要 tauri-cli
```

桌面端 supervisor 固定使用 `127.0.0.1:17621`，数据目录升级不覆盖。页面说明见 [桌面端指南](./desktop.zh.md)。

## 下一步

- 配置调优（级联/判官）：[配置参考](./configuration.zh.md)
- 理解路由行为：[架构精编](../architecture.zh.md)
- 管理 API：[契约](../contracts/admin-api.zh.md)