import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { apiFetch, postSetup, refreshGatewayStatusAfterRestart } from '../lib/gateway'
import { useAppStore } from '../stores/appStore'
import { useSetupStore } from '../stores/setupStore'
import type { LogsResponse, GatewayStatus, StatsSnapshot, UpstreamSetupUpdate, UpstreamSetupView } from '../types/gateway'
import { queryKeys } from './keys'
import { normalizeClientGatewayBase } from '../lib/gateway'
import { gatewayReadLogs, gatewayRestart, gatewayStart, gatewayStop, isTauri } from '../lib/tauri'

export function useGatewayStatusQuery(enabled = true) {
  const connected = useAppStore((s) => s.connected)
  return useQuery({
    queryKey: queryKeys.gatewayStatus,
    queryFn: () => apiFetch<GatewayStatus>('/v1/admin/status'),
    enabled: enabled && connected,
    refetchInterval: connected ? 10_000 : false,
  })
}

export function useGatewaySetupQuery(agentId?: string) {
  const connected = useAppStore((s) => s.connected)
  return useQuery({
    queryKey: queryKeys.gatewaySetup(agentId),
    queryFn: async () => {
      const path = agentId
        ? `/v1/admin/setup?agent_id=${encodeURIComponent(agentId)}`
        : '/v1/admin/setup'
      return apiFetch<UpstreamSetupView>(path)
    },
    enabled: connected,
  })
}

export function useGatewayStatsQuery(scope: 'session' | 'global') {
  const connected = useAppStore((s) => s.connected)
  return useQuery({
    queryKey: queryKeys.gatewayStats(scope),
    queryFn: () => apiFetch<StatsSnapshot>(`/v1/admin/stats?scope=${scope}`),
    enabled: connected,
    refetchInterval: connected ? 10_000 : false,
  })
}

export async function fetchGatewayLogs(params: {
  offset?: number | null
  beforeOffset?: number | null
}): Promise<LogsResponse> {
  try {
    const q = new URLSearchParams()
    if (params.beforeOffset != null) q.set('before_offset', String(params.beforeOffset))
    else if (params.offset != null) q.set('offset', String(params.offset))
    const qs = q.toString()
    return await apiFetch<LogsResponse>(`/v1/admin/logs${qs ? `?${qs}` : ''}`)
  } catch {
    if (isTauri()) return gatewayReadLogs(params.offset, params.beforeOffset)
    throw new Error('logs unavailable')
  }
}

export function useGatewayLogsQuery(offset: number | null, enabled: boolean) {
  const connected = useAppStore((s) => s.connected)
  return useQuery({
    queryKey: queryKeys.gatewayLogs(offset),
    queryFn: () => fetchGatewayLogs({ offset }),
    enabled: connected && enabled,
    refetchInterval: enabled && connected ? 1000 : false,
  })
}

export function useSaveSetupMutation() {
  const qc = useQueryClient()
  const showToast = useAppStore((s) => s.showToast)
  const setSetup = useSetupStore((s) => s.setSetup)
  return useMutation({
    mutationFn: (body: UpstreamSetupUpdate) => postSetup(body),
    onSuccess: (res) => {
      if (res.upstream) setSetup(res.upstream)
      void qc.invalidateQueries({ queryKey: ['gateway'] })
      if (res.message) showToast(res.message)
    },
    onError: (e: Error) => showToast(e.message, false),
  })
}

export function useGatewayControlMutations() {
  const qc = useQueryClient()
  const start = useMutation({
    mutationFn: async () => {
      if (!isTauri()) throw new Error('offline')
      const { setGatewayBase } = useAppStore.getState()
      const url = await gatewayStart()
      if (url) setGatewayBase(normalizeClientGatewayBase(url))
      return url
    },
    onSuccess: async () => {
      const { setConnected, showToast } = useAppStore.getState()
      try {
        await refreshGatewayStatusAfterRestart()
        setConnected(true)
      } catch (e) {
        showToast(e instanceof Error ? e.message : String(e), false)
      }
      void qc.invalidateQueries({ queryKey: ['gateway'] })
    },
    onError: (e: Error) => useAppStore.getState().showToast(e.message, false),
  })
  const stop = useMutation({
    mutationFn: () => gatewayStop(),
    onSuccess: () => {
      const { setConnected, setStatus, setUptimeAnchor, status } = useAppStore.getState()
      setConnected(false)
      setUptimeAnchor(null)
      if (status) {
        setStatus({ ...status, status: 'stopped', uptime_secs: 0 })
      }
      void qc.invalidateQueries({ queryKey: ['gateway'] })
    },
    onError: (e: Error) => useAppStore.getState().showToast(e.message, false),
  })
  const restart = useMutation({
    mutationFn: async () => {
      const { setGatewayBase } = useAppStore.getState()
      const url = await gatewayRestart()
      if (url) setGatewayBase(normalizeClientGatewayBase(url))
      return url
    },
    onSuccess: async () => {
      const { setConnected, showToast } = useAppStore.getState()
      try {
        await refreshGatewayStatusAfterRestart()
        setConnected(true)
      } catch (e) {
        showToast(e instanceof Error ? e.message : String(e), false)
      }
      void qc.invalidateQueries({ queryKey: ['gateway'] })
    },
    onError: (e: Error) => useAppStore.getState().showToast(e.message, false),
  })
  return { start, stop, restart }
}
