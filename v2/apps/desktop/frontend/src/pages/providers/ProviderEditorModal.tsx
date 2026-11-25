/**
 * 供应商编辑器（单页分区，替代原「账号 / 目录 / 配额 / 高级」四页签）。
 *
 * 布局对齐 Magpie 式供应商编辑器：头部厂商标识 + 名称 + 状态 + 启用开关；
 * 单页滚动分区 A 基本信息 / B 账号 / C 模型 / D 端点 / E 高级（默认折叠）。
 * - 底部「保存」只提交 provider 级字段（名称 / 类型 / 启用 / base_url / catalog_url）；
 * - 账号与模型操作走现有 mutation，即时保存，不假装需要点底部保存；
 * - 类型可改：保存后后端按新类型重编译，并把仍是旧默认协议的候选迁到新协议。
 */
import { useState } from 'react'
import type { ProviderInfo } from '../../api/types'
import {
  useAccountsQuery,
  useCompileCandidatesMutation,
  useDeleteProviderMutation,
  useProvidersQuery,
  useUpdateProviderMutation,
  useVerifyAccountMutation,
} from '../../queries/admin'
import { Modal } from '../../components/common/Modal'
import { Icon } from '../../components/common/Icon'
import { errorMessage } from '../../components/common/QueryState'
import { useAppStore } from '../../stores/appStore'
import { ProviderAccountsSection } from './ProviderAccountsSection'
import { ProviderModelsSection } from './ProviderModelsSection'
import {
  PROVIDER_KIND_LABEL,
  PROVIDER_STATUS_LABEL,
  PROVIDER_STATUS_PILL,
  errorMessageText,
  providerKindLabel,
  statusOf,
} from './shared'

/** 类型下拉：值就是候选 `provider` 的默认值，保存后按新类型重编译。 */
const PROVIDER_KIND_OPTIONS: Array<{ value: ProviderInfo['kind']; label: string }> = [
  { value: 'open_ai_compatible', label: PROVIDER_KIND_LABEL.open_ai_compatible },
  { value: 'responses', label: PROVIDER_KIND_LABEL.responses },
  { value: 'anthropic', label: PROVIDER_KIND_LABEL.anthropic },
]

function providerInitial(name: string): string {
  return name.trim().charAt(0).toUpperCase() || '?'
}

function trimBase(url: string): string {
  return url.trim().replace(/\/+$/, '')
}

/** base 已以 /v1 结尾时不再重复追加。 */
function protocolPath(base: string, path: string): string {
  return base.endsWith('/v1') ? `${base}${path}` : `${base}/v1${path}`
}

