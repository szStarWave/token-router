# 模型元数据自动补全（models.dev）

> 状态：已实现（2026-11-23）
> 范围：`v2/crates/{contracts,agent,control}` + 桌面端供应商页。未改动内核语义（`core/decide/proto` diff = 0）。

## 1. 问题

模型的价格、上下文窗口、最大输出、推理档位目前要用户在供应商页手工填写。这些数据在
[models.dev](https://models.dev) 的公开目录里已经存在（`https://models.dev/api.json`，约 7MB，
覆盖 300+ 供应商），重复填写既费时也容易错。

## 2. 数据映射

| models.dev 字段 | 我们的字段 | 说明 |
|---|---|---|
| `limit.context` | `catalog_models.context_window` | 上下文窗口 |
| `limit.output` | `catalog_models.max_output` | 最大输出 |
| `cost.input/output/cache_read/cache_write` | `catalog_models.price_json` | 单位 $/百万 token → micros；转成 `PriceDto` |
| `reasoning_options[]`（`type = effort`） | `catalog_models.thinking_levels_json` | 取 `values`，归一为 low / medium / high / xhigh / max；`minimal`→`low`，`none` 丢弃；`toggle` / `budget_tokens` 不产生离散档位 |

推理档位在编译候选时作为默认值写入 `[[candidates]].thinking_levels`；候选上已有的显式档位优先
（用户改过的不会被目录覆盖）。内核只在"候选声明了档位"时按声明收口 reasoning，语义不变。

## 3. 行为

- **只填空**：窗口 / 输出 / 价格 / 档位只在该字段为空时写入，绝不覆盖用户值；
- **供应商匹配**：`provider.id` 前缀（`siliconflow-bccdda58` → `siliconflow`，最长匹配）→
  展示名忽略大小写 → `base_url` host 与 models.dev `api` 同源；匹配不到返回 0，前端提示手工填写；
- **缓存**：`api.json` 落在数据目录 `modelsdev.json`，24h TTL；失效才重新下载；
- **失败不影响目录**：网络失败只记录日志，模型列表与路由照常；
- **来源标记**：实际补全过的模型 `source = "models.dev"`（UI 明细行显示来源）。

## 4. 入口

| 入口 | 说明 |
|---|---|
| 供应商页「同步模型信息」 | `POST /v2/admin/providers/{id}/catalog/enrich`（write 权限）；返回 `{source_provider, matched, filled}`，成功后自动重编译候选 |
| 「刷新目录」 | 刷新完成后**尽力**补全一次（`enrich_after_refresh`），失败只记日志 |

补全后立即 `compile_candidates`：窗口/价格/档位进入候选，热加载生效，不需要重启。

## 4.1 供应商模板列表

「添加供应商」的模板列表同样来自 models.dev：

- 内置模板（常用厂商，含中文名与目录端点）始终排在前面；
- models.dev 里 `api` 非空、且 `npm` 为 `@ai-sdk/openai` / `@ai-sdk/openai-compatible` /
  `@ai-sdk/anthropic` 的供应商追加在后（Google、Bedrock 等内核不支持的协议过滤掉）；
- 按 id 与 `base_url` host 去重，避免与内置模板重复；
- 目录端点按约定推导：OpenAI 兼容 `{base}/models`，Anthropic `{base}/v1/models`
  （与 probe 的默认回退一致）；
- `GET /v2/admin/provider-templates` 对 models.dev 最多等 12s；超时或失败只返回内置模板，
  添加供应商流程不被网络阻塞。

合并后当前约 300 个模板（内置 + models.dev 长尾），前端按名称 / id / base_url / 协议搜索过滤。

## 4.2 非对话模型过滤

目录里混入语音（TTS/ASR/实时语音）和图像/视频生成模型会让模型选择变得嘈杂，且它们无法走
文本对话路由。过滤分两层：

1. **命名规则**（离线兜底）：embedding / rerank 词根，以及音频（`cosyvoice`、`sensevoice`、
   `whisper`、`tts`、`audio`、`realtime`…）、图像/视频生成（`image`、`kolors`、`flux`、`sdxl`、
   `seedream`、`veo-`、`t2v`、`video`…）；
2. **models.dev 元数据**：`type` 为 image / embedding / video / transcription / speech /
   reranking / realtime，或 `modalities.output` 不含 `text`、`modalities.input` 不含 `text`
   （纯音频/图像输入）。

生效点：
- `刷新目录`：上游列表先过两层过滤再写库；
- 补全 / 刷新后：已有行里判定为非对话模型的，置 `hidden = 1` 并取消勾选；
- 前端默认不显示 hidden，用户可在「显示已移除」里恢复（误判有出路）。

## 5. 存储

- migration 14：`catalog_models.thinking_levels_json TEXT`；
- 目录刷新（`upsert_catalog_models`）保留已填充的档位（与窗口/价格/隐藏标记同一套 preserved 逻辑）；
- 契约变更：`CatalogModel.thinking_levels: string[]`（默认空数组，旧客户端忽略即可）。

## 6. 测试

- `modelsdev` 单元测试：供应商匹配（前缀 / host 回退）、档位归一化与排序、toggle/budget 不产生档位、价格换算；
- `store::providers::thinking_levels_persist_and_survive_refresh`：落库与刷新保留；
- `provider_service::compile_uses_catalog_thinking_levels_as_default`：档位进入候选默认值。

## 7. 后续可扩展

- `attachment` / `modalities.input` → 目录级"支持图片输入"（当前候选级，需新增目录列）；
- `reasoning_options` 的 `budget_tokens` → 数值预算档位（需要内核/上游支持预算参数）；
- 定时全量刷新 + 差异提示（新增模型 / 下架模型）。
