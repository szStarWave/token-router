import type { StatsSnapshot } from '../types/gateway'
import { formatCompactNum } from './format-number'

export function tierTokenTotal(tier: Record<string, unknown> | undefined): number {
  if (!tier) return 0
  return ['input', 'output', 'cached'].reduce(
    (sum, k) => sum + (Number(tier[k]) || 0),
    0,
  )
}

export function sidebarTokenShares(tb: Record<string, unknown> | undefined) {
  const edge = tierTokenTotal(tb?.edge as Record<string, unknown>)
  const cloud = tierTokenTotal(tb?.cloud as Record<string, unknown>)
  const total = edge + cloud
  if (total <= 0) return { edgePct: 0, cloudPct: 0 }
  if (typeof tb?.edge_share_pct === 'number' && typeof tb?.cloud_share_pct === 'number') {
    return { edgePct: tb.edge_share_pct as number, cloudPct: tb.cloud_share_pct as number }
  }
  return { edgePct: (edge / total) * 100, cloudPct: (cloud / total) * 100 }
}

export function formatUptime(secs: number, t: (k: string, v?: Record<string, string | number>) => string): string {
  if (!Number.isFinite(secs) || secs < 0) return '—'
  const h = Math.floor(secs / 3600)
  const m = Math.floor((secs % 3600) / 60)
  const s = Math.floor(secs % 60)
  if (h > 0) return t('uptime.hms', { h, m, s })
  if (m > 0) return t('uptime.ms', { m, s })
  return t('uptime.s', { s })
}

export function fmtPct(n: number | null | undefined): string {
  if (n == null || Number.isNaN(n)) return '—'
  return `${Math.round(n)}%`
}

export function fmtMs(ms: number | null | undefined, t: (k: string, v?: Record<string, string | number>) => string): string {
  if (!ms) return '—'
  return t('stat.msUnit', { n: Math.round(ms) })
}

export function fmtNum(n: unknown, locale?: string): string {
  return formatCompactNum(n, locale)
}

export function formatSavedCreditsAmount(
  savedPoints: number | null | undefined,
  locale?: string,
): string {
  if (savedPoints == null || Number.isNaN(savedPoints)) return '—'
  return Math.round(savedPoints).toLocaleString(locale)
}

export function formatSavedCredits(
  savedPoints: number | null | undefined,
  t: (k: string, v?: Record<string, string | number>) => string,
): string {
  if (savedPoints == null || Number.isNaN(savedPoints)) return '—'
  return t('routeStats.savedCredits', { n: formatSavedCreditsAmount(savedPoints) })
}

export function stepKindLabel(key: string, t: (k: string) => string): string {
  const i18nKey = `stepKind.${key}`
  const translated = t(i18nKey)
  return translated !== i18nKey ? translated : key
}

export function topStepKinds(stats: StatsSnapshot | null, limit = 5) {
  const kinds = stats?.step_kinds ?? {}
  const entries = Object.entries(kinds).sort((a, b) => (b[1] as number) - (a[1] as number))
  const total = entries.reduce((s, [, v]) => s + (Number(v) || 0), 0)
  return entries.slice(0, limit).map(([kind, count]) => ({
    kind,
    count: Number(count) || 0,
    pct: total > 0 ? ((Number(count) || 0) / total) * 100 : 0,
  }))
}

export function tokenTableRows(tb: Record<string, unknown> | undefined) {
  const edge = (tb?.edge as Record<string, number>) ?? {}
  const cloud = (tb?.cloud as Record<string, number>) ?? {}
  const keys = ['input', 'output', 'cached'] as const
  return keys.map((k) => ({
    key: k,
    edge: Number(edge[k]) || 0,
    cloud: Number(cloud[k]) || 0,
    total: (Number(edge[k]) || 0) + (Number(cloud[k]) || 0),
  }))
}

export function fmtTps(n: number | null | undefined): string {
  if (n == null || Number.isNaN(n) || n <= 0) return '—'
  return n.toFixed(1)
}

export function errorRatePct(stats: StatsSnapshot | null | undefined): number | null {
  const total = stats?.requests_total ?? 0
  const errors = stats?.errors?.total ?? 0
  if (total <= 0) return null
  return (errors / total) * 100
}

