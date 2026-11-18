/**
 * Tokens（§7.3）：创建/撤销/一次性明文/scope。
 */
import { Fragment, useState } from 'react'
import {
  useTokensQuery,
  useCreateTokenMutation,
  useRevokeTokenMutation,
  useUpdateTokenLimitsMutation,
} from '../queries/admin'
import { QueryState, errorMessage } from '../components/common/QueryState'
import { Icon } from '../components/common/Icon'
import { useAppStore } from '../stores/appStore'

const SCOPES = [
  { id: 'inference', label: 'inference（数据面）' },
  { id: 'admin_read', label: 'admin:read（管理读）' },
  { id: 'admin_write', label: 'admin:write（管理写）' },
] as const

export function TokensPage() {
  const tokensQuery = useTokensQuery()
  const createMutation = useCreateTokenMutation()
  const revokeMutation = useRevokeTokenMutation()
  const limitsMutation = useUpdateTokenLimitsMutation()
  const showToast = useAppStore((s) => s.showToast)

  const [name, setName] = useState('')
  const [scopes, setScopes] = useState<Array<'inference' | 'admin_read' | 'admin_write'>>(['inference'])
  const [created, setCreated] = useState<string | null>(null)
  const [confirming, setConfirming] = useState<string | null>(null)
  // key 限额编辑（v3 P5-B）：tokenId 非空时展开行内表单。
  const [limitsFor, setLimitsFor] = useState<string | null>(null)
  const [limitWindow, setLimitWindow] = useState('day')
  const [limitTokens, setLimitTokens] = useState('')
  const [limitCost, setLimitCost] = useState('')
  const [limitModels, setLimitModels] = useState('')

  const create = async () => {
    if (!name.trim()) return
    try {
      const createdToken = await createMutation.mutateAsync({
        name: name.trim(),
        scopes,
        expires_at: null as number | null,
      })
      setCreated(createdToken.token)
      setName('')
      showToast('token.created')
    } catch (error) {
      showToast(errorMessage(error), false)
    }
  }

  const revoke = async (id: string) => {
    try {
      await revokeMutation.mutateAsync(id)
      setConfirming(null)
      showToast('token.revoked')
    } catch (error) {
      showToast(errorMessage(error), false)
    }
  }

  return (
    <div className="app-page">
      <section className="app-card" aria-label="创建 token">
        <h2 className="app-card-title">创建 token</h2>
        <div className="app-form-row">
          <label className="app-field">
            名称
            <input
              type="text"
              className="oc-field app-field-light"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="如 hermes-laptop"
            />
          </label>
        </div>
        <fieldset className="app-scope-fieldset">
          <legend className="app-scope-legend">作用域</legend>
          {SCOPES.map((scope) => (
            <label key={scope.id} className="app-scope-check">
              <input
                type="checkbox"
                className="accent-primary"
                checked={scopes.includes(scope.id)}
                onChange={(e) => {
                  setScopes((current) =>
                    e.target.checked
                      ? [...current, scope.id]
                      : current.filter((id) => id !== scope.id),
                  )
                }}
              />
              {scope.label}
            </label>
          ))}
        </fieldset>
        <button
          type="button"
          className="oc-btn-default self-start"
          disabled={!name.trim() || scopes.length === 0 || createMutation.isPending}
          onClick={() => void create()}
        >
          <Icon name="add" />
          {createMutation.isPending ? '创建中…' : '创建'}
        </button>
        {created && (
          <div className="app-banner app-banner-ok flex-col items-start gap-2">
            <div className="app-state-title">token 明文（只显示这一次，请立即复制保存）</div>
            <code className="app-inline-code break-all">{created}</code>
            <button
              type="button"
              className="oc-btn-default oc-btn-sm"
              onClick={() => {
                void navigator.clipboard.writeText(created)
                showToast('common.copied')
              }}
            >
              <Icon name="copy" />
              复制
            </button>
            <button type="button" className="oc-btn-default oc-btn-sm" onClick={() => setCreated(null)}>
              我已保存
            </button>
          </div>
        )}
        <p className="app-card-sub">
          服务端只保存 sha256；列表只显示前 8 字符。撤销立即生效。
        </p>
      </section>

      <section className="app-card" aria-label="token 列表">
        <h2 className="app-card-title">token 列表</h2>
        <QueryState
          loading={tokensQuery.isLoading}
          error={tokensQuery.error}
          empty={(tokensQuery.data?.items.length ?? 0) === 0}
          onRetry={() => void tokensQuery.refetch()}
        >
          <table className="app-table">
            <thead>
              <tr>
                <th>前缀</th>
                <th>名称</th>
                <th>作用域</th>
                <th>创建时间</th>
                <th>最近使用</th>
                <th>限额</th>
                <th>状态</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {tokensQuery.data?.items.map((token) => (
                <Fragment key={token.id}>
                  <tr className={token.revoked_at ? 'app-row-revoked' : undefined}>
                  <td className="font-mono">{token.prefix}…</td>
                  <td>{token.name}</td>
                  <td>{token.scopes.join(', ')}</td>
                  <td>{new Date(token.created_at).toLocaleDateString()}</td>
                  <td>
                    {token.last_used_at ? new Date(token.last_used_at).toLocaleString() : '—'}
                  </td>
                  <td>
                    {token.limits ? (
                      <button
                        type="button"
                        className="text-meta underline"
                        onClick={() => {
                          setLimitsFor(token.id)
                          setLimitWindow(token.limits?.window ?? 'day')
                          setLimitTokens(
                            token.limits?.tokens && token.limits.tokens < Number.MAX_SAFE_INTEGER
                              ? String(token.limits.tokens)
                              : '',
                          )
                          setLimitCost(
                            token.limits?.cost_micros ? String(token.limits.cost_micros) : '',
                          )
                          setLimitModels((token.limits?.models ?? []).join(','))
                        }}
                      >
                        {token.limits.window}
                        {token.limits.tokens ? ` · ${token.limits.tokens} tok` : ''}
                        {token.limits.cost_micros ? ` · ${token.limits.cost_micros} µ` : ''}
                      </button>
                    ) : (
                      <button
                        type="button"
                        className="text-meta text-muted-foreground underline"
                        onClick={() => {
                          setLimitsFor(token.id)
                          setLimitWindow('day')
                          setLimitTokens('')
                          setLimitCost('')
                          setLimitModels('')
                        }}
                      >
                        设置
                      </button>
                    )}
                  </td>
                  <td>
                    {token.revoked_at ? (
                      <span className="app-pill app-pill-error">已撤销</span>
                    ) : (
                      <span className="app-pill app-pill-ok">活跃</span>
                    )}
                  </td>
                  <td>
                    {!token.revoked_at &&
                      (confirming === token.id ? (
                        <span className="app-confirm">
                          确认撤销？
                          <button
                            type="button"
                            className="oc-btn-destructive oc-btn-sm"
                            onClick={() => void revoke(token.id)}
                          >
                            <Icon name="remove" />
                            撤销
                          </button>
                          <button
                            type="button"
                            className="oc-btn-default oc-btn-sm"
                            onClick={() => setConfirming(null)}
                          >
                            取消
                          </button>
                        </span>
                      ) : (
                        <button
                          type="button"
                          className="oc-btn-default oc-btn-sm"
                          onClick={() => setConfirming(token.id)}
                        >
                          <Icon name="remove" />
                          撤销
                        </button>
                      ))}
                  </td>
                </tr>
                {limitsFor === token.id && (
                  <tr>
                    <td colSpan={8}>
                      <div className="app-form-row py-2">
                        <label className="app-field">
                          窗口
                          <select
                            className="app-select-sm"
                            value={limitWindow}
                            onChange={(e) => setLimitWindow(e.target.value)}
                          >
                            <option value="hour">每小时</option>
                            <option value="day">每天</option>
                            <option value="week">每周</option>
                            <option value="month">每月</option>
                            <option value="hours:5">每 5 小时</option>
                            <option value="hours:8">每 8 小时</option>
                          </select>
                        </label>
                        <label className="app-field">
                          token 预算（空 = 不限）
                          <input
                            className="oc-field app-field-light w-32"
                            type="number"
                            min={0}
                            value={limitTokens}
                            onChange={(e) => setLimitTokens(e.target.value)}
                          />
                        </label>
                        <label className="app-field">
                          成本预算 micros（空 = 不限）
                          <input
                            className="oc-field app-field-light w-36"
                            type="number"
                            min={0}
                            value={limitCost}
                            onChange={(e) => setLimitCost(e.target.value)}
                          />
                        </label>
                        <label className="app-field">
                          仅这些模型（逗号分隔，空 = 全部）
                          <input
                            className="oc-field app-field-light font-mono min-w-52"
                            value={limitModels}
                            onChange={(e) => setLimitModels(e.target.value)}
                          />
                        </label>
                        <button
                          type="button"
                          className="oc-btn-primary oc-btn-sm"
                          disabled={limitsMutation.isPending}
                          onClick={() => {
                            const tokens = limitTokens.trim() === '' ? 0 : Number(limitTokens)
                            const cost = limitCost.trim() === '' ? 0 : Number(limitCost)
                            const models = limitModels
                              .split(',')
                              .map((model) => model.trim())
                              .filter(Boolean)
                            const limits =
                              tokens === 0 && cost === 0
                                ? null
                                : { window: limitWindow, tokens, cost_micros: cost, models }
                            limitsMutation.mutate(
                              { id: token.id, limits },
                              {
                                onSuccess: () => {
                                  setLimitsFor(null)
                                  showToast('token.limits.saved')
                                },
                                onError: (error) => showToast(errorMessage(error), false),
                              },
                            )
                          }}
                        >
                          保存
                        </button>
                        <button
                          type="button"
                          className="oc-btn-default oc-btn-sm"
                          onClick={() => setLimitsFor(null)}
                        >
                          取消
                        </button>
                      </div>
                    </td>
                  </tr>
                )}
                </Fragment>
              ))}
            </tbody>
          </table>
        </QueryState>
      </section>
    </div>
  )
}
