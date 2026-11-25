/**
 * 路由组（§7.3）：Magpie 式单列可展开编辑器。
 *
 * 一个组一行；展开后是名称/ID、成员模型、账号（只读推导）、规则、路由判断和高级。
 * 只提供 Token Router 真实生效的字段；Magpie 独有的组级限流/超时/会话保持等不渲染。
 *
 * 所有编辑写入全局共享草稿（useConfigDraft）；自动保存引擎负责提交。
 */
import { useState } from 'react'
import { Link } from '@tanstack/react-router'
import { useAllAccountsQuery } from '../queries/admin'
import { QueryState, EmptyState } from '../components/common/QueryState'
import { Icon } from '../components/common/Icon'
import { Modal } from '../components/common/Modal'
import { ConfigSaveBar } from '../components/config/ConfigSaveBar'
import { useConfigDraft } from '../hooks/useConfigDraft'
import { providerOfCandidate } from '../lib/candidate-source'
import {
  emptyVirtualModel,
  issuesByPath,
  type CandidateDraft,
  type VirtualModelDraft,
} from '../lib/config-edit'
import { GroupEditor } from './groups/GroupEditor'
import {
  groupMemberViews,
  groupModelChips,
  groupTitle,
  judgePurpose,
  judgeSelectableCandidates,
} from './groups/group-view'

