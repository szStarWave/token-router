/**
 * Providers 页：供应商行列表（Magpie 式）+ 编辑器弹窗。
 *
 * - 列表：一行一个供应商 —— 图标 + 名称 + 类型徽标 + 端点/模型摘要，
 *   右侧是账号摘要（可切换打码）、启用开关与进入编辑器的箭头；
 *   停用的供应商收进「已关闭的供应商」折叠区。
 * - 工具条：查找供应商 + 添加 + 「账号打码」开关（打码时只显示密钥指纹，
 *   关掉打码显示凭据引用 env:/file:；**任何时候都不显示明文**）。
 * - 编辑器：单页分区（基本信息 / 账号 / 模型 / 端点 / 高级），账号与模型改动即时保存；
 *   「应用模型选择」仍影响全部厂商的勾选投影。
 */
import { useMemo, useState } from 'react'
import type { ProviderInfo } from '../api/types'
import {
  useAccountsQuery,
  useHealthQuery,
  useProvidersQuery,
  useQuotasQuery,
  useUpdateProviderMutation,
} from '../queries/admin'
import { QueryState } from '../components/common/QueryState'
import { Icon } from '../components/common/Icon'
import { AddProviderDialog } from './providers/AddProviderDialog'
import { ProviderEditorModal } from './providers/ProviderEditorModal'
import {
  PROVIDER_STATUS_LABEL,
  PROVIDER_STATUS_PILL,
  providerCatalogSummary,
  providerKindLabel,
  statusOf,
} from './providers/shared'
import { usePrefsStore } from '../stores/prefsStore'

function providerInitial(name: string): string {
  return name.trim().charAt(0).toUpperCase() || '?'
}

