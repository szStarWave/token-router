/**
 * 用量（§7.3）：观测视角的用量看板。
 *
 * 结构参照 Magpie 的「观测 / 用量 / 会话」页：
 *   1. 时间范围（今天 / 7 天 / 30 天 / 90 天 / 全部）+ 粒度
 *   2. 六个指标格（会话数 / Token / 费用 / 缓存读取 / 活跃时长 / 参与身份）
 *   3. 每日趋势（Token / 输出 Token / 请求 / 费用）
 *   4. 两个排行（维度 + 模型）
 *   5. 按时段热力 + 最耗会话
 *   6. 会话形态 + 常用授权身份
 *
 * 口径：全部来自 admin 契约（统计 / 用量聚合 / 热力 / 请求账本）。
 * Magpie 里的「项目」「消息」「工具调用」「SKILLS」我们没有对应字段，
 * 用会话、请求数、授权身份顶替，并按真实口径标注，不硬套名字。
 */
import { useEffect, useMemo, useState } from 'react'
import {
  useHeatmapQuery,
  useRequestsQuery,
  useStatsSeriesQuery,
  useStatsSummaryQuery,
  useUsageQuery,
} from '../queries/admin'
import { QueryState, errorMessage } from '../components/common/QueryState'
import { formatCompactNum } from '../lib/format-number'
import {
  RankList,
  SessionShape,
  TopSessions,
  TrendChart,
  WeekdayHeatmap,
  aggregateSessions,
  rankRowsFromUsage,
  type SessionMetric,
  type TrendMetric,
  TREND_METRICS,
} from './usage/panels'
import { fmtCost } from './activity/shared'
import { formatModelLabel } from './activity/shared'
import { usePrefsStore } from '../stores/prefsStore'

type RangeKey = 'today' | '7d' | '30d' | '90d' | 'all'
type Bucket = 'hour' | 'day'
type Dimension = 'session' | 'candidate' | 'account' | 'agent'
type SessionTab = SessionMetric

const RANGES: Array<{ key: RangeKey; label: string }> = [
  { key: 'today', label: '今天' },
  { key: '7d', label: '7 天' },
  { key: '30d', label: '30 天' },
  { key: '90d', label: '90 天' },
  { key: 'all', label: '全部' },
]

const DIMENSIONS: Array<{ key: Dimension; label: string }> = [
  { key: 'session', label: '会话' },
  { key: 'candidate', label: '候选' },
  { key: 'account', label: '账号' },
  { key: 'agent', label: '授权身份' },
]

const SESSION_TABS: Array<{ key: SessionTab; label: string }> = [
  { key: 'token', label: 'Token' },
  { key: 'cost', label: '费用' },
  { key: 'active', label: '活跃' },
]

/**
 * 时间窗必须稳定：`to` 若在渲染期取 `Date.now()`，查询键每次都变，
 * 每个响应都会触发一组全新查询（无限重取）。这里由调用方传入按分钟刷新的 now。
 */
function windowFor(range: RangeKey, to: number): { from: number; to: number } {
  const day = 86_400_000
  if (range === 'today') {
    const start = new Date(to)
    start.setHours(0, 0, 0, 0)
    return { from: start.getTime(), to }
  }
  if (range === '7d') return { from: to - 7 * day, to }
  if (range === '30d') return { from: to - 30 * day, to }
  if (range === '90d') return { from: to - 90 * day, to }
  return { from: 0, to }
}

function percentile(values: number[], p: number): number | null {
  if (values.length === 0) return null
  const sorted = [...values].sort((a, b) => a - b)
  const index = Math.min(sorted.length - 1, Math.max(0, Math.ceil((p / 100) * sorted.length) - 1))
  return sorted[index] ?? null
}