export function tokenSummary(tb: Record<string, unknown> | undefined) {
  const total = (tb?.total as Record<string, unknown>) ?? {}
  const input = Number(total.input) || 0
  const output = Number(total.output) || 0
  return { input, output, total: input + output }
}

export function tierTokenSummary(tier: Record<string, unknown> | undefined) {
  const input = Number(tier?.input) || 0
  const output = Number(tier?.output) || 0
  return { input, output, total: tierTokenTotal(tier) }
}

export function tierMaxPerRequest(tier: Record<string, unknown> | undefined) {
  return {
    input: Number(tier?.max_input) || 0,
    output: Number(tier?.max_output) || 0,
    atMaxInput: {
      total: Number(tier?.max_input_request_total) || 0,
      output: Number(tier?.max_input_request_output) || 0,
    },
    atMaxOutput: {
      total: Number(tier?.max_output_request_total) || 0,
      input: Number(tier?.max_output_request_input) || 0,
    },
  }
}

export function classifierPriorEdgePct(clf: { prior?: { edge_ok?: number; cloud_needed?: number } } | null | undefined): number | null {
  const edge = clf?.prior?.edge_ok ?? 0
  const cloud = clf?.prior?.cloud_needed ?? 0
  const total = edge + cloud
  if (!total) return null
  return (edge / total) * 100
}

export function classifierFeatureLabel(key: string, t: (k: string) => string): string {
  if (!key) return '—'
  const idx = key.indexOf(':')
  if (idx === -1) return key
  const kind = key.slice(0, idx)
  const val = key.slice(idx + 1)
  if (kind === 'step_kind') return stepKindLabel(val, t)
  if (kind === 'intent') {
    const intentKey = `intent.${val}`
    const translated = t(intentKey)
    return translated !== intentKey ? translated : val
  }
  if (kind === 'ctx_bucket' || kind === 'tool_bucket' || kind === 'loop_bucket' || kind === 'turn_bucket') {
    return `${kind.replace('_bucket', '')}:${val}`
  }
  if (kind === 'flag') return val
  return key
}

type ClassifierSummaryKey =
  | 'enabled'
  | 'totalSamples'
  | 'totalUpdates'
  | 'priorEdge'
  | 'minSamples'
  | 'decayGen'
  | 'priorAlpha'
  | 'decayHalfLife'

export function formatClassifierSummaryValue(
  key: ClassifierSummaryKey,
  value: unknown,
  t: (k: string) => string,
): string {
  if (key === 'enabled') return value ? t('bool.yes') : t('bool.no')
  if (key === 'priorEdge') return value == null ? '—' : fmtPct(value as number)
  if (typeof value === 'number' && Number.isInteger(value)) return value.toLocaleString()
  if (typeof value === 'number') return value.toFixed(2)
  if (value == null) return '—'
  return String(value)
}

export function classifierSummaryRows(clf: Record<string, unknown>) {
  const priorPct = classifierPriorEdgePct(clf as { prior?: { edge_ok?: number; cloud_needed?: number } })
  return [
    { key: 'enabled' as const, value: clf.enabled },
    { key: 'totalSamples' as const, value: clf.total_samples },
    { key: 'totalUpdates' as const, value: clf.total_updates },
    { key: 'priorEdge' as const, value: priorPct },
    { key: 'minSamples' as const, value: clf.min_samples },
    { key: 'decayGen' as const, value: clf.decay_generation },
    { key: 'priorAlpha' as const, value: clf.prior_alpha },
    { key: 'decayHalfLife' as const, value: clf.decay_half_life_hours },
  ]
}

export function snapshotFromTokenBreakdown(tb: Record<string, unknown> | undefined) {
  const edge = (tb?.edge as Record<string, number>) ?? {}
  const cloud = (tb?.cloud as Record<string, number>) ?? {}
  return {
    edgeIn: Number(edge.input) || 0,
    edgeOut: Number(edge.output) || 0,
    cloudIn: Number(cloud.input) || 0,
    cloudOut: Number(cloud.output) || 0,
  }
}
