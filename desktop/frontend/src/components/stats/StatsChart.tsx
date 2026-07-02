import { useEffect, useRef, useCallback } from 'react'
import * as echarts from 'echarts'
import { formatAxisNum } from '../../lib/format-number'
import { useI18n } from '../../hooks/useI18n'
import { apiFetch } from '../../lib/gateway'
import type { StatsScope, StatsTimelineResponse } from '../../types/gateway'

export type ChartRange = 'h24' | 'd7' | 'd30'

interface StatsChartProps {
  scope: StatsScope
  range: ChartRange
}

function themeColors() {
  const dark = document.documentElement.dataset.effectiveTheme === 'dark'
  return {
    axis: dark ? '#a1a1aa' : '#71717a',
    split: dark ? 'rgba(255,255,255,0.06)' : 'rgba(0,0,0,0.06)',
    edge: getComputedStyle(document.documentElement).getPropertyValue('--edge').trim() || 'hsl(160, 60%, 45%)',
    cloud: getComputedStyle(document.documentElement).getPropertyValue('--cloud').trim() || 'hsl(262, 70%, 58%)',
  }
}

function colorAlpha(base: string, alpha: number) {
  return base.startsWith('hsl') ? base.replace(')', `, ${alpha})`).replace('hsl', 'hsla') : base
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

async function fetchTimeline(scope: StatsScope, range: ChartRange): Promise<StatsTimelineResponse> {
  const tzOffset = new Date().getTimezoneOffset()
  return apiFetch<StatsTimelineResponse>(
    `/v1/admin/stats/timeline?scope=${scope}&range=${range}&tz_offset=${tzOffset}`,
  )
}

export function StatsChart({ scope, range }: StatsChartProps) {
  const { locale, t } = useI18n()
  const chartRef = useRef<HTMLDivElement>(null)
  const instanceRef = useRef<echarts.ECharts | null>(null)
  const timelineRef = useRef<StatsTimelineResponse | null>(null)

  const labels = {
    edgeIn: t('chart.edgeInput'),
    edgeOut: t('chart.edgeOutput'),
    cloudIn: t('chart.cloudInput'),
    cloudOut: t('chart.cloudOutput'),
  }

  const render = useCallback(() => {
    const el = chartRef.current
    const timeline = timelineRef.current
    if (!el || !timeline) return
    if (!instanceRef.current) instanceRef.current = echarts.init(el)
    const chart = instanceRef.current
    chart.resize()
    const colors = themeColors()
    const times = timeline.points.map((p) => formatTimeLabel(p.bucket_ts, timeline.granularity))
    const legend = [labels.edgeIn, labels.edgeOut, labels.cloudIn, labels.cloudOut]
    const makeSeries = (name: string, data: number[], color: string, alpha: number) => ({
      name,
      type: 'line' as const,
      smooth: true,
      showSymbol: false,
      emphasis: { focus: 'series' as const },
      lineStyle: { width: 2, color },
      itemStyle: { color },
      areaStyle: { opacity: alpha, color },
      data,
    })
    chart.setOption(
      {
        animationDuration: 400,
        tooltip: {
          trigger: 'axis',
          axisPointer: { type: 'cross', label: { backgroundColor: '#6a7985' } },
          valueFormatter: (v: number) => formatAxisNum(v, locale),
        },
        legend: { data: legend, textStyle: { color: colors.axis }, top: 0 },
        grid: { left: '3%', right: '4%', bottom: '3%', top: 36, containLabel: true },
        xAxis: {
          type: 'category',
          boundaryGap: false,
          data: times,
          axisLine: { lineStyle: { color: colors.split } },
          axisLabel: { color: colors.axis, fontSize: 11 },
        },
        yAxis: {
          type: 'value',
          min: 0,
          axisLine: { show: false },
          axisLabel: { color: colors.axis, formatter: (v: number) => formatAxisNum(v, locale) },
          splitLine: { lineStyle: { color: colors.split } },
        },
        series: [
          makeSeries(labels.edgeIn, timeline.points.map((p) => p.edge_in), colors.edge, 0.18),
          makeSeries(labels.edgeOut, timeline.points.map((p) => p.edge_out), colorAlpha(colors.edge, 0.72), 0.12),
          makeSeries(labels.cloudIn, timeline.points.map((p) => p.cloud_in), colors.cloud, 0.18),
          makeSeries(labels.cloudOut, timeline.points.map((p) => p.cloud_out), colorAlpha(colors.cloud, 0.72), 0.12),
        ],
      },
      { notMerge: true },
    )
  }, [labels, locale])

  useEffect(() => {
    let cancelled = false
    const load = () => {
      void fetchTimeline(scope, range)
        .then((data) => {
          if (cancelled) return
          timelineRef.current = data
          render()
        })
        .catch((e) => console.warn('stats timeline', e))
    }
    load()
    const poll = setInterval(load, 10_000)
    return () => {
      cancelled = true
      clearInterval(poll)
    }
  }, [scope, range, render])

  useEffect(() => {
    const onResize = () => instanceRef.current?.resize()
    window.addEventListener('resize', onResize)
    const observer = new MutationObserver(render)
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-effective-theme'] })
    return () => {
      window.removeEventListener('resize', onResize)
      observer.disconnect()
      instanceRef.current?.dispose()
      instanceRef.current = null
    }
  }, [render])

  return <div ref={chartRef} id="token-chart" className="token-chart" />
}

export function StatsChartRange({
  range,
  onChange,
}: {
  range: ChartRange
  onChange: (r: ChartRange) => void
}) {
  const { t } = useI18n()
  const modes: { mode: ChartRange; labelKey: string }[] = [
    { mode: 'h24', labelKey: 'chart.h24' },
    { mode: 'd7', labelKey: 'chart.d7' },
    { mode: 'd30', labelKey: 'chart.d30' },
  ]
  return (
    <div className="segment-tabs token-chart-granularity" id="token-chart-granularity">
      {modes.map(({ mode, labelKey }) => (
        <button
          key={mode}
          type="button"
          className={`segment-tab${range === mode ? ' active' : ''}`}
          data-range={mode}
          onClick={() => onChange(mode)}
        >
          {t(labelKey)}
        </button>
      ))}
    </div>
  )
}
