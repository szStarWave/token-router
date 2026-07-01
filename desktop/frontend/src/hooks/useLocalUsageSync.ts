import { reportLocalModelUsage } from '../lib/flowy/api'
import { getAuthToken } from '../stores/authStore'
import { useAppStore } from '../stores/appStore'
import type { StatsSnapshot } from '../types/gateway'
import { getEdgeModelValue } from '../lib/edge-upstream'

let lastReportKey: string | null = null
let syncInFlight: { key: string; promise: Promise<number | null> } | null = null

function normalizeUsageNumber(value: unknown): number {
  if (typeof value === 'number' && Number.isFinite(value)) return Math.max(0, Math.floor(value))
  if (typeof value === 'string' && value.trim()) {
    const parsed = Number(value)
    if (Number.isFinite(parsed)) return Math.max(0, Math.floor(parsed))
  }
  return 0
}

function extractSavedPoints(data: Record<string, unknown> | null | undefined): number {
  if (!data || typeof data !== 'object') return 0
  return normalizeUsageNumber(data.savedPoints ?? data.saved_points ?? data.totalSavedPoints)
}

function edgeTokensFromStats(stats: StatsSnapshot | null | undefined) {
  const edge = stats?.token_breakdown?.edge as Record<string, unknown> | undefined
  return {
    promptTokens: normalizeUsageNumber(edge?.input),
    completionTokens: normalizeUsageNumber(edge?.output),
    cacheTokens: normalizeUsageNumber(edge?.cached),
  }
}

function hasUsage(tokens: { promptTokens: number; completionTokens: number; cacheTokens: number }) {
  return tokens.promptTokens > 0 || tokens.completionTokens > 0 || tokens.cacheTokens > 0
}

export async function syncLocalUsageFromStats(
  stats: StatsSnapshot | null | undefined,
  options: { scope?: string; modelId?: string } = {},
): Promise<number | null> {
  const token = getAuthToken()
  const setSavedPoints = useAppStore.getState().setSavedPoints
  if (!token) {
    setSavedPoints(null)
    lastReportKey = null
    return null
  }

  const usage = edgeTokensFromStats(stats)
  if (!hasUsage(usage)) {
    return useAppStore.getState().savedPoints
  }

  const scope = options.scope || 'global'
  const modelId = (options.modelId || getEdgeModelValue() || '').trim()
  const idempotencyKey = `token-router:${scope}:${usage.promptTokens}:${usage.completionTokens}:${usage.cacheTokens}`
  const currentSaved = useAppStore.getState().savedPoints
  if (idempotencyKey === lastReportKey && currentSaved != null) {
    return currentSaved
  }

  if (syncInFlight?.key === idempotencyKey) {
    return syncInFlight.promise
  }

  const promise = (async () => {
    try {
      const result = await reportLocalModelUsage(
        {
          modelId,
          promptTokens: usage.promptTokens,
          completionTokens: usage.completionTokens,
          cacheTokens: usage.cacheTokens,
          sessionId: scope,
          clientId: 'token-router',
          idempotencyKey,
          extra: { source: 'token-router-desktop', scope },
        },
        token,
      )
      lastReportKey = idempotencyKey
      const saved = extractSavedPoints(result as Record<string, unknown>)
      setSavedPoints(saved)
      return saved
    } catch (e) {
      console.warn('[local-usage-sync]', e)
      return useAppStore.getState().savedPoints
    } finally {
      if (syncInFlight?.key === idempotencyKey) syncInFlight = null
    }
  })()

  syncInFlight = { key: idempotencyKey, promise }
  return promise
}

export function resetLocalUsageSync() {
  lastReportKey = null
  syncInFlight = null
  useAppStore.getState().setSavedPoints(null)
}
