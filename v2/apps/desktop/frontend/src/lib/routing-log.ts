import type { RoutingLogApiEntry } from '../types/gateway'

export type RouteTier = 'edge' | 'cloud' | 'cascade'

export interface RoutingLogEntry {
  id: number
  timestamp: string
  timeLabel: string
  route: RouteTier
  stepKind: string
  model?: string
  servedModel?: string | null
  userPreview: string
  hasUserPreview: boolean
  reasonCodes: string[]
  difficulty?: number | null
  errorReason?: string | null
  tokensIn?: number | null
  tokensOut?: number | null
  cachedTokens?: number | null
  raw: string
}

export const ROUTING_PREVIEW_MAX = 80

const ROUTING_MARKER = ' routing: '
const SERVED_MARKER = ' served: '

const TIMESTAMP_RE = /^(\d{4}-\d{2}-\d{2}T[\d:.]+Z)/
const ROUTE_FIELD_RE = /\bserved=(edge|cloud|cascade)\b|\broute=(edge|cloud|cascade)\b/
const ROUTE_ARROW_RE = /routing:\s[\w_]+\s→\s(edge|cloud|cascade)\b/
const STEP_KIND_RE = /\bstep_kind=([\w_]+)\b/
const MODEL_QUOTED_RE = /\bmodel="([^"]*)"/
const MODEL_BARE_RE = /\bmodel=([^\s]+)/
const REASON_CODES_RE = /\breason_codes=([^\s]+)/
const USER_PREVIEW_QUOTED_RE = /\buser_preview="([^"]*)"/
const USER_PREVIEW_BARE_RE = /\buser_preview=([^\s]+)/

const DECISIVE_PREFIXES = [
  'GATE_',
  'CONFIG_ROUTE_',
  'UPSTREAM_',
  'PLAN_',
  'INITIAL_PLAN_',
  'LONG_GEN_',
  'MULTIMODAL_',
  'WORK_',
  'STICKY_',
  'CASUAL_',
  'LEXICAL_',
  'BAYES_',
  'EXP_BIAS_',
  'TOOL_ERROR_STREAK_',
  'TOOL_LOOP_',
  'RISKY_TOOL_',
] as const

export function isRoutingLogLine(msg: string): boolean {
  return msg.includes(ROUTING_MARKER) && !msg.includes(SERVED_MARKER)
}

export function truncatePreview(text: string, max = ROUTING_PREVIEW_MAX): string {
  const trimmed = text.trim()
  if (trimmed.length <= max) return trimmed
  return `${trimmed.slice(0, max)}…`
}

export function formatTimeLabel(iso: string): string {
  const d = new Date(iso)
  if (!Number.isNaN(d.getTime())) {
    return d.toLocaleTimeString(undefined, {
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
      hour12: false,
    })
  }
  const match = iso.match(/T(\d{2}):(\d{2}):(\d{2})/)
  if (!match) return iso
  return `${match[1]}:${match[2]}:${match[3]}`
}

export function effectiveRouteTier(entry: {
  route: string
  served_route?: string | null
    served_model?: string | null
}): RouteTier {
  const raw = entry.served_route ?? entry.route
  if (raw === 'edge' || raw === 'cloud' || raw === 'cascade') return raw
  return 'edge'
}

export function mapApiRoutingEntry(entry: RoutingLogApiEntry): RoutingLogEntry {
  return {
    id: entry.id,
    timestamp: entry.timestamp,
    timeLabel: formatTimeLabel(entry.timestamp),
    route: effectiveRouteTier(entry),
    stepKind: entry.step_kind,
    model: entry.model,
      servedModel: entry.served_model ?? null,
    userPreview: entry.user_preview,
    hasUserPreview: entry.user_preview.length > 0,
    reasonCodes: entry.reason_codes,
    difficulty: entry.difficulty ?? null,
    errorReason: entry.error_reason ?? null,
    tokensIn: entry.tokens_in ?? null,
    tokensOut: entry.tokens_out ?? null,
    cachedTokens: entry.cached_tokens ?? null,
    raw: '',
  }
}

export function isDecisiveReasonCode(code: string): boolean {
  if (isDifficultyMetaCode(code)) return false
  return DECISIVE_PREFIXES.some((p) => code.startsWith(p))
}

export interface DifficultyScorePart {
  key: string
  linear: number | null
  scoreDelta: number | null
}

export interface DifficultyBreakdown {
  parts: DifficultyScorePart[]
  linearSum: number | null
  heuristic: number | null
  fuse: { heur: number; bayes: number; w: number; final: number } | null
  adjustments: Array<{ key: string; scoreDelta: number }>
  final: number | null
}

const DIFF_L_RE = /^DIFF_L:([^:]+):([+-]?\d+(?:\.\d+)?)$/
const DIFF_D_RE = /^DIFF_D:([^:]+):([+-]?\d+(?:\.\d+)?)$/
const DIFF_HEUR_RE = /^DIFF_HEUR:([+-]?\d+(?:\.\d+)?)$/
const DIFF_LINEAR_SUM_RE = /^DIFF_LINEAR_SUM:([+-]?\d+(?:\.\d+)?)$/
const DIFF_FUSE_RE = /^DIFF_FUSE:heur=([\d.]+)[|,]bayes=([\d.]+)[|,]w=([\d.]+)[|,]final=([\d.]+)$/

