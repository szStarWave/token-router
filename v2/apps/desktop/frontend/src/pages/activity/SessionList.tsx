/**
 * 会话列表：服务端会话聚合（`GET /v2/admin/sessions`）的一行一个会话。
 *
 * 汇总数（请求/失败/判官调用/成本）全部来自服务端聚合，不受请求列表分页影响；
 * 点一行交给调用方拉该会话的完整调用明细。
 */
import type { SessionSummary } from '../../api/types'
import { formatCompactNum } from '../../lib/format-number'
import { fmtClock, fmtCost } from './shared'

export interface SessionListProps {
  sessions: SessionSummary[]
  selectedSession: string | null
  onSelect: (sessionKey: string) => void
}

export function SessionList({ sessions, selectedSession, onSelect }: SessionListProps) {
  if (sessions.length === 0) {
    return <p className="text-meta text-muted-foreground">这个时间范围里没有会话。</p>
  }
  return (
    <div className="flex flex-col gap-2">
      {sessions.map((session) => {
        const tokens = session.tokens.input + session.tokens.output + session.tokens.cache_read
        const active = session.session_key === selectedSession
        return (
          <button
            key={session.session_key}
            type="button"
            className={active ? 'app-feed-row app-feed-row-active' : 'app-feed-row'}
            onClick={() => onSelect(session.session_key)}
          >
            <div className="flex items-center gap-2 flex-wrap min-w-0">
              <span className="font-mono text-meta truncate">{session.session_key}</span>
              <span className="app-pill">{session.requests} 个请求</span>
              {session.failures > 0 && (
                <span className="app-pill app-pill-warn">{session.failures} 个失败</span>
              )}
              {session.judge_calls > 0 && (
                <span className="app-pill">{session.judge_calls} 次判官</span>
              )}
              {session.judge_failures > 0 && (
                <span className="app-pill app-pill-warn">
                  {session.judge_failures} 次判官失败
                </span>
              )}
            </div>
            <div className="flex items-center gap-3 flex-wrap text-meta text-muted-foreground">
              <span>最后 {fmtClock(session.last_at)}</span>
              <span>合计 {formatCompactNum(tokens)} token</span>
              <span>{fmtCost(session.cost_micros.answer + session.cost_micros.routing)}</span>
              {session.models.slice(0, 3).map((model) => (
                <span key={model} className="font-mono">
                  {model}
                </span>
              ))}
            </div>
          </button>
        )
      })}
    </div>
  )
}
