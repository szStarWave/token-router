/**
 * Providers 页（v3 P5-A）：厂商实例 → 账号（key）→ 模型目录勾选 → 候选编译。
 *
 * 全流程不写 TOML：添加厂商（模板）→ 添加账号（key 落服务端凭据文件）→
 * 刷新/勾选模型 → 「编译为候选」写回 config（走 CAS + 审计）。
 */
import { useState } from 'react'
import { api } from '../api/client'
import type {
  AccountInfo,
  CatalogModel,
  ImportedProviderDraft,
  ProviderInfo,
  ProviderStatus,
  ProviderTemplate,
} from '../api/types'
import {
  useAccountsQuery,
  useCatalogQuery,
  useCompileCandidatesMutation,
  useCreateAccountMutation,
  useCreateProviderMutation,
  useDeleteAccountMutation,
  useDeleteProviderMutation,
  useProviderTemplatesQuery,
  useProvidersQuery,
  useRefreshCatalogMutation,
  useSelectCatalogModelMutation,
  useVerifyAccountMutation,
} from '../queries/admin'
import { EmptyState, ErrorState, Loading } from '../components/common/QueryState'

const STATUS_LABEL: Record<ProviderStatus, string> = {
  not_configured: '未配置',
  awaiting_catalog: '待刷新目录',
  ready: '就绪',
  catalog_error: '目录失败',
}

const STATUS_PILL: Record<ProviderStatus, string> = {
  not_configured: 'app-pill app-pill-warn',
  awaiting_catalog: 'app-pill app-pill-warn',
  ready: 'app-pill app-pill-ok',
  catalog_error: 'app-pill app-pill-error',
}

function statusOf(provider: ProviderInfo): ProviderStatus {
  if (provider.account_count === 0) return 'not_configured'
  return provider.status
}