export function isOrphanReasonFragment(code: string): boolean {
  if (/^(heur|bayes|w|final)=/.test(code)) return true
  if (/^\d+\.\d+\)$/.test(code)) return true
  if (/^ADAPTIVE_THETA\([\d.]+$/.test(code)) return true
  return false
}

export function isDifficultyMetaCode(code: string): boolean {
  return (
    code.startsWith('DIFF_L:') ||
    code.startsWith('DIFF_D:') ||
    code.startsWith('DIFF_HEUR:') ||
    code.startsWith('DIFF_LINEAR_SUM:') ||
    code.startsWith('DIFF_FUSE:')
  )
}

export function parseDifficultyBreakdown(codes: string[]): DifficultyBreakdown | null {
  const linearMap = new Map<string, number>()
  const scoreMap = new Map<string, number>()
  let linearSum: number | null = null
  let heuristic: number | null = null
  let fuse: DifficultyBreakdown['fuse'] = null
  const adjustments: Array<{ key: string; scoreDelta: number }> = []
  const heuristicKeys = new Set<string>()

  for (const code of codes) {
    const linearMatch = DIFF_L_RE.exec(code)
    if (linearMatch) {
      linearMap.set(linearMatch[1], Number.parseFloat(linearMatch[2]))
      continue
    }
    const scoreMatch = DIFF_D_RE.exec(code)
    if (scoreMatch) {
      const key = scoreMatch[1]
      const delta = Number.parseFloat(scoreMatch[2])
      if (key.startsWith('CALIB_') || key === 'BAYES_FUSE' || key.startsWith('PRIVACY_') || key === 'CASUAL_CLASSIFIER_GUARD') {
        adjustments.push({ key, scoreDelta: delta })
      } else {
        scoreMap.set(key, delta)
        heuristicKeys.add(key)
      }
      continue
    }
    if (code.startsWith('DIFF_HEUR:')) {
      const m = DIFF_HEUR_RE.exec(code)
      if (m) heuristic = Number.parseFloat(m[1])
      continue
    }
    if (code.startsWith('DIFF_LINEAR_SUM:')) {
      const m = DIFF_LINEAR_SUM_RE.exec(code)
      if (m) linearSum = Number.parseFloat(m[1])
      continue
    }
    if (code.startsWith('DIFF_FUSE:')) {
      const m = DIFF_FUSE_RE.exec(code)
      if (m) {
        fuse = {
          heur: Number.parseFloat(m[1]),
          bayes: Number.parseFloat(m[2]),
          w: Number.parseFloat(m[3]),
          final: Number.parseFloat(m[4]),
        }
      }
    }
  }

  const partKeys = new Set([...linearMap.keys(), ...scoreMap.keys()])
  const parts = [...partKeys]
    .map((key) => ({
      key,
      linear: linearMap.get(key) ?? null,
      scoreDelta: scoreMap.get(key) ?? null,
    }))
    .sort((a, b) => Math.abs(b.scoreDelta ?? 0) - Math.abs(a.scoreDelta ?? 0))

  const final = extractDifficultyScore(codes)

  if (parts.length === 0 && adjustments.length === 0 && fuse == null && heuristic == null) {
    return null
  }

  return { parts, linearSum, heuristic, fuse, adjustments, final }
}

/** Config / upstream availability — the only codes that bypass difficulty scoring. */
export function isHardRouteOverrideCode(code: string): boolean {
  if (code.startsWith('CONFIG_ROUTE_')) return true
  return code === 'UPSTREAM_EDGE_ONLY' || code === 'UPSTREAM_CLOUD_ONLY'
}

/** @deprecated Use isHardRouteOverrideCode; kept for tag ordering of legacy decisive codes. */
export function isRouteOverrideCode(code: string): boolean {
  return isHardRouteOverrideCode(code)
}

/**
 * Whether a hard override code actually explains the *served* route.
 * CONFIG_ROUTE_edge + quality fallback can still serve cloud — that mismatch
 * must not become "because fixed edge, go cloud".
 */
export function hardOverrideExplainsRoute(code: string, route: RouteTier): boolean {
  if (code === 'UPSTREAM_EDGE_ONLY') return route === 'edge'
  if (code === 'UPSTREAM_CLOUD_ONLY') return route === 'cloud'
  if (code.startsWith('CONFIG_ROUTE_')) {
    const tier = code.slice('CONFIG_ROUTE_'.length).toLowerCase()
    if (tier === 'edge') return route === 'edge'
    if (tier === 'cloud') return route === 'cloud'
    // Cascade may serve edge or cloud after escalation.
    if (tier === 'cascade') return true
  }
  return true
}

