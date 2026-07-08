# 配置示例

本目录提供可复制到应用 home 目录的 TOML 模板。完整字段说明见 [README.md §6](../README.md#6-配置字段说明)。

## 文件一览

| 文件 | 场景 | 要点 |
|------|------|------|
| [config.toml](./config.toml) | **推荐默认** | Ollama + DeepSeek，`route = auto`，级联 + 经验学习 + 自适应路由（注释说明） |
| [config.edge-only.toml](./config.edge-only.toml) | 全端侧 / 离线 | `route = edge`，仅 `[upstream.edge]` |
| [config.minimal.toml](./config.minimal.toml) | 最小可启动 | 无上游；聊天前须补 `[upstream.edge]` 或 `[upstream.cloud]` |
| [config.economy.toml](./config.economy.toml) | 提高端侧占比 | `economy` profile + 略高的 work 校验基线，适合已稳定运行的 OpenClaw |

## 快速使用

**复制为默认配置**

```bash
mkdir -p ~/.token-router
cp example/config.toml ~/.token-router/config.toml
# 编辑 upstream.base_url、api_key（均为选填）
token-router gateway restart
```

**使用自定义 home 目录**

```bash
mkdir -p /tmp/token-router-dev
cp example/config.toml /tmp/token-router-dev/
token-router --home /tmp/token-router-dev gateway start
make start HOME=/tmp/token-router-dev PORT=11080
```

## 运行后可观测

Gateway 启动后会在应用 home 目录（默认 `~/.token-router/`）写入（或更新）：

| 文件 | 内容 |
|------|------|
| `stats.json` | 路由决策、Token、延迟等累计统计 |
| `experience.json` | 按 `step_kind` 的经验偏置与 `edge_trusted` |
| `sessions/` | 每会话路由状态 |

```bash
token-router stats --lang zh              # 当前会话（中文）
token-router stats --global --lang zh     # 全部历史
make stats-zh
make stats-global-zh HOME=/tmp/token-router-dev
```

`stats` 输出中的 **经验学习**、**自适应路由（运行时）** 分区可查看学习偏置与生效中的校验率 / θ 阈值。

## 与 Agent 对接

OpenClaw / Hermes 的 provider `baseUrl` 须指向：

```text
http://<gateway.listen>/v1
```

示例中默认为 `http://127.0.0.1:11080/v1`。详见 [README §3.5–3.6](../README.md#35-接入-openclaw)。

## 调参提示

| 目标 | 建议 |
|------|------|
| 更多走端侧 | `default_profile = "economy"` 或参考 [config.economy.toml](./config.economy.toml)；保持 `adaptive_routing_enabled = true` |
| OpenClaw 日常仍升云 | `step_kind` 应为 `direct_chat`/`heartbeat_ack`；路由可为 **edge 或 cascade**（非强制端侧）；大 system/tools 不计入 casual 预算 |
| 复杂任务更稳 | 提高 `work_verify_sample_rate`（如 0.2）；或 `default_profile = "premium"` |
| 关闭运行时微调 | `adaptive_routing_enabled = false`（仍保留经验学习） |
| 固定路由 | `route = edge` / `cloud` / `cascade`（见 [config.edge-only.toml](./config.edge-only.toml)） |