export function ProvidersPage() {
  const providersQuery = useProvidersQuery()
  const templatesQuery = useProviderTemplatesQuery()
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [showAdd, setShowAdd] = useState(false)
  const [importDrafts, setImportDrafts] = useState<ImportedProviderDraft[] | null>(null)
  const [importBusy, setImportBusy] = useState(false)
  const createProvider = useCreateProviderMutation()
  const createAccount = useCreateAccountMutation()

  const selected = providersQuery.data?.find((p) => p.id === selectedId) ?? null

  async function scanAgents() {
    setImportBusy(true)
    try {
      const response = await api.providerImport()
      setImportDrafts(response.drafts)
    } catch {
      setImportDrafts([])
    } finally {
      setImportBusy(false)
    }
  }

  async function importDraft(draft: ImportedProviderDraft) {
    setImportBusy(true)
    try {
      const provider = await createProvider.mutateAsync({
        display_name: draft.display_name,
        base_url: draft.base_url,
        kind: 'open_ai_compatible',
      })
      if (draft.credential_ref) {
        await createAccount.mutateAsync({
          providerId: provider.id,
          request: { label: '导入（引用）', credential_ref: draft.credential_ref },
        })
      }
      setImportDrafts((current) => (current ?? []).filter((item) => item.base_url !== draft.base_url))
      setSelectedId(provider.id)
    } catch {
      // 错误由 mutation 的 error 状态呈现。
    } finally {
      setImportBusy(false)
    }
  }

  if (providersQuery.isPending) return <Loading label="加载厂商…" />
  if (providersQuery.isError) {
    return <ErrorState message={providersQuery.error.message} onRetry={() => providersQuery.refetch()} />
  }

  return (
    <div className="app-page-content">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="app-card-title">Providers</h1>
          <p className="app-card-sub">
            厂商与账号管理；勾选模型后「编译为候选」即可被虚拟模型引用。密钥只提交一次，服务端落盘后不可回读。
          </p>
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            className="oc-btn-default"
            disabled={importBusy}
            onClick={() => void scanAgents()}
          >
            {importBusy ? '扫描中…' : '从 agent 配置导入'}
          </button>
          <button type="button" className="oc-btn-primary" onClick={() => setShowAdd((v) => !v)}>
            {showAdd ? '收起' : '添加厂商'}
          </button>
        </div>
      </div>

      {importDrafts && (
        <section className="app-card" aria-label="导入草稿">
          <h2 className="app-card-title">从本机 agent 配置导入</h2>
          {importDrafts.length === 0 ? (
            <p className="app-card-sub">
              没有发现可导入的上游端点（已装 agent 的配置文件里没有 base_url/api_key_env）。
              也可以先「添加厂商」手动配置。
            </p>
          ) : (
            <ul className="flex flex-col gap-1.5">
              {importDrafts.map((draft) => (
                <li
                  key={`${draft.base_url}-${draft.source}`}
                  className="flex items-center gap-2 flex-wrap"
                >
                  <span className="font-mono text-meta truncate flex-1">{draft.base_url}</span>
                  <span className="app-pill">{draft.credential_ref ?? '无 key 引用'}</span>
                  <span className="text-meta text-muted-foreground truncate">{draft.source}</span>
                  <button
                    type="button"
                    className="oc-btn-primary oc-btn-sm"
                    disabled={importBusy}
                    onClick={() => void importDraft(draft)}
                  >
                    创建
                  </button>
                </li>
              ))}
            </ul>
          )}
          <p className="app-card-sub">
            明文 key 不会导入；只有 `env:`/`file:` 引用会变成账号引用。无引用的厂商需要手动添加账号。
          </p>
        </section>
      )}

      {showAdd && (
        <AddProviderCard
          templates={templatesQuery.data ?? []}
          templatesLoading={templatesQuery.isPending}
          onDone={() => setShowAdd(false)}
        />
      )}

      {providersQuery.data.length === 0 ? (
        <EmptyState
          title="还没有厂商实例"
          hint="从模板添加 DeepSeek / OpenAI / Kimi / Ollama 等 20+ 厂商，或自定义 OpenAI 兼容端点。"
        />
      ) : (
        <div className="app-page-split">
          <section className="app-card" aria-label="厂商列表">
            <h2 className="app-card-title">厂商</h2>
            <ul className="flex flex-col gap-1.5">
              {providersQuery.data.map((provider) => (
                <li key={provider.id}>
                  <button
                    type="button"
                    className={
                      provider.id === selectedId
                        ? 'app-nav-item app-nav-item-active w-full'
                        : 'app-nav-item w-full'
                    }
                    onClick={() => setSelectedId(provider.id)}
                  >
                    <span className="flex-1 min-w-0 truncate">{provider.display_name}</span>
                    <span className={STATUS_PILL[statusOf(provider)]}>
                      {STATUS_LABEL[statusOf(provider)]}
                    </span>
                    <span className="text-meta text-muted-foreground whitespace-nowrap">
                      {provider.account_count} 账号 · {provider.candidate_count} 候选
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </section>
          {selected && <ProviderDetail provider={selected} />}
        </div>
      )}
    </div>
  )
}

function AddProviderCard({
  templates,
  templatesLoading,
  onDone,
}: {
  templates: ProviderTemplate[]
  templatesLoading: boolean
  onDone: () => void
}) {
  const create = useCreateProviderMutation()
  const [templateId, setTemplateId] = useState(templates[0]?.id ?? '')
  const [displayName, setDisplayName] = useState('')
  const [baseUrl, setBaseUrl] = useState('')

  const template = templates.find((t) => t.id === templateId)

  function submit() {
    if (!template) return
    create.mutate(
      {
        template_id: templateId,
        display_name: displayName || template.display_name,
        kind: template.kind,
        base_url: baseUrl || template.base_url,
        catalog_url: template.catalog_url ?? undefined,
      },
      { onSuccess: onDone },
    )
  }

  if (templatesLoading) return <Loading label="加载模板…" />

  return (
    <section className="app-card" aria-label="添加厂商">
      <h2 className="app-card-title">从模板添加</h2>
      <div className="app-form-row">
        <label className="app-field">
          <span>模板</span>
          <select
            className="app-select app-field-ctl"
            value={templateId}
            onChange={(event) => setTemplateId(event.target.value)}
          >
            {templates.map((t) => (
              <option key={t.id} value={t.id}>
                {t.display_name}
              </option>
            ))}
          </select>
        </label>
        <label className="app-field">
          <span>显示名（可选）</span>
          <input
            className="oc-field app-field-light app-field-ctl"
            value={displayName}
            placeholder={template?.display_name}
            onChange={(event) => setDisplayName(event.target.value)}
          />
        </label>
        <label className="app-field">
          <span>Base URL（可选，默认用模板）</span>
          <input
            className="oc-field app-field-light font-mono min-w-60"
            value={baseUrl}
            placeholder={template?.base_url || 'https://…/v1'}
            onChange={(event) => setBaseUrl(event.target.value)}
          />
        </label>
      </div>
      <div className="app-card-actions">
        <button
          type="button"
          className="oc-btn-primary"
          disabled={create.isPending || !template}
          onClick={submit}
        >
          {create.isPending ? '创建中…' : '创建'}
        </button>
        {create.isError && <span className="text-meta text-status-error">{create.error.message}</span>}
      </div>
    </section>
  )
}

function ProviderDetail({ provider }: { provider: ProviderInfo }) {
  const accountsQuery = useAccountsQuery(provider.id)
  const catalogQuery = useCatalogQuery(provider.id)
  const remove = useDeleteProviderMutation()
  const compile = useCompileCandidatesMutation()
  const [compileResult, setCompileResult] = useState<string | null>(null)

  function confirmDelete() {
    if (window.confirm(`删除厂商「${provider.display_name}」及其全部账号与凭据？`)) {
      remove.mutate(provider.id)
    }
  }

  function runCompile() {
    compile.mutate(
      {},
      {
        onSuccess: (response) => {
          setCompileResult(
            `已写入 ${response.candidate_ids.length} 个候选（共 ${response.candidates_total} 候选 / ${response.virtual_models_total} 虚拟模型）。重启服务后生效。`,
          )
        },
        onError: (error) => setCompileResult(`编译失败：${error.message}`),
      },
    )
  }

  return (
    <section className="app-card" aria-label="厂商详情">
      <div className="flex items-center gap-2 flex-wrap">
        <h2 className="app-card-title">{provider.display_name}</h2>
        <span className="text-meta text-muted-foreground font-mono">{provider.base_url}</span>
        <span className="flex-1" />
        <button
          type="button"
          className="oc-btn-destructive oc-btn-sm"
          onClick={confirmDelete}
          disabled={remove.isPending}
        >
          删除厂商
        </button>
      </div>

      <AccountsSection provider={provider} accountsQuery={accountsQuery} />
      <CatalogSection provider={provider} catalogQuery={catalogQuery} />

      <div className="app-card-actions">
        <button
          type="button"
          className="oc-btn-primary"
          onClick={runCompile}
          disabled={compile.isPending || provider.candidate_count === 0}
        >
          {compile.isPending ? '编译中…' : '编译为候选'}
        </button>
        <span className="text-meta text-muted-foreground">
          勾选模型 × 启用账号 → config 的 candidates（校验 + CAS + 审计）
        </span>
      </div>
      {compileResult && <p className="app-banner app-banner-ok">{compileResult}</p>}
      {provider.catalog_error && (
        <p className="app-banner app-banner-warn">目录刷新错误：{provider.catalog_error}</p>
      )}
    </section>
  )
}

function AccountsSection({
  provider,
  accountsQuery,
}: {
  provider: ProviderInfo
  accountsQuery: ReturnType<typeof useAccountsQuery>
}) {
  const create = useCreateAccountMutation()
  const verify = useVerifyAccountMutation()
  const remove = useDeleteAccountMutation()
  const [label, setLabel] = useState('')
  const [key, setKey] = useState('')
  const [proxy, setProxy] = useState('')
  const [headers, setHeaders] = useState('')
  const [showAdd, setShowAdd] = useState(false)

  function addAccount() {
    // header 文本：每行 `Name: Value`（非敏感值）。
    const extra = headers
      .split('\n')
      .map((line) => line.trim())
      .filter(Boolean)
      .map((line) => {
        const index = line.indexOf(':')
        return index > 0
          ? { name: line.slice(0, index).trim(), value: line.slice(index + 1).trim() }
          : null
      })
      .filter((item): item is { name: string; value: string } => item !== null)
    create.mutate(
      {
        providerId: provider.id,
        request: {
          label: label || '账号',
          key,
          proxy_url: proxy.trim() === '' ? undefined : proxy.trim(),
          extra_headers: extra.length === 0 ? undefined : extra,
        },
      },
      {
        onSuccess: () => {
          setLabel('')
          setKey('')
          setProxy('')
          setHeaders('')
          setShowAdd(false)
        },
      },
    )
  }

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <h3 className="text-ui-label font-medium">账号</h3>
        <span className="flex-1" />
        <button type="button" className="oc-btn-default oc-btn-sm" onClick={() => setShowAdd((v) => !v)}>
          {showAdd ? '取消' : '添加账号'}
        </button>
      </div>

      {showAdd && (
        <div className="flex flex-col gap-2">
          <div className="app-form-row">
            <input
              className="oc-field app-field-light app-field-ctl"
              placeholder="备注（可选）"
              value={label}
              onChange={(event) => setLabel(event.target.value)}
            />
            <input
              className="oc-field app-field-light font-mono min-w-60"
              type="password"
              placeholder="API Key（服务端落盘后不可回读）"
              value={key}
              onChange={(event) => setKey(event.target.value)}
            />
            <input
              className="oc-field app-field-light font-mono min-w-52"
              placeholder="代理（可选，http://127.0.0.1:7890）"
              value={proxy}
              onChange={(event) => setProxy(event.target.value)}
            />
          </div>
          <textarea
            className="oc-field app-field-light font-mono text-meta min-h-16"
            placeholder={'自定义 header（每行 Name: Value，非敏感值）\nHTTP-Referer: https://example.com'}
            value={headers}
            onChange={(event) => setHeaders(event.target.value)}
          />
          <div className="app-card-actions">
            <button
              type="button"
              className="oc-btn-primary oc-btn-sm"
              disabled={create.isPending || key.trim() === ''}
              onClick={addAccount}
            >
              保存
            </button>
            {create.isError && (
              <span className="text-meta text-status-error">{create.error.message}</span>
            )}
          </div>
        </div>
      )}

      {accountsQuery.isPending ? (
        <Loading />
      ) : accountsQuery.isError ? (
        <ErrorState message={accountsQuery.error.message} onRetry={() => accountsQuery.refetch()} />
      ) : accountsQuery.data && accountsQuery.data.length > 0 ? (
        <table className="app-table">
          <thead>
            <tr>
              <th>备注</th>
              <th>Key 指纹</th>
              <th>验证</th>
              <th>启用</th>
              <th aria-label="操作" />
            </tr>
          </thead>
          <tbody>
            {accountsQuery.data.map((account: AccountInfo) => (
              <tr key={account.id}>
                <td>{account.label}</td>
                <td className="font-mono">…{account.fingerprint}</td>
                <td>
                  {account.verify_status === 'ok' ? (
                    <span className="app-pill app-pill-ok">通过</span>
                  ) : account.verify_status ? (
                    <span className="app-pill app-pill-error">{account.verify_status}</span>
                  ) : (
                    <span className="text-meta text-muted-foreground">未验证</span>
                  )}
                </td>
                <td>{account.enabled ? '是' : '否'}</td>
                <td>
                  <div className="flex items-center gap-1.5 justify-end">
                    <button
                      type="button"
                      className="oc-btn-default oc-btn-sm"
                      disabled={verify.isPending}
                      onClick={() => verify.mutate({ providerId: provider.id, accountId: account.id })}
                    >
                      验证
                    </button>
                    <button
                      type="button"
                      className="oc-btn-destructive oc-btn-sm"
                      onClick={() => remove.mutate(account.id)}
                    >
                      删除
                    </button>
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : (
        <p className="text-meta text-muted-foreground">没有账号。添加一把 key 才能拉目录与编译候选。</p>
      )}
    </div>
  )
}

function CatalogSection({
  provider,
  catalogQuery,
}: {
  provider: ProviderInfo
  catalogQuery: ReturnType<typeof useCatalogQuery>
}) {
  const refresh = useRefreshCatalogMutation()
  const select = useSelectCatalogModelMutation()
  const [manualModel, setManualModel] = useState('')
  const [autoPattern, setAutoPattern] = useState('')
  const [autoBusy, setAutoBusy] = useState(false)
  const [autoMessage, setAutoMessage] = useState<string | null>(null)

  async function applyAutoInclude() {
    setAutoBusy(true)
    setAutoMessage(null)
    try {
      await api.providerAutoInclude(provider.id, autoPattern.trim())
      setAutoMessage(
        autoPattern.trim() === ''
          ? '已清除自动纳入规则。'
          : `已应用 \`${autoPattern.trim()}\`：匹配的新模型自动勾选。`,
      )
      await catalogQuery.refetch()
    } catch (error) {
      setAutoMessage(error instanceof Error ? error.message : String(error))
    } finally {
      setAutoBusy(false)
    }
  }

  function toggle(model: CatalogModel) {
    select.mutate({
      providerId: provider.id,
      request: { model_id: model.model_id, selected: !model.selected },
    })
  }

  function addManual() {
    if (manualModel.trim() === '') return
    select.mutate({
      providerId: provider.id,
      request: { model_id: manualModel.trim(), selected: true },
    })
    setManualModel('')
  }

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2 flex-wrap">
        <h3 className="text-ui-label font-medium">模型目录</h3>
        <span className="flex-1" />
        <input
          className="oc-field app-field-light font-mono h-7 min-w-52 text-meta"
          placeholder="手工加模型 id（本地推理）"
          value={manualModel}
          onChange={(event) => setManualModel(event.target.value)}
        />
        <button type="button" className="oc-btn-default oc-btn-sm" onClick={addManual} disabled={select.isPending}>
          添加
        </button>
        <button
          type="button"
          className="oc-btn-default oc-btn-sm"
          onClick={() => refresh.mutate(provider.id)}
          disabled={refresh.isPending || provider.account_count === 0}
        >
          {refresh.isPending ? '拉取中…' : '刷新目录'}
        </button>
      </div>

      <div className="app-form-row">
        <input
          className="oc-field app-field-light font-mono h-7 min-w-52 text-meta"
          placeholder="自动纳入通配符（如 gpt-*），新模型自动勾选"
          value={autoPattern}
          onChange={(event) => setAutoPattern(event.target.value)}
        />
        <button
          type="button"
          className="oc-btn-default oc-btn-sm"
          disabled={autoBusy}
          onClick={() => void applyAutoInclude()}
        >
          {autoBusy ? '应用中…' : '应用'}
        </button>
        {autoMessage && (
          <span className="text-meta text-muted-foreground">{autoMessage}</span>
        )}
      </div>

      {catalogQuery.isPending ? (
        <Loading />
      ) : catalogQuery.isError ? (
        <ErrorState message={catalogQuery.error.message} onRetry={() => catalogQuery.refetch()} />
      ) : catalogQuery.data && catalogQuery.data.models.length > 0 ? (
        <ul className="flex flex-col gap-1 max-h-72 overflow-y-auto">
          {catalogQuery.data.models.map((model) => (
            <li key={model.model_id}>
              <label className="flex items-center gap-2 text-meta cursor-pointer">
                <input
                  type="checkbox"
                  checked={model.selected}
                  onChange={() => toggle(model)}
                />
                <span className="font-mono">{model.model_id}</span>
                {model.display_name !== model.model_id && (
                  <span className="text-muted-foreground">{model.display_name}</span>
                )}
              </label>
            </li>
          ))}
        </ul>
      ) : (
        <p className="text-meta text-muted-foreground">
          目录为空。有账号时点「刷新目录」，或手工添加模型 id。
        </p>
      )}
    </div>
  )
}
