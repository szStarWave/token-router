/**
 * 模型设置（供应商编辑器内联面板，对齐 Magpie 的「模型设置」）。
 *
 * 位置：供应商编辑器的「模型」分区内，模型 chips 之下（不是单独弹窗）。
 * 形态：**每个模型一张设置卡**，卡内字段：
 *   显示名 / 上下文窗口 / 最大输出 /
 *   价格（$/百万 token）/ 决策模型 / 支持图片输入 / 协议模式 / 推理档位 /
 *   请求头 / 用途与能力标签。
 *
 * 两类字段，两个落点：
 * - **目录元数据**（上下文窗口 / 最大输出 / 价格 / 用途 / 能力标签）：
 *   服务端 catalog_models（context_window / max_output / price_json 列已存在），
 *   通过 catalog/select 接口即时保存；编译时写进候选。
 * - **候选字段**（输入模态 / 推理档位 / 请求头）：编译后的候选在
 *   配置草稿里，改动进全局草稿并自动保存；没有候选时
 *   可以「生成候选」把它建出来。
 */
import { useState } from 'react'
import type { CatalogModel, ModelPurposeDto, PriceDto, ProviderInfo } from '../../api/types'
import { useSelectCatalogModelMutation } from '../../queries/admin'
import { ConfigSaveBar } from '../../components/config/ConfigSaveBar'
import { ModelSwitcher } from './ModelSwitcher'
import { sanitizeCandidatePart } from '../../lib/candidate-source'
import { useConfigDraft } from '../../hooks/useConfigDraft'
import type { CandidateDraft, HeaderDraft } from '../../lib/config-edit'
import { errorMessageText } from './shared'

/** 推理档位：配置里是自由字符串，按客户端常见 effort 取值给出 low / medium / high / max。 */
const THINKING_LEVELS: Array<{ value: string; label: string }> = [
  { value: 'low', label: '低' },
  { value: 'medium', label: '中' },
  { value: 'high', label: '高' },
  { value: 'xhigh', label: '很高' },
  { value: 'max', label: '最高' },
]

/** 协议模式：写候选 `provider`（上游按此选 wire 协议）；与供应商类型一致时显示「自动」。 */
const PROTOCOL_MODES: Array<{ value: string; label: string }> = [
  { value: 'auto', label: '自动' },
  { value: 'openai', label: 'OpenAI' },
  { value: 'responses', label: 'Responses' },
  { value: 'anthropic', label: 'Anthropic' },
]

/** micros → $（每百万 token）。 */
function microsToDollars(micros: number | null | undefined): string {
  if (micros === null || micros === undefined) return ''
  return (micros / 1_000_000).toFixed(6).replace(/0+$/, '').replace(/\.$/, '')
}

/** $（每百万 token）→ micros。 */
function dollarsToMicros(text: string): number | null {
  const trimmed = text.trim()
  if (trimmed === '') return null
  const value = Number(trimmed)
  if (!Number.isFinite(value) || value < 0) return null
  return Math.round(value * 1_000_000)
}

/** 供应商类型 → 候选的 protocol 文本（内核认 openai / anthropic / responses）。 */
function protocolText(kind: ProviderInfo['kind']): string {
  switch (kind) {
    case 'anthropic':
      return 'anthropic'
    case 'responses':
      return 'responses'
    default:
      return 'openai'
  }
}

