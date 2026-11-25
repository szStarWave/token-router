# 决策模型调用与完整会话调用

> 状态：已实现（2026-11-23）
> 范围：`v2/crates/{contracts,agent,control}` + 桌面端活动页。未改内核（`core/decide/proto` diff = 0）。

## 1. 问题

- **决策模型（判官）调用** 早已写进 `judge_calls` 审计表，但 `request_id` 是当时新生成的随机 UUID，
  与账本行对不上：查不到「这次请求的判官花了多少、失败在哪」，也无法按会话汇总。
- **会话调用** 只有账本里的逐请求记录；活动页「按会话」是前端把最近 200 条请求分组，既看不到
  更早的会话，也看不到判官调用。

## 2. 记录口径

| 数据 | 落点 | 关联键 |
|---|---|---|
| 客户端请求 | `ledger`（已有） | `request_id`（主键之一次序键）、`session_key` |
| 判官调用 | `judge_calls`（已有表） | `request_id` → 账本 `request_id`（修复后同一 id） |

修复方式：`request_id` 在 `Pipeline::prepare` **开头**生成，贯穿 `Prepared` → 判官/组判断 →
`build_ledger`，三处写的是同一个 id。判官审计以结构化字段为主（模型、耗时、token、结果类别、
失败码）。

### 2.1 判官/JEV 的详细调用（migration 16）

`judge_calls` 另存三列，覆盖组判断（JEV / LLM）与全局判官：

| 列 | 内容 |
|---|---|
| `request_json` | JEV：`{task, prompt, choices}`；LLM：`{messages: [{role, content}]}`——与上游收到的 messages 同一构造（`tr_decide::llm::build_messages`），含 system 规则与 TASK/QUESTION/ALLOWED VALUES |
| `response_json` | `{kind, value, confidence, reason}`（`kind ∈ bool/choice/score/uncertain`；理由截断 2,000 字符） |
| `failure_message` | 失败码之外的错误消息（如 `Timeout: …`） |

只存摘要，不存完整 HTTP 请求体；判官上下文本身只含最后一条 user 消息（JEV 只发 `state.task`）。
`effective_source` 区分 `external_jev / external_llm / fallback / heuristic / disabled`，
界面用「JEV / LLM / 回退 / 启发式 / 未调用」标注。

## 3. 接口（契约只增）

| 端点 | 说明 |
|---|---|
| `GET /v2/admin/requests/{id}` | 新增 `judge_calls: JudgeCallSummary[]`：这次请求触发的全部判官调用 |
| `GET /v2/admin/judge/calls?request_id=&session=&from=&to=&limit=` | 判官调用审计；`session` 走账本 join |
| `GET /v2/admin/sessions?from&to&limit` | 会话级完整汇总：请求数/失败数/tokens/成本/判官调用数、失败数与成本/涉及模型/首末时间 |

会话汇总在服务端按 `session_key` 聚合，不受请求列表分页影响；判官调用按 `request_id` join 账本
归到会话。列表字段 `SessionSummary.tokens/cost_micros` 复用既有 `TokenUsage/CostUsage`。

口径两处容易算错，已统一：

- **失败**：`outcome` 不属于 `answered / selected / no_match` 才算没拿到结论（`unavailable`、
  `failed`、`budget_exceeded`）。`summary.judge.failed` 与会话的 `judge_failures` 用同一口径
  （之前只数 `failed`，超时类的 `unavailable` 全部漏计）。
- **成本**：账本 `routing_cost_micros` 已包含判官成本，`judge_cost_micros` 只是单列出来便于观测，
  会话界面的总成本 = `answer + routing`，判官成本以「其中判官 $x」呈现，不重复相加。

## 4. 顺带修掉的游标 bug

`/v2/admin/requests` 的游标由 control 层用「毫秒 created_at + request_id 哈希」拼，而 SQL 比较的是
「秒 created_at + 账本行 id」——单位与键都对不上，带 `cursor` 的第二页会重复第一页。
现在游标在 store 里生成（`list_requests_page`），键为 `(created_at 秒, 账本 id)`，与 `ORDER BY` 同口径；
只有取满一页才给游标。回归测试 `cursor_paging_does_not_repeat_rows`。

## 5. 界面

- 活动页新增一级子标签「会话」（`/sessions`）：**左右分栏**——左侧会话列表独立滚动（窄窗口退化为纵向），
  右侧固定显示选中会话的详情，不会因为会话多而把详情挤到列表底部。列表按 `session_key` 一行一个会话，
  显示请求数/失败数/判官调用与失败/最后时间/token/成本/涉及模型；时间范围可选今天/昨天/近 7 天/全部
  （默认全部，窗口上限 89 天、最多 200 个会话）。
- 点会话 → 会话面板：把该会话的请求按游标翻到底，并叠加判官调用，按时间倒序合并成一张表
  （类型：请求 / 决策）；请求行可继续点开单请求详情。这是查看「所有模型调用的完整会话记录」的固定入口。
- 活动页「路由」子页只保留路由视图（路由图/指标/叙事/上下文/配额），**不再重复摆请求列表**：
  请求级明细统一在「会话」里看，避免同一事实写两遍。
- 会话表里点「请求」行 → 单请求详情（路由时间线 / 消息 / 尝试，区块带边框）；策略贡献不再单独
  摆一份裸 JSON——「路由时间线」里已有可读的「策略（N 条贡献）」胶囊，原始 JSON 仍可通过
  `GET /v2/admin/requests/{id}` 的 `contributions` 取（AX/调试用）。
- 点「决策」行 → 决策模型调用面板：LLM 判官显示实际发出的 system/user messages，
  JEV 显示任务 + 提示词 + 选项；后面跟答案（含置信度）与失败信息。
  点「决策」行 → **决策模型调用面板**：这次调用发送的任务、prompt、选项、答案（含置信度）与失败信息。
  判官调用表带来源胶囊（JEV / LLM / 回退 / 启发式），行内也可展开同一份详情。

## 6. 测试

- `request_detail_reads_columns_by_name`：详情按列名读取 + `judge_calls` 关联。
- `sessions_aggregate_includes_judge_calls`：会话聚合含判官调用数/成本与涉及模型。
- `judge_calls_filter_by_request_and_session`：两种过滤与毫秒换算；断言 `request/response` 详情
  （输入摘要与答案）随 `JudgeCallSummary` 返回。
- `judge_detail::*`：选项展开、答案扁平化（choice/uncertain）、任务截断。
- `cursor_paging_does_not_repeat_rows`：游标翻页不重复。
- 判官单测断言 `outcome.calls[0].request_id` 等于传入的请求 id。
