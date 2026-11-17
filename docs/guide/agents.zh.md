# Agent 接入与 WSL

> 事实基准：`v2/crates/onboard/`（2026-11-17）。桌面端入口：Onboarding 页（Overview 页也有一键配置入口）。

## 支持的 agent（8 个）

| Agent | 配置文件 |
|---|---|
| OpenClaw | `~/.openclaw/openclaw.json` |
| Hermes | `~/.hermes/config.yaml` |
| Hermes Flash | `~/.hermes-flash/config.yaml` |
| Claude Code | `~/.claude/settings.json` |
| Codex | `~/.codex/config.toml` + `auth.json` |
| OpenCode | `~/.config/opencode/opencode.json` |
| CodeBuddy | `~/.codebuddy/models.json` |
| WorkBuddy | `~/.workbuddy/models.json` |

扩充到 ~20 个在路线图 P5-D。

## 接入流程：detect → plan → apply → verify

1. **detect**：扫描本机已安装的 agent 与现有配置。
2. **plan**：生成配置改写计划，UI 展示 diff 预览（脱敏，token 只出现 `env:`/`file:` 引用）。
3. **apply**：确认后精确合并——
   - 幂等：重复执行不产生重复条目；
   - 保留用户已有配置与 TOML 注释（Hermes/Codex/Claude Code 的合并语义由 16 个 golden 测试固定）；
   - 写入前生成 `.bak-<timestamp>` 备份，临时文件 + rename 原子写；
   - 路径逃逸校验，不越界写文件。
4. **verify**：接入后自动经网关发真实请求验证连通。

合并语义只改模型入口相关字段，其余配置原样保留。

## WSL（仅 Windows 宿主）

- `wsl -l -v` 枚举发行版（UTF-16LE 容错解码）；发行版名白名单校验（`^[A-Za-z0-9][A-Za-z0-9._-]*$`）。
- home 目录走 UNC：先 `\\wsl.localhost`，回退 `\\wsl$`。
- WSL2 NAT 网络下，agent 配置里的网关地址用 `ip route show default` 取宿主网关 IP；失败回退 `127.0.0.1`。

## 手动接入

不走向导时，把 agent 的 base URL 指向网关、模型名填虚拟模型 id：

- OpenAI 兼容 agent：`http://127.0.0.1:16621/v1`（桌面端托管时为 `17621`），模型如 `auto`。
- Anthropic 协议 agent：同一地址的 `/v1/messages`。
- 网关开启入站鉴权（`auth_enabled`）时带上 `inbound_key`。

协议端点清单见 [架构 §3](../architecture.zh.md)。