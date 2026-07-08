export const queryKeys = {
  gatewayStatus: ['gateway', 'status'] as const,
  gatewaySetup: (agentId?: string) => ['gateway', 'setup', agentId ?? ''] as const,
  gatewayStats: (scope: string) => ['gateway', 'stats', scope] as const,
  statsTimeline: (scope: string, range: string) => ['gateway', 'stats', 'timeline', scope, range] as const,
  modelTokenTimeline: (scope: string, tier: string, model: string, range: string) =>
    ['gateway', 'stats', 'model-timeline', scope, tier, model, range] as const,
  allModelsTokenTimeline: (scope: string, range: string) =>
    ['gateway', 'stats', 'model-timeline-all', scope, range] as const,
  gatewayLogs: (offset: number | null) => ['gateway', 'logs', offset] as const,
  gatewayRoutingLogs: (afterId: number | null) => ['gateway', 'routing-logs', afterId] as const,
  gatewayAuthKeys: ['gateway', 'auth-keys'] as const,
  flowyCredits: ['flowy', 'credits'] as const,
  flowyUsage: ['flowy', 'usage'] as const,
  flowyModels: ['flowy', 'models'] as const,
}
