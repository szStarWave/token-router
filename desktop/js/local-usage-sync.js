import { reportLocalModelUsage } from './flowy/api.js';
import { getAuthToken } from './flowy/auth-store.js';

let savedPoints = null;
let lastReportKey = null;
let syncInFlight = null;

function normalizeUsageNumber(value) {
  if (typeof value === 'number' && Number.isFinite(value)) return Math.max(0, Math.floor(value));
  if (typeof value === 'string' && value.trim()) {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return Math.max(0, Math.floor(parsed));
  }
  return 0;
}

function edgeTokensFromStats(stats) {
  const edge = stats?.token_breakdown?.edge;
  return {
    promptTokens: normalizeUsageNumber(edge?.input ?? stats?.edge_tokens_in),
    completionTokens: normalizeUsageNumber(edge?.output ?? stats?.edge_tokens_out),
    cacheTokens: normalizeUsageNumber(edge?.cached ?? stats?.edge_cached_tokens),
  };
}

function hasUsage(tokens) {
  return tokens.promptTokens > 0 || tokens.completionTokens > 0 || tokens.cacheTokens > 0;
}

export function getLocalUsageSavedPoints() {
  return savedPoints;
}

export function resetLocalUsageSavedPoints() {
  savedPoints = null;
  lastReportKey = null;
  syncInFlight = null;
}

export async function syncLocalUsageFromStats(stats, options = {}) {
  const token = getAuthToken();
  if (!token) {
    savedPoints = null;
    lastReportKey = null;
    console.info('[local-usage-sync] skip: not logged in');
    return null;
  }

  const usage = edgeTokensFromStats(stats);
  if (!hasUsage(usage)) {
    savedPoints = savedPoints ?? 0;
    console.info('[local-usage-sync] skip: no edge token usage', usage);
    return savedPoints;
  }

  // Align with FlowyClaw: report cumulative global edge tokens, not per-session UI scope.
  const scope = options.scope || 'global';
  const modelId = (options.modelId || '').trim();
  const idempotencyKey = `token-router:${scope}:${usage.promptTokens}:${usage.completionTokens}:${usage.cacheTokens}`;
  if (idempotencyKey === lastReportKey && savedPoints != null) {
    return savedPoints;
  }

  if (syncInFlight?.key === idempotencyKey) {
    return syncInFlight.promise;
  }

  const promise = (async () => {
    try {
      const result = await reportLocalModelUsage({
        modelId,
        promptTokens: usage.promptTokens,
        completionTokens: usage.completionTokens,
        cacheTokens: usage.cacheTokens,
        sessionId: scope,
        clientId: 'token-router',
        idempotencyKey,
        extra: {
          source: 'token-router-desktop',
          scope,
        },
      }, token);
      lastReportKey = idempotencyKey;
      savedPoints = typeof result.savedPoints === 'number' ? result.savedPoints : 0;
      console.info('[local-usage-sync] reported', { usage, savedPoints, duplicate: result.duplicate });
      options.onSynced?.(savedPoints);
      return savedPoints;
    } catch (e) {
      console.warn('[local-usage-sync] report failed', e);
      return savedPoints;
    } finally {
      if (syncInFlight?.key === idempotencyKey) syncInFlight = null;
    }
  })();

  syncInFlight = { key: idempotencyKey, promise };
  return promise;
}

export function installLocalUsageSync() {
  window.__localUsageSync = {
    syncFromStats: syncLocalUsageFromStats,
    getSavedPoints: getLocalUsageSavedPoints,
    reset: resetLocalUsageSavedPoints,
  };
}
