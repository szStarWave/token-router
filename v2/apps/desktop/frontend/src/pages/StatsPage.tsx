/**
 * Stats（§7.3）：summary + series + models；时间窗/分组/指标切换。
 * 窗口过大（413）→ 缩窗提示。
 */
import { useState } from 'react'
import {
  useHeatmapQuery,
  useStatsModelsQuery,
  useStatsSeriesQuery,
  useStatsSummaryQuery,
  useUsageQuery,
} from '../queries/admin'
import { QueryState, errorMessage } from '../components/common/QueryState'

type Bucket = 'hour' | 'day'
type GroupBy = 'dispatch_model' | 'outcome' | 'protocol'
type Metric = 'requests' | 'successes' | 'cost' | 'input_tokens' | 'output_tokens'

const RANGES = [
  { label: '最近 24 小时', short: '24 小时', hours: 24 },
  { label: '最近 3 天', short: '3 天', hours: 72 },
  { label: '最近 7 天', short: '7 天', hours: 168 },
  { label: '最近 30 天', short: '30 天', hours: 720 },
] as const

function windowFor(hours: number): { from: number; to: number } {
  const to = Date.now()
  return { from: to - hours * 3600 * 1000, to }
}

export function StatsPage() {
  const [hours, setHours] = useState<number>(24)
  const [bucket] = useState<Bucket>(`hour` as Bucket)
  const [groupBy, setGroupBy] = useState<GroupBy>('dispatch_model')
  const [metric, setMetric] = useState<Metric>('requests')

  const window = windowFor(hours)
  const summaryQuery = useStatsSummaryQuery(window)
  const seriesQuery = useStatsSeriesQuery({
    from: window.from,
    to: window.to,
    bucket: hours >= 168 ? 'day' : 'hour',
    group_by: groupBy,
    metric,
  })
  const modelsQuery = useStatsModelsQuery(window)

  const summary = summaryQuery.data
  const series = seriesQuery.data
  const maxPoint = series
    ? Math.max(1, ...series.series.flatMap((group) => group.points.map((point) => point.value)))
    : 1

  return (
    <div className="app-page">
      <section className="app-card" aria-label="统计汇总">
        <h2 className="app-card-title">汇总</h2>
        <div className="app-filter-bar">
          <div className="oc-seg" role="group" aria-label="时间窗">
            {RANGES.map((range) => (
              <button
                key={range.hours}
                type="button"
                className={hours === range.hours ? 'oc-seg-item-active' : 'oc-seg-item'}
                aria-pressed={hours === range.hours}
                title={range.label}
                onClick={() => setHours(range.hours)}
              >
                {range.short}
              </button>
            ))}
          </div>
        </div>
        <QueryState
          loading={summaryQuery.isLoading}
          error={summaryQuery.error}
          onRetry={() => void summaryQuery.refetch()}
        >
          {summary && (
            <dl className="app-kv">
              <div className="app-kv-item">
                <dt className="app-kv-key">请求</dt>
                <dd className="app-kv-value">{summary.requests}</dd>
              </div>
              <div className="app-kv-item">
                <dt className="app-kv-key">成功</dt>
                <dd className="app-kv-value">{summary.successes}</dd>
              </div>
              <div className="app-kv-item">
                <dt className="app-kv-key">成功率</dt>
                <dd className="app-kv-value">{(summary.success_rate * 100).toFixed(1)}%</dd>
              </div>
              <div className="app-kv-item">
                <dt className="app-kv-key">上游错误</dt>
                <dd className="app-kv-value">{summary.upstream_errors}</dd>
              </div>
              <div className="app-kv-item">
                <dt className="app-kv-key">质量失败</dt>
                <dd className="app-kv-value">{summary.quality_fails}</dd>
              </div>
              <div className="app-kv-item">
                <dt className="app-kv-key">取消</dt>
                <dd className="app-kv-value">{summary.cancelled}</dd>
              </div>
              <div className="app-kv-item">
                <dt className="app-kv-key">拒绝</dt>
                <dd className="app-kv-value">{summary.rejected}</dd>
              </div>
              <div className="app-kv-item">
                <dt className="app-kv-key">切换</dt>
                <dd className="app-kv-value">{summary.switches}</dd>
              </div>
              <div className="app-kv-item">
                <dt className="app-kv-key">TTFT p95</dt>
                <dd className="app-kv-value">{summary.latency.ttft_p95 ?? '—'}ms</dd>
              </div>
              <div className="app-kv-item">
                <dt className="app-kv-key">耗时 p95</dt>
                <dd className="app-kv-value">{summary.latency.duration_p95 ?? '—'}ms</dd>
              </div>
              <div className="app-kv-item">
                <dt className="app-kv-key">tokens (in/out)</dt>
                <dd className="app-kv-value">
                  {summary.tokens.input} / {summary.tokens.output}
                </dd>
              </div>
              <div className="app-kv-item">
                <dt className="app-kv-key">cache (read/write)</dt>
                <dd className="app-kv-value">
                  {summary.tokens.cache_read} / {summary.tokens.cache_write}
                </dd>
              </div>
              <div className="app-kv-item">
                <dt className="app-kv-key">成本 answer</dt>
                <dd className="app-kv-value">{summary.cost_micros.answer} micros</dd>
              </div>
              <div className="app-kv-item">
                <dt className="app-kv-key">成本 routing</dt>
                <dd className="app-kv-value">{summary.cost_micros.routing} micros</dd>
              </div>
              <div className="app-kv-item">
                <dt className="app-kv-key">判官调用</dt>
                <dd className="app-kv-value">
                  {summary.judge.calls} (失败 {summary.judge.failed})
                </dd>
              </div>
            </dl>
          )}
        </QueryState>
      </section>

      <section className="app-card" aria-label="时间序列">
        <h2 className="app-card-title">时间序列</h2>
        <div className="app-filter-bar">
          <label className="app-filter-label">
            分组
            <select
              className="app-select-sm"
              value={groupBy}
              onChange={(e) => setGroupBy(e.target.value as GroupBy)}
            >
              <option value="dispatch_model">派发模型</option>
              <option value="outcome">结果</option>
              <option value="protocol">协议</option>
            </select>
          </label>
          <label className="app-filter-label">
            指标
            <select
              className="app-select-sm"
              value={metric}
              onChange={(e) => setMetric(e.target.value as Metric)}
            >
              <option value="requests">请求数</option>
              <option value="successes">成功数</option>
              <option value="cost">成本</option>
              <option value="input_tokens">输入 token</option>
              <option value="output_tokens">输出 token</option>
            </select>
          </label>
        </div>
        <QueryState
          loading={seriesQuery.isLoading}
          error={seriesQuery.error}
          empty={(series?.series.length ?? 0) === 0}
          onRetry={() => void seriesQuery.refetch()}
        >
          {seriesQuery.error !== null &&
            errorMessage(seriesQuery.error).includes('window_too_large') && (
              <div className="app-banner app-banner-warn">窗口过大，请缩小时间范围。</div>
            )}
          {series && (
            <div className="app-series" role="img" aria-label={`${groupBy} 按 ${bucket} 的${metric}序列`}>
              {series.series.map((group) => (
                <div key={group.key} className="app-series-row">
                  <div className="app-series-key font-mono">{group.key}</div>
                  <div className="app-series-bars">
                    {group.points.map((point) => (
                      <div
                        key={point.t}
                        className="app-series-bar"
                        style={{
                          height: `${Math.max(2, (point.value / maxPoint) * 64)}px`,
                          opacity: point.value === 0 ? 0.15 : 1,
                        }}
                        title={`${new Date(point.t).toLocaleString()} = ${point.value}`}
                      />
                    ))}
                  </div>
                </div>
              ))}
            </div>
          )}
          {series && (
            <details className="text-meta text-muted-foreground">
              <summary className="cursor-pointer py-1">表格替代视图（无障碍）</summary>
              <table className="app-table">
                <thead>
                  <tr>
                    <th>{groupBy}</th>
                    <th>时间</th>
                    <th>{metric}</th>
                  </tr>
                </thead>
                <tbody>
                  {series.series.flatMap((group) =>
                    group.points
                      .filter((point) => point.value > 0)
                      .map((point) => (
                        <tr key={`${group.key}-${point.t}`}>
                          <td className="font-mono">{group.key}</td>
                          <td>{new Date(point.t).toLocaleString()}</td>
                          <td>{point.value}</td>
                        </tr>
                      )),
                  )}
                </tbody>
              </table>
            </details>
          )}
        </QueryState>
      </section>

      <section className="app-card" aria-label="模型维度">
        <h2 className="app-card-title">模型维度</h2>
        <QueryState
          loading={modelsQuery.isLoading}
          error={modelsQuery.error}
          empty={(modelsQuery.data?.items.length ?? 0) === 0}
          onRetry={() => void modelsQuery.refetch()}
        >
          <table className="app-table">
            <thead>
              <tr>
                <th>模型</th>
                <th>请求</th>
                <th>成功</th>
                <th>输入</th>
                <th>输出</th>
                <th>成本</th>
              </tr>
            </thead>
            <tbody>
              {modelsQuery.data?.items.map((item) => (
                <tr key={String(item.model)}>
                  <td className="font-mono">{String(item.model)}</td>
                  <td>{String(item.requests)}</td>
                  <td>{String(item.successes)}</td>
                  <td>{String(item.input_tokens)}</td>
                  <td>{String(item.output_tokens)}</td>
                  <td>{String(item.cost_micros)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </QueryState>
      </section>

      <UsageBreakdown window={window} />
      <UsageHeatmap window={window} />
    </div>
  )
}

/** 成本分解（v3 P5-C）：按 agent/model/candidate/account/session 五维出账。 */
function UsageBreakdown({ window }: { window: { from: number; to: number } }) {
  const [by, setBy] = useState('agent')
  const usageQuery = useUsageQuery(by, window.from, window.to)
  const totals = usageQuery.data?.totals

  return (
    <section className="app-card" aria-label="成本分解">
      <div className="flex items-center gap-2 flex-wrap">
        <h2 className="app-card-title flex-1">成本分解</h2>
        <label className="app-filter-label">
          维度
          <select className="app-select-sm" value={by} onChange={(e) => setBy(e.target.value)}>
            <option value="agent">agent</option>
            <option value="model">虚拟模型</option>
            <option value="candidate">物理候选</option>
            <option value="account">账号</option>
            <option value="session">会话</option>
          </select>
        </label>
      </div>
      <QueryState
        loading={usageQuery.isLoading}
        error={usageQuery.error}
        empty={usageQuery.data?.rows.length === 0}
        onRetry={() => void usageQuery.refetch()}
      >
        <table className="app-table">
          <thead>
            <tr>
              <th>{by}</th>
              <th>请求</th>
              <th>输入 / 输出</th>
              <th>cache 读 / 写</th>
              <th>命中率</th>
              <th>TTFT 均值</th>
              <th>成本（answer / routing）</th>
            </tr>
          </thead>
          <tbody>
            {usageQuery.data?.rows.map((row) => (
              <tr key={row.key}>
                <td className="font-mono">{row.key}</td>
                <td>{row.requests}</td>
                <td>
                  {row.input_tokens} / {row.output_tokens}
                </td>
                <td>
                  {row.cache_read_tokens} / {row.cache_write_tokens}
                </td>
                <td>
                  {row.cache_hit_ratio === null
                    ? '—'
                    : `${(row.cache_hit_ratio * 100).toFixed(0)}%`}
                </td>
                <td>{row.avg_ttft_ms === null ? '—' : `${row.avg_ttft_ms.toFixed(0)}ms`}</td>
                <td>
                  {row.answer_cost_micros} / {row.routing_cost_micros}
                </td>
              </tr>
            ))}
            {totals && (
              <tr className="font-semibold">
                <td>合计</td>
                <td>{totals.requests}</td>
                <td>
                  {totals.input_tokens} / {totals.output_tokens}
                </td>
                <td>
                  {totals.cache_read_tokens} / {totals.cache_write_tokens}
                </td>
                <td>
                  {totals.cache_hit_ratio === null
                    ? '—'
                    : `${(totals.cache_hit_ratio * 100).toFixed(0)}%`}
                </td>
                <td>{totals.avg_ttft_ms === null ? '—' : `${totals.avg_ttft_ms.toFixed(0)}ms`}</td>
                <td>
                  {totals.answer_cost_micros} / {totals.routing_cost_micros}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </QueryState>
    </section>
  )
}

/** 活动热力图（UTC 天 × 小时）。 */
function UsageHeatmap({ window }: { window: { from: number; to: number } }) {
  const heatmapQuery = useHeatmapQuery(window.from, window.to)
  const data = heatmapQuery.data

  // day → [24] 单元格。
  const days = new Map<string, number[]>()
  for (const cell of data?.cells ?? []) {
    if (!days.has(cell.day)) days.set(cell.day, Array.from({ length: 24 }, () => 0))
    const row = days.get(cell.day)
    if (row) row[cell.hour] = cell.requests
  }
  const sortedDays = Array.from(days.keys()).sort()
  const max = Math.max(1, data?.max_requests ?? 1)

  return (
    <section className="app-card" aria-label="活动热力图">
      <h2 className="app-card-title">活动热力图（UTC 小时）</h2>
      <QueryState
        loading={heatmapQuery.isLoading}
        error={heatmapQuery.error}
        empty={sortedDays.length === 0}
        onRetry={() => void heatmapQuery.refetch()}
      >
        <div className="flex flex-col gap-1 overflow-x-auto">
          {sortedDays.map((day) => (
            <div key={day} className="flex items-center gap-2">
              <span className="text-meta text-muted-foreground font-mono w-21 shrink-0">{day}</span>
              <div className="flex gap-0.5">
                {(days.get(day) ?? []).map((requests, hour) => {
                  const intensity = requests === 0 ? 0 : 0.15 + 0.85 * (requests / max)
                  return (
                    <div
                      key={hour}
                      className="size-2.5 rounded-sm bg-primary"
                      style={{ opacity: intensity }}
                      title={`${day} ${String(hour).padStart(2, '0')}:00 · ${requests} 请求`}
                    />
                  )
                })}
              </div>
            </div>
          ))}
        </div>
      </QueryState>
    </section>
  )
}