export function VirtualModelsPage() {
  const accountsQuery = useAllAccountsQuery()
  const providers = accountsQuery.providers
  const accounts = accountsQuery.accounts

  const {
    draft,
    queryError,
    queryLoading,
    refetch,
    update,
    saving,
    localIssues: allLocalIssues,
  } = useConfigDraft()

  const [expandedIndex, setExpandedIndex] = useState<number | null>(0)
  const [search, setSearch] = useState('')
  const [deleteIndex, setDeleteIndex] = useState<number | null>(null)
  const [createOpen, setCreateOpen] = useState(false)
  const [newGroupId, setNewGroupId] = useState('')
  const [newGroupName, setNewGroupName] = useState('')
  const [newGroupFeedback, setNewGroupFeedback] = useState('')
  // 添加回答模型的 Magpie 式选择器；记下目标组，避免展开切换后加错。
  const [pickerIndex, setPickerIndex] = useState<number | null>(null)
  const [modelQuery, setModelQuery] = useState('')

  const vmIssues = allLocalIssues.filter(
    (issue) =>
      issue.path.startsWith('virtual_models') ||
      issue.path.startsWith('profiles') ||
      issue.path === 'default_virtual_model',
  )

  if (queryLoading || (!draft && !queryError)) {
    return (
      <div className="app-page">
        <QueryState loading={queryLoading} error={queryError} onRetry={refetch}>
          <span />
        </QueryState>
      </div>
    )
  }
  if (queryError && !draft) {
    return (
      <div className="app-page">
        <QueryState loading={false} error={queryError} onRetry={refetch}>
          <span />
        </QueryState>
      </div>
    )
  }

  const updateModel = (index: number, patch: Partial<VirtualModelDraft>) => {
    if (!draft) return
    update({
      ...draft,
      virtualModels: draft.virtualModels.map((model, i) =>
        i === index ? { ...model, ...patch } : model,
      ),
    })
  }

  /** 启用/停用组；停用默认组时自动把默认入口切到下一个启用组。 */
  const setModelEnabled = (index: number, enabled: boolean) => {
    if (!draft) return
    const target = draft.virtualModels[index]
    if (!target) return
    const virtualModels = draft.virtualModels.map((model, i) =>
      i === index ? { ...model, enabled } : model,
    )
    let defaultVirtualModel = draft.defaultVirtualModel
    if (!enabled && defaultVirtualModel === target.id) {
      // 停用默认组：切到下一个启用组；没有就回到“未设置”（后端会自动挑）。
      const fallback = virtualModels.find((model) => model.enabled)
      defaultVirtualModel = fallback?.id ?? ''
    } else if (enabled && defaultVirtualModel === '' && target.id.trim() !== '') {
      // 原本未设置默认，启用第一组时把它显式设为默认（用户意图更清楚）。
      defaultVirtualModel = target.id
    }
    update({ ...draft, virtualModels, defaultVirtualModel })
  }

  /** 指定默认入口（客户端未传 model 时用）。 */
  const setAsDefault = (index: number) => {
    if (!draft) return
    const target = draft.virtualModels[index]
    if (!target || !target.enabled || target.candidate_pool.length === 0) return
    update({ ...draft, defaultVirtualModel: target.id })
  }

  const toggleGroup = (index: number) => {
    setExpandedIndex((prev) => (prev === index ? null : index))
    setPickerIndex(null)
  }

  const openCreate = () => {
    setNewGroupId('')
    setNewGroupName('')
    setNewGroupFeedback('')
    setCreateOpen(true)
  }

  /** 按原型输入稳定的组 ID 建组；名称可留空（沿用 ID）。 */
  const createNamedGroup = () => {
    if (!draft) return
    const id = newGroupId.trim().toLowerCase()
    if (id === '') {
      setNewGroupFeedback('请输入新组 ID。')
      return
    }
    if (!/^[a-z][a-z0-9._-]*$/.test(id)) {
      setNewGroupFeedback('组 ID 需以小写字母开头，仅含小写字母、数字、. _ -。')
      return
    }
    if (draft.virtualModels.some((model) => model.id === id)) {
      setNewGroupFeedback(`组 ID 重复：${id} 已存在。`)
      return
    }
    const model = { ...emptyVirtualModel(), id, name: newGroupName.trim() || id }
    update({
      ...draft,
      virtualModels: [...draft.virtualModels, model],
    })
    setExpandedIndex(draft.virtualModels.length)
    setCreateOpen(false)
    setSearch('')
    setNewGroupId('')
    setNewGroupName('')
    setNewGroupFeedback('')
  }

  /** 删除只改草稿；若删的是默认组，同时指向下一个启用组。 */
  const deleteModel = (index: number) => {
    if (!draft) return
    const target = draft.virtualModels[index]
    if (!target) {
      setDeleteIndex(null)
      return
    }
    const nextModels = draft.virtualModels.filter((_, i) => i !== index)
    const clearDefault = target.id !== '' && draft.defaultVirtualModel === target.id
    // 不能写空串——后端会把 `default_virtual_model = ""` 当作不存在的模型。
    const fallback = clearDefault ? nextModels.find((model) => model.enabled) : undefined
    update({
      ...draft,
      virtualModels: nextModels,
      ...(clearDefault ? { defaultVirtualModel: fallback?.id ?? '' } : {}),
    })
    setDeleteIndex(null)
    setPickerIndex(null)
    setExpandedIndex((prev) => {
      if (nextModels.length === 0) return null
      if (prev === null) return null
      if (prev > index) return prev - 1
      if (prev === index) return Math.min(index, nextModels.length - 1)
      return prev
    })
  }

  const vmIssueMap = issuesByPath(vmIssues)
  const issueText = (path: string): string | null => {
    const list = vmIssueMap.get(path)
    return list && list.length > 0 ? list.map((issue) => issue.message).join('；') : null
  }

  const models = draft?.virtualModels ?? []
  const candidates = draft?.candidates ?? []

  const groupIssueCounts = new Map<number, number>()
  const bumpIssueCount = (index: number) => {
    if (index < 0) return
    groupIssueCounts.set(index, (groupIssueCounts.get(index) ?? 0) + 1)
  }
  for (const issue of vmIssues) {
    const numeric = /^virtual_models\.(\d+)(?:\.|$)/.exec(issue.path)
    if (numeric) {
      bumpIssueCount(Number(numeric[1]))
      continue
    }
    const bracket = /^virtual_models\[([^\]]*)\]/.exec(issue.path)
    if (bracket) {
      bumpIssueCount(models.findIndex((model) => model.id === bracket[1]))
    }
  }

  const profileIssueCount = vmIssues.filter((issue) =>
    issue.path.startsWith('profiles'),
  ).length
  const enabledCount = models.filter((model) => model.enabled).length

  // 判断模型：任何启用候选都可直接选，不要求供应商页预设用途。
  const judgeCandidates = judgeSelectableCandidates(candidates)

  const providerOf = (candidateId: string) => providerOfCandidate(providers, candidateId)

  const pickerGroup = pickerIndex !== null ? models[pickerIndex] : undefined
  // 只列供应商页编译出的候选：没有供应商前缀的候选不属于模型来源，不能加进组。
  const addableCandidates = pickerGroup
    ? candidates.filter(
        (candidate) =>
          candidate.id.trim() !== '' &&
          candidate.enabled &&
          judgePurpose(candidate) === null &&
          providerOf(candidate.id) !== undefined &&
          !pickerGroup.candidate_pool.includes(candidate.id),
      )
    : []
  const modelNeedle = modelQuery.trim().toLowerCase()
  const addableGroups = (() => {
    const groups = new Map<
      string,
      { key: string; label: string; items: CandidateDraft[] }
    >()
    for (const candidate of addableCandidates) {
      const provider = providerOf(candidate.id)
      if (!provider) continue
      const entry = groups.get(provider.id) ?? {
        key: provider.id,
        label: provider.display_name,
        items: [],
      }
      entry.items.push(candidate)
      groups.set(provider.id, entry)
    }
    return [...groups.values()]
      .map((group) => ({
        ...group,
        items: group.items.filter(
          (candidate) =>
            modelNeedle === '' ||
            candidate.model.toLowerCase().includes(modelNeedle) ||
            candidate.id.toLowerCase().includes(modelNeedle) ||
            group.label.toLowerCase().includes(modelNeedle),
        ),
      }))
      .filter((group) => group.items.length > 0)
  })()

  const needle = search.trim().toLowerCase()
  const visibleModels = models
    .map((model, index) => ({ model, index }))
    .filter(
      ({ model }) =>
        needle === '' ||
        model.id.toLowerCase().includes(needle) ||
        model.name.toLowerCase().includes(needle),
    )

  const deleteTarget = deleteIndex !== null ? models[deleteIndex] : undefined
  const deletingDefault =
    deleteTarget !== undefined &&
    deleteTarget.id !== '' &&
    draft?.defaultVirtualModel === deleteTarget.id

  const addPoolMember = (index: number, candidateId: string) => {
    const model = models[index]
    if (!model || model.candidate_pool.includes(candidateId)) return
    updateModel(index, { candidate_pool: [...model.candidate_pool, candidateId] })
  }

  const removePoolMember = (index: number, candidateId: string) => {
    const model = models[index]
    if (!model) return
    updateModel(index, {
      candidate_pool: model.candidate_pool.filter((id) => id !== candidateId),
    })
  }

  return (
    <div className="app-page">
      <div className="flex items-center gap-2 flex-wrap">
        <input
          className="app-field-sm max-w-72"
          type="search"
          value={search}
          placeholder="筛选路由组和模型…"
          aria-label="搜索路由组"
          onChange={(event) => setSearch(event.target.value)}
        />
        <span className="text-meta text-muted-foreground">
          {models.length} 个组 · {enabledCount} 个启用
        </span>
        <span className="flex-1" />
        <button type="button" className="oc-btn-primary" onClick={openCreate}>
          <Icon name="add" />
          新建组
        </button>
      </div>

      {vmIssues.length > 0 && (
        <div className="app-banner app-banner-warn">
          {vmIssues.length} 个配置问题待解决（见各组的标红字段）。
        </div>
      )}
      {profileIssueCount > 0 && (
        <div className="app-banner app-banner-warn">
          profile 定义有 {profileIssueCount} 个问题，请在 config.toml 修复。
        </div>
      )}
      {accountsQuery.error && (
        <div className="app-banner app-banner-warn">
          账号标签加载失败，成员只显示账号 id。
        </div>
      )}

      {/* 保存期间草稿被 store 锁定：整块编辑区 inert，避免输入被静默丢弃。 */}
      <div className="flex flex-col gap-3" inert={saving} aria-busy={saving}>
        {visibleModels.length === 0 ? (
          <section className="app-card" aria-label="路由组">
            <h2 className="app-card-title">
              {models.length === 0 ? '还没有路由组' : '没有匹配的路由组'}
            </h2>
            {models.length === 0 && (
              <button type="button" className="oc-btn-primary self-start" onClick={openCreate}>
                <Icon name="add" />
                新建组
              </button>
            )}
          </section>
        ) : (
          visibleModels.map(({ model, index }) => {
            const expanded = index === expandedIndex
            const issueCount = groupIssueCounts.get(index) ?? 0
            const isDefault =
              draft?.defaultVirtualModel === model.id && model.id.trim() !== ''
            const chips = groupModelChips(
              groupMemberViews(model, candidates, providers, accounts),
            )
            return (
              <section
                key={index}
                className="app-group-card"
                aria-label={`路由组 ${groupTitle(model)}`}
              >
                <div className="app-group-head">
                  <button
                    type="button"
                    className="app-group-toggle"
                    aria-expanded={expanded}
                    onClick={() => toggleGroup(index)}
                  >
                    <span className="app-group-avatar" aria-hidden>
                      {groupTitle(model).slice(0, 1).toUpperCase()}
                    </span>
                    <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                      <span className="flex items-center gap-1.5 flex-wrap">
                        <span className="text-ui-label font-semibold truncate">
                          {groupTitle(model)}
                        </span>
                        {isDefault && <span className="app-pill app-pill-ok">默认入口</span>}
                        {!model.enabled && (
                          <span className="app-pill app-pill-warn">已停用</span>
                        )}
                        {issueCount > 0 && (
                          <span className="app-pill app-pill-warn">
                            {issueCount} 个问题
                          </span>
                        )}
                      </span>
                      <span className="truncate font-mono text-micro text-muted-foreground">
                        {model.id.trim() || '(无 ID)'}
                      </span>
                      <span className="flex items-center gap-1 flex-wrap">
                        {chips.slice(0, 3).map((chip) => (
                          <span
                            key={chip.key}
                            className={
                              chip.disabled ? 'app-pill app-pill-warn' : 'app-pill'
                            }
                            title={
                              chip.disabled
                                ? '该模型的成员候选已停用'
                                : chip.manual
                                  ? '配置里手写的候选'
                                  : undefined
                            }
                          >
                            {chip.label}
                            {chip.count > 1 ? ` ×${chip.count}` : ''}
                          </span>
                        ))}
                        {chips.length > 3 && (
                          <span className="app-pill">+{chips.length - 3}</span>
                        )}
                      </span>
                    </span>
                    <Icon
                      name={expanded ? 'collapse' : 'expand'}
                      className="size-3.5 shrink-0 text-muted-foreground"
                    />
                  </button>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={model.enabled}
                    aria-label={`${model.enabled ? '停用' : '启用'} ${groupTitle(model)}`}
                    className={model.enabled ? 'app-switch app-switch-on' : 'app-switch'}
                    onClick={() => setModelEnabled(index, !model.enabled)}
                  >
                    <span className="app-switch-knob" aria-hidden />
                  </button>
                  <button
                    type="button"
                    className="oc-btn-default oc-btn-sm"
                    onClick={() => toggleGroup(index)}
                  >
                    {expanded ? '收起' : '编辑'}
                  </button>
                </div>
                {expanded && (
                  <div className="app-group-body">
                    <GroupEditor
                      group={model}
                      index={index}
                      draft={draft!}
                      providers={providers}
                      accounts={accounts}
                      candidates={candidates}
                      judgeCandidates={judgeCandidates}
                      isDefault={isDefault}
                      issueText={issueText}
                      onUpdate={(patch) => updateModel(index, patch)}
                      onSetDefault={() => setAsDefault(index)}
                      onRemoveMember={(candidateId) => removePoolMember(index, candidateId)}
                      onOpenPicker={() => {
                        setModelQuery('')
                        setPickerIndex(index)
                      }}
                      onDelete={() => setDeleteIndex(index)}
                    />
                  </div>
                )}
              </section>
            )
          })
        )}
      </div>

      <ConfigSaveBar />

      {createOpen && (
        <Modal
          title="新建路由组"
          onClose={() => setCreateOpen(false)}
          footer={
            <>
              <span className="flex-1" />
              <button
                type="button"
                className="oc-btn-default"
                onClick={() => setCreateOpen(false)}
              >
                取消
              </button>
              <button type="button" className="oc-btn-primary" onClick={createNamedGroup}>
                创建
              </button>
            </>
          }
        >
          <div className="flex flex-col gap-3">
            <label className="app-field">
              组 ID（客户端模型名）
              <input
                className="oc-field app-field-light font-mono"
                value={newGroupId}
                placeholder="如 work"
                aria-label="新组 ID"
                autoComplete="off"
                spellCheck={false}
                autoFocus
                onChange={(event) => {
                  setNewGroupId(event.target.value)
                  setNewGroupFeedback('')
                }}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') createNamedGroup()
                }}
              />
            </label>
            <label className="app-field">
              名称（可选）
              <input
                className="oc-field app-field-light"
                value={newGroupName}
                placeholder="如 DeepSeek 主力"
                aria-label="新组名称"
                onChange={(event) => setNewGroupName(event.target.value)}
              />
            </label>
            {newGroupFeedback !== '' && (
              <span className="app-cell-error" role="status" aria-live="polite">
                {newGroupFeedback}
              </span>
            )}
          </div>
        </Modal>
      )}

      {pickerGroup !== undefined && (
        <Modal
          size="wide"
          title={`添加模型 · ${groupTitle(pickerGroup)}`}
          onClose={() => setPickerIndex(null)}
        >
          <div className="flex flex-col gap-3">
            <input
              className="app-field-sm"
              type="search"
              value={modelQuery}
              placeholder="筛选供应商 / 模型 / 候选 ID…"
              aria-label="筛选可添加模型"
              autoFocus
              onChange={(event) => setModelQuery(event.target.value)}
            />

            {addableCandidates.length === 0 ? (
              <EmptyState
                title="没有可加入的对话模型"
                action={
                  <Link
                    to="/providers"
                    className="oc-btn-default oc-btn-sm"
                    onClick={() => setPickerIndex(null)}
                  >
                    去供应商页
                  </Link>
                }
              />
            ) : addableGroups.length === 0 ? (
              <p className="text-meta text-muted-foreground">没有匹配的模型。</p>
            ) : (
              <div className="flex max-h-96 flex-col gap-3 overflow-y-auto">
                {addableGroups.map((group) => (
                  <section key={group.key} className="flex flex-col gap-1">
                    <div className="flex items-center gap-2">
                      <span className="text-ui-label font-semibold">{group.label}</span>
                      <span className="app-pill">{group.items.length}</span>
                    </div>
                    {group.items.map((candidate) => (
                      <button
                        key={candidate.id}
                        type="button"
                        className="flex w-full items-center gap-2 rounded-lg border border-border px-2.5 py-2 text-left hover:bg-interactive-hover"
                        onClick={() => {
                          if (pickerIndex !== null) addPoolMember(pickerIndex, candidate.id)
                          setPickerIndex(null)
                        }}
                      >
                        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                          <span className="text-ui-label font-semibold truncate">
                            {candidate.model || candidate.id}
                          </span>
                          <span className="font-mono text-micro text-muted-foreground truncate">
                            {candidate.id}
                          </span>
                        </span>
                        <span className="app-pill">加入</span>
                      </button>
                    ))}
                  </section>
                ))}
              </div>
            )}
          </div>
        </Modal>
      )}

      {deleteTarget !== undefined && deleteIndex !== null && (
        <Modal
          title="删除路由组"
          subtitle={deleteTarget.id || deleteTarget.name || '未命名组'}
          onClose={() => setDeleteIndex(null)}
          footer={
            <>
              <span className="flex-1" />
              <button
                type="button"
                className="oc-btn-default"
                onClick={() => setDeleteIndex(null)}
              >
                取消
              </button>
              <button
                type="button"
                className="oc-btn-primary"
                onClick={() => deleteModel(deleteIndex)}
              >
                删除
              </button>
            </>
          }
        >
          <div className="flex flex-col gap-2">
            {deletingDefault && (
              <div className="app-banner app-banner-warn">
                该组是默认虚拟模型（default_virtual_model）；删除时会同时清空该默认项。
              </div>
            )}
          </div>
        </Modal>
      )}
    </div>
  )
}
