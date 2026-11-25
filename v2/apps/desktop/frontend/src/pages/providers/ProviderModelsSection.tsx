/**
 * 供应商编辑器 · 模型分区（对齐 Magpie：模型 chips + 内联「模型设置」面板）。
 *
 * - 模型以 chips 呈现（已选在前，点 chip 选中它并展开它的模型设置）；
 * - 「模型设置」是**内联面板**（不是单独弹窗）：用途 / 能力标签（目录侧即时保存）
 *   + 显示名 / 上下文窗口 / 价格 / 支持图片输入（编译候选，改动进配置草稿）；
 * - 「展开列表 ▾」展开明细行：选择、来源、删除/恢复、测试；
 * - 勾选、全选、清空、手工添加、刷新、删除/恢复都走现有 catalog 端点；
 *   全选 / 清空是逐个调用 select（无批量接口），全部成功后再 invalidate。
 */
import { useState } from 'react'
import type { CatalogModel, ModelPurposeDto, ProviderInfo } from '../../api/types'
import {
  useAccountsQuery,
  useCatalogQuery,
  useDeleteCatalogModelMutation,
  useEnrichCatalogMutation,
  useRefreshCatalogMutation,
  useSelectCatalogModelMutation,
} from '../../queries/admin'
import { useAppStore } from '../../stores/appStore'
import { ErrorState, Loading } from '../../components/common/QueryState'
import { Modal } from '../../components/common/Modal'
import { Icon } from '../../components/common/Icon'
import { ModelSettingsPanel } from './ModelSettingsPanel'
import { AutoIncludeEditor } from './AutoIncludeEditor'
import { errorMessageText, modelPurposeTag, MODEL_PURPOSE_LABEL } from './shared'

type FilterMode = 'all' | 'selected'

const PURPOSE_OPTIONS: Array<{ value: ModelPurposeDto; label: string }> = [
  { value: 'chat', label: MODEL_PURPOSE_LABEL.chat },
  { value: 'judge_llm', label: MODEL_PURPOSE_LABEL.judge_llm },
  { value: 'jev', label: MODEL_PURPOSE_LABEL.jev },
]

function sourceLabel(model: CatalogModel): string {
  if (model.source === 'manual') return '手工'
  if (model.source === 'models.dev') return 'models.dev'
  return '目录'
}

