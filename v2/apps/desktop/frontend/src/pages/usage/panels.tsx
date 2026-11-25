/**
 * 用量页（§7.3）的面板组件：趋势、排行、按时段热力、会话榜、会话形态。
 *
 * 口径全部来自 admin 契约：
 * - 趋势：/v2/admin/stats/series（按天分桶的堆叠柱）
 * - 排行：/v2/admin/usage?by=…（会话 / 模型 / 候选 / 账号 / 授权身份）
 * - 按时段：/v2/admin/usage/heatmap（天 × 小时）
 * - 会话榜与形态：由请求账本聚合（请求数 / 耗时），没有的量不画。
 */
import type { HeatmapResponse, RequestSummary, SeriesResponse, UsageRow } from '../../api/types'
import { formatCompactNum } from '../../lib/format-number'
import { fmtClock, fmtCost, fmtDay } from '../activity/shared'

// ---------------------------------------------------------------------------
// 每日趋势（堆叠柱）
// ---------------------------------------------------------------------------

export type TrendMetric = 'token' | 'output_token' | 'requests' | 'cost'

export const TREND_METRICS: Array<{ key: TrendMetric; label: string }> = [
  { key: 'token', label: 'Token' },
  { key: 'output_token', label: '输出 Token' },
  { key: 'requests', label: '请求' },
  { key: 'cost', label: '费用' },
]

export function TrendChart({
  parts,
  metric,
}: {
  /** 每个分片是一条序列（如「输入 token」与「输出 token」堆叠在一起）。 */
  parts: Array<{ label: string; series: SeriesResponse | undefined }>
  metric: TrendMetric
}) {
  const loaded = parts.filter((part) => part.series && part.series.series.length > 0)
  if (loaded.length === 0) {
    return <p className="text-meta text-muted-foreground">这个时间范围里没有数据。</p>
  }

  // 桶 → 分组 → 值。
  const buckets = new Map<number, Map<string, number>>()
  for (const part of loaded) {
    for (const group of part.series!.series) {
      for (const point of group.points) {
        const inner = buckets.get(point.t) ?? new Map<string, number>()
        const key = `${part.label} · ${group.key}`
        inner.set(key, (inner.get(key) ?? 0) + point.value)
        buckets.set(point.t, inner)
      }
    }
  }
  const times = [...buckets.keys()].sort((a, b) => a - b)
  const totals = times.map((time) => {
    const inner = buckets.get(time)
    let sum = 0
    if (inner) for (const value of inner.values()) sum += value
    return sum
  })
  const max = Math.max(1, ...totals)

  const formatValue = (value: number): string => {
    if (metric === 'cost') return fmtCost(value)
    if (metric === 'requests') return formatCompactNum(value)
    return `${formatCompactNum(value)} token`
  }

  return (
    <div className="flex flex-col gap-2">
      <div className="app-trend-chart" role="img" aria-label="每日趋势">
        {times.map((time, index) => {
          const total = totals[index]
          const inner = buckets.get(time)
          const height = (total / max) * 100
          return (
            <div key={time} className="app-trend-col" title={`${fmtDay(time)} · ${formatValue(total)}`}>
              <div
                className="app-trend-col-inner"
                style={{ height: `${Math.max(total > 0 ? 2 : 0, height)}%` }}
              >
                {inner &&
                  [...inner.entries()]
                    .filter(([, value]) => value > 0)
                    .sort((a, b) => b[1] - a[1])
                    .map(([key, value]) => (
                      <div
                        key={key}
                        className="app-trend-seg"
                        style={{ flexGrow: value }}
                        title={`${key} · ${formatValue(value)}`}
                      />
                    ))}
              </div>
            </div>
          )
        })}
      </div>
      <div className="app-trend-axis">
        <span>{times.length > 0 ? fmtDay(times[0]) : ''}</span>
        <span>{times.length > 1 ? fmtDay(times[times.length - 1]) : ''}</span>
      </div>
    </div>
  )
}

// ---------------------------------------------------------------------------
// 排行（横向条）
// ---------------------------------------------------------------------------

export interface RankRow {
  key: string
  tokens: number
  cost: number
  requests: number
}

