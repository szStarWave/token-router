# 会话轮次的消息记录

> 状态：已实现（2026-11-24）
> 范围：`v2/crates/{contracts,agent,control}` + 桌面端活动页。未改内核（`core/decide/proto` diff = 0）。

## 1. 问题

活动页能看到会话与每轮请求的元信息（模型、耗时、token、成本、判官调用），但看不到**每轮到底发了什么、
模型回了什么**。账本、判官审计、`RequestDetail` 都刻意不存消息正文（§3.1），所以没有任何地方能读。

## 2. 记录口径

| 数据 | 落点 | 关联键 |
|---|---|---|
| 每轮输入消息 | `request_transcripts.input_json` | `request_id` → 账本 |
| 每轮模型回复 | `request_transcripts.output_json` | 同上 |

- **只存这一轮的完整输入 + 输出**，各自是 `TranscriptMessage[]`。输入含 `system` 块（合并在最前）与
  `messages`；不存 tools 定义、采样参数（它们在决策快照/候选里，不属于"消息"）。
- **展平文本**：`ContentBlock` → 文本；`Image`/`Audio` 只留 `[图片]`/`[音频]`（**不存 base64**）；
  `ToolUse`/`ToolResult`/`Reasoning` 展开为 `[工具调用 …]` / `[工具结果 …]` / `[思考] …`。
- **上限**：单条 20,000 字符、单轮（input/output 各自）200,000 字符；超出截断并置 `truncated`。
- **流式**：`forward_stream` 累积 `ContentDelta` 与 `ToolCallDelta`，流结束（Done/失败/取消）后落库；
  后端重启期间的请求不受影响，只影响那一轮的完整度（与账本一致）。
- **失败轮次**：照记输入、输出为空，便于定位"这一轮发的是什么"。
- **写库失败不影响请求**：与账本提交分开，失败只打日志。

## 3. 开关与隐私

- 配置 `[recording] transcript = true | false`（默认 `true`）。关闭后新请求不再写；**历史记录保留**。
  桌面端入口：设置 → 存储与安全 → 「记录消息正文」（写入配置并自动保存）。
- 消息只存在本机 SQLite（`<数据目录>/token-router-v2.db`），不进账本、不进判官审计、不进日志；
  管理 API 只在显式请求 `/v2/admin/requests/{id}/transcript` 时返回。
- 暂不自动清理：`request_transcripts` 与账本同寿命（每轮一行，单轮有上限）；如需按天裁剪，
  后续在配置里加保留期，不在本功能里默认删数据。
- 与 §3.1 的口径不冲突：**账本与决策快照仍不含正文**，正文是单独表、单独端点、可关闭。

## 4. 接口

| 端点 | 说明 |
|---|---|
| `GET /v2/admin/requests/{id}/transcript` | `{request_id, session_key, created_at, input[], output[], truncated, recorded}`；无记录 → `200 + recorded:false` |

DTO 见契约 §4；`recorded:false` 同时覆盖"关闭记录"与"早于该能力的请求"，UI 用同一句空状态。

## 5. 界面

- 单请求详情（会话页点「请求」行）新增「消息」段：输入按 `系统/用户/助手/工具` 分组显示，
  回复在下方；长文本在段内滚动，不撑破面板；截断时显示「已截断」胶囊（`title` 说明上限）。
- 活动 → 会话：点某一轮的「请求」行 → 详情 → 消息，即可逐轮读会话。

## 6. 测试

- `store::transcript::*`：秒/毫秒换算、缺失返回 `None`、同 request 覆盖。
- `transcript::*`：system 置顶、媒体占位符不含 base64、单条截断标记、流式工具调用展开、空轮不产生消息。
- `pipeline::success_records_transcript_turn` / `stream_records_transcript_turn`：真实 pipeline 写库
  （含流式后台任务时序）。
- `store::providers::migration_6_applied` 与 `tests/store.rs` 的 schema 版本断言更新（当前 16）。
