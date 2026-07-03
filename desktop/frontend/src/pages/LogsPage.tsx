import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import {
  fetchGatewayLogs,
  fetchRoutingLogs,
  useGatewayLogsQuery,
  useRoutingLogsQuery,
} from '../queries/gateway'
import { useAppStore } from '../stores/appStore'
import { useI18n } from '../hooks/useI18n'
import { gatewayOpenLogsDir } from '../lib/tauri'
import { LOG_LOAD_OLDER_THRESHOLD, LOG_MAX_LINES } from '../constants/defaults'
import { isRoutingLogLine, mapApiRoutingEntry, type RoutingLogEntry } from '../lib/routing-log'
import { RoutingLogCard } from '../components/logs/RoutingLogCard'

interface LogEntry {
  id: number
  level: string
  msg: string
}

type LogView = 'routing' | 'other'

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

function queryErrorMessage(err: unknown): string {
  if (err instanceof Error && err.message) return err.message
  if (typeof err === 'string' && err) return err
  return 'unknown error'
}

export function LogsPage() {
  const { t } = useI18n()
  const connected = useAppStore((s) => s.connected)
  const showToast = useAppStore((s) => s.showToast)
  const [logView, setLogView] = useState<LogView>('routing')
  const [tailOffset, setTailOffset] = useState<number | null>(null)
  const [headOffset, setHeadOffset] = useState(0)
  const [hasOlder, setHasOlder] = useState(false)
  const [loadingOlder, setLoadingOlder] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [lines, setLines] = useState<LogEntry[]>([])
  const [routingEntries, setRoutingEntries] = useState<RoutingLogEntry[]>([])
  const [routingHasOlder, setRoutingHasOlder] = useState(false)
  const viewRef = useRef<HTMLDivElement>(null)
  const stickToBottomRef = useRef(true)
  const loadingOlderRef = useRef(false)
  const scrollAnchorRef = useRef<{ prevHeight: number; prevTop: number } | null>(null)
  const trimNewestOnCapRef = useRef(false)
  const prevScrollTopRef = useRef(0)
  const headOffsetRef = useRef(0)

  const updateStickToBottom = (view: HTMLDivElement) => {
    stickToBottomRef.current = view.scrollHeight - view.scrollTop - view.clientHeight < 48
  }

  const pollAfterId =
    routingEntries.length > 0 ? routingEntries[routingEntries.length - 1].id : null
  const logsQuery = useGatewayLogsQuery(tailOffset, logView === 'other')
  const routingQuery = useRoutingLogsQuery(pollAfterId, logView === 'routing')

  const otherLines = useMemo(
    () => lines.filter((line) => !isRoutingLogLine(line.msg)),
    [lines],
  )

  useEffect(() => {
    headOffsetRef.current = headOffset
  }, [headOffset])

  useEffect(() => {
    const data = logsQuery.data
    if (!data || logView !== 'other') return

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
  }, [logsQuery.data, logView])

  useEffect(() => {
    const data = routingQuery.data
    if (!data || logView !== 'routing') return

    const incoming = data.entries.map(mapApiRoutingEntry)
    setRoutingHasOlder(data.has_older)

    if (pollAfterId == null) {
      setRoutingEntries(incoming)
      return
    }
    if (!incoming.length) return

    setRoutingEntries((prev) => {
      const seen = new Set(prev.map((e) => e.id))
      const next = [...prev]
      for (const entry of incoming) {
        if (!seen.has(entry.id)) next.push(entry)
      }
      return next
    })
  }, [routingQuery.data, logView, pollAfterId])

  const emptyMessage = useMemo(() => {
    if (!connected) return t('logs.offline')
    if (logView === 'routing') {
      if (routingQuery.isError) {
        return t('logs.loadFail', { msg: queryErrorMessage(routingQuery.error) })
      }
      if (routingQuery.isLoading && !routingEntries.length) return t('logs.loading')
      if (!routingEntries.length) return t('logs.routingEmpty')
      return ''
    }
    if (logsQuery.isError) return t('logs.loadFail', { msg: queryErrorMessage(logsQuery.error) })
    if (logsQuery.isLoading && !lines.length) return t('logs.loading')
    if (!lines.length) return t('logs.waiting')
    if (!otherLines.length) return t('logs.empty')
    return ''
  }, [
    connected,
    logView,
    logsQuery.isError,
    logsQuery.isLoading,
    logsQuery.error,
    routingQuery.isError,
    routingQuery.isLoading,
    routingQuery.error,
    lines.length,
    routingEntries.length,
    otherLines.length,
    t,
  ])

  const scrollContentKey =
    logView === 'routing'
      ? routingEntries.map((e) => e.id).join(',')
      : otherLines.map((l) => l.id).join(',')

  useEffect(() => {
    const view = viewRef.current
    if (!view) return
    const ro = new ResizeObserver(() => {
      const el = viewRef.current
      if (el) updateStickToBottom(el)
    })
    ro.observe(view)
    return () => ro.disconnect()
  }, [logView, scrollContentKey])

  useLayoutEffect(() => {
    const view = viewRef.current
    if (!view) return

    if (scrollAnchorRef.current) {
      const { prevHeight, prevTop } = scrollAnchorRef.current
      scrollAnchorRef.current = null
      view.scrollTop = prevTop + (view.scrollHeight - prevHeight)
      updateStickToBottom(view)
      return
    }

    if (!scrollContentKey || !stickToBottomRef.current) return
    view.scrollTop = view.scrollHeight - view.clientHeight
  }, [scrollContentKey, logView])

  const loadOlderOther = async () => {
    const before = headOffsetRef.current
    if (!hasOlder || loadingOlderRef.current || before <= 0) return
    loadingOlderRef.current = true
    setLoadingOlder(true)
    try {
      const data = await fetchGatewayLogs({ beforeOffset: before })
      setHeadOffset(data.offset)
      setHasOlder(data.offset > 0)
      if (!data.lines.length) return
      const view = viewRef.current
      if (view) {
        scrollAnchorRef.current = { prevHeight: view.scrollHeight, prevTop: view.scrollTop }
        trimNewestOnCapRef.current = view.scrollTop > LOG_LOAD_OLDER_THRESHOLD
      }
      setLines((prev) => {
        let next = [...mapLogLines(data.lines), ...prev]
        if (next.length > LOG_MAX_LINES) {
          next = trimNewestOnCapRef.current
            ? next.slice(-LOG_MAX_LINES)
            : next.slice(0, LOG_MAX_LINES)
        }
        return next
      })
    } catch (e) {
      showToast('logs.loadFail', false, { msg: e instanceof Error ? e.message : String(e) })
    } finally {
      loadingOlderRef.current = false
      setLoadingOlder(false)
    }
  }

  const loadOlderRouting = async () => {
    if (!routingHasOlder || loadingOlderRef.current || !routingEntries.length) return
    loadingOlderRef.current = true
    setLoadingOlder(true)
    try {
      const data = await fetchRoutingLogs({ beforeId: routingEntries[0].id })
      setRoutingHasOlder(data.has_older)
      if (!data.entries.length) return
      const view = viewRef.current
      if (view) {
        scrollAnchorRef.current = { prevHeight: view.scrollHeight, prevTop: view.scrollTop }
      }
      const incoming = data.entries.map(mapApiRoutingEntry)
      setRoutingEntries((prev) => {
        const seen = new Set(prev.map((e) => e.id))
        const prepended = incoming.filter((e) => !seen.has(e.id))
        return [...prepended, ...prev]
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
    const scrollingUp = view.scrollTop < prevScrollTopRef.current
    prevScrollTopRef.current = view.scrollTop
    updateStickToBottom(view)
    const atTop = view.scrollTop <= LOG_LOAD_OLDER_THRESHOLD
    const canScroll = view.scrollHeight > view.clientHeight
    if (!scrollingUp || !atTop || !canScroll || loadingOlderRef.current) return
    if (logView === 'routing') {
      if (routingHasOlder) void loadOlderRouting()
      return
    }
    if (hasOlder) void loadOlderOther()
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
      if (logView === 'routing') {
        const data = await fetchRoutingLogs({})
        setRoutingEntries(data.entries.map(mapApiRoutingEntry))
        setRoutingHasOlder(data.has_older)
      } else {
        const data = await fetchGatewayLogs({ offset: null })
        setTailOffset(data.next_offset)
        setHeadOffset(data.offset)
        setHasOlder(data.offset > 0)
        let next = mapLogLines(data.lines)
        if (next.length > LOG_MAX_LINES) next = next.slice(-LOG_MAX_LINES)
        setLines(next)
      }
    } catch (e) {
      showToast('logs.loadFail', false, { msg: e instanceof Error ? e.message : String(e) })
    } finally {
      setRefreshing(false)
    }
  }

  const showEmpty = logView === 'routing' ? !routingEntries.length : !otherLines.length

  return (
    <section className="page active" id="page-logs">
      <div className="panel">
        <div className="panel-title" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
          <span>{t('logs.title')}</span>
          <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
            {loadingOlder && (
              <span className="log-loading-hint">{t('logs.loadingOlder')}</span>
            )}
            <div className="log-view-toggle" role="tablist" aria-label={t('logs.title')}>
              <button
                type="button"
                role="tab"
                aria-selected={logView === 'routing'}
                className={`log-view-toggle__btn${logView === 'routing' ? ' active' : ''}`}
                onClick={() => {
                  stickToBottomRef.current = true
                  setLogView('routing')
                }}
              >
                {t('logs.tabRouting')}
              </button>
              <button
                type="button"
                role="tab"
                aria-selected={logView === 'other'}
                className={`log-view-toggle__btn${logView === 'other' ? ' active' : ''}`}
                onClick={() => {
                  stickToBottomRef.current = true
                  setLogView('other')
                }}
              >
                {t('logs.tabOther')}
              </button>
            </div>
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
        <div
          className={logView === 'routing' ? 'routing-log-list' : 'log-view'}
          id="log-view"
          ref={viewRef}
          onScroll={onScroll}
        >
          {emptyMessage && showEmpty ? (
            <div className="log-line info">{emptyMessage}</div>
          ) : logView === 'routing' ? (
            routingEntries.map((entry) => <RoutingLogCard key={entry.id} entry={entry} />)
          ) : (
            otherLines.map((line) => (
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
