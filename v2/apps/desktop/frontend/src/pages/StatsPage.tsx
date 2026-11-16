/**
 * Stats（§7.3）：summary + series + models；时间窗/分组/指标切换。
 * 窗口过大（413）→ 缩窗提示。
 */
import { useState } from 'react'
import { useStatsSummaryQuery, useStatsSeriesQuery, useStatsModelsQuery } from '../queries/admin'
import { QueryState, errorMessage } from '../components/common/QueryState'

type Bucket = 'hour' | 'day'
type GroupBy = 'dispatch_model' | 'outcome' | 'protocol'
type Metric = 'requests' | 'successes' | 'cost' | 'input_tokens' | 'output_tokens'

const RANGES = [
  { label: '最近 24 小时', hours: 24 },
  { label: '最近 3 天', hours: 72 },
  { label: '最近 7 天', hours: 168 },
  { label: '最近 30 天', hours: 720 },
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
    <div className="page">
      <section className="card" aria-label="统计汇总">
        <h2>汇总</h2>
        <div className="filter-bar">
          <label>
            时间窗{' '}
            <select
              value={hours}
              onChange={(e) => setHours(Number(e.target.value))}
              aria-label="时间窗"
            >
              {RANGES.map((range) => (
                <option key={range.hours} value={range.hours}>{range.label}</option>
              ))}
            </select>
          </label>
        </div>
        <QueryState
          loading={summaryQuery.isLoading}
          error={summaryQuery.error}
          onRetry={() => void summaryQuery.refetch()}
        >
          {summary && (
            <dl className="kv-grid">
              <div><dt>请求</dt><dd>{summary.requests}</dd></div>
              <div><dt>成功</dt><dd>{summary.successes}</dd></div>
              <div><dt>成功率</dt><dd>{(summary.success_rate * 100).toFixed(1)}%</dd></div>
              <div><dt>上游错误</dt><dd>{summary.upstream_errors}</dd></div>
              <div><dt>质量失败</dt><dd>{summary.quality_fails}</dd></div>
              <div><dt>取消</dt><dd>{summary.cancelled}</dd></div>
              <div><dt>拒绝</dt><dd>{summary.rejected}</dd></div>
              <div><dt>切换</dt><dd>{summary.switches}</dd></div>
              <div><dt>TTFT p95</dt><dd>{summary.latency.ttft_p95 ?? '—'}ms</dd></div>
              <div><dt>耗时 p95</dt><dd>{summary.latency.duration_p95 ?? '—'}ms</dd></div>
              <div><dt>tokens (in/out)</dt><dd>{summary.tokens.input} / {summary.tokens.output}</dd></div>
              <div><dt>cache (read/write)</dt><dd>{summary.tokens.cache_read} / {summary.tokens.cache_write}</dd></div>
              <div><dt>成本 answer</dt><dd>{summary.cost_micros.answer} micros</dd></div>
              <div><dt>成本 routing</dt><dd>{summary.cost_micros.routing} micros</dd></div>
              <div><dt>判官调用</dt><dd>{summary.judge.calls} (失败 {summary.judge.failed})</dd></div>
            </dl>
          )}
        </QueryState>
      </section>

      <section className="card" aria-label="时间序列">
        <h2>时间序列</h2>
        <div className="filter-bar">
          <label>
            分组{' '}
            <select value={groupBy} onChange={(e) => setGroupBy(e.target.value as GroupBy)}>
              <option value="dispatch_model">派发模型</option>
              <option value="outcome">结果</option>
              <option value="protocol">协议</option>
            </select>
          </label>
          <label>
            指标{' '}
            <select value={metric} onChange={(e) => setMetric(e.target.value as Metric)}>
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
              <div className="banner banner-warn">窗口过大，请缩小时间范围。</div>
            )}
          {series && (
            <div className="series-chart" role="img" aria-label={`${groupBy} 按 ${bucket} 的${metric}序列`}>
              {series.series.map((group) => (
                <div key={group.key} className="series-row">
                  <div className="series-key mono">{group.key}</div>
                  <div className="series-bars">
                    {group.points.map((point) => (
                      <div
                        key={point.t}
                        className="series-bar"
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
            <details className="series-table-wrap">
              <summary>表格替代视图（无障碍）</summary>
              <table className="table">
                <thead>
                  <tr><th>{groupBy}</th><th>时间</th><th>{metric}</th></tr>
                </thead>
                <tbody>
                  {series.series.flatMap((group) =>
                    group.points
                      .filter((point) => point.value > 0)
                      .map((point) => (
                        <tr key={`${group.key}-${point.t}`}>
                          <td className="mono">{group.key}</td>
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

      <section className="card" aria-label="模型维度">
        <h2>模型维度</h2>
        <QueryState
          loading={modelsQuery.isLoading}
          error={modelsQuery.error}
          empty={(modelsQuery.data?.items.length ?? 0) === 0}
          onRetry={() => void modelsQuery.refetch()}
        >
          <table className="table">
            <thead>
              <tr><th>模型</th><th>请求</th><th>成功</th><th>输入</th><th>输出</th><th>成本</th></tr>
            </thead>
            <tbody>
              {modelsQuery.data?.items.map((item) => (
                <tr key={String(item.model)}>
                  <td className="mono">{String(item.model)}</td>
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
    </div>
  )
}
