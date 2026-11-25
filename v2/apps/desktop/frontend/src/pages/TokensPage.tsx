/**
 * 网关密钥（网关模块 → 网关密钥 tab）。
 *
 * 页面只有一张表：密钥本体。创建走对话框，明文只在创建后于对话框内出现一次，
 * 关闭即消失——不留横幅，避免明文长期挂在页面上。
 *
 * 口径：服务端只存 sha256，列表只有前缀 8 字符；作用域显示用服务端规范写法
 * `admin:read`（写请求体时才用 snake_case 的 `admin_read`）。
 */
import { Fragment, useState } from 'react'
import {
  useTokensQuery,
  useCreateTokenMutation,
  useRevokeTokenMutation,
  useRotateTokenMutation,
  usePurgeTokenMutation,
  useUpdateTokenLimitsMutation,
} from '../queries/admin'
import { QueryState, errorMessage } from '../components/common/QueryState'
import { Icon } from '../components/common/Icon'
import { Modal } from '../components/common/Modal'
import { useAppStore } from '../stores/appStore'
import type { TokenInfo } from '../api/types'

/** 作用域：请求体用 snake_case（serde），展示用 CLI 同样的规范写法 `admin:read`。 */
type ScopeId = 'inference' | 'admin_read' | 'admin_write'

const SCOPES: ReadonlyArray<{ id: ScopeId; canonical: string; label: string }> = [
  { id: 'inference', canonical: 'inference', label: '数据面' },
  { id: 'admin_read', canonical: 'admin:read', label: '管理读' },
  { id: 'admin_write', canonical: 'admin:write', label: '管理写' },
]

const SCOPE_BY_ID: Record<ScopeId, { canonical: string; label: string }> = {
  inference: { canonical: 'inference', label: '数据面' },
  admin_read: { canonical: 'admin:read', label: '管理读' },
  admin_write: { canonical: 'admin:write', label: '管理写' },
}

const LIMITS_WINDOWS = [
  { value: 'hour', label: '每小时' },
  { value: 'day', label: '每天' },
  { value: 'week', label: '每周' },
  { value: 'month', label: '每月' },
  { value: 'hours:5', label: '每 5 小时' },
  { value: 'hours:8', label: '每 8 小时' },
] as const