export function rankRowsFromUsage(rows: UsageRow[] | undefined): RankRow[] {
  return (rows ?? []).map((row) => ({
    key: row.key,
    tokens: row.input_tokens + row.output_tokens + row.cache_read_tokens + row.cache_write_tokens,
    cost: row.answer_cost_micros + row.routing_cost_micros,
    requests: row.requests,
  }))
}

export function RankList({
  title,
  rows,
  limit = 6,
  emptyHint,
  labelOf,
}: {
  title: string
  rows: RankRow[]
  limit?: number
  emptyHint?: string
  /** 显示名改写（设置页「模型名带供应商」）。 */
  labelOf?: (key: string) => string
}) {
  const sorted = [...rows].sort((a, b) => b.tokens - a.tokens)
  const max = Math.max(1, ...sorted.map((row) => row.tokens))
  const shown = sorted.slice(0, limit)
  const rest = sorted.length - shown.length

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <h3 className="text-ui-label font-semibold">{title}</h3>
        <span className="ml-auto text-meta text-muted-foreground">{sorted.length} 项</span>
      </div>
      {shown.length === 0 ? (
        <p className="text-meta text-muted-foreground">{emptyHint ?? '—'}</p>
      ) : (
        shown.map((row) => (
          <div key={row.key} className="app-rank-row">
            <span className="font-mono text-meta truncate" title={row.key}>
              {labelOf ? labelOf(row.key) : row.key}
            </span>
            <span className="app-rank-bar">
              <span
                className="app-rank-fill"
                style={{ width: `${Math.max(2, (row.tokens / max) * 100)}%` }}
              />
            </span>
            <span className="text-meta tabular-nums text-right">
              {formatCompactNum(row.tokens)}
            </span>
            <span className="text-meta text-muted-foreground text-right tabular-nums">
              {fmtCost(row.cost)}
            </span>
          </div>
        ))
      )}
      {rest > 0 && <p className="text-meta text-muted-foreground">还有 {rest} 个</p>}
    </div>
  )
}

// ---------------------------------------------------------------------------
// 按时段（周几 × 小时）
// ---------------------------------------------------------------------------

const WEEKDAYS = ['周一', '周二', '周三', '周四', '周五', '周六', '周日']

export function WeekdayHeatmap({ data }: { data: HeatmapResponse | undefined }) {
  const cells = data?.cells ?? []
  if (cells.length === 0) {
    return <p className="text-meta text-muted-foreground">这个时间范围里没有活动。</p>
  }

  // day（UTC 日期）→ 星期索引；按本地时间对齐小时。
  const grid = Array.from({ length: 7 }, () => Array.from({ length: 24 }, () => 0))
  let busiest = { weekday: 0, hour: 0, requests: 0 }
  for (const cell of cells) {
    const date = new Date(`${cell.day}T00:00:00Z`)
    const weekday = (date.getUTCDay() + 6) % 7
    grid[weekday]![cell.hour] = cell.requests
    if (cell.requests > busiest.requests) {
      busiest = { weekday, hour: cell.hour, requests: cell.requests }
    }
  }
  const max = Math.max(1, data?.max_requests ?? 1)

  return (
    <div className="flex flex-col gap-2">
      <div className="app-heat-wrap">
        <div className="app-heat-labels">
          {WEEKDAYS.map((label) => (
            <span key={label} className="app-heat-label">
              {label}
            </span>
          ))}
        </div>
        <div className="app-heat-grid">
          {grid.map((row, weekday) => (
            <div key={weekday} className="app-heat-row">
              {row.map((requests, hour) => (
                <span
                  key={hour}
                  className="app-heat-cell"
                  style={{ opacity: requests === 0 ? 0.08 : 0.2 + 0.8 * (requests / max) }}
                  title={`${WEEKDAYS[weekday]} ${String(hour).padStart(2, '0')}:00 · ${requests} 请求`}
                />
              ))}
            </div>
          ))}
        </div>
      </div>
      <div className="flex items-center gap-2 text-meta text-muted-foreground">
        <span>最忙：{WEEKDAYS[busiest.weekday]} {String(busiest.hour).padStart(2, '0')}:00</span>
        <span className="ml-auto inline-flex items-center gap-1">
          少
          {[0.1, 0.3, 0.5, 0.7, 0.9].map((step) => (
            <span key={step} className="app-heat-cell" style={{ opacity: step }} aria-hidden />
          ))}
          多
        </span>
      </div>
    </div>
  )
}