export function ProviderEditorModal({
  provider,
  onClose,
}: {
  provider: ProviderInfo
  onClose: () => void
}) {
  const update = useUpdateProviderMutation()
  const status = statusOf(provider)

  const [displayName, setDisplayName] = useState(provider.display_name)
  const [kind, setKind] = useState(provider.kind)
  const [enabled, setEnabled] = useState(provider.enabled)
  const [baseUrl, setBaseUrl] = useState(provider.base_url)
  const [catalogUrl, setCatalogUrl] = useState(provider.catalog_url ?? '')
  const [confirmDelete, setConfirmDelete] = useState(false)
  // 名称/启用/端点不是即时保存：关闭前必须给一次明确确认，不能静默丢。
  const [confirmClose, setConfirmClose] = useState(false)

  const catalogChanged =
    catalogUrl.trim() !== '' && catalogUrl.trim() !== (provider.catalog_url ?? '')
  const dirty =
    displayName.trim() !== provider.display_name ||
    kind !== provider.kind ||
    enabled !== provider.enabled ||
    baseUrl.trim() !== provider.base_url ||
    catalogChanged
  const nameInvalid = displayName.trim() === ''
  const baseUrlInvalid = baseUrl.trim() === ''
  const catalogClearBlocked =
    provider.catalog_url !== null && catalogUrl.trim() === ''
  // 连通性测试走已保存的 provider 记录：端点/类型有未保存修改时先保存再测。
  const endpointsDirty =
    baseUrl.trim() !== provider.base_url || catalogChanged || kind !== provider.kind

  function requestClose() {
    if (dirty) {
      setConfirmClose(true)
      return
    }
    onClose()
  }

  function save() {
    if (nameInvalid || baseUrlInvalid || catalogClearBlocked) return
    update.mutate({
      id: provider.id,
      request: {
        display_name: displayName.trim(),
        kind,
        enabled,
        base_url: baseUrl.trim(),
        catalog_url: catalogChanged ? catalogUrl.trim() : undefined,
      },
    })
  }

  return (
    <>
      <Modal
        size="wide"
        title={
          <>
            <span className="truncate">{displayName.trim() || provider.display_name}</span>
            <span className={PROVIDER_STATUS_PILL[status]}>
              {PROVIDER_STATUS_LABEL[status]}
            </span>
            {!enabled && <span className="app-pill app-pill-warn">停用</span>}
            <label className="app-scope-check ml-auto">
              <input
                type="checkbox"
                checked={enabled}
                onChange={(event) => setEnabled(event.target.checked)}
                aria-label="启用供应商"
              />
              <span className="text-meta">启用</span>
            </label>
          </>
        }
        subtitle={`${providerKindLabel(provider.kind)} · ${provider.base_url}`}
        leading={
          <span className="app-provider-icon" aria-hidden>
            {providerInitial(displayName.trim() || provider.display_name)}
          </span>
        }
        onClose={requestClose}
        footer={
          confirmClose ? (
            <>
              <span className="text-meta text-status-warning">有未保存的供应商修改</span>
              <span className="flex-1" />
              <button
                type="button"
                className="oc-btn-default"
                onClick={() => setConfirmClose(false)}
              >
                继续编辑
              </button>
              <button type="button" className="oc-btn-destructive" onClick={onClose}>
                放弃并关闭
              </button>
            </>
          ) : (
            <>
              <button
                type="button"
                className="oc-btn-destructive oc-btn-sm"
                onClick={() => setConfirmDelete(true)}
              >
                <Icon name="remove" className="size-4" />
                删除供应商
              </button>
              <span className="flex-1" />
              <span className="text-meta text-muted-foreground">
                {dirty ? '有未保存修改' : update.isSuccess ? '已保存' : ''}
              </span>
              <button type="button" className="oc-btn-default" onClick={requestClose}>
                关闭
              </button>
              <button
                type="button"
                className="oc-btn-primary"
                disabled={
                  update.isPending || nameInvalid || baseUrlInvalid || catalogClearBlocked
                }
                onClick={save}
              >
                {update.isPending ? '保存中…' : '保存'}
              </button>
            </>
          )
        }
      >
        <div className="flex flex-col gap-4">
          {provider.catalog_error !== null && (
            <div className="app-banner app-banner-warn">
              目录刷新错误：{provider.catalog_error}
            </div>
          )}
          {update.isError && (
            <div className="app-banner app-banner-err">
              保存失败：{errorMessage(update.error)}
            </div>
          )}

          {/* A. 基本信息 */}
          <section className="flex flex-col gap-2" aria-labelledby="provider-basic-title">
            <h3 id="provider-basic-title" className="text-ui-label font-medium">
              基本信息
            </h3>
            <div className="app-form-row items-start">
              <label className="app-field">
                <span>名称</span>
                <input
                  className="oc-field app-field-light app-field-ctl"
                  value={displayName}
                  onChange={(event) => setDisplayName(event.target.value)}
                  aria-label="供应商名称"
                  aria-invalid={nameInvalid}
                />
                {nameInvalid && <span className="app-cell-error">名称不能为空。</span>}
              </label>
              <div className="app-field">
                <span>ID</span>
                <code className="app-inline-code max-w-64 truncate" title={provider.id}>
                  {provider.id}
                </code>
              </div>
              <label className="app-field">
                <span>类型</span>
                <select
                  className="app-select-sm"
                  value={kind}
                  aria-label="供应商类型"
                  title="类型决定目录探测鉴权与新建候选的默认协议；保存后仍是旧默认协议的候选会一起迁移，显式改过协议的模型保持不变。"
                  onChange={(event) => setKind(event.target.value as ProviderInfo['kind'])}
                >
                  {PROVIDER_KIND_OPTIONS.map((option) => (
                    <option key={option.value} value={option.value}>
                      {option.label}
                    </option>
                  ))}
                </select>
              </label>
            </div>
          </section>

          {/* B. 账号 */}
          <section
            className="flex flex-col gap-2 border-t border-border pt-4"
          >
            <ProviderAccountsSection provider={provider} />
          </section>

          {/* C. 模型 */}
          <section
            className="flex flex-col gap-2 border-t border-border pt-4"
          >
            <ProviderModelsSection provider={provider} />
          </section>

          {/* D. 端点 */}
          <section
            className="flex flex-col gap-2 border-t border-border pt-4"
            aria-labelledby="provider-endpoints-title"
          >
            <EndpointSection
              provider={provider}
              kind={kind}
              baseUrl={baseUrl}
              onBaseUrlChange={setBaseUrl}
              catalogUrl={catalogUrl}
              onCatalogUrlChange={setCatalogUrl}
              baseUrlInvalid={baseUrlInvalid}
              catalogClearBlocked={catalogClearBlocked}
              endpointsDirty={endpointsDirty}
            />
          </section>

          {/* E. 高级 */}
          <details className="flex flex-col gap-2 border-t border-border pt-4">
            <summary className="cursor-pointer text-ui-label font-medium">高级</summary>
            <div className="flex flex-col gap-4 pt-3">
              <CompileSection provider={provider} />
            </div>
          </details>
        </div>
      </Modal>

      {confirmDelete && (
        <DeleteProviderDialog
          provider={provider}
          onClose={() => setConfirmDelete(false)}
          onDeleted={onClose}
        />
      )}
    </>
  )
}