export function TokensPage() {
  const tokensQuery = useTokensQuery()
  const createMutation = useCreateTokenMutation()
  const revokeMutation = useRevokeTokenMutation()
  const rotateMutation = useRotateTokenMutation()
  const purgeMutation = usePurgeTokenMutation()
  const limitsMutation = useUpdateTokenLimitsMutation()
  const showToast = useAppStore((s) => s.showToast)

  const [createOpen, setCreateOpen] = useState(false)
  const [name, setName] = useState('')
  const [scopes, setScopes] = useState<ScopeId[]>(['inference'])
  // 创建成功后的一次性明文：只在对话框内存在，关闭即丢弃。
  const [plaintext, setPlaintext] = useState<string | null>(null)

  // 重新签发：确认态与结果态共用一个对话框。
  const [rotateTarget, setRotateTarget] = useState<TokenInfo | null>(null)
  const [rotatePlaintext, setRotatePlaintext] = useState<string | null>(null)

  // 硬删除已撤销的记录（不可撤销）。
  const [purgeTarget, setPurgeTarget] = useState<TokenInfo | null>(null)

  const [confirming, setConfirming] = useState<string | null>(null)
  // key 限额编辑（v3 P5-B）：tokenId 非空时展开行内表单。
  const [limitsFor, setLimitsFor] = useState<string | null>(null)
  const [limitWindow, setLimitWindow] = useState('day')
  const [limitTokens, setLimitTokens] = useState('')
  const [limitCost, setLimitCost] = useState('')
  const [limitModels, setLimitModels] = useState('')

  const closeCreate = () => {
    setCreateOpen(false)
    setName('')
    setScopes(['inference'])
    setPlaintext(null)
  }

  const create = async () => {
    if (!name.trim()) return
    try {
      const createdToken = await createMutation.mutateAsync({
        name: name.trim(),
        scopes,
        expires_at: null as number | null,
      })
      setPlaintext(createdToken.token)
      setName('')
      showToast('toast.tokenCreated')
    } catch (error) {
      showToast(errorMessage(error), false)
    }
  }

  const revoke = async (id: string) => {
    try {
      await revokeMutation.mutateAsync(id)
      setConfirming(null)
      showToast('toast.tokenRevoked')
    } catch (error) {
      showToast(errorMessage(error), false)
    }
  }

  // 复制到剪贴板。失败时回退到「手动选中」提示，不静默吞掉。
  const copyText = async (text: string, label: string) => {
    try {
      if (!navigator.clipboard?.writeText) throw new Error('clipboard unavailable')
      await navigator.clipboard.writeText(text)
      showToast(label ? `已复制${label}` : 'toast.copied')
    } catch {
      showToast('复制失败，请手动选择内容复制', false)
    }
  }

  // 明文不可恢复：只能重新签发。确认后才动手，旧明文即刻失效。
  const rotate = async () => {
    if (!rotateTarget) return
    try {
      const rotated = await rotateMutation.mutateAsync(rotateTarget.id)
      setRotatePlaintext(rotated.token)
      showToast('toast.tokenRotated')
    } catch (error) {
      showToast(errorMessage(error), false)
    }
  }

  const closeRotate = () => {
    setRotateTarget(null)
    setRotatePlaintext(null)
  }

  // 硬删除已撤销的记录。
  const purge = async () => {
    if (!purgeTarget) return
    try {
      await purgeMutation.mutateAsync(purgeTarget.id)
      setPurgeTarget(null)
      showToast('toast.tokenPurged')
    } catch (error) {
      showToast(errorMessage(error), false)
    }
  }

  const tokens = tokensQuery.data?.items ?? []

  return (
    <div className="app-page">
      <section className="app-card" aria-label="网关密钥">
        <div className="flex items-center gap-2 flex-wrap">
          <span className="app-card-title">网关密钥</span>
          <span className="text-meta text-muted-foreground">{tokens.length}</span>
          <button
            type="button"
            className="oc-btn-primary oc-btn-sm ml-auto"
            onClick={() => setCreateOpen(true)}
          >
            <Icon name="add" />
            新建密钥
          </button>
        </div>

        <QueryState
          loading={tokensQuery.isLoading}
          error={tokensQuery.error}
          empty={tokens.length === 0}
          onRetry={() => void tokensQuery.refetch()}
          emptyTitle="还没有网关密钥"
          emptyHint="客户端需要一个网关 token 才能请求 /v1。"
          emptyAction={
            <button
              type="button"
              className="oc-btn-primary oc-btn-sm"
              onClick={() => setCreateOpen(true)}
            >
              <Icon name="add" />
              新建密钥
            </button>
          }
        >
          <table className="app-table">
            <thead>
              <tr>
                <th>名称</th>
                <th>作用域</th>
                <th>状态</th>
                <th>最近使用</th>
                <th>限额</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {tokens.map((token) => (
                <Fragment key={token.id}>
                  <tr className={token.revoked_at ? 'app-row-revoked' : undefined}>
                    <td>
                      <div className="font-medium">{token.name}</div>
                      <div className="font-mono text-meta text-muted-foreground">
                        {token.prefix}…
                      </div>
                    </td>
                    <td>
                      <div className="flex items-center gap-1 flex-wrap">
                        {token.scopes.map((scope) => {
                          const meta = SCOPE_BY_ID[scope]
                          return (
                            <span key={scope} className="app-pill">
                              {meta?.canonical ?? scope}
                            </span>
                          )
                        })}
                      </div>
                    </td>
                    <td>
                      {token.revoked_at ? (
                        <span className="app-pill app-pill-error">已撤销</span>
                      ) : (
                        <span className="app-pill app-pill-ok">活跃</span>
                      )}
                    </td>
                    <td className="text-meta tabular-nums">
                      {token.last_used_at
                        ? new Date(token.last_used_at).toLocaleString()
                        : '—'}
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
                        <button
                          type="button"
                          className="oc-btn-default oc-btn-sm"
                          aria-label={`删除已撤销的密钥 ${token.name}`}
                          onClick={() => setPurgeTarget(token)}
                        >
                          <Icon name="remove" />
                          删除记录
                        </button>
                      ) : (
                        <div className="flex items-center gap-1.5 flex-wrap">
                          <button
                            type="button"
                            className="oc-btn-default oc-btn-sm"
                            aria-label={`复制密钥 ${token.name}`}
                            onClick={() => {
                              setRotateTarget(token)
                              setRotatePlaintext(null)
                            }}
                          >
                            <Icon name="copy" />
                            复制
                          </button>
                          {confirming === token.id ? (
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
                          )}
                        </div>
                      )}
                    </td>
                  </tr>
                  {limitsFor === token.id && (
                    <tr>
                      <td colSpan={6}>
                        <div className="app-form-row py-2">
                          <label className="app-field">
                            窗口
                            <select
                              className="app-select-sm"
                              value={limitWindow}
                              onChange={(e) => setLimitWindow(e.target.value)}
                            >
                              {LIMITS_WINDOWS.map((option) => (
                                <option key={option.value} value={option.value}>
                                  {option.label}
                                </option>
                              ))}
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
                              // 只有“预算和模型白名单都没填”才是清空限额；
                              // 只设模型白名单时不能把 models/window 一起丢掉。
                              const limits =
                                tokens === 0 && cost === 0 && models.length === 0
                                  ? null
                                  : { window: limitWindow, tokens, cost_micros: cost, models }
                              limitsMutation.mutate(
                                { id: token.id, limits },
                                {
                                  onSuccess: () => {
                                    setLimitsFor(null)
                                    showToast('toast.tokenLimitsSaved')
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

      {createOpen && (
        <Modal
          title={plaintext ? '密钥已创建' : '新建网关密钥'}
          leading={<Icon name="nav-tokens" className="size-4 shrink-0" />}
          onClose={closeCreate}
          footer={
            plaintext ? (
              <button type="button" className="oc-btn-primary" onClick={closeCreate}>
                我已保存
              </button>
            ) : (
              <>
                <button type="button" className="oc-btn-default" onClick={closeCreate}>
                  取消
                </button>
                <button
                  type="button"
                  className="oc-btn-primary"
                  disabled={!name.trim() || scopes.length === 0 || createMutation.isPending}
                  onClick={() => void create()}
                >
                  {createMutation.isPending ? '创建中…' : '创建'}
                </button>
              </>
            )
          }
        >
          {plaintext ? (
            <div className="flex flex-col gap-2 items-start">
              <div className="app-state-title">token 明文（只显示这一次，请立即复制保存）</div>
              <code className="app-inline-code break-all">{plaintext}</code>
              <button
                type="button"
                className="oc-btn-default oc-btn-sm"
                onClick={() => void copyText(plaintext, 'token 明文')}
              >
                <Icon name="copy" />
                复制
              </button>
            </div>
          ) : (
            <div className="flex flex-col gap-3">
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
                    {scope.canonical}
                    <span className="text-muted-foreground">（{scope.label}）</span>
                  </label>
                ))}
              </fieldset>
            </div>
          )}
        </Modal>
      )}

      {rotateTarget && (
        <Modal
          title={rotatePlaintext ? '密钥已重新生成' : '重新生成密钥'}
          leading={<Icon name="nav-tokens" className="size-4 shrink-0" />}
          onClose={closeRotate}
          footer={
            rotatePlaintext ? (
              <button type="button" className="oc-btn-primary" onClick={closeRotate}>
                我已保存
              </button>
            ) : (
              <>
                <button type="button" className="oc-btn-default" onClick={closeRotate}>
                  取消
                </button>
                <button
                  type="button"
                  className="oc-btn-primary"
                  disabled={rotateMutation.isPending}
                  onClick={() => void rotate()}
                >
                  {rotateMutation.isPending ? '生成中…' : '重新生成'}
                </button>
              </>
            )
          }
        >
          {rotatePlaintext ? (
            <div className="flex flex-col gap-2 items-start">
              <div className="app-state-title">token 明文（只显示这一次，请立即复制保存）</div>
              <code className="app-inline-code break-all">{rotatePlaintext}</code>
              <button
                type="button"
                className="oc-btn-default oc-btn-sm"
                onClick={() => void copyText(rotatePlaintext, 'token 明文')}
              >
                <Icon name="copy" />
                复制
              </button>
            </div>
          ) : (
            <div className="flex flex-col gap-2">
              <div className="app-state-title">{rotateTarget.name}</div>
              <div className="app-state-hint">
                服务端只存哈希，明文无法找回。重新生成后旧密钥立即失效，客户端需同步替换。
              </div>
            </div>
          )}
        </Modal>
      )}

      {purgeTarget && (
        <Modal
          title="删除密钥记录"
          leading={<Icon name="nav-tokens" className="size-4 shrink-0" />}
          onClose={() => setPurgeTarget(null)}
          footer={
            <>
              <button
                type="button"
                className="oc-btn-default"
                onClick={() => setPurgeTarget(null)}
              >
                取消
              </button>
              <button
                type="button"
                className="oc-btn-destructive"
                disabled={purgeMutation.isPending}
                onClick={() => void purge()}
              >
                {purgeMutation.isPending ? '删除中…' : '确认删除'}
              </button>
            </>
          }
        >
          <div className="flex flex-col gap-2">
            <div className="app-state-title">{purgeTarget.name}</div>
            <div className="app-state-hint">
              该密钥已撤销，删除后不保留记录。历史请求账本不受影响。
            </div>
          </div>
        </Modal>
      )}
    </div>
  )
}