// ---------------------------------------------------------------------------
// 最耗会话
// ---------------------------------------------------------------------------

export interface SessionAggregate {
  key: string
  requests: number
  tokens: number
  cost: number
  lastAt: number
  models: string[]
}

export function aggregateSessions(requests: RequestSummary[]): SessionAggregate[] {
  const map = new Map<string, SessionAggregate>()
  for (const row of requests) {
    const current =
      map.get(row.session_key) ??
      ({
        key: row.session_key,
        requests: 0,
        tokens: 0,
        cost: 0,
        lastAt: 0,
        models: [],
      } satisfies SessionAggregate)
    current.requests += 1
    current.tokens += row.tokens.input + row.tokens.cache_read + row.tokens.output
    current.cost += row.cost_micros.answer + row.cost_micros.routing
    current.lastAt = Math.max(current.lastAt, row.created_at)
    const model = row.selection_model
    if (!current.models.includes(model)) current.models.push(model)
    map.set(row.session_key, current)
  }
  return [...map.values()]
}

export type SessionMetric = 'token' | 'cost' | 'active'

export function TopSessions({
  sessions,
  metric,
  limit = 7,
}: {
  sessions: SessionAggregate[]
  metric: SessionMetric
  limit?: number
}) {
  const sorted = [...sessions].sort((a, b) => {
    if (metric === 'cost') return b.cost - a.cost
    if (metric === 'active') return b.lastAt - a.lastAt
    return b.tokens - a.tokens
  })
  const shown = sorted.slice(0, limit)
  if (shown.length === 0) {
    return <p className="text-meta text-muted-foreground">还没有会话。</p>
  }
  return (
    <div className="flex flex-col gap-1">
      {shown.map((session) => (
        <div key={session.key} className="app-session-row">
          <span className="app-session-badge" aria-hidden>
            {session.key.slice(0, 2).toUpperCase()}
          </span>
          <span className="min-w-0 flex-1 flex flex-col gap-0.5">
            <span className="font-mono text-ui-label truncate" title={session.key}>
              {session.key}
            </span>
            <span className="text-micro text-muted-foreground truncate">
              {session.models.slice(0, 2).join('、')} · {session.requests} 次 · {fmtClock(session.lastAt)}
            </span>
          </span>
          <span className="text-meta tabular-nums">
            {metric === 'cost'
              ? fmtCost(session.cost)
              : metric === 'active'
                ? fmtClock(session.lastAt)
                : formatCompactNum(session.tokens)}
          </span>
        </div>
      ))}
    </div>
  )
}

// ---------------------------------------------------------------------------
// 会话形态（每个会话的请求数 / 时长分布）
// ---------------------------------------------------------------------------

const REQUEST_BUCKETS = [
  { label: '1-5', min: 1, max: 5 },
  { label: '6-15', min: 6, max: 15 },
  { label: '16-30', min: 16, max: 30 },
  { label: '31-60', min: 31, max: 60 },
  { label: '61-120', min: 61, max: 120 },
  { label: '121+', min: 121, max: Number.MAX_SAFE_INTEGER },
]

export function SessionShape({ sessions }: { sessions: SessionAggregate[] }) {
  if (sessions.length === 0) {
    return <p className="text-meta text-muted-foreground">还没有会话。</p>
  }
  const counts = REQUEST_BUCKETS.map((bucket) =>
    sessions.filter((session) => session.requests >= bucket.min && session.requests <= bucket.max)
      .length,
  )
  const max = Math.max(1, ...counts)
  return (
    <div className="flex flex-col gap-2">
      <div className="app-bucket-chart">
        {REQUEST_BUCKETS.map((bucket, index) => (
          <div key={bucket.label} className="app-bucket-col">
            <span className="text-meta tabular-nums">{counts[index]}</span>
            <span
              className="app-bucket-bar"
              style={{ height: `${Math.max(2, (counts[index] / max) * 100)}%` }}
            />
            <span className="text-micro text-muted-foreground">{bucket.label}</span>
          </div>
        ))}
      </div>
      <p className="text-meta text-muted-foreground">
        横轴：一个会话里的请求数（提示加载更多）；柱高：会话数
      </p>
    </div>
  )
}
