/**
 * 供应商编辑器 · 账号分区（紧凑行，替代原「账号」页签）。
 *
 * - 每行只显示指纹、备注、协议/代理摘要、配额摘要与状态；
 * - 新增密钥：label + 明文 key 一次性提交，提交即清空，可选代理 / header 折叠；
 * - 编辑：label、轮换 key、代理、base_url 覆盖、header、启用开关；
 * - PATCH 语义：null = 不改；proxy / base_url 传空串 = 清除；
 * - 所有写操作走现有 useCreateAccountMutation / useUpdateAccountMutation /
 *   useDeleteAccountMutation / useVerifyAccountMutation，账号改动即时保存。
 */
import { Fragment, useState } from 'react'
import type { AccountInfo, ProviderInfo, QuotaWindowInfo } from '../../api/types'
import {
  useAccountsQuery,
  useCreateAccountMutation,
  useDeleteAccountMutation,
  useQuotasQuery,
  useUpdateAccountMutation,
  useVerifyAccountMutation,
} from '../../queries/admin'
import { ErrorState, Loading } from '../../components/common/QueryState'
import {
  errorMessageText,
  parseHeaders,
  providerKindLabel,
} from './shared'

/** 账号窗口摘要：按候选窗口独立展示，不求和。 */
function quotaSummary(windows: QuotaWindowInfo[]): string | null {
  if (windows.length === 0) return null
  const first = windows[0]
  const remaining = first.remaining === null ? '未知' : String(first.remaining)
  const limit = first.limit === null ? '' : ` / ${first.limit}`
  const extra = windows.length > 1 ? ` +${windows.length - 1}` : ''
  return `${remaining}${limit}${extra}`
}

/** 账号编辑面板（label / 轮换 key / 代理 / 端点覆盖 / header / 启用）。 */
function AccountEditPanel({
  account,
  onDone,
}: {
  account: AccountInfo
  onDone: () => void
}) {
  const update = useUpdateAccountMutation()
  const [label, setLabel] = useState(account.label)
  const [proxy, setProxy] = useState(account.proxy_url ?? '')
  const [baseUrlOverride, setBaseUrlOverride] = useState(account.base_url_override ?? '')
  const [headers, setHeaders] = useState(
    (account.extra_headers ?? [])
      .map((header) => `${header.name}: ${header.value}`)
      .join('\n'),
  )
  const [key, setKey] = useState('')
  const [enabled, setEnabled] = useState(account.enabled)

  function submit() {
    // 提交即清空：明文 key 不在 state 里滞留（失败时需重新输入）。
    const submittedKey = key.trim()
    setKey('')
    update.mutate(
      {
        accountId: account.id,
        request: {
          label: label.trim() === '' ? null : label.trim(),
          proxy_url: proxy.trim(),
          base_url_override: baseUrlOverride.trim(),
          extra_headers: parseHeaders(headers),
          key: submittedKey === '' ? null : submittedKey,
          enabled,
        },
      },
      { onSuccess: onDone },
    )
  }

  return (
    <div className="flex flex-col gap-2 p-3 rounded-lg bg-[color-mix(in_srgb,var(--surface-foreground)_3%,transparent)]">
      <div className="app-form-row">
        <input
          className="oc-field app-field-light app-field-ctl"
          placeholder="备注"
          value={label}
          onChange={(event) => setLabel(event.target.value)}
          aria-label="账号备注"
        />
        <input
          className="oc-field app-field-light font-mono min-w-52"
          placeholder="代理（可选，留空清除）"
          value={proxy}
          onChange={(event) => setProxy(event.target.value)}
          aria-label="代理 URL"
        />
        <input
          className="oc-field app-field-light font-mono min-w-52"
          placeholder="Base URL 覆盖（可选，留空用厂商的）"
          value={baseUrlOverride}
          onChange={(event) => setBaseUrlOverride(event.target.value)}
          aria-label="Base URL 覆盖"
        />
      </div>
      <textarea
        className="oc-field app-field-light font-mono text-meta min-h-16"
        placeholder="自定义 header（每行 Name: Value，非敏感值；保存 = 全量替换）"
        value={headers}
        onChange={(event) => setHeaders(event.target.value)}
        aria-label="自定义 header"
      />
      <input
        className="oc-field app-field-light font-mono min-w-60"
        type="password"
        placeholder="轮换 key（可选；留空不换）"
        value={key}
        onChange={(event) => setKey(event.target.value)}
        aria-label="轮换 API key"
      />
      <label className="app-scope-check">
        <input
          type="checkbox"
          checked={enabled}
          onChange={(event) => setEnabled(event.target.checked)}
        />
        <span className="text-meta">启用该账号</span>
      </label>
      <div className="app-card-actions">
        <button
          type="button"
          className="oc-btn-primary oc-btn-sm"
          disabled={update.isPending}
          onClick={submit}
        >
          {update.isPending ? '保存中…' : '保存修改'}
        </button>
        <button type="button" className="oc-btn-default oc-btn-sm" onClick={onDone}>
          取消
        </button>
        {update.isError && (
          <span className="text-meta text-status-error">
            {errorMessageText(update.error)}
          </span>
        )}
      </div>
    </div>
  )
}