export function ProviderModelsSection({ provider }: { provider: ProviderInfo }) {
  const catalogQuery = useCatalogQuery(provider.id)
  const accountsQuery = useAccountsQuery(provider.id)
  const defaultCredential =
    (accountsQuery.data ?? []).find((account) => account.enabled)?.credential_ref ?? ''
  const refresh = useRefreshCatalogMutation()
  const enrich = useEnrichCatalogMutation()
  const showToast = useAppStore((s) => s.showToast)
  const select = useSelectCatalogModelMutation()
  const remove = useDeleteCatalogModelMutation()

  const [search, setSearch] = useState('')
  const [filter, setFilter] = useState<FilterMode>('all')
  const [showHidden, setShowHidden] = useState(false)
  const [manualModel, setManualModel] = useState('')
  const [deleteTarget, setDeleteTarget] = useState<CatalogModel | null>(null)
  const [focusedId, setFocusedId] = useState<string | null>(null)
  const [chipsExpanded, setChipsExpanded] = useState(false)
  const [listExpanded, setListExpanded] = useState(false)
  const [settingsOpen, setSettingsOpen] = useState(true)
  const [rowError, setRowError] = useState<string | null>(null)
  const [bulkAction, setBulkAction] = useState<'select' | 'clear' | null>(null)
  // 清空会连同判断模型一起取消：必须让用户看见影响再确认。
  const [confirmClear, setConfirmClear] = useState(false)

  const models = catalogQuery.data?.models ?? []
  const selectedModels = models.filter((model) => model.selected && !model.hidden)
  const needle = search.trim().toLowerCase()
  const visibleModels = models.filter((model) => {
    if (!showHidden && model.hidden) return false
    // 「全部」只列尚未选中的模型；已选模型由上方 chips 呈现，避免重复。
    if (filter === 'all' && model.selected && !model.hidden) return false
    if (filter === 'selected' && !model.selected) return false
    if (
      needle !== '' &&
      !model.model_id.toLowerCase().includes(needle) &&
      !model.display_name.toLowerCase().includes(needle)
    ) {
      return false
    }
    return true
  })

  const focusedModel =
    focusedId !== null ? (models.find((model) => model.model_id === focusedId) ?? null) : null
  const selectableCount = visibleModels.filter(
    (model) => !model.selected && !model.hidden,
  ).length
  // chips：已选在前，未选在后；折叠时只显示前 24 个。
  const orderedModels = [
    ...selectedModels,
    ...models.filter((model) => !model.selected),
  ]
  // 搜索命中 id / 显示名 / 候选 id；搜索时不再折叠（直接展示全部命中）。
  const matchedChips =
    needle === ''
      ? orderedModels
      : orderedModels.filter(
          (model) =>
            model.model_id.toLowerCase().includes(needle) ||
            model.display_name.toLowerCase().includes(needle) ||
            (model.candidate_id ?? '').toLowerCase().includes(needle),
        )
  const chipModels = chipsExpanded || needle !== '' ? matchedChips : matchedChips.slice(0, 24)
  const hiddenChipCount = matchedChips.length - chipModels.length

  function toggle(model: CatalogModel) {
    setRowError(null)
    select.mutate(
      {
        providerId: provider.id,
        request: { model_id: model.model_id, selected: !model.selected },
      },
      { onError: (error) => setRowError(errorMessageText(error)) },
    )
  }

  function addManual() {
    const modelId = manualModel.trim()
    if (modelId === '') return
    setRowError(null)
    select.mutate(
      {
        providerId: provider.id,
        request: { model_id: modelId, selected: true },
      },
      {
        onSuccess: () => setManualModel(''),
        onError: (error) => setRowError(errorMessageText(error)),
      },
    )
  }

  function restore(model: CatalogModel) {
    setRowError(null)
    select.mutate(
      {
        providerId: provider.id,
        request: { model_id: model.model_id, selected: false, hidden: false },
      },
      { onError: (error) => setRowError(errorMessageText(error)) },
    )
  }

  async function runBulk(action: 'select' | 'clear') {
    const targets =
      action === 'select'
        ? visibleModels.filter((model) => !model.selected && !model.hidden)
        : models.filter((model) => model.selected)
    if (targets.length === 0) return
    setRowError(null)
    setBulkAction(action)
    try {
      for (const model of targets) {
        await select.mutateAsync({
          providerId: provider.id,
          request: { model_id: model.model_id, selected: action === 'select' },
        })
      }
    } catch (error) {
      setRowError(errorMessageText(error))
    } finally {
      setBulkAction(null)
    }
  }

  const busy = bulkAction !== null || select.isPending

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2 flex-wrap">
        <h3 className="text-ui-label font-medium">模型</h3>
        <span className="text-meta text-muted-foreground">
          已选 {selectedModels.length} / {models.length}
        </span>
        <span className="flex-1" />
        <input
          className="oc-field app-field-light font-mono h-8 w-56 text-meta"
          placeholder="搜索模型 id / 名称 / 候选"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          aria-label="搜索模型"
        />
        <button
          type="button"
          className="oc-btn-default oc-btn-sm"
          disabled={focusedModel === null}
          title={focusedModel === null ? '先点一个模型 chip' : undefined}
          onClick={() => {
            if (focusedModel !== null) {
              setFocusedId(focusedModel.model_id)
            }
          }}
        >
          模型设置
        </button>
        <button
          type="button"
          className="oc-btn-default oc-btn-sm"
          onClick={() => refresh.mutate(provider.id)}
          disabled={refresh.isPending || provider.account_count === 0}
          title={
            provider.account_count === 0 ? '先添加账号才能拉取目录' : undefined
          }
        >
          <Icon name="refresh" className="size-3.5" />
          {refresh.isPending ? '刷新中…' : '刷新目录'}
        </button>
        <button
          type="button"
          className="oc-btn-default oc-btn-sm"
          disabled={enrich.isPending || models.length === 0}
          title="从 models.dev 补全模型的上下文窗口、输出上限、价格和推理档位（只填空，不覆盖手填值）"
          onClick={() =>
            enrich.mutate(provider.id, {
              onSuccess: (report) =>
                showToast(
                  report.source_provider === null
                    ? 'models.dev 未匹配到该供应商，请手工填写模型信息'
                    : `已从 models.dev 补全 ${report.filled}/${report.matched} 个模型`,
                  report.source_provider !== null,
                ),
              onError: (error) => showToast(errorMessageText(error), false),
            })
          }
        >
          <Icon name="apply" className="size-3.5" />
          {enrich.isPending ? '同步中…' : '同步模型信息'}
        </button>
      </div>

      {models.length > 0 && (
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-1.5 flex-wrap" aria-label="模型">
            {chipModels.map((model) => {
              const tag = modelPurposeTag(model.purpose)
              const active = focusedId === model.model_id
              return (
                <span
                  key={model.model_id}
                  className={
                    active
                      ? 'app-model-chip app-model-chip-active'
                      : model.selected
                        ? 'app-model-chip app-model-chip-selected'
                        : 'app-model-chip'
                  }
                >
                  <button
                    type="button"
                    className="font-mono text-foreground bg-transparent border-0 p-0 cursor-pointer inline-flex items-center gap-1"
                    title={model.selected ? '取消选择（点一下反选）' : '选中这个模型'}
                    aria-pressed={model.selected}
                    onClick={() => {
                      setFocusedId(model.model_id)
                      toggle(model)
                    }}
                  >
                    {model.selected && <Icon name="approve" className="size-3" />}
                    {model.model_id}
                  </button>
                  {tag !== null && <span className="app-pill">{tag}</span>}
                </span>
              )
            })}
            {chipModels.length === 0 && needle !== '' && (
              <span className="text-meta text-muted-foreground">没有匹配的模型</span>
            )}
            {hiddenChipCount > 0 && (
              <button
                type="button"
                className="oc-btn-default oc-btn-xs"
                onClick={() => setChipsExpanded(true)}
              >
                显示其余 {hiddenChipCount} 个
              </button>
            )}
          </div>
          <button
            type="button"
            className="oc-btn-default oc-btn-sm self-start"
            aria-expanded={listExpanded}
            aria-controls="provider-model-list"
            onClick={() => setListExpanded((value) => !value)}
          >
            {listExpanded ? '收起列表' : '展开列表'}
            <span aria-hidden>{listExpanded ? '▴' : '▾'}</span>
          </button>
        </div>
      )}

      {/* 内联「模型设置」面板（Magpie 式：每个模型一张卡，不是单独弹窗） */}
      {settingsOpen && (
        <ModelSettingsPanel
          provider={provider}
          models={models}
          focusedId={focusedModel?.model_id ?? null}
          onFocus={setFocusedId}
          onCollapse={() => setSettingsOpen(false)}
          defaultCredential={defaultCredential}
        />
      )}
      {!settingsOpen && (
        <button
          type="button"
          className="oc-btn-default oc-btn-sm self-start"
          onClick={() => setSettingsOpen(true)}
        >
          展开模型设置
        </button>
      )}

      {/* 操作行 */}
      <div className="app-form-row items-center">
        <input
          className="oc-field app-field-light font-mono h-7 min-w-52 text-meta"
          placeholder="添加模型 ID（本地推理 / 上游未列出）"
          value={manualModel}
          onChange={(event) => setManualModel(event.target.value)}
          aria-label="添加模型 ID"
          onKeyDown={(event) => {
            if (event.key === 'Enter') {
              event.preventDefault()
              addManual()
            }
          }}
        />
        <button
          type="button"
          className="oc-btn-default oc-btn-sm"
          onClick={addManual}
          disabled={select.isPending || manualModel.trim() === ''}
        >
          添加模型
        </button>
        <button
          type="button"
          className="oc-btn-default oc-btn-sm"
          disabled={refresh.isPending || provider.account_count === 0}
          title={provider.account_count === 0 ? '先添加账号才能拉取目录' : undefined}
          onClick={() => refresh.mutate(provider.id)}
        >
          <Icon name="refresh" className="size-3.5" />
          {refresh.isPending ? '重新获取中…' : '重新获取模型'}
        </button>
        {focusedModel?.candidate_id && focusedModel.purpose === 'chat' ? (
          <a
            className="oc-btn-default oc-btn-sm"
            href={`#/playground?target=${encodeURIComponent(`candidate:${focusedModel.candidate_id}`)}`}
          >
            测试模型
          </a>
        ) : null}
      </div>

      {/* 供应商列表策略（对应参考图的「只通过路由分组使用 / 拉取模型列表」） */}
      <AutoIncludeEditor provider={provider} />

      {refresh.isError && (
        <p className="app-banner app-banner-err text-meta">
          目录刷新失败：{refresh.error.message}
        </p>
      )}
      {rowError !== null && (
        <p className="app-banner app-banner-err text-meta">{rowError}</p>
      )}

      {listExpanded && (
        <>
          <div className="flex items-center gap-2 flex-wrap">
            <div className="oc-seg" role="group" aria-label="模型筛选">
              <button
                type="button"
                aria-pressed={filter === 'all'}
                className={filter === 'all' ? 'oc-seg-item-active' : 'oc-seg-item'}
                onClick={() => setFilter('all')}
              >
                全部
              </button>
              <button
                type="button"
                aria-pressed={filter === 'selected'}
                className={filter === 'selected' ? 'oc-seg-item-active' : 'oc-seg-item'}
                onClick={() => setFilter('selected')}
              >
                仅已选
              </button>
            </div>
            <label className="app-scope-check">
              <input
                type="checkbox"
                checked={showHidden}
                onChange={(event) => setShowHidden(event.target.checked)}
              />
              <span className="text-meta">显示已移除</span>
            </label>
            <span className="flex-1" />
            <button
              type="button"
              className="oc-btn-default oc-btn-sm"
              disabled={busy || selectableCount === 0}
              title="选择当前列表里所有未选中的模型（不含已移除）"
              onClick={() => void runBulk('select')}
            >
              {bulkAction === 'select' ? '全选中…' : '全选'}
            </button>
            {confirmClear ? (
              <span className="app-confirm">
                取消 {selectedModels.length} 个已选模型（含判断模型）？
                <button
                  type="button"
                  className="oc-btn-destructive oc-btn-sm"
                  onClick={() => {
                    setConfirmClear(false)
                    void runBulk('clear')
                  }}
                >
                  确认清空
                </button>
                <button
                  type="button"
                  className="oc-btn-default oc-btn-sm"
                  onClick={() => setConfirmClear(false)}
                >
                  取消
                </button>
              </span>
            ) : (
              <button
                type="button"
                className="oc-btn-default oc-btn-sm"
                disabled={busy || selectedModels.length === 0}
                title="取消本厂商所有已选模型（含判断模型）"
                onClick={() => setConfirmClear(true)}
              >
                {bulkAction === 'clear' ? '清空中…' : '清空'}
              </button>
            )}
          </div>

      {catalogQuery.isPending ? (
        <Loading />
      ) : catalogQuery.isError ? (
        <ErrorState
          message={catalogQuery.error.message}
          onRetry={() => catalogQuery.refetch()}
        />
      ) : visibleModels.length > 0 ? (
        <ul
          className="flex flex-col gap-0.5 max-h-80 overflow-y-auto"
          aria-label="模型列表"
        >
          {visibleModels.map((model) => (
            <li
              key={model.model_id}
              className="flex items-center gap-2 py-1 px-1 rounded-md text-meta flex-wrap"
            >
              <label className="flex items-center gap-2 cursor-pointer min-w-0">
                <input
                  type="checkbox"
                  checked={model.selected}
                  disabled={busy || model.hidden}
                  onChange={() => toggle(model)}
                  aria-label={`选择模型 ${model.model_id}`}
                />
                <span className="font-mono truncate">{model.model_id}</span>
              </label>
              {model.display_name !== model.model_id && (
                <span className="text-muted-foreground truncate">
                  {model.display_name}
                </span>
              )}
              <span className="app-pill">{sourceLabel(model)}</span>
              {model.hidden && <span className="app-pill app-pill-warn">已移除</span>}
              <span className="flex-1" />
              {/* 用途与能力标签放在明细行（模型设置卡里只留模型参数）。 */}
              <select
                className="app-select-sm"
                value={model.purpose}
                disabled={busy || model.hidden}
                aria-label={`${model.model_id} 用途`}
                onChange={(event) =>
                  select.mutate({
                    providerId: provider.id,
                    request: {
                      model_id: model.model_id,
                      selected: model.selected,
                      purpose: event.target.value as ModelPurposeDto,
                      caps: model.caps,
                    },
                  })
                }
              >
                {PURPOSE_OPTIONS.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
              <input
                className="oc-field app-field-light font-mono h-7 min-w-40 max-w-56"
                defaultValue={model.caps.join(', ')}
                placeholder="能力标签（逗号分隔）"
                disabled={busy || model.hidden}
                aria-label={`${model.model_id} 能力标签`}
                onBlur={(event) => {
                  const caps = Array.from(
                    new Set(
                      event.target.value
                        .split(',')
                        .map((item) => item.trim())
                        .filter(Boolean),
                    ),
                  )
                  if (caps.join(',') === model.caps.join(',')) return
                  select.mutate({
                    providerId: provider.id,
                    request: {
                      model_id: model.model_id,
                      selected: model.selected,
                      purpose: model.purpose,
                      caps,
                    },
                  })
                }}
              />
              {!model.hidden && (
                <button
                  type="button"
                  className="oc-btn-default oc-btn-sm"
                  disabled={select.isPending}
                  onClick={() => setFocusedId(model.model_id)}
                >
                  模型设置
                </button>
              )}
              {model.candidate_id && !model.hidden && (
                <a
                  className="oc-btn-default oc-btn-sm"
                  href={`#/playground?target=${encodeURIComponent(`candidate:${model.candidate_id}`)}`}
                >
                  测试
                </a>
              )}
              {model.hidden ? (
                <button
                  type="button"
                  className="oc-btn-default oc-btn-sm"
                  onClick={() => restore(model)}
                  disabled={select.isPending}
                >
                  恢复
                </button>
              ) : (
                <button
                  type="button"
                  className="oc-btn-destructive oc-btn-sm"
                  onClick={() => {
                    remove.reset()
                    setDeleteTarget(model)
                  }}
                  disabled={remove.isPending}
                >
                  删除
                </button>
              )}
            </li>
          ))}
        </ul>
      ) : models.length === 0 ? (
        <p className="text-meta text-muted-foreground">
          目录为空。有账号时点「重新获取模型」，或手工添加模型 ID。
        </p>
      ) : filter === 'all' && needle === '' && selectedModels.length > 0 ? (
        <p className="text-meta text-muted-foreground">
          全部模型已选；在上面的 chips 里点 × 可取消选择。
        </p>
      ) : (
        <p className="text-meta text-muted-foreground">没有匹配过滤条件的模型。</p>
      )}
        </>
      )}

      {deleteTarget && (
        <Modal
          title={`删除模型「${deleteTarget.model_id}」？`}
          subtitle="从目录与候选池移除；刷新目录不会自动带回。"
          onClose={() => {
            remove.reset()
            setDeleteTarget(null)
          }}
          footer={
            <>
              <span className="flex-1" />
              <button
                type="button"
                className="oc-btn-default"
                onClick={() => {
                  remove.reset()
                  setDeleteTarget(null)
                }}
              >
                取消
              </button>
              <button
                type="button"
                className="oc-btn-destructive"
                disabled={remove.isPending}
                onClick={() =>
                  remove.mutate(
                    { providerId: provider.id, modelId: deleteTarget.model_id },
                    {
                      onSuccess: () => setDeleteTarget(null),
                    },
                  )
                }
              >
                {remove.isPending ? '删除中…' : '确认删除'}
              </button>
            </>
          }
        >
          <p className="text-meta text-muted-foreground">
            若该模型仍被虚拟模型的候选池或规则引用，服务端会拒绝删除并列出引用位置。
            删除后需重新编译候选才会从配置中移除。
          </p>
          {remove.isError && (
            <p className="app-cell-error">{remove.error.message}</p>
          )}
        </Modal>
      )}
    </div>
  )
}