export function ProvidersPage() {
  const providersQuery = useProvidersQuery()
  const healthQuery = useHealthQuery()
  const quotasQuery = useQuotasQuery()
  const [search, setSearch] = useState('')
  const [offExpanded, setOffExpanded] = useState(false)
  const [editorId, setEditorId] = useState<string | null>(null)
  const [addMode, setAddMode] = useState<'templates' | 'import' | null>(null)
  const maskAccounts = usePrefsStore((s) => s.maskAccounts)
  const setMaskAccounts = usePrefsStore((s) => s.setMaskAccounts)

  const providers = providersQuery.data ?? []
  const needle = search.trim().toLowerCase()
  const matchedProviders =
    needle === ''
      ? providers
      : providers.filter((provider) =>
          [
            provider.display_name,
            provider.base_url,
            provider.kind,
            providerKindLabel(provider.kind),
          ].some((value) => value.toLowerCase().includes(needle)),
        )
  const activeProviders = matchedProviders.filter((provider) => provider.enabled)
  const disabledProviders = matchedProviders.filter((provider) => !provider.enabled)
  const totalAccounts = providers.reduce((sum, provider) => sum + provider.account_count, 0)
  const totalCandidates = providers.reduce(
    (sum, provider) => sum + provider.candidate_count,
    0,
  )
  const editorProvider =
    (editorId !== null ? providers.find((provider) => provider.id === editorId) : undefined) ??
    null

  // 供应商健康：用真实熔断状态按候选→provider 映射汇总；没有 7 日 5xx 历史指标。
  const healthByProvider = useMemo(() => {
    const providerOf = new Map<string, string>()
    for (const item of quotasQuery.data?.items ?? []) {
      if (item.provider_id) providerOf.set(item.candidate_id, item.provider_id)
    }
    const summary = new Map<string, { open: number; halfOpen: number }>()
    for (const item of healthQuery.data?.items ?? []) {
      const providerId = providerOf.get(item.candidate_id)
      if (!providerId) continue
      const entry = summary.get(providerId) ?? { open: 0, halfOpen: 0 }
      if (item.status === 'open') entry.open += 1
      else if (item.status === 'half_open') entry.halfOpen += 1
      summary.set(providerId, entry)
    }
    return summary
  }, [healthQuery.data, quotasQuery.data])

  return (
    <div className="app-page flex-1">
      {/* 工具条 */}
      <section className="flex items-center gap-3 flex-wrap">
        <input
          className="oc-field app-field-light app-setting-search"
          type="search"
          value={search}
          placeholder="查找供应商"
          aria-label="查找供应商"
          onChange={(event) => setSearch(event.target.value)}
        />
        <button
          type="button"
          className="oc-btn-primary oc-btn-sm"
          onClick={() => setAddMode('templates')}
        >
          <Icon name="add" />
          添加供应商
        </button>
        <button
          type="button"
          className="oc-btn-default oc-btn-sm"
          onClick={() => setAddMode('import')}
        >
          <Icon name="open-dir" />
          导入本地配置
        </button>
        <label className="app-filter-label">
          <input
            type="checkbox"
            className="accent-primary"
            checked={maskAccounts}
            aria-label="账号打码"
            onChange={(event) => setMaskAccounts(event.target.checked)}
          />
          账号打码
        </label>
        <span className="flex-1" />
        <span className="text-meta text-muted-foreground">
          {providers.length} 个供应商 · {totalAccounts} 个账号 · {totalCandidates} 个已选模型
        </span>
        <button
          type="button"
          className="oc-btn-default oc-btn-sm"
          onClick={() => void providersQuery.refetch()}
        >
          <Icon name="refresh" />
          更新
        </button>
      </section>

      <QueryState
        loading={providersQuery.isPending}
        error={providersQuery.error}
        empty={providers.length === 0}
        onRetry={() => providersQuery.refetch()}
        emptyTitle="还没有供应商"
        emptyAction={
          <span className="flex items-center gap-2">
            <button
              type="button"
              className="oc-btn-primary oc-btn-sm"
              onClick={() => setAddMode('templates')}
            >
              添加第一个供应商
            </button>
            <button
              type="button"
              className="oc-btn-default oc-btn-sm"
              onClick={() => setAddMode('import')}
            >
              导入…
            </button>
          </span>
        }
      >
        <section className="app-card" aria-label="供应商列表">
          {matchedProviders.length === 0 ? (
            <p className="text-meta text-muted-foreground">没有匹配的供应商。</p>
          ) : (
            <>
              {activeProviders.length > 0 && (
                <ul className="app-provider-list" aria-label="供应商">
                  {activeProviders.map((provider) => (
                    <ProviderRow
                      key={provider.id}
                      provider={provider}
                      health={healthByProvider.get(provider.id) ?? { open: 0, halfOpen: 0 }}
                      maskAccounts={maskAccounts}
                      onEdit={() => setEditorId(provider.id)}
                    />
                  ))}
                </ul>
              )}

              {disabledProviders.length > 0 && (
                <div className="flex flex-col gap-2">
                  <button
                    type="button"
                    className="oc-btn-default oc-btn-sm self-start"
                    aria-expanded={offExpanded}
                    aria-controls="providers-disabled-list"
                    onClick={() => setOffExpanded((expanded) => !expanded)}
                  >
                    已关闭的供应商 {disabledProviders.length}
                    <span aria-hidden>{offExpanded ? '▾' : '▸'}</span>
                  </button>
                  {offExpanded && (
                    <ul
                      id="providers-disabled-list"
                      className="app-provider-list"
                      aria-label="已关闭的供应商"
                    >
                      {disabledProviders.map((provider) => (
                        <ProviderRow
                          key={provider.id}
                          provider={provider}
                          health={healthByProvider.get(provider.id) ?? { open: 0, halfOpen: 0 }}
                          maskAccounts={maskAccounts}
                          onEdit={() => setEditorId(provider.id)}
                        />
                      ))}
                    </ul>
                  )}
                </div>
              )}
            </>
          )}
        </section>
      </QueryState>

      {addMode !== null && (
        <AddProviderDialog
          initialMode={addMode}
          onClose={() => setAddMode(null)}
          onCreated={(provider) => {
            setAddMode(null)
            setEditorId(provider.id)
          }}
        />
      )}

      {editorProvider !== null && (
        <ProviderEditorModal
          key={editorProvider.id}
          provider={editorProvider}
          onClose={() => setEditorId(null)}
        />
      )}
    </div>
  )
}