function pickDominantDifficultyFactor(codes: string[], route: RouteTier): string | null {
  const breakdown = parseDifficultyBreakdown(codes)
  if (!breakdown || breakdown.parts.length === 0) return null

  const wantCloud = route !== 'edge'
  const aligned = breakdown.parts.filter((p) => {
    const delta = p.scoreDelta
    if (delta == null || delta === 0) return false
    return wantCloud ? delta > 0 : delta < 0
  })

  if (aligned.length === 0) return null

  aligned.sort((a, b) => Math.abs(b.scoreDelta ?? 0) - Math.abs(a.scoreDelta ?? 0))
  return aligned[0].key
}

export function extractDifficultyScore(
  codes: string[],
  fallback?: number | null,
): number | null {
  if (fallback != null && Number.isFinite(fallback)) return fallback
  for (let i = codes.length - 1; i >= 0; i--) {
    const code = codes[i]
    if (!code.startsWith('DIFFICULTY_')) continue
    const score = Number.parseFloat(code.slice('DIFFICULTY_'.length))
    if (Number.isFinite(score)) return score
  }
  return null
}

/**
 * Primary factor for the routing conclusion: hard override (when it matches
 * the served route), then quality-fallback escalation, then the largest
 * difficulty contributor aligned with the chosen route, then DIFFICULTY_*.
 */
export function pickFinalRouteFactorCode(
  codes: string[],
  route: RouteTier = 'edge',
): string | null {
  for (let i = codes.length - 1; i >= 0; i--) {
    const code = codes[i]
    if (isHardRouteOverrideCode(code) && hardOverrideExplainsRoute(code, route)) {
      return code
    }
  }

  // Decision was edge (e.g. CONFIG_ROUTE_edge) but quality gate escalated to cloud.
  if (route === 'cloud') {
    for (let i = codes.length - 1; i >= 0; i--) {
      if (codes[i] === 'CASUAL_EDGE_FALLBACK') return codes[i]
    }
  }

  const dominant = pickDominantDifficultyFactor(codes, route)
  if (dominant) return dominant

  for (let i = codes.length - 1; i >= 0; i--) {
    if (codes[i].startsWith('DIFFICULTY_')) return codes[i]
  }
  return null
}

const DISPLAY_PRIORITY_PREFIXES = ['GATE_', 'CONFIG_ROUTE_', 'UPSTREAM_', 'LONG_GEN_'] as const

function displayTagPriority(code: string): number {
  const idx = DISPLAY_PRIORITY_PREFIXES.findIndex((p) => code.startsWith(p))
  return idx >= 0 ? idx : DISPLAY_PRIORITY_PREFIXES.length
}

export function pickDisplayReasonCodes(codes: string[], max = 4): { shown: string[]; overflow: number } {
  const hardOverrides = codes.filter(isHardRouteOverrideCode)
  const decisive = codes.filter(isDecisiveReasonCode)
  const pool = [
    ...hardOverrides,
    ...(decisive.length ? decisive : codes.filter((c) => !c.startsWith('STEP_') && !c.startsWith('TOK_'))),
  ]
    .filter((code, index, arr) => arr.indexOf(code) === index)
    .sort((a, b) => displayTagPriority(a) - displayTagPriority(b))
  const shown = pool.slice(0, max)
  const overflow = Math.max(0, pool.length - shown.length)
  return { shown, overflow }
}

export function parseRoutingLogLine(msg: string, id: number): RoutingLogEntry | null {
  if (!isRoutingLogLine(msg)) return null

  const timestamp = TIMESTAMP_RE.exec(msg)?.[1] ?? ''
  const routeMatch = ROUTE_FIELD_RE.exec(msg)
  const route =
    (routeMatch?.[1] as RouteTier | undefined) ??
    (routeMatch?.[2] as RouteTier | undefined) ??
    (ROUTE_ARROW_RE.exec(msg)?.[1] as RouteTier | undefined) ??
    'edge'
  const stepKind = STEP_KIND_RE.exec(msg)?.[1] ?? 'unknown'
  const model = MODEL_QUOTED_RE.exec(msg)?.[1] ?? MODEL_BARE_RE.exec(msg)?.[1]
  const reasonRaw = REASON_CODES_RE.exec(msg)?.[1] ?? ''
  const reasonCodes = reasonRaw ? reasonRaw.split(',').filter(Boolean) : []
  const userPreviewRaw =
    USER_PREVIEW_QUOTED_RE.exec(msg)?.[1] ?? USER_PREVIEW_BARE_RE.exec(msg)?.[1] ?? ''

  return {
    id,
    timestamp,
    timeLabel: timestamp ? formatTimeLabel(timestamp) : '',
    route,
    stepKind,
    model,
    userPreview: userPreviewRaw,
    hasUserPreview: userPreviewRaw.length > 0,
    reasonCodes,
    raw: msg,
  }
}

export function parseRoutingLogLines(
  lines: Array<{ id: number; msg: string }>,
): RoutingLogEntry[] {
  const out: RoutingLogEntry[] = []
  for (const line of lines) {
    const entry = parseRoutingLogLine(line.msg, line.id)
    if (entry) out.push(entry)
  }
  return out
}
