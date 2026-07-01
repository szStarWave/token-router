export const queryKeys = {
  gatewayStatus: ['gateway', 'status'] as const,
  gatewaySetup: (agentId?: string) => ['gateway', 'setup', agentId ?? ''] as const,
  gatewayStats: (scope: string) => ['gateway', 'stats', scope] as const,
  gatewayLogs: (offset: number | null) => ['gateway', 'logs', offset] as const,
  flowyCredits: ['flowy', 'credits'] as const,
  flowyUsage: ['flowy', 'usage'] as const,
  flowyModels: ['flowy', 'models'] as const,
}
