import { useI18n } from '../../hooks/useI18n'
import { formatAuthKeyCreatedAt } from '../../lib/gateway-auth-keys'
import { fmtMs, fmtNum, fmtPct, fmtTps } from '../../lib/stats-utils'
import type { AuthKeyStatsSnapshot } from '../../types/gateway'

function formatLastUsedAt(ts: number | null | undefined, locale: string): string {
  if (!ts) return '—'
  return formatAuthKeyCreatedAt(ts, locale)
}

export function AuthKeyStatsPanel({ keys }: { keys: AuthKeyStatsSnapshot[] }) {
  const { t, locale } = useI18n()

  return (
    <div className="panel" id="auth-key-stats-panel">
      <div className="panel-title">{t('stats.authKeys')}</div>
      <div className="table-wrap auth-key-stats-table-wrap">
        <table className="auth-key-stats-table">
          <thead>
            <tr>
              <th>{t('col.authKey')}</th>
              <th>{t('col.authKeyName')}</th>
              <th className="num">{t('stat.totalReq')}</th>
              <th className="num">{t('stat.tokens')}</th>
              <th className="num">{t('stat.avgReqMs')}</th>
              <th className="num">{t('stat.avgTps')}</th>
              <th className="num">{t('stat.edgeRoute')}</th>
              <th className="num">{t('stat.cloudRoute')}</th>
              <th>{t('stat.authKeyLastUsed')}</th>
            </tr>
          </thead>
          <tbody id="auth-key-stats-table">
            {keys.map((row) => (
              <tr key={row.id} className={row.deleted ? 'auth-key-stats-row-deleted' : undefined}>
                <td>
                  <code title={row.id}>{row.key_preview}</code>
                </td>
                <td>
                  <span className="auth-key-stats-name">{row.name}</span>
                  {row.deleted && (
                    <span className="tag bordered neutral auth-key-stats-deleted">{t('authKeys.deleted')}</span>
                  )}
                </td>
                <td className="num">{fmtNum(row.requests_total, locale)}</td>
                <td className="num">
                  {row.tokens.total > 0 ? fmtNum(row.tokens.total, locale) : '—'}
                  {row.tokens.total > 0 && (
                    <div className="table-cell-sub">
                      {t('stat.tokenInOut', {
                        input: fmtNum(row.tokens.input, locale),
                        output: fmtNum(row.tokens.output, locale),
                      })}
                    </div>
                  )}
                </td>
                <td className="num">{fmtMs(row.latency.avg_request_ms, t)}</td>
                <td className="num">{fmtTps(row.latency.avg_tps)}</td>
                <td className="num edge">{fmtPct(row.routing.edge_pct)}</td>
                <td className="num cloud">{fmtPct(row.routing.cloud_pct)}</td>
                <td className="auth-key-stats-last-used">
                  {formatLastUsedAt(row.last_used_at_unix, locale)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}