/**
 * 端点分区：只读展示**上游厂商端点**（并非本机网关入口）。
 * 测试按钮复用账号验证（取第一个启用账号），没有账号时明确提示。
 */
function EndpointSection({
  provider,
  kind,
  baseUrl,
  onBaseUrlChange,
  catalogUrl,
  onCatalogUrlChange,
  baseUrlInvalid,
  catalogClearBlocked,
  endpointsDirty,
}: {
  provider: ProviderInfo
  kind: ProviderInfo['kind']
  baseUrl: string
  onBaseUrlChange: (value: string) => void
  catalogUrl: string
  onCatalogUrlChange: (value: string) => void
  baseUrlInvalid: boolean
  catalogClearBlocked: boolean
  endpointsDirty: boolean
}) {
  const accountsQuery = useAccountsQuery(provider.id)
  const verify = useVerifyAccountMutation()
  const showToast = useAppStore((state) => state.showToast)
  const [message, setMessage] = useState<{ text: string; ok: boolean } | null>(null)

  // 用输入框里的值而不是 provider 快照：保存前也能看到端点真实形态。
  const base = trimBase(baseUrl)
  const firstEnabledAccount =
    accountsQuery.data?.find((account) => account.enabled) ?? null

  const rows: Array<{ label: string; value: string; note: string }> = [
    {
      label: '厂商 Base URL',
      value: baseUrl,
      note: '账号请求的基础地址；账号可单独覆盖。',
    },
  ]
  if (kind === 'anthropic') {
    rows.push({
      label: 'Anthropic Messages',
      value: protocolPath(base, '/messages'),
      note: '上游厂商端点，请求头使用 x-api-key 与 anthropic-version。本机网关的 /anthropic 入口是另一条地址。',
    })
  } else if (kind === 'responses') {
    rows.push({
      label: 'OpenAI Responses',
      value: protocolPath(base, '/responses'),
      note: '上游厂商端点。本机网关的 /responses 入口是另一条地址，不要混用。',
    })
  } else {
    rows.push({
      label: 'OpenAI 兼容 · Chat Completions',
      value: `${base}/chat/completions`,
      note: '上游厂商端点；base_url 通常已含 /v1（以 /v1 结尾时不会重复追加）。本机网关的 /v1 入口不是上游地址。',
    })
  }
  if (catalogUrl.trim() !== '') {
    rows.push({
      label: '模型目录端点',
      value: catalogUrl.trim(),
      note: '目录刷新使用；未配置时默认 base_url + /models。',
    })
  }

  async function copyText(text: string) {
    try {
      if (!navigator.clipboard?.writeText) throw new Error('clipboard unavailable')
      await navigator.clipboard.writeText(text)
      showToast('toast.copied')
    } catch {
      showToast('复制失败，请手动选择地址复制', false)
    }
  }

  function test() {
    if (firstEnabledAccount === null) {
      setMessage({ text: '需先添加并启用一个账号，才能测试上游连通性。', ok: false })
      return
    }
    setMessage(null)
    verify.mutate(
      { providerId: provider.id, accountId: firstEnabledAccount.id },
      {
        onSuccess: (account) =>
          setMessage(
            account.verify_status === 'ok'
              ? { text: '验证通过。', ok: true }
              : { text: `验证结果：${account.verify_status ?? '未知'}`, ok: false },
          ),
        onError: (error) =>
          setMessage({ text: `验证失败：${errorMessageText(error)}`, ok: false }),
      },
    )
  }

  return (
    <>
      <div className="flex items-center gap-2 flex-wrap">
        <h3 id="provider-endpoints-title" className="text-ui-label font-medium">
          端点
        </h3>
        <span className="text-meta text-muted-foreground">
          上游厂商端点，不是本机网关地址
        </span>
        <span className="flex-1" />
        <button
          type="button"
          className="oc-btn-default oc-btn-sm"
          disabled={verify.isPending || firstEnabledAccount === null || endpointsDirty}
          title={
            firstEnabledAccount === null
              ? '先添加并启用一个账号'
              : endpointsDirty
                ? '端点有未保存修改，先保存再测试'
                : undefined
          }
          onClick={test}
        >
          {verify.isPending ? '测试中…' : '测试连通性'}
        </button>
      </div>
      <div className="app-form-row items-start">
        <label className="app-field flex-1 min-w-60">
          Base URL
          <input
            className="oc-field app-field-light font-mono"
            value={baseUrl}
            onChange={(event) => onBaseUrlChange(event.target.value)}
            aria-label="供应商 Base URL"
            aria-invalid={baseUrlInvalid}
          />
          {baseUrlInvalid && <span className="app-cell-error">Base URL 不能为空。</span>}
        </label>
        <label className="app-field flex-1 min-w-60">
          目录端点（可选）
          <input
            className="oc-field app-field-light font-mono"
            value={catalogUrl}
            placeholder="https://…/models"
            onChange={(event) => onCatalogUrlChange(event.target.value)}
            aria-label="模型目录端点"
          />
        </label>
      </div>
      {catalogClearBlocked && (
        <p className="app-cell-error">目录端点暂不支持清空；请恢复原值后再保存。</p>
      )}
      {message !== null && (
        <p
          className={
            message.ok
              ? 'app-banner app-banner-info text-meta'
              : 'app-banner app-banner-err text-meta'
          }
        >
          {message.text}
        </p>
      )}
      <ul className="flex flex-col gap-1">
        {rows.map((row) => (
          <li
            key={row.label}
            className="flex items-start gap-2 flex-wrap rounded-lg border border-border px-2.5 py-2"
            title={row.note}
          >
            <div className="flex flex-1 min-w-0 flex-col gap-0.5">
              <span className="text-ui-label">{row.label}</span>
              <span className="font-mono text-meta text-foreground break-all">
                {row.value}
              </span>
            </div>
            <button
              type="button"
              className="oc-btn-default oc-btn-sm"
              onClick={() => void copyText(row.value)}
            >
              <Icon name="copy" className="size-3.5" />
              复制
            </button>
          </li>
        ))}
      </ul>
    </>
  )
}

