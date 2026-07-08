import { useCallback, useEffect, useMemo, useRef } from 'react'
import * as echarts from 'echarts'
import { useQueryClient } from '@tanstack/react-query'
import { formatAxisNum, formatAxisTick } from '../../lib/format-number'
import { formatRelativeTime } from '../../lib/format-relative-time'
import { fmtNum, modelStatsKey } from '../../lib/stats-utils'
import { useI18n } from '../../hooks/useI18n'
import { useAllModelsTokenTimelineQuery } from '../../queries/gateway'
import { queryKeys } from '../../queries/keys'
import { useAppStore } from '../../stores/appStore'
import type { AllModelsTimelineResponse, ModelTokenStats, StatsScope } from '../../types/gateway'
import { StatsChartRange, type ChartRange } from './StatsChart'

function routeTagClass(tier: string): string {
  return tier === 'cloud' ? 'info' : 'ok'
}

function themeColors() {
  const dark = document.documentElement.dataset.effectiveTheme === 'dark'
  return {
    axis: dark ? '#a1a1aa' : '#71717a',
    split: dark ? 'rgba(255,255,255,0.06)' : 'rgba(0,0,0,0.06)',
  }
}

function modelMetricColors(index: number, dark: boolean) {
  const hue = (index * 137.508) % 360
  return {
    input: `hsl(${hue}, 58%, ${dark ? 54 : 44}%)`,
    output: `hsl(${(hue + 28) % 360}, 68%, ${dark ? 60 : 50}%)`,
    cached: `hsl(${(hue + 56) % 360}, 52%, ${dark ? 66 : 56}%)`,
  }
}

function pad2(n: number) {
  return String(n).padStart(2, '0')
}

function formatTimeLabel(bucketTs: number, granularity: string) {
  const d = new Date(bucketTs * 1000)
  if (granularity === 'hour') {
    return `${pad2(d.getMonth() + 1)}/${pad2(d.getDate())} ${pad2(d.getHours())}:00`
  }
  return `${d.getFullYear()}/${pad2(d.getMonth() + 1)}/${pad2(d.getDate())}`
}

function hourBucketTs(unixSecs: number): number {
  return Math.floor(unixSecs / 3600) * 3600
}

function bucketPointsForRange(range: ChartRange) {
  const now = Math.floor(Date.now() / 1000)
  if (range === 'h24') {
    const end = hourBucketTs(now)
    return {
      granularity: 'hour',
      points: Array.from({ length: 24 }, (_, i) => ({
        bucket_ts: end - (23 - i) * 3600,
        input: 0,
        output: 0,
        cached: 0,
      })),
    }
  }
  const daySecs = 86400
  const localDay = Math.floor(now / daySecs) * daySecs
  const days = range === 'd7' ? 7 : 30
  return {
    granularity: 'day',
    points: Array.from({ length: days }, (_, i) => ({
      bucket_ts: localDay - (days - 1 - i) * daySecs,
      input: 0,
      output: 0,
      cached: 0,
    })),
  }
}

function emptyTimelineSkeleton(range: ChartRange): AllModelsTimelineResponse {
  const { granularity } = bucketPointsForRange(range)
  return {
    scope: 'session',
    range,
    granularity,
    models: [],
  }
}

interface ModelTokenPanelProps {
  scope: StatsScope
  modelStats: ModelTokenStats[] | undefined
  chartRange: ChartRange
  onRangeChange: (range: ChartRange) => void
}

function timelineMatchesRequest(
  timeline: AllModelsTimelineResponse | undefined,
  range: ChartRange,
  panelScope: StatsScope,
): timeline is AllModelsTimelineResponse {
  return !!timeline && timeline.scope === panelScope && timeline.range === range
}