export function StatsPage() {
  const [range, setRange] = useState<RangeKey>('30d')
  const [bucket, setBucket] = useState<Bucket>('day')
  const [dimension, setDimension] = useState<Dimension>('session')
  const [trendMetric, setTrendMetric] = useState<TrendMetric>('token')
  const [sessionTab, setSessionTab] = useState<SessionTab>('token')
  const modelNamePref = usePrefsStore((s) => s.modelName)
  // 每分钟走一格：窗口只在分钟边界变化，避免每次渲染都生成新查询键。
  const [nowMs, setNowMs] = useState(() => Date.now())
  useEffect(() => {
    const timer = window.setInterval(() => setNowMs(Date.now()), 60_000)
    return () => window.clearInterval(timer)
  }, [])
  const timeWindow = useMemo(() => windowFor(range, nowMs), [range, nowMs])
  const summaryQuery = useStatsSummaryQuery(timeWindow)
  const usageQuery = useUsageQuery(dimension, timeWindow.from, timeWindow.to)
  const modelUsageQuery = useUsageQuery('model', timeWindow.from, timeWindow.to)
  const agentUsageQuery = useUsageQuery('agent', timeWindow.from, timeWindow.to)
  const heatmapQuery = useHeatmapQuery(timeWindow.from, timeWindow.to)
  const requestsQuery = useRequestsQuery({ limit: 200 })

  const inputSeries = useStatsSeriesQuery({
    from: timeWindow.from,
    to: timeWindow.to,
    bucket,
    group_by: 'dispatch_model',
    metric: 'input_tokens',
  })
  const outputSeries = useStatsSeriesQuery({
    from: timeWindow.from,
    to: timeWindow.to,
    bucket,
    group_by: 'dispatch_model',
    metric: 'output_tokens',
  })
  const requestSeries = useStatsSeriesQuery({
    from: timeWindow.from,
    to: timeWindow.to,
    bucket,
    group_by: 'dispatch_model',
    metric: 'requests',
  })
  const costSeries = useStatsSeriesQuery({
    from: timeWindow.from,
    to: timeWindow.to,
    bucket,
    group_by: 'dispatch_model',
    metric: 'cost',
  })

  const summary = summaryQuery.data
  const totals = usageQuery.data?.totals
  const requests = requestsQuery.data?.items ?? []

  const sessions = useMemo(() => aggregateSessions(requests), [requests])

  const sessionTokens = useMemo(
    () => sessions.map((session) => session.tokens).filter((value) => value > 0),
    [sessions],
  )
  const medianSession = percentile(sessionTokens, 50)
  const p90Session = percentile(sessionTokens, 90)

  const inputTokens = totals?.input_tokens ?? summary?.tokens.input ?? 0
  const outputTokens = totals?.output_tokens ?? summary?.tokens.output ?? 0
  const cacheRead = totals?.cache_read_tokens ?? summary?.tokens.cache_read ?? 0
  const cacheWrite = totals?.cache_write_tokens ?? summary?.tokens.cache_write ?? 0
  const cost = (totals?.answer_cost_micros ?? summary?.cost_micros.answer ?? 0) +
    (totals?.routing_cost_micros ?? summary?.cost_micros.routing ?? 0)
  const cacheRatio =
    inputTokens + cacheRead > 0 ? cacheRead / (inputTokens + cacheRead) : null

  // 活跃时长：窗口内已加载请求的耗时合计（账本没做更细的活跃区间）。
  const activeMs = requests.reduce((sum, row) => sum + row.duration_ms, 0)
  const activeHours = activeMs / 3_600_000

  const trendParts = (() => {
    if (trendMetric === 'token') {
      return [
        { label: '输入', series: inputSeries.data },
        { label: '输出', series: outputSeries.data },
      ]
    }
    if (trendMetric === 'output_token') return [{ label: '输出', series: outputSeries.data }]
    if (trendMetric === 'requests') return [{ label: '请求', series: requestSeries.data }]
    return [{ label: '费用', series: costSeries.data }]
  })()

  const windowTooLarge = [summaryQuery, inputSeries, requestSeries].some(
    (query) => query.error !== null && errorMessage(query.error).includes('window_too_large'),
  )

  const refreshAll = () => {
    void summaryQuery.refetch()
    void usageQuery.refetch()
    void modelUsageQuery.refetch()
    void agentUsageQuery.refetch()
    void heatmapQuery.refetch()
    void requestsQuery.refetch()
    void inputSeries.refetch()
    void outputSeries.refetch()
    void requestSeries.refetch()
    void costSeries.refetch()
  }

  return (
    <div className="app-page">
      {/* 头部：范围 + 粒度 */}
      <section className="flex flex-col gap-3">
        <div className="flex items-center gap-3 flex-wrap">
          <div className="ml-auto flex items-center gap-2">
            <button type="button" className="oc-btn-default oc-btn-sm" onClick={refreshAll}>
              更新
            </button>
            <label className="app-filter-label">
              粒度
              <select
                className="app-select-sm"
                value={bucket}
                onChange={(e) => setBucket(e.target.value as Bucket)}
              >
                <option value="hour">小时</option>
                <option value="day">天</option>
              </select>
            </label>
          </div>
        </div>
        <div className="app-filter-bar">
          {RANGES.map((item) => (
            <button
              key={item.key}
              type="button"
              className={range === item.key ? 'app-chip app-chip-active' : 'app-chip'}
              onClick={() => setRange(item.key)}
            >
              {item.label}
            </button>
          ))}
          <label className="app-filter-label ml-auto">
            排行维度
            <select
              className="app-select-sm"
              value={dimension}
              onChange={(e) => setDimension(e.target.value as Dimension)}
            >
              {DIMENSIONS.map((item) => (
                <option key={item.key} value={item.key}>
                  {item.label}
                </option>
              ))}
            </select>
          </label>
        </div>
        {windowTooLarge && (
          <div className="app-banner app-banner-warn">
            窗口过大，趋势接口拒绝了这个范围——缩小时间范围或改用「天」粒度。
          </div>
        )}
      </section>

      {/* 指标格 */}
      <section className="app-usage-metrics" aria-label="指标">
        <div className="app-usage-metric">
          <span className="app-metric-value">{formatCompactNum(sessions.length)}</span>
          <span className="app-metric-key">会话数</span>
          <span className="app-metric-note">
            最近 200 条请求 · 中位{' '}
            {medianSession === null ? '—' : formatCompactNum(medianSession)} · p90{' '}
            {p90Session === null ? '—' : formatCompactNum(p90Session)}
          </span>
        </div>
        <div className="app-usage-metric">
          <span className="app-metric-value">{formatCompactNum(inputTokens + outputTokens)}</span>
          <span className="app-metric-key">Token</span>
          <span className="app-metric-note">
            输入 {formatCompactNum(inputTokens)} · 输出 {formatCompactNum(outputTokens)}
          </span>
        </div>
        <div className="app-usage-metric">
          <span className="app-metric-value">{fmtCost(cost)}</span>
          <span className="app-metric-key">费用</span>
        </div>
        <div className="app-usage-metric">
          <span className="app-metric-value">{formatCompactNum(cacheRead + cacheWrite)}</span>
          <span className="app-metric-key">缓存读取</span>
          <span className="app-metric-note">
            命中率 {cacheRatio === null ? '—' : `${(cacheRatio * 100).toFixed(2)}%`}
          </span>
        </div>
        <div className="app-usage-metric">
          <span className="app-metric-value">
            {activeHours < 1
              ? `${Math.round(activeMs / 1000)} 秒`
              : `${activeHours.toFixed(1)} 小时`}
          </span>
          <span className="app-metric-key">活跃时长</span>
        </div>
        <div className="app-usage-metric">
          <span className="app-metric-value">
            {formatCompactNum(agentUsageQuery.data?.rows.length ?? 0)}
          </span>
          <span className="app-metric-key">授权身份</span>
          <span className="app-metric-note">
            模型 {formatCompactNum(modelUsageQuery.data?.rows.length ?? 0)} 个
          </span>
        </div>
      </section>

      {/* 每日趋势 */}
      <section className="app-card" aria-label="每日趋势">
        <div className="flex items-center gap-2 flex-wrap">
          <h2 className="app-card-title">每日趋势</h2>
          <div className="oc-seg ml-auto" role="group" aria-label="趋势指标">
            {TREND_METRICS.map((item) => (
              <button
                key={item.key}
                type="button"
                className={trendMetric === item.key ? 'oc-seg-item-active' : 'oc-seg-item'}
                aria-pressed={trendMetric === item.key}
                onClick={() => setTrendMetric(item.key)}
              >
                {item.label}
              </button>
            ))}
          </div>
        </div>
        <QueryState
          loading={inputSeries.isLoading && outputSeries.isLoading}
          error={inputSeries.error}
          onRetry={() => void inputSeries.refetch()}
        >
          <TrendChart parts={trendParts} metric={trendMetric} />
        </QueryState>
      </section>

      {/* 两个排行 */}
      <section className="app-two-col" aria-label="排行">
        <div className="app-card">
          <RankList
            title={`按${
              DIMENSIONS.find((item) => item.key === dimension)?.label ?? '维度'
            }排行`}
            rows={rankRowsFromUsage(usageQuery.data?.rows)}
          />
        </div>
        <div className="app-card">
          <RankList
            title="按模型排行"
            rows={rankRowsFromUsage(modelUsageQuery.data?.rows)}
            labelOf={(key) => formatModelLabel(key, modelNamePref)}
          />
        </div>
      </section>

      {/* 按时段 + 最耗会话 */}
      <section className="app-two-col" aria-label="时段与会话">
        <div className="app-card">
          <h2 className="app-card-title">按时段</h2>
          <QueryState
            loading={heatmapQuery.isLoading}
            error={heatmapQuery.error}
            onRetry={() => void heatmapQuery.refetch()}
          >
            <WeekdayHeatmap data={heatmapQuery.data} />
          </QueryState>
        </div>
        <div className="app-card">
          <div className="flex items-center gap-2 flex-wrap">
            <h2 className="app-card-title">最耗会话</h2>
            <div className="oc-seg ml-auto" role="group" aria-label="会话排序">
              {SESSION_TABS.map((item) => (
                <button
                  key={item.key}
                  type="button"
                  className={sessionTab === item.key ? 'oc-seg-item-active' : 'oc-seg-item'}
                  aria-pressed={sessionTab === item.key}
                  onClick={() => setSessionTab(item.key)}
                >
                  {item.label}
                </button>
              ))}
            </div>
          </div>
          <TopSessions sessions={sessions} metric={sessionTab} />
        </div>
      </section>

      {/* 会话形态 + 常用授权身份 */}
      <section className="app-two-col" aria-label="会话形态">
        <div className="app-card">
          <h2 className="app-card-title">会话形态</h2>
          <SessionShape sessions={sessions} />
        </div>
        <div className="app-card">
          <h2 className="app-card-title">常用授权身份</h2>
          <RankList
            title="按请求数"
            rows={rankRowsFromUsage(agentUsageQuery.data?.rows)}
          />
        </div>
      </section>
    </div>
  )
}