/**
 * 重新编译候选：把**全部厂商**已勾选的目录模型写回 config 的 candidates。
 *
 * 目录 / 账号 / 类型变更都会自动编译；这里只用于自动编译失败后的重试，
 * 或手改过 TOML / 数据库后的手动重建。
 */
function CompileSection({ provider }: { provider: ProviderInfo }) {
  const compile = useCompileCandidatesMutation()
  // 编译作用于**全部厂商**：门控必须看全局已选模型数，而不是当前厂商的计数。
  const providersQuery = useProvidersQuery()
  const selectedTotal = (providersQuery.data ?? []).reduce(
    (sum, item) => sum + item.candidate_count,
    0,
  )

  return (
    <div className="flex flex-col gap-2">
      <div className="app-card-actions">
        <button
          type="button"
          className="oc-btn-default oc-btn-sm"
          onClick={() => compile.mutate({})}
          disabled={compile.isPending || selectedTotal === 0}
          title={
            selectedTotal === 0
              ? '先在任何厂商里勾选模型'
              : '把全部厂商已勾选的模型重新写回配置（编译候选，CAS + 审计）；目录/账号变更会自动编译，这里用于失败重试或手改后重建。'
          }
        >
          {compile.isPending ? '编译中…' : '重新编译候选'}
        </button>
        <span className="text-meta text-muted-foreground">
          已选 {selectedTotal} 个模型（本厂商 {provider.candidate_count} 个）
        </span>
      </div>
      {compile.isError && (
        <div className="app-banner app-banner-err">
          <span className="font-medium">编译失败。</span>
          <span className="text-meta">{compile.error.message}</span>
        </div>
      )}
      {compile.isSuccess && compile.data && (
        <div className="app-banner app-banner-ok">
          已写入 {compile.data.candidate_ids.length} 个候选（共{' '}
          {compile.data.candidates_total} 候选 / {compile.data.virtual_models_total} 虚拟模型），已立即生效。
        </div>
      )}
    </div>
  )
}

