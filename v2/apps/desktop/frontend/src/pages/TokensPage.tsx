/**
 * Tokens（§7.3）：创建/撤销/一次性明文/scope。
 */
import { useState } from 'react'
import {
  useTokensQuery,
  useCreateTokenMutation,
  useRevokeTokenMutation,
} from '../queries/admin'
import { QueryState, errorMessage } from '../components/common/QueryState'
import { useAppStore } from '../stores/appStore'

const SCOPES = [
  { id: 'inference', label: 'inference（数据面）' },
  { id: 'admin_read', label: 'admin:read（管理读）' },
  { id: 'admin_write', label: 'admin:write（管理写）' },
  { id: 'admin_approve', label: 'admin:approve（审批）' },
] as const

export function TokensPage() {
  const tokensQuery = useTokensQuery()
  const createMutation = useCreateTokenMutation()
  const revokeMutation = useRevokeTokenMutation()
  const showToast = useAppStore((s) => s.showToast)

  const [name, setName] = useState('')
  const [scopes, setScopes] = useState<Array<'inference' | 'admin_read' | 'admin_write' | 'admin_approve'>>(['inference'])
  const [created, setCreated] = useState<string | null>(null)
  const [confirming, setConfirming] = useState<string | null>(null)

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
    <div className="page">
      <section className="card" aria-label="创建 token">
        <h2>创建 token</h2>
        <div className="form-row">
          <label>
            名称{' '}
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="如 hermes-laptop"
            />
          </label>
        </div>
        <fieldset className="scope-fieldset">
          <legend>作用域</legend>
          {SCOPES.map((scope) => (
            <label key={scope.id} className="scope-check">
              <input
                type="checkbox"
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
          className="btn"
          disabled={!name.trim() || scopes.length === 0 || createMutation.isPending}
          onClick={() => void create()}
        >
          {createMutation.isPending ? '创建中…' : '创建'}
        </button>
        {created && (
          <div className="banner banner-ok one-time-secret">
            <div className="state-title">token 明文（只显示这一次，请立即复制保存）</div>
            <code className="mono">{created}</code>
            <button
              type="button"
              className="btn btn-sm"
              onClick={() => {
                void navigator.clipboard.writeText(created)
                showToast('common.copied')
              }}
            >
              复制
            </button>
            <button type="button" className="btn btn-sm" onClick={() => setCreated(null)}>
              我已保存
            </button>
          </div>
        )}
        <p className="text-dim">
          服务端只保存 sha256；列表只显示前 8 字符。撤销立即生效。
        </p>
      </section>

      <section className="card" aria-label="token 列表">
        <h2>token 列表</h2>
        <QueryState
          loading={tokensQuery.isLoading}
          error={tokensQuery.error}
          empty={(tokensQuery.data?.items.length ?? 0) === 0}
          onRetry={() => void tokensQuery.refetch()}
        >
          <table className="table">
            <thead>
              <tr><th>前缀</th><th>名称</th><th>作用域</th><th>创建时间</th><th>最近使用</th><th>状态</th><th /></tr>
            </thead>
            <tbody>
              {tokensQuery.data?.items.map((token) => (
                <tr key={token.id} className={token.revoked_at ? 'row-revoked' : ''}>
                  <td className="mono">{token.prefix}…</td>
                  <td>{token.name}</td>
                  <td>{token.scopes.join(', ')}</td>
                  <td>{new Date(token.created_at).toLocaleDateString()}</td>
                  <td>{token.last_used_at ? new Date(token.last_used_at).toLocaleString() : '—'}</td>
                  <td>
                    {token.revoked_at ? (
                      <span className="pill pill-open">已撤销</span>
                    ) : (
                      <span className="pill pill-closed">活跃</span>
                    )}
                  </td>
                  <td>
                    {!token.revoked_at &&
                      (confirming === token.id ? (
                        <span className="confirm-inline">
                          确认撤销？
                          <button type="button" className="btn btn-sm btn-danger" onClick={() => void revoke(token.id)}>
                            撤销
                          </button>
                          <button type="button" className="btn btn-sm" onClick={() => setConfirming(null)}>
                            取消
                          </button>
                        </span>
                      ) : (
                        <button type="button" className="btn btn-sm" onClick={() => setConfirming(token.id)}>
                          撤销
                        </button>
                      ))}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </QueryState>
      </section>
    </div>
  )
}