export function ModelSettingsPanel({
  provider,
  models,
  focusedId,
  onFocus,
  onCollapse,
  defaultCredential,
}: {
  provider: ProviderInfo
  models: CatalogModel[]
  /** 当前要配置的模型；为空时默认第一个。 */
  focusedId: string | null
  onFocus: (modelId: string) => void
  onCollapse: () => void
  /** 新建候选时使用的凭据引用（首个启用账号）。 */
  defaultCredential?: string
}) {
  const { draft, update } = useConfigDraft()
  // 已勾选的模型排前面（与上方 chips 一致），未勾选按目录顺序。
  const orderedModels = [
    ...models.filter((model) => model.selected),
    ...models.filter((model) => !model.selected),
  ]
  const focused = orderedModels.find((model) => model.model_id === focusedId) ?? orderedModels[0] ?? null
  const focusedIndex = focused
    ? orderedModels.findIndex((model) => model.model_id === focused.model_id)
    : -1

  // 候选归属按 **id 前缀** 判定：config 里的 `provider` 字段是协议
  // （openai / anthropic / responses），不是供应商 id——按它过滤会把所有编译候选漏掉，
  // 于是每次点选字段都新建一个手工候选（还会叠成 -2 / -3 …）。
  const providerPrefix = `${sanitizeCandidatePart(provider.id)}-`
  const candidatesByModel = new Map<string, CandidateDraft[]>()
  for (const candidate of draft?.candidates ?? []) {
    if (!candidate.id.startsWith(providerPrefix)) continue
    const list = candidatesByModel.get(candidate.model) ?? []
    list.push(candidate)
    candidatesByModel.set(candidate.model, list)
  }

  const patchCandidate = (candidate: CandidateDraft, patch: Partial<CandidateDraft>) => {
    if (!draft) return
    update({
      ...draft,
      candidates: draft.candidates.map((item) =>
        item.id === candidate.id ? { ...item, ...patch } : item,
      ),
    })
  }

  /**
   * 候选级字段的写入入口：已有候选就改它；还没有候选就按默认值建一个，
   * 并把这次的改动一起写进去（一步到位，不让界面处于半可用状态）。
   */
  const ensureCandidate = (model: CatalogModel, patch: Partial<CandidateDraft>): void => {
    if (!draft) return
    const existing = candidatesByModel.get(model.model_id)?.[0]
    if (existing) {
      patchCandidate(existing, patch)
      return
    }
    // 优先沿用服务端给的候选 id（`provider-model-account`，与编译产物一致），
    // 这样以后勾选该模型重新编译时不会产生第二份候选。
    const base =
      model.candidate_id ??
      `${sanitizeCandidatePart(provider.id)}-${sanitizeCandidatePart(model.model_id)}`.toLowerCase()
    let id = base
    let suffix = 2
    while (draft.candidates.some((item) => item.id === id)) {
      id = `${base}-${suffix}`
      suffix += 1
    }
    update({
      ...draft,
      candidates: [
        ...draft.candidates,
        {
          id,
          tier: 'cloud',
          strength: 50,
          provider: protocolText(provider.kind),
          base_url: provider.base_url,
          model: model.model_id,
          window: model.context_window ?? 128_000,
          credential: defaultCredential ?? '',
          proxy: '',
          extra_headers: [],
          input: [],
          thinking_levels: [],
          disable_thinking: false,
          max_tokens: null,
          caps: [],
          price: null,
          enabled: true,
          ...patch,
        },
      ],
    })
  }


  return (
    <section className="app-model-settings" aria-label="模型设置">
      <div className="flex items-center gap-2 flex-wrap">
        <h3
          className="text-ui-label font-semibold"
          title="上游模型 / 图片输入 / 推理档位 / 请求头是候选级字段：直接改会按默认值生成一个候选（进配置草稿）。"
        >
          模型设置
        </h3>
        {models.length > 0 && focused !== null && (
          <>
            <button
              type="button"
              className="oc-btn-default oc-btn-xs"
              disabled={focusedIndex <= 0}
              aria-label="上一个模型"
              onClick={() => {
                const previous = orderedModels[focusedIndex - 1]
                if (previous) onFocus(previous.model_id)
              }}
            >
              ‹
            </button>
            <ModelSwitcher
              models={orderedModels}
              value={focused.model_id}
              onChange={onFocus}
            />
            <button
              type="button"
              className="oc-btn-default oc-btn-xs"
              disabled={focusedIndex >= models.length - 1}
              aria-label="下一个模型"
              onClick={() => {
                const next = orderedModels[focusedIndex + 1]
                if (next) onFocus(next.model_id)
              }}
            >
              ›
            </button>
            <span className="text-meta text-muted-foreground">
              {focusedIndex + 1} / {orderedModels.length}
            </span>
          </>
        )}
        <button type="button" className="oc-btn-default oc-btn-sm ml-auto" onClick={onCollapse}>
          收起
        </button>
      </div>

      {orderedModels.length === 0 ? (
        <p className="text-meta text-muted-foreground">目录里还没有模型，先刷新目录或手工添加。</p>
      ) : focused === null ? (
        <p className="text-meta text-muted-foreground">选择一个模型开始配置</p>
      ) : (
        <ModelSettingsCard
          key={focused.model_id}
          provider={provider}
          model={focused}
          candidate={candidatesByModel.get(focused.model_id)?.[0] ?? null}
          siblingCount={candidatesByModel.get(focused.model_id)?.length ?? 0}
          ensureCandidate={ensureCandidate}
        />
      )}

      <ConfigSaveBar />
    </section>
  )
}