function ProviderRow({
  provider,
  health,
  maskAccounts,
  onEdit,
}: {
  provider: ProviderInfo
  health: { open: number; halfOpen: number }
  maskAccounts: boolean
  onEdit: () => void
}) {
  const status = statusOf(provider)
  const summary = providerCatalogSummary(provider)
  const updateProvider = useUpdateProviderMutation()
  // 每个可见行一个查询：列表只取账号做摘要（明文不可回读）。
  const accountsQuery = useAccountsQuery(provider.id)
  const firstAccount = accountsQuery.data?.[0]
  const accountSummary = (() => {
    if (accountsQuery.isPending) return '读取中…'
    if (accountsQuery.isError) return '—'
    if (!firstAccount) return '无账号'
    if (maskAccounts) return `…${firstAccount.fingerprint || '····'}`
    return firstAccount.credential_ref
  })()
  const accountCount = accountsQuery.data?.length ?? provider.account_count
  const healthLabel =
    health.open > 0
      ? `${health.open} 个候选熔断`
      : health.halfOpen > 0
        ? `${health.halfOpen} 个候选半开`
        : '健康正常'

  return (
    <li>
      <div className="app-provider-row-magpie">
        <button
          type="button"
          className="app-provider-main-btn"
          aria-label={`打开供应商 ${provider.display_name} 编辑器`}
          onClick={onEdit}
        >
          <span className="app-provider-icon" aria-hidden>
            {providerInitial(provider.display_name)}
          </span>
          <span className="app-provider-main">
            <span className="app-provider-name">
              <span className="truncate">{provider.display_name}</span>
              <span className={PROVIDER_STATUS_PILL[status]}>
                {PROVIDER_STATUS_LABEL[status]}
              </span>
              <span className="app-pill">{providerKindLabel(provider.kind)}</span>
            </span>
            <span className="app-provider-sub">
              <span className="font-mono">{provider.base_url}</span>
              <span> · {provider.candidate_count} 个已选模型</span>
              <span> · {summary}</span>
            </span>
          </span>
        </button>

        <span className="flex items-center gap-2 min-w-0 justify-end shrink-0">
          <span
            className={
              health.open > 0
                ? 'app-status-dot bg-status-error'
                : health.halfOpen > 0
                  ? 'app-status-dot bg-status-warning'
                  : 'app-status-dot bg-status-success'
            }
            title={healthLabel}
            aria-hidden
          />
          <span className="font-mono text-meta truncate max-w-36" title={accountSummary}>
            {accountSummary}
          </span>
          <span className="text-meta text-muted-foreground whitespace-nowrap">
            {accountCount} 账号
          </span>
        </span>

        <button
          type="button"
          role="switch"
          aria-checked={provider.enabled}
          aria-label={`${provider.enabled ? '停用' : '启用'} ${provider.display_name}`}
          className={provider.enabled ? 'app-switch app-switch-on' : 'app-switch'}
          disabled={updateProvider.isPending}
          onClick={() =>
            updateProvider.mutate({
              id: provider.id,
              request: { enabled: !provider.enabled },
            })
          }
        >
          <span className="app-switch-knob" aria-hidden />
        </button>

        <button
          type="button"
          className="oc-btn-default oc-btn-sm"
          aria-label={`打开供应商 ${provider.display_name} 编辑器`}
          onClick={onEdit}
        >
          <Icon name="next" className="size-3.5" />
        </button>
      </div>
    </li>
  )
}
