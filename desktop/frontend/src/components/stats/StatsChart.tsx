import { useEffect, useRef, useCallback } from 'react'
import * as echarts from 'echarts'
import { formatAxisNum } from '../../lib/format-number'
import { useI18n } from '../../hooks/useI18n'
import { snapshotFromTokenBreakdown } from '../../lib/stats-utils'

export type ChartRange = 'h24' | 'd7' | 'd30'

const CHART_RANGE_CONFIG: Record<ChartRange, { bucket: 'hour' | 'day'; limit: number }> = {
  h24: { bucket: 'hour', limit: 24 },
  d7: { bucket: 'day', limit: 7 },
  d30: { bucket: 'day', limit: 30 },
}

const MAX_SNAPSHOT_AGE_MS = 30 * 24 * 60 * 60 * 1000
const MAX_SNAPSHOTS = 50_000

type BucketMode = 'minute' | 'hour' | 'day'
type Scope = 'session' | 'global'

interface Snapshot {
  ts: number
  edgeIn: number
  edgeOut: number
  cloudIn: number
  cloudOut: number
}

const historyByScope: Record<Scope, Snapshot[]> = { session: [], global: [] }

const BUCKET_MS: Record<BucketMode, number> = {
  minute: 60 * 1000,
  hour: 60 * 60 * 1000,
  day: 24 * 60 * 60 * 1000,
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

function bucketStartTs(ts: number, mode: BucketMode) {
  const d = new Date(ts)
  if (mode === 'day') d.setHours(0, 0, 0, 0)
  else if (mode === 'hour') d.setMinutes(0, 0, 0)
  else d.setSeconds(0, 0)
  return d.getTime()
}

function formatTimeLabel(bucketTs: number, mode: BucketMode) {
  const d = new Date(bucketTs)
  if (mode === 'hour') return `${pad2(d.getMonth() + 1)}/${pad2(d.getDate())} ${pad2(d.getHours())}:00`
  return `${d.getFullYear()}/${pad2(d.getMonth() + 1)}/${pad2(d.getDate())}`
}

function countersReset(cur: Snapshot, prev: Snapshot) {
  return cur.edgeIn < prev.edgeIn || cur.edgeOut < prev.edgeOut || cur.cloudIn < prev.cloudIn || cur.cloudOut < prev.cloudOut
}

function subtractCumulative(
  cur: Pick<Snapshot, 'edgeIn' | 'edgeOut' | 'cloudIn' | 'cloudOut'>,
  prev: Pick<Snapshot, 'edgeIn' | 'edgeOut' | 'cloudIn' | 'cloudOut'>,
) {
  return {
    edgeIn: Math.max(0, cur.edgeIn - prev.edgeIn),
    edgeOut: Math.max(0, cur.edgeOut - prev.edgeOut),
    cloudIn: Math.max(0, cur.cloudIn - prev.cloudIn),
    cloudOut: Math.max(0, cur.cloudOut - prev.cloudOut),
  }
}

function trimHistory(hist: Snapshot[]) {
  const cutoff = Date.now() - MAX_SNAPSHOT_AGE_MS
  while (hist.length > 0 && hist[0].ts < cutoff) hist.shift()
  if (hist.length > MAX_SNAPSHOTS) hist.splice(0, hist.length - MAX_SNAPSHOTS)
}

function latestSnapshotPerBucket(snapshots: Snapshot[], mode: BucketMode) {
  const map = new Map<number, Snapshot & { bucket: number }>()
  for (const snap of snapshots) {
    const bucket = bucketStartTs(snap.ts, mode)
    const existing = map.get(bucket)
    if (!existing || snap.ts >= existing.ts) map.set(bucket, { ...snap, bucket })
  }
  return map
}

function aggregateHistory(snapshots: Snapshot[], range: ChartRange) {
  const { bucket, limit } = CHART_RANGE_CONFIG[range]
  const step = BUCKET_MS[bucket]
  const windowEnd = bucketStartTs(Date.now(), bucket)
  const windowStart = windowEnd - (limit - 1) * step

  const zeroPoint = (bucketTs: number) => ({
    bucket: bucketTs,
    edgeIn: 0,
    edgeOut: 0,
    cloudIn: 0,
    cloudOut: 0,
  })

  const windowBuckets: number[] = []
  for (let bucketTs = windowStart; bucketTs <= windowEnd; bucketTs += step) {
    windowBuckets.push(bucketTs)
  }

  if (!snapshots.length) {
    return windowBuckets.map(zeroPoint)
  }

  const bucketMap = latestSnapshotPerBucket(snapshots, bucket)
  const bucketStarts = [...bucketMap.keys()].sort((a, b) => a - b)

  let prevCumulative = { edgeIn: 0, edgeOut: 0, cloudIn: 0, cloudOut: 0 }
  for (const bucketTs of bucketStarts) {
    if (bucketTs >= windowStart) break
    const snap = bucketMap.get(bucketTs)!
    prevCumulative = {
      edgeIn: snap.edgeIn,
      edgeOut: snap.edgeOut,
      cloudIn: snap.cloudIn,
      cloudOut: snap.cloudOut,
    }
  }

  return windowBuckets.map((bucketTs) => {
    const snap = bucketMap.get(bucketTs)
    if (!snap) return zeroPoint(bucketTs)
    const delta = subtractCumulative(snap, prevCumulative)
    prevCumulative = {
      edgeIn: snap.edgeIn,
      edgeOut: snap.edgeOut,
      cloudIn: snap.cloudIn,
      cloudOut: snap.cloudOut,
    }
    return { bucket: bucketTs, ...delta }
  })
}

export function recordStatsChart(scope: Scope, tb: Record<string, unknown> | undefined) {
  if (!scope) return
  const cur = snapshotFromTokenBreakdown(tb)
  const hist = historyByScope[scope]
  const ts = Date.now()
  const minuteBucket = bucketStartTs(ts, 'minute')
  const last = hist[hist.length - 1]
  if (last && countersReset({ ts, ...cur }, last)) hist.length = 0
  const tail = hist[hist.length - 1]
  if (tail && bucketStartTs(tail.ts, 'minute') === minuteBucket) {
    tail.ts = ts
    Object.assign(tail, cur)
  } else {
    hist.push({ ts, ...cur })
  }
  trimHistory(hist)
}

interface StatsChartProps {
  scope: Scope
  range: ChartRange
  tokenBreakdown?: Record<string, unknown> | null
}

export function StatsChart({ scope, range, tokenBreakdown }: StatsChartProps) {
  const { locale, t } = useI18n()
  const chartRef = useRef<HTMLDivElement>(null)
  const instanceRef = useRef<echarts.ECharts | null>(null)
  const bucketMode = CHART_RANGE_CONFIG[range].bucket

  const labels = {
    edgeIn: t('chart.edgeInput'),
    edgeOut: t('chart.edgeOutput'),
    cloudIn: t('chart.cloudInput'),
    cloudOut: t('chart.cloudOutput'),
  }

  const render = useCallback(() => {
    const el = chartRef.current
    if (!el) return
    if (!instanceRef.current) instanceRef.current = echarts.init(el)
    const chart = instanceRef.current
    const snapshots = historyByScope[scope] || []
    const history = aggregateHistory(snapshots, range)
    chart.resize()
    const colors = themeColors()
    const times = history.map((p) => formatTimeLabel(p.bucket, bucketMode))
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
          makeSeries(labels.edgeIn, history.map((p) => p.edgeIn), colors.edge, 0.18),
          makeSeries(labels.edgeOut, history.map((p) => p.edgeOut), colorAlpha(colors.edge, 0.72), 0.12),
          makeSeries(labels.cloudIn, history.map((p) => p.cloudIn), colors.cloud, 0.18),
          makeSeries(labels.cloudOut, history.map((p) => p.cloudOut), colorAlpha(colors.cloud, 0.72), 0.12),
        ],
      },
      { notMerge: true },
    )
  }, [scope, range, bucketMode, labels, locale])

  useEffect(() => {
    if (tokenBreakdown) recordStatsChart(scope, tokenBreakdown)
    render()
  }, [scope, range, tokenBreakdown, render])

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
