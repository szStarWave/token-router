import { reportLocalModelUsage } from '../lib/flowy/api'
import { getAuthToken } from '../stores/authStore'
import { useAppStore } from '../stores/appStore'
import type { StatsScope, StatsSnapshot } from '../types/gateway'
import { getEdgeModelValue } from '../lib/edge-upstream'

type UsageScope = StatsScope | 'session' | 'global'

const lastReportKey: Partial<Record<UsageScope, string>> = {}
const syncInFlight: Partial<Record<UsageScope, { key: string; promise: Promise<number | null> }>> = {}

function normalizeUsageNumber(value: unknown): number {
  if (typeof value === 'number' && Number.isFinite(value)) return Math.max(0, Math.floor(value))
  if (typeof value === 'string' && value.trim()) {
    const parsed = Number(value)
    if (Number.isFinite(parsed)) return Math.max(0, Math.floor(parsed))
  }
  return 0
}

function extractSavedPoints(
  data: Record<string, unknown> | null | undefined,
  scope: UsageScope,
): number {
  if (!data || typeof data !== 'object') return 0
  const scoped =
    scope === 'session'
      ? data.sessionSavedPoints ?? data.session_saved_points
      : data.globalSavedPoints ?? data.global_saved_points
  if (scoped != null) return normalizeUsageNumber(scoped)
  return normalizeUsageNumber(data.savedPoints ?? data.saved_points)
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

function savedPointsForScope(scope: UsageScope): number | null {
  const state = useAppStore.getState()
  return scope === 'session' ? state.sessionSavedPoints : state.globalSavedPoints
}

export async function syncLocalUsageFromStats(
  stats: StatsSnapshot | null | undefined,
  options: { scope?: UsageScope; modelId?: string } = {},
): Promise<number | null> {
  const scope = options.scope || 'global'
  const setSavedPoints = useAppStore.getState().setSavedPoints
  const token = getAuthToken()
  if (!token) {
    setSavedPoints(scope, null)
    delete lastReportKey[scope]
    return null
  }

  const usage = edgeTokensFromStats(stats)
  if (!hasUsage(usage)) {
    return savedPointsForScope(scope)
  }

  const modelId = (options.modelId || getEdgeModelValue() || '').trim()
  const idempotencyKey = `token-router:${scope}:${usage.promptTokens}:${usage.completionTokens}:${usage.cacheTokens}`
  const currentSaved = savedPointsForScope(scope)
  if (idempotencyKey === lastReportKey[scope] && currentSaved != null) {
    return currentSaved
  }

  const inFlight = syncInFlight[scope]
  if (inFlight?.key === idempotencyKey) {
    return inFlight.promise
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
      lastReportKey[scope] = idempotencyKey
      const saved = extractSavedPoints(result as Record<string, unknown>, scope)
      setSavedPoints(scope, saved)
      return saved
    } catch (e) {
      console.warn('[local-usage-sync]', scope, e)
      return savedPointsForScope(scope)
    } finally {
      if (syncInFlight[scope]?.key === idempotencyKey) delete syncInFlight[scope]
    }
  })()

  syncInFlight[scope] = { key: idempotencyKey, promise }
  return promise
}

export function resetLocalUsageSync() {
  delete lastReportKey.session
  delete lastReportKey.global
  delete syncInFlight.session
  delete syncInFlight.global
  const setSavedPoints = useAppStore.getState().setSavedPoints
  setSavedPoints('session', null)
  setSavedPoints('global', null)
}
