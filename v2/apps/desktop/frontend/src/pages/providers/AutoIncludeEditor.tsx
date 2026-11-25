/**
 * 自动纳入规则（provider 级）：拉取目录时，按通配符自动勾选新模型。
 *
 * 服务端不回显已配置规则（接口无回读字段），因此只有用户明确点「应用规则」
 * 才提交非空值，点「清除规则」才提交空串；不会因输入框为空而覆盖现有规则。
 */
import { useState } from 'react'
import type { ProviderInfo } from '../../api/types'
import { api } from '../../api/client'
import { useCatalogQuery } from '../../queries/admin'
import { errorMessageText } from './shared'

export function AutoIncludeEditor({ provider }: { provider: ProviderInfo }) {
  const catalogQuery = useCatalogQuery(provider.id)
  const [pattern, setPattern] = useState('')
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  async function apply(nextPattern: string) {
    setBusy(true)
    setMessage(null)
    setError(null)
    try {
      await api.providerAutoInclude(provider.id, nextPattern)
      setMessage(
        nextPattern === ''
          ? '已清除自动纳入规则。'
          : '已应用规则：匹配的新模型自动勾选。',
      )
      await catalogQuery.refetch()
    } catch (cause) {
      setError(errorMessageText(cause))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="flex flex-col gap-2">
      <h4 className="text-ui-label font-medium">自动纳入模型</h4>
      <div className="app-form-row items-center">
        <input
          className="oc-field app-field-light font-mono h-7 min-w-52 text-meta"
          placeholder="通配符（如 gpt-*，* 匹配任意段）"
          value={pattern}
          onChange={(event) => setPattern(event.target.value)}
          aria-label="自动纳入通配符"
        />
        <button
          type="button"
          className="oc-btn-default oc-btn-sm"
          disabled={busy || pattern.trim() === ''}
          onClick={() => void apply(pattern.trim())}
        >
          {busy ? '应用中…' : '应用规则'}
        </button>
        <button
          type="button"
          className="oc-btn-default oc-btn-sm"
          disabled={busy}
          onClick={() => void apply('')}
        >
          清除规则
        </button>
      </div>
      {message !== null && (
        <span className="text-meta text-status-success">{message}</span>
      )}
      {error !== null && <span className="text-meta text-status-error">{error}</span>}
    </div>
  )
}
