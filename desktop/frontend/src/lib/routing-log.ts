export type RouteTier = 'edge' | 'cloud' | 'cascade'

export interface RoutingLogEntry {
  id: number
  timestamp: string
  timeLabel: string
  route: RouteTier
  stepKind: string
  model?: string
  userPreview: string
  hasUserPreview: boolean
  reasonCodes: string[]
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
}): RouteTier {
  const raw = entry.served_route ?? entry.route
  if (raw === 'edge' || raw === 'cloud' || raw === 'cascade') return raw
  return 'edge'
}

export function mapApiRoutingEntry(entry: {
  id: number
  timestamp: string
  route: string
  served_route?: string | null
  step_kind: string
  model: string
  user_preview: string
  reason_codes: string[]
}): RoutingLogEntry {
  return {
    id: entry.id,
    timestamp: entry.timestamp,
    timeLabel: formatTimeLabel(entry.timestamp),
    route: effectiveRouteTier(entry),
    stepKind: entry.step_kind,
    model: entry.model,
    userPreview: entry.user_preview,
    hasUserPreview: entry.user_preview.length > 0,
    reasonCodes: entry.reason_codes,
    raw: '',
  }
}

export function isDecisiveReasonCode(code: string): boolean {
  return DECISIVE_PREFIXES.some((p) => code.startsWith(p))
}

export function pickDisplayReasonCodes(codes: string[], max = 4): { shown: string[]; overflow: number } {
  const decisive = codes.filter(isDecisiveReasonCode)
  const pool = decisive.length ? decisive : codes.filter((c) => !c.startsWith('STEP_') && !c.startsWith('TOK_'))
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
