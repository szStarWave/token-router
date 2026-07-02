import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { fetchGatewayLogs, useGatewayLogsQuery } from '../queries/gateway'
import { useAppStore } from '../stores/appStore'
import { useI18n } from '../hooks/useI18n'
import { gatewayOpenLogsDir } from '../lib/tauri'
import { LOG_MAX_LINES, LOG_SCROLL_LOAD_THRESHOLD } from '../constants/defaults'

interface LogEntry {
  id: number
  level: string
  msg: string
}

let nextLogId = 0

const LOG_LINE_START = /^\d{4}-\d{2}-\d{2}T/

function mapLogLines(raw: Array<{ level: string; text?: string; msg?: string }>): LogEntry[] {
  return raw.map((line) => ({
    id: nextLogId++,
    level: line.level,
    msg: line.text ?? line.msg ?? '',
  }))
}

function mergeAppendedLogLines(prev: LogEntry[], incoming: LogEntry[]): LogEntry[] {
  if (!incoming.length) return prev
  if (!prev.length) return incoming
  if (LOG_LINE_START.test(incoming[0].msg)) return [...prev, ...incoming]
  const last = prev[prev.length - 1]
  return [
    ...prev.slice(0, -1),
    { ...last, msg: last.msg + incoming[0].msg },
    ...incoming.slice(1),
  ]
}

export function LogsPage() {
  const { t } = useI18n()
  const connected = useAppStore((s) => s.connected)
  const showToast = useAppStore((s) => s.showToast)
  const [tailOffset, setTailOffset] = useState<number | null>(null)
  const [headOffset, setHeadOffset] = useState(0)
  const [hasOlder, setHasOlder] = useState(false)
  const [loadingOlder, setLoadingOlder] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [lines, setLines] = useState<LogEntry[]>([])
  const viewRef = useRef<HTMLDivElement>(null)
  const stickToBottomRef = useRef(true)
  const loadingOlderRef = useRef(false)
  const prependScrollRef = useRef<number | null>(null)
  const headOffsetRef = useRef(0)

  const logsQuery = useGatewayLogsQuery(tailOffset, true)

  useEffect(() => {
    headOffsetRef.current = headOffset
  }, [headOffset])

  useEffect(() => {
    const data = logsQuery.data
    if (!data) return

    setTailOffset(data.next_offset)

    if (data.reset) {
      setHeadOffset(data.offset)
      setHasOlder(data.offset > 0)
      let next = mapLogLines(data.lines)
      if (next.length > LOG_MAX_LINES) next = next.slice(-LOG_MAX_LINES)
      setLines(next)
      return
    }

    if (!data.lines.length) return

    if (headOffsetRef.current === 0) {
      setHeadOffset(data.offset)
      setHasOlder(data.offset > 0)
    }

    setLines((prev) => {
      let next = mergeAppendedLogLines(prev, mapLogLines(data.lines))
      if (next.length > LOG_MAX_LINES) next = next.slice(-LOG_MAX_LINES)
      return next
    })
  }, [logsQuery.data])

  const emptyMessage = useMemo(() => {
    if (!connected) return t('logs.offline')
    if (logsQuery.isError) return t('logs.loadFail', { msg: logsQuery.error?.message ?? '' })
    if (logsQuery.isLoading && !lines.length) return t('logs.loading')
    if (!lines.length) return t('logs.waiting')
    return ''
  }, [connected, logsQuery.isError, logsQuery.isLoading, logsQuery.error, lines.length, t])

  useLayoutEffect(() => {
    const view = viewRef.current
    if (!view) return

    if (prependScrollRef.current != null) {
      const prevHeight = prependScrollRef.current
      prependScrollRef.current = null
      view.scrollTop += view.scrollHeight - prevHeight
      return
    }

    if (!lines.length || !stickToBottomRef.current) return
    view.scrollTop = view.scrollHeight
  }, [lines])

  const loadOlder = async () => {
    const before = headOffsetRef.current
    if (!hasOlder || loadingOlderRef.current || before <= 0) return
    loadingOlderRef.current = true
    setLoadingOlder(true)
    const view = viewRef.current
    const prevScrollHeight = view?.scrollHeight ?? 0
    try {
      const data = await fetchGatewayLogs({ beforeOffset: before })
      setHeadOffset(data.offset)
      setHasOlder(data.offset > 0)
      if (!data.lines.length) return
      prependScrollRef.current = prevScrollHeight
      setLines((prev) => {
        let next = [...mapLogLines(data.lines), ...prev]
        if (next.length > LOG_MAX_LINES) next = next.slice(0, LOG_MAX_LINES)
        return next
      })
    } catch (e) {
      showToast('logs.loadFail', false, { msg: e instanceof Error ? e.message : String(e) })
    } finally {
      loadingOlderRef.current = false
      setLoadingOlder(false)
    }
  }

  const onScroll = () => {
    const view = viewRef.current
    if (!view) return
    stickToBottomRef.current = view.scrollHeight - view.scrollTop - view.clientHeight < 48
    if (view.scrollTop < LOG_SCROLL_LOAD_THRESHOLD && hasOlder && !loadingOlderRef.current) {
      void loadOlder()
    }
  }

  const openLogsDir = async () => {
    try {
      await gatewayOpenLogsDir()
    } catch (e) {
      showToast('toast.openLogsDirFail', false, { msg: e instanceof Error ? e.message : String(e) })
    }
  }

  const refreshLogs = async () => {
    if (refreshing) return
    setRefreshing(true)
    stickToBottomRef.current = true
    try {
      const data = await fetchGatewayLogs({ offset: null })
      setTailOffset(data.next_offset)
      setHeadOffset(data.offset)
      setHasOlder(data.offset > 0)
      let next = mapLogLines(data.lines)
      if (next.length > LOG_MAX_LINES) next = next.slice(-LOG_MAX_LINES)
      setLines(next)
    } catch (e) {
      showToast('logs.loadFail', false, { msg: e instanceof Error ? e.message : String(e) })
    } finally {
      setRefreshing(false)
    }
  }

  return (
    <section className="page active" id="page-logs">
      <div className="panel">
        <div className="panel-title" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
          <span>{t('logs.title')}</span>
          <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
            {loadingOlder && (
              <span className="log-loading-hint">{t('logs.loadingOlder')}</span>
            )}
            <button type="button" className="btn btn-ghost btn-sm" onClick={() => void openLogsDir()}>
              {t('action.openLogsDir')}
            </button>
            <button
              type="button"
              className="btn btn-primary btn-sm"
              disabled={refreshing}
              onClick={() => void refreshLogs()}
            >
              {t('action.refresh')}
            </button>
          </div>
        </div>
        <div className="log-view" id="log-view" ref={viewRef} onScroll={onScroll}>
          {emptyMessage && !lines.length ? (
            <div className="log-line info">{emptyMessage}</div>
          ) : (
            lines.map((line) => (
              <div key={line.id} className={`log-line ${line.level}`}>
                {line.msg}
              </div>
            ))
          )}
        </div>
      </div>
    </section>
  )
}
