/**
 * Logs（§7.3）：服务端日志尾；level/target 过滤；打开日志目录。
 */
import { useState } from 'react'
import { useLogsQuery } from '../queries/admin'
import { QueryState } from '../components/common/QueryState'
import { Icon } from '../components/common/Icon'
import { logLineClass } from '../lib/status'
import { hostOpenPath, isTauri } from '../lib/tauri'
import { usePrefsStore } from '../stores/prefsStore'

const LEVELS = ['', 'error', 'warn', 'info', 'debug'] as const

export function LogsPage() {
  const [level, setLevel] = useState<string>('')
  const [target, setTarget] = useState('')
  const [limit, setLimit] = useState(200)
  const liteMode = usePrefsStore((s) => s.liteMode)

  const logsQuery = useLogsQuery(
    {
      limit,
      level: level || undefined,
      target: target || undefined,
    },
    true,
    // 轻量模式关掉 SSE 后必须靠轮询，否则日志页永远停在打开时的那一屏。
    liteMode ? 10_000 : false,
  )

  return (
    <div className="app-page">
      <section className="app-card" aria-label="服务端日志">
        <div className="app-filter-bar">
          <label className="app-filter-label">
            级别
            <select
              className="app-select-sm"
              value={level}
              onChange={(e) => setLevel(e.target.value)}
            >
              {LEVELS.map((value) => (
                <option key={value} value={value}>
                  {value || '全部'}
                </option>
              ))}
            </select>
          </label>
          <label className="app-filter-label">
            目标前缀
            <input
              type="text"
              className="app-field-sm h-7 w-36"
              value={target}
              placeholder="如 tr_agent"
              onChange={(e) => setTarget(e.target.value)}
            />
          </label>
          <label className="app-filter-label">
            条数
            <select
              className="app-select-sm"
              value={limit}
              onChange={(e) => setLimit(Number(e.target.value))}
            >
              {[100, 200, 500].map((value) => (
                <option key={value} value={value}>
                  {value}
                </option>
              ))}
            </select>
          </label>
          {isTauri() && (
            <button type="button" className="oc-btn-default" onClick={() => void hostOpenPath('logs')}>
              <Icon name="open-dir" />
              打开日志目录
            </button>
          )}
        </div>
        <QueryState
          loading={logsQuery.isLoading}
          error={logsQuery.error}
          empty={(logsQuery.data?.items.length ?? 0) === 0}
          onRetry={() => void logsQuery.refetch()}
          emptyTitle="暂无日志"
        >
          <div className="app-log" role="log" aria-live="polite">
            {logsQuery.data?.items.map((entry) => (
              <div key={entry.seq} className={logLineClass(entry.level)}>
                <span className="app-log-time">{new Date(entry.at).toLocaleTimeString()}</span>
                <span className="app-log-level">[{entry.level}]</span>
                <span className="app-log-target">{entry.target}</span>
                <span className="app-log-msg">{entry.message}</span>
              </div>
            ))}
          </div>
        </QueryState>
      </section>
    </div>
  )
}