export function ProviderAccountsSection({ provider }: { provider: ProviderInfo }) {
  const accountsQuery = useAccountsQuery(provider.id)
  const quotasQuery = useQuotasQuery()
  const create = useCreateAccountMutation()
  const verify = useVerifyAccountMutation()
  const remove = useDeleteAccountMutation()

  const [showAdd, setShowAdd] = useState(false)
  const [label, setLabel] = useState('')
  const [key, setKey] = useState('')
  const [proxy, setProxy] = useState('')
  const [headers, setHeaders] = useState('')
  const [editingId, setEditingId] = useState<string | null>(null)
  const [verifyingId, setVerifyingId] = useState<string | null>(null)
  const [rowError, setRowError] = useState<string | null>(null)

  const accounts = accountsQuery.data ?? []
  const quotaItems = quotasQuery.data?.items ?? []

  function resetAddForm() {
    setLabel('')
    setKey('')
    setProxy('')
    setHeaders('')
    setRowError(null)
  }

  function toggleAdd() {
    if (showAdd) {
      // 收起即清空：明文 key 不在隐藏的表单里滞留。
      resetAddForm()
    } else {
      // 展开时清掉上一次失败的错误提示。
      create.reset()
    }
    setShowAdd(!showAdd)
  }

  function addAccount() {
    // 提交即清空：明文 key 只在表单瞬时持有，失败时需重新输入。
    const submittedKey = key.trim()
    setKey('')
    setRowError(null)
    const parsedHeaders = parseHeaders(headers)
    create.mutate(
      {
        providerId: provider.id,
        request: {
          label: label.trim() === '' ? '账号' : label.trim(),
          key: submittedKey,
          proxy_url: proxy.trim() === '' ? undefined : proxy.trim(),
          extra_headers: parsedHeaders.length === 0 ? undefined : parsedHeaders,
        },
      },
      {
        onSuccess: () => {
          resetAddForm()
          setShowAdd(false)
        },
        onError: (error) => setRowError(errorMessageText(error)),
      },
    )
  }

  function verifyAccount(account: AccountInfo) {
    setRowError(null)
    setVerifyingId(account.id)
    verify.mutate(
      { providerId: provider.id, accountId: account.id },
      {
        onError: (error) => setRowError(errorMessageText(error)),
        onSettled: () => setVerifyingId(null),
      },
    )
  }

  function deleteAccount(account: AccountInfo) {
    if (!window.confirm(`删除账号「${account.label}」及其凭据文件？`)) return
    setRowError(null)
    remove.mutate(account.id, {
      onError: (error) => setRowError(errorMessageText(error)),
    })
  }

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2 flex-wrap">
        <h3 className="text-ui-label font-medium">账号</h3>
        <span className="text-meta text-muted-foreground">{accounts.length} 个</span>
        <span className="flex-1" />
        <button
          type="button"
          className="oc-btn-default oc-btn-sm"
          onClick={toggleAdd}
          aria-expanded={showAdd}
        >
          {showAdd ? '取消' : '添加密钥'}
        </button>
      </div>

      {showAdd && (
        <div className="flex flex-col gap-2 p-3 rounded-lg bg-[color-mix(in_srgb,var(--surface-foreground)_3%,transparent)]">
          <div className="app-form-row">
            <input
              className="oc-field app-field-light app-field-ctl"
              placeholder="备注（可选，如「主账号」）"
              value={label}
              onChange={(event) => setLabel(event.target.value)}
              aria-label="新账号备注"
            />
            <input
              className="oc-field app-field-light font-mono min-w-60"
              type="password"
              placeholder="API Key（只提交一次）"
              value={key}
              onChange={(event) => setKey(event.target.value)}
              aria-label="新账号 API Key"
            />
          </div>
          <details>
            <summary className="text-meta text-muted-foreground cursor-pointer">
              代理 / 自定义 header（可选）
            </summary>
            <div className="flex flex-col gap-2 pt-2">
              <input
                className="oc-field app-field-light font-mono min-w-52"
                placeholder="代理（可选，http://127.0.0.1:7890）"
                value={proxy}
                onChange={(event) => setProxy(event.target.value)}
                aria-label="新账号代理 URL"
              />
              <textarea
                className="oc-field app-field-light font-mono text-meta min-h-16"
                placeholder={'自定义 header（每行 Name: Value，非敏感值）\nHTTP-Referer: https://example.com'}
                value={headers}
                onChange={(event) => setHeaders(event.target.value)}
                aria-label="新账号自定义 header"
              />
            </div>
          </details>
          <div className="app-card-actions">
            <button
              type="button"
              className="oc-btn-primary oc-btn-sm"
              disabled={create.isPending || key.trim() === ''}
              onClick={addAccount}
            >
              {create.isPending ? '保存中…' : '保存密钥'}
            </button>
          </div>
          {create.isError && (
            <span className="text-meta text-status-error">
              {errorMessageText(create.error)}
            </span>
          )}
        </div>
      )}

      {rowError !== null && (
        <p className="app-banner app-banner-err text-meta">{rowError}</p>
      )}

      {accountsQuery.isPending ? (
        <Loading />
      ) : accountsQuery.isError ? (
        <ErrorState
          message={accountsQuery.error.message}
          onRetry={() => accountsQuery.refetch()}
        />
      ) : accounts.length > 0 ? (
        <ul className="flex flex-col gap-1" aria-label="账号">
          {accounts.map((account) => {
            const editing = editingId === account.id
            const quota = quotaSummary(
              quotaItems.filter(
                (item) =>
                  item.account_id === account.id &&
                  item.provider_id === provider.id,
              ),
            )
            return (
              <Fragment key={account.id}>
                <li className="flex flex-col gap-1 rounded-lg border border-border px-2.5 py-2">
                  <div className="flex items-center gap-2 flex-wrap">
                    <span
                      className="font-mono text-ui-label shrink-0"
                      title="密钥指纹（明文不可回读）"
                    >
                      …{account.fingerprint}
                    </span>
                    <span className="truncate text-ui-label">{account.label}</span>
                    <span
                      className={
                        account.enabled
                          ? 'app-pill app-pill-ok'
                          : 'app-pill app-pill-warn'
                      }
                    >
                      {account.enabled ? '使用中' : '已停用'}
                    </span>
                    {quota !== null && (
                      <span
                        className="app-pill"
                        title="配额按候选窗口观测，独立，不求和"
                      >
                        配额 {quota}
                      </span>
                    )}
                    <span className="flex-1" />
                    <div className="flex items-center gap-1.5">
                      <button
                        type="button"
                        className="oc-btn-default oc-btn-sm"
                        onClick={() => setEditingId(editing ? null : account.id)}
                        aria-expanded={editing}
                      >
                        {editing ? '收起' : '编辑'}
                      </button>
                      <button
                        type="button"
                        className="oc-btn-default oc-btn-sm"
                        disabled={verify.isPending}
                        onClick={() => verifyAccount(account)}
                      >
                        {verifyingId === account.id ? '验证中…' : '验证'}
                      </button>
                      <button
                        type="button"
                        className="oc-btn-destructive oc-btn-sm"
                        disabled={remove.isPending}
                        onClick={() => deleteAccount(account)}
                      >
                        删除
                      </button>
                    </div>
                  </div>
                  <div className="flex items-center gap-x-3 gap-y-1 flex-wrap text-meta text-muted-foreground">
                    <span>协议：{providerKindLabel(provider.kind)}</span>
                    {account.proxy_url !== null && (
                      <span className="truncate font-mono" title={account.proxy_url}>
                        代理 {account.proxy_url}
                      </span>
                    )}
                    {account.base_url_override !== null && (
                      <span
                        className="truncate font-mono"
                        title={account.base_url_override}
                      >
                        端点覆盖 {account.base_url_override}
                      </span>
                    )}
                    <span>
                      验证：
                      {account.verify_status === 'ok' ? (
                        <span className="text-status-success">通过</span>
                      ) : account.verify_status ? (
                        <span className="text-status-error">{account.verify_status}</span>
                      ) : (
                        '未验证'
                      )}
                    </span>
                  </div>
                </li>
                {editing && (
                  <li>
                    <AccountEditPanel
                      account={account}
                      onDone={() => setEditingId(null)}
                    />
                  </li>
                )}
              </Fragment>
            )
          })}
        </ul>
      ) : (
        <p className="text-meta text-muted-foreground">
          还没有账号。添加一把 key 才能拉取目录、编译候选与测试端点。
        </p>
      )}
    </div>
  )
}