function buildChartSeries(
  timeline: AllModelsTimelineResponse,
  labels: { input: string; output: string; cached: string },
  dark: boolean,
  range: ChartRange,
) {
  const skeleton = bucketPointsForRange(range)
  const granularity = timeline.models[0]?.points.length
    ? timeline.granularity
    : skeleton.granularity
  const bucketPoints = timeline.models[0]?.points ?? skeleton.points
  const times = bucketPoints.map((p) => formatTimeLabel(p.bucket_ts, granularity))
  const series: Array<{
    name: string
    type: 'line'
    smooth: boolean
    showSymbol: boolean
    emphasis: { focus: 'series' }
    lineStyle: { width: number; color: string }
    itemStyle: { color: string }
    areaStyle: { opacity: number; color: string }
    data: number[]
  }> = []
  const legend: string[] = []
  let peak = 0

  timeline.models.forEach((modelSeries, index) => {
    const colors = modelMetricColors(index, dark)
    const metrics: Array<{ key: 'input' | 'output' | 'cached'; label: string; color: string; alpha: number }> = [
      { key: 'input', label: labels.input, color: colors.input, alpha: 0.16 },
      { key: 'output', label: labels.output, color: colors.output, alpha: 0.12 },
      { key: 'cached', label: labels.cached, color: colors.cached, alpha: 0.08 },
    ]
    for (const metric of metrics) {
      const name = `${modelSeries.model} · ${metric.label}`
      const data = modelSeries.points.map((p) => p[metric.key])
      peak = Math.max(peak, ...data)
      legend.push(name)
      series.push({
        name,
        type: 'line',
        smooth: true,
        showSymbol: false,
        emphasis: { focus: 'series' },
        lineStyle: { width: 2, color: metric.color },
        itemStyle: { color: metric.color },
        areaStyle: { opacity: metric.alpha, color: metric.color },
        data,
      })
    }
  })

  return { times, series, legend, peak }
}