function DeleteProviderDialog({
  provider,
  onClose,
  onDeleted,
}: {
  provider: ProviderInfo
  onClose: () => void
  onDeleted: () => void
}) {
  const remove = useDeleteProviderMutation()
  const [error, setError] = useState<string | null>(null)

  async function confirm() {
    setError(null)
    try {
      await remove.mutateAsync(provider.id)
      onDeleted()
    } catch (cause) {
      setError(errorMessage(cause))
    }
  }

  return (
    <Modal
      title="删除供应商"
      subtitle={provider.display_name}
      onClose={onClose}
      footer={
        <>
          <span className="flex-1" />
          <button
            type="button"
            className="oc-btn-default"
            disabled={remove.isPending}
            onClick={onClose}
          >
            取消
          </button>
          <button
            type="button"
            className="oc-btn-destructive"
            disabled={remove.isPending}
            onClick={() => void confirm()}
          >
            {remove.isPending ? '删除中…' : '确认删除'}
          </button>
        </>
      }
    >
      <div className="flex flex-col gap-2">
        <p>
          删除「{provider.display_name}」会级联删除其 {provider.account_count} 个账号与凭据文件、
          模型目录与配额观测，不可撤销。
        </p>
        <div className="app-banner app-banner-warn">
          编译影响：已应用（编译）到配置中的候选不会自动清理；删除后需重新勾选目录并
          「应用模型选择」，避免虚拟模型引用失效候选。
        </div>
        <p className="text-meta text-muted-foreground">
          当前候选数：{provider.candidate_count}。只想暂时停用请用头部的「启用」开关；
          删除后只能重新创建。
        </p>
        {error !== null && <div className="app-banner app-banner-err">{error}</div>}
      </div>
    </Modal>
  )
}
