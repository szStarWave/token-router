/**
 * Quotas 页（v3 P5-B）：上游配额窗口 + 网关 key 限额。
 *
 * 数据来源：
 * - `GET /v2/admin/quotas`：每候选的剩余/重置/是否估算/连续 429；
 * - token 限额在 Tokens 页设置，这里只读展示提示。
 */
import { useQuotasQuery, useResetQuotaMutation } from '../queries/admin'
import { EmptyState, ErrorState, Loading } from '../components/common/QueryState'
import type { QuotaWindowInfo } from '../api/types'

function formatReset(resetAt: number | null): string {
  if (resetAt === null) return '未知'
  const diff = resetAt - Date.now()
  if (diff <= 0) return '已过期'
  const minutes = Math.ceil(diff / 60_000)
  if (minutes < 60) return `${minutes} 分钟后`
  const hours = Math.ceil(minutes / 60)
  if (hours < 48) return `${hours} 小时后`
  return `${Math.ceil(hours / 24)} 天后`
}

export function QuotasPage() {
  const quotasQuery = useQuotasQuery()
  const reset = useResetQuotaMutation()

  if (quotasQuery.isPending) return <Loading label="加载配额…" />
  if (quotasQuery.isError) {
    return (
      <ErrorState
        message={quotasQuery.error.message}
        onRetry={() => quotasQuery.refetch()}
      />
    )
  }

  const items = quotasQuery.data.items

  return (
    <div className="app-page-content">
      <div>
        <h1 className="app-card-title">配额</h1>
        <p className="app-card-sub">
          上游限流头（`x-ratelimit-*` / `anthropic-ratelimit-*`）与 429 的观测结果。
          「估算」= 上游没给标准头，按 429 次数与本地窗口推断；耗尽候选会被路由自动过滤。
        </p>
      </div>

      {items.length === 0 ? (
        <EmptyState
          title="还没有配额观测"
          hint="发起请求并收到带限流头的响应（或 429）后，这里会出现每个候选的窗口状态。"
        />
      ) : (
        <section className="app-card" aria-label="配额窗口">
          <table className="app-table">
            <thead>
              <tr>
                <th>候选</th>
                <th>剩余 / 上限</th>
                <th>重置</th>
                <th>单位</th>
                <th>连续 429</th>
                <th>来源</th>
                <th aria-label="操作" />
              </tr>
            </thead>
            <tbody>
              {items.map((item: QuotaWindowInfo) => (
                <tr key={item.candidate_id}>
                  <td className="font-mono text-meta">{item.candidate_id}</td>
                  <td>
                    {item.remaining === null ? '未知' : item.remaining}
                    {item.limit === null ? '' : ` / ${item.limit}`}
                  </td>
                  <td>{formatReset(item.reset_at)}</td>
                  <td>{item.unit === 'unknown' ? '—' : item.unit}</td>
                  <td>
                    {item.consecutive_429 > 0 ? (
                      <span className="app-pill app-pill-error">{item.consecutive_429}</span>
                    ) : (
                      '0'
                    )}
                  </td>
                  <td>
                    {item.estimated ? (
                      <span className="app-pill app-pill-warn">估算</span>
                    ) : (
                      <span className="app-pill app-pill-ok">精确</span>
                    )}
                  </td>
                  <td>
                    <button
                      type="button"
                      className="oc-btn-default oc-btn-sm"
                      disabled={reset.isPending}
                      onClick={() => reset.mutate(item.candidate_id)}
                    >
                      重置
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      )}
    </div>
  )
}