export function ModelTokenPanel({ scope, modelStats, chartRange, onRangeChange }: ModelTokenPanelProps) {
  const { locale, t } = useI18n()
  const connected = useAppStore((s) => s.connected)
  const qc = useQueryClient()
  const sortedModels = useMemo(
    () =>
      [...(modelStats ?? [])].sort((a, b) => {
        const ta = a.last_used_at_unix ?? 0
        const tb = b.last_used_at_unix ?? 0
        if (tb !== ta) return tb - ta
        return (b.input + b.output) - (a.input + a.output)
      }),
    [modelStats],
  )

  const { data: timeline, refetch, isFetching } = useAllModelsTokenTimelineQuery(scope, chartRange)

  const chartRef = useRef<HTMLDivElement>(null)
  const instanceRef = useRef<echarts.ECharts | null>(null)

  const labels = useMemo(
    () => ({
      input: t('chart.modelInput'),
      output: t('chart.modelOutput'),
      cached: t('chart.modelCached'),
    }),
    [t],
  )

  const chartData = useMemo(() => {
    if (timelineMatchesRequest(timeline, chartRange, scope)) return timeline
    return emptyTimelineSkeleton(chartRange)
  }, [timeline, chartRange, scope])

  const render = useCallback(
    (timelineData: AllModelsTimelineResponse) => {
      const el = chartRef.current
      if (!el) return
      if (!instanceRef.current) instanceRef.current = echarts.init(el)
      const chart = instanceRef.current
      chart.resize()
      const colors = themeColors()
      const dark = document.documentElement.dataset.effectiveTheme === 'dark'
      const { times, series, legend, peak } = buildChartSeries(timelineData, labels, dark, chartRange)
      const legendTop = legend.length > 6 ? 0 : 0
      const gridTop = legend.length > 6 ? 72 : legend.length > 3 ? 56 : 40

      chart.setOption(
        {
          animationDuration: timeline ? 400 : 0,
          tooltip: {
            trigger: 'axis',
            axisPointer: {
              type: 'cross',
              label: {
                backgroundColor: '#6a7985',
                formatter: (params: { value: number; axisDimension?: string }) => {
                  if (params.axisDimension === 'y') {
                    return formatAxisTick(params.value, locale)
                  }
                  return String(params.value ?? '')
                },
              },
            },
            valueFormatter: (v: number) => formatAxisNum(v, locale),
          },
          legend: {
            data: legend,
            type: legend.length > 8 ? 'scroll' : 'plain',
            textStyle: { color: colors.axis },
            top: legendTop,
          },
          grid: { left: '3%', right: '4%', bottom: '3%', top: gridTop, containLabel: true },
          xAxis: {
            type: 'category',
            boundaryGap: false,
            data: times,
            axisLine: { show: true, lineStyle: { color: colors.split } },
            axisTick: { show: true, lineStyle: { color: colors.split } },
            axisLabel: { color: colors.axis, fontSize: 11 },
          },
          yAxis: {
            type: 'value',
            min: 0,
            minInterval: 1,
            max: peak === 0 ? 5 : undefined,
            axisLine: { show: true, lineStyle: { color: colors.split } },
            axisTick: { show: true, lineStyle: { color: colors.split } },
            axisLabel: { color: colors.axis, formatter: (v: number) => formatAxisTick(v, locale) },
            splitLine: { show: true, lineStyle: { color: colors.split } },
          },
          series,
        },
        { notMerge: true },
      )
    },
    [labels, locale, timeline, chartRange],
  )

  const handleRefresh = () => {
    void refetch()
    void qc.invalidateQueries({ queryKey: queryKeys.gatewayStats(scope) })
  }

  useEffect(() => {
    render(chartData)
  }, [chartData, render])

  useEffect(() => {
    const onResize = () => instanceRef.current?.resize()
    window.addEventListener('resize', onResize)
    const observer = new MutationObserver(() => {
      render(chartData)
    })
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-effective-theme'] })
    return () => {
      window.removeEventListener('resize', onResize)
      observer.disconnect()
      instanceRef.current?.dispose()
      instanceRef.current = null
    }
  }, [chartData, render])

  return (
    <div className="panel">
      <div className="token-chart-header">
        <div className="panel-title" style={{ marginBottom: 0 }}>{t('stats.tokenTrend')}</div>
        <div className="token-chart-header-actions">
          <button
            type="button"
            className="btn btn-ghost btn-sm"
            onClick={handleRefresh}
            disabled={!connected}
            aria-busy={isFetching}
          >
            {t('action.refresh')}
          </button>
          <StatsChartRange range={chartRange} onChange={onRangeChange} />
        </div>
      </div>
      <div ref={chartRef} id="token-chart" className="token-chart" />
      <div className="table-wrap" style={{ marginTop: 16 }}>
        <table className="model-token-table">
          <thead>
            <tr>
              <th>{t('col.modelName')}</th>
              <th className="num">{t('stat.inputToken')}</th>
              <th className="num">{t('stat.outputToken')}</th>
              <th className="num">{t('col.cachedTokens')}</th>
              <th className="num">{t('col.lastUsed')}</th>
            </tr>
          </thead>
          <tbody id="model-token-table">
            {sortedModels.length ? (
              sortedModels.map((row) => (
                <tr key={modelStatsKey(row.tier, row.model)} className="model-token-row">
                  <td>
                    <span className="model-token-name">
                      <span className={`tag bordered ${routeTagClass(row.tier)}`}>{t(`route.${row.tier}` as 'route.edge')}</span>
                      <code title={row.model}>{row.model}</code>
                    </span>
                  </td>
                  <td className="num">{fmtNum(row.input, locale)}</td>
                  <td className="num">{fmtNum(row.output, locale)}</td>
                  <td className="num">{fmtNum(row.cached, locale)}</td>
                  <td className="num">{formatRelativeTime(row.last_used_at_unix, t, locale)}</td>
                </tr>
              ))
            ) : (
              <tr>
                <td colSpan={5} style={{ color: 'var(--default-400)' }}>{t('stats.modelTokenEmpty')}</td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  )
}