function ModelSettingsCard({
  provider,
  model,
  candidate,
  siblingCount,
  ensureCandidate,
}: {
  provider: ProviderInfo
  model: CatalogModel
  candidate: CandidateDraft | null
  siblingCount: number
  ensureCandidate: (model: CatalogModel, patch: Partial<CandidateDraft>) => void
}) {
  const select = useSelectCatalogModelMutation()
  const [headerDraft, setHeaderDraft] = useState<HeaderDraft[]>(
    candidate?.extra_headers ?? [],
  )

  /** 目录元数据保存：用途 / 窗口 / 最大输出 / 价格（即时生效）。 */
  const saveCatalog = (patch: {
    purpose?: ModelPurposeDto
    context_window?: number
    max_output?: number
    price?: PriceDto
  }) => {
    select.mutate({
      providerId: provider.id,
      request: {
        model_id: model.model_id,
        selected: model.selected,
        ...(patch.purpose === undefined ? {} : { purpose: patch.purpose }),
        ...(patch.context_window === undefined
          ? {}
          : { context_window: patch.context_window }),
        ...(patch.max_output === undefined ? {} : { max_output: patch.max_output }),
        ...(patch.price === undefined ? {} : { price: patch.price }),
      },
    })
  }

  const priceOf = (key: keyof PriceDto): number | null => {
    const price = model.price
    if (!price) return null
    const value = price[key]
    return typeof value === 'number' ? value : null
  }

  const savePrice = (key: keyof PriceDto, text: string) => {
    const current: PriceDto = model.price ?? {
      currency: 'USD',
      input_micros_per_mtok: 0,
      output_micros_per_mtok: 0,
      cache_read_micros_per_mtok: null,
      cache_write_micros_per_mtok: null,
      request_micros: 0,
    }
    const micros = dollarsToMicros(text)
    const next: PriceDto = { ...current, [key]: micros } as PriceDto
    saveCatalog({ price: next })
  }

  const toggleImage = () => {
    const current = candidate?.input ?? []
    const inputs = current.includes('image')
      ? current.filter((item) => item !== 'image')
      : [...current, 'image']
    ensureCandidate(model, { input: inputs })
  }

  // 候选没有显式档位时回落到目录默认值（models.dev 自动派生），勾选即覆盖。
  const thinkingLevels = candidate?.thinking_levels ?? model.thinking_levels

  const toggleThinking = (level: string) => {
    const current = thinkingLevels
    const levels = current.includes(level)
      ? current.filter((item) => item !== level)
      : [...current, level]
    ensureCandidate(model, { thinking_levels: levels })
  }

  const addHeader = () => {
    setHeaderDraft([...headerDraft, { name: '', value: '' }])
  }

  const commitHeaders = () => {
    const cleaned = headerDraft
      .filter((header) => header.name.trim() !== '')
      .map((header) => ({ name: header.name.trim(), value: header.value }))
    ensureCandidate(model, { extra_headers: cleaned })
  }

  // 协议模式：候选的 provider 字段（openai / responses / anthropic）；
  // 与供应商类型一致时显示「自动」。
  const currentProtocol = candidate?.provider ?? protocolText(provider.kind)
  const protocolMode =
    currentProtocol === protocolText(provider.kind) ? 'auto' : currentProtocol

  return (
    <div className="app-model-settings-card">
      {/* 模型名 */}
      <label className="flex items-center gap-3">
        <span className="w-24 shrink-0 text-meta text-muted-foreground">模型名</span>
        <input
          className="oc-field app-field-light h-8 min-w-0 flex-1 font-mono text-meta"
          value={candidate?.id ?? model.display_name ?? model.model_id}
          readOnly
          aria-label={`${model.model_id} 的显示名`}
          title="候选 id 就是 Agent 侧看到的模型名；改它会打断路由组引用，因此只读"
        />
      </label>

      {/* 窗口 / 输出 */}
      <div className="grid grid-cols-2 gap-x-6 gap-y-2">
        <label className="flex min-w-0 items-center gap-3">
          <span className="w-24 shrink-0 text-meta text-muted-foreground">上下文窗口</span>
          <input
            type="number"
            min={0}
            step={1024}
            className="oc-field app-field-light h-8 min-w-0 flex-1 font-mono"
            value={model.context_window ?? ''}
            placeholder="未设置"
            aria-label={`${model.model_id} 的上下文窗口`}
            onChange={(event) =>
              saveCatalog({ context_window: Math.max(0, Number(event.target.value) || 0) })
            }
          />
        </label>
        <label className="flex min-w-0 items-center gap-3">
          <span
            className="w-24 shrink-0 text-meta text-muted-foreground"
            title="编译为候选的输出上限：请求 max_tokens 取 min(客户端值, 上限)，未指定时填上限；被截断时账本记 OUTPUT_CLAMPED"
          >
            最大输出
          </span>
          <input
            type="number"
            min={0}
            step={1024}
            className="oc-field app-field-light h-8 min-w-0 flex-1 font-mono"
            value={model.max_output ?? ''}
            placeholder="未设置"
            aria-label={`${model.model_id} 的最大输出`}
            onChange={(event) =>
              saveCatalog({ max_output: Math.max(0, Number(event.target.value) || 0) })
            }
          />
        </label>
      </div>

      {/* 价格（$/百万 token） */}
      <div className="flex flex-col gap-1.5">
        <span className="text-meta text-muted-foreground">价格（$/百万 token）</span>
        <div className="grid grid-cols-2 gap-x-6 gap-y-2">
          {(
            [
              ['input_micros_per_mtok', '输入'],
              ['output_micros_per_mtok', '输出'],
              ['cache_read_micros_per_mtok', '缓存读取'],
              ['cache_write_micros_per_mtok', '缓存写入'],
            ] as Array<[keyof PriceDto, string]>
          ).map(([key, label]) => (
            <label key={key} className="flex min-w-0 items-center gap-3">
              <span className="w-24 shrink-0 text-meta text-muted-foreground">{label}</span>
              <input
                type="number"
                min={0}
                step="0.001"
                className="oc-field app-field-light h-8 min-w-0 flex-1 font-mono"
                value={microsToDollars(priceOf(key))}
                placeholder="—"
                aria-label={`${model.model_id} 价格 ${label}`}
                onChange={(event) => savePrice(key, event.target.value)}
              />
            </label>
          ))}
        </div>
      </div>

      {/* 决策模型（SystemOne） · 图片输入 · 协议模式 */}
      <div className="flex items-center gap-2 flex-wrap">
        <label
          className="app-confirm"
          title="勾选后按 SystemOne 决策模型编译（judge:jev，POST /v1/systemone，不进入对话候选池）；对话式判官请在模型列表的「用途」里选「判断 LLM」"
        >
          <input
            type="checkbox"
            className="accent-primary"
            checked={model.purpose === 'jev'}
            aria-label={`${model.model_id} 是决策模型`}
            onChange={(event) =>
              saveCatalog({ purpose: event.target.checked ? 'jev' : 'chat' })
            }
          />
          决策模型（SystemOne）
        </label>
        {model.purpose === 'chat' && (
          <>
            <label className="app-confirm">
              <input
                type="checkbox"
                className="accent-primary"
                checked={candidate?.input.includes('image') ?? false}
                aria-label={`${model.model_id} 支持图片输入`}
                onChange={toggleImage}
              />
              支持图片输入
            </label>
            <div className="oc-seg" role="group" aria-label="协议模式">
              {PROTOCOL_MODES.map((mode) => (
                <button
                  key={mode.value}
                  type="button"
                  className={protocolMode === mode.value ? 'oc-seg-item-active' : 'oc-seg-item'}
                  aria-pressed={protocolMode === mode.value}
                  title={
                    mode.value === 'auto'
                      ? '跟随供应商类型'
                      : '写进候选的 provider 字段（内核按此选协议）'
                  }
                  onClick={() => {
                    if (mode.value === 'auto') {
                      ensureCandidate(model, { provider: protocolText(provider.kind) })
                      return
                    }
                    ensureCandidate(model, { provider: mode.value })
                  }}
                >
                  {mode.label}
                </button>
              ))}
            </div>
          </>
        )}
      </div>

      {/* 推理档位 · 请求头 */}
      <div className="flex items-center gap-2 flex-wrap">
        <span
          className="text-meta text-muted-foreground"
          title="默认来自模型目录（models.dev 的 reasoning_options）；勾选/取消即覆盖候选"
        >
          推理档位
        </span>
        {THINKING_LEVELS.map((level) => (
          <label key={level.value} className="app-confirm">
            <input
              type="checkbox"
              className="accent-primary"
              checked={thinkingLevels.includes(level.value)}
              aria-label={`${model.model_id} 推理档位 ${level.label}`}
              onChange={() => toggleThinking(level.value)}
            />
            {level.label}
          </label>
        ))}
        {model.purpose === 'chat' && (
          <label
            className="app-confirm"
            title="勾选后该候选的请求一律不发推理参数；已知厂商（硅基流动 / DashScope / 智谱 / OpenRouter）附带关闭开关"
          >
            <input
              type="checkbox"
              className="accent-primary"
              checked={candidate?.disable_thinking ?? false}
              aria-label={`${model.model_id} 关闭思考`}
              onChange={() =>
                ensureCandidate(model, { disable_thinking: !(candidate?.disable_thinking ?? false) })
              }
            />
            关闭思考
          </label>
        )}
        <button type="button" className="oc-btn-default oc-btn-sm" onClick={addHeader}>
          + 添加请求头
        </button>
      </div>

      {headerDraft.length > 0 && (
        <div className="flex flex-col gap-1">
          {headerDraft.map((header, index) => (
            <div key={index} className="flex items-center gap-2 flex-wrap">
              <input
                className="oc-field app-field-light font-mono h-7 min-w-32"
                value={header.name}
                placeholder="header 名"
                aria-label={`请求头名 ${index + 1}`}
                onChange={(event) =>
                  setHeaderDraft(
                    headerDraft.map((item, i) =>
                      i === index ? { ...item, name: event.target.value } : item,
                    ),
                  )
                }
              />
              <input
                className="oc-field app-field-light font-mono h-7 min-w-45"
                value={header.value}
                placeholder="值（静态非敏感）"
                aria-label={`请求头值 ${index + 1}`}
                onChange={(event) =>
                  setHeaderDraft(
                    headerDraft.map((item, i) =>
                      i === index ? { ...item, value: event.target.value } : item,
                    ),
                  )
                }
              />
              <button
                type="button"
                className="oc-btn-default oc-btn-sm"
                onClick={() => setHeaderDraft(headerDraft.filter((_, i) => i !== index))}
              >
                移除
              </button>
            </div>
          ))}
          <button
            type="button"
            className="oc-btn-default oc-btn-sm self-start"
            onClick={commitHeaders}
          >
            保存请求头
          </button>
        </div>
      )}

      {candidate === null ? null : (
        siblingCount > 1 && (
          <p className="text-micro text-muted-foreground">
            该模型在 {provider.display_name} 下有 {siblingCount} 个账号候选，这里改的是第一个。
          </p>
        )
      )}

      {select.isError && (
        <p className="text-micro text-status-error">
          保存失败：{errorMessageText(select.error)}
        </p>
      )}
    </div>
  )
}
