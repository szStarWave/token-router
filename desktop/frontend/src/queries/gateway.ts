import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { apiFetch, postSetup, refreshGatewayStatusAfterRestart } from '../lib/gateway'
import { useAppStore } from '../stores/appStore'
import { useSetupStore } from '../stores/setupStore'
import type { LogsResponse, RoutingLogsResponse, GatewayStatus, StatsSnapshot, StatsTimelineResponse, UpstreamSetupUpdate, UpstreamSetupView } from '../types/gateway'
import { toastErrorKey } from '../lib/toast-i18n'
import { queryKeys } from './keys'
import { normalizeClientGatewayBase } from '../lib/gateway'
import { gatewayReadLogs, gatewayReadRoutingLogs, gatewayRestart, gatewayStart, gatewayStop, invokeErrorMessage, isTauri } from '../lib/tauri'
import { parseRoutingLogLines } from '../lib/routing-log'

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

export function useStatsTimelineQuery(
  scope: 'session' | 'global',
  range: 'h24' | 'd7' | 'd30',
  refetchIntervalMs = 10_000,
) {
  const connected = useAppStore((s) => s.connected)
  return useQuery({
    queryKey: queryKeys.statsTimeline(scope, range),
    queryFn: async () => {
      const tzOffset = new Date().getTimezoneOffset()
      return apiFetch<StatsTimelineResponse>(
        `/v1/admin/stats/timeline?scope=${scope}&range=${range}&tz_offset=${tzOffset}`,
      )
    },
    enabled: connected,
    refetchInterval: connected ? refetchIntervalMs : false,
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

async function fetchRoutingLogsFromFile(): Promise<RoutingLogsResponse> {
  const data = await fetchGatewayLogs({ offset: null })
  let nextId = 0
  const lines = data.lines.map((line) => ({
    id: nextId++,
    msg: line.text ?? line.msg ?? '',
  }))
  const entries = parseRoutingLogLines(lines).map((entry) => ({
    id: entry.id,
    timestamp: entry.timestamp,
    route: entry.route,
    served_route: entry.route,
    step_kind: entry.stepKind,
    model: entry.model ?? 'auto',
    user_preview: entry.userPreview,
    reason_codes: entry.reasonCodes,
  }))
  return { entries, has_older: false }
}

export async function fetchRoutingLogs(params: {
  afterId?: number | null
  beforeId?: number | null
  limit?: number
}): Promise<RoutingLogsResponse> {
  const errors: string[] = []

  if (isTauri()) {
    try {
      return await gatewayReadRoutingLogs(params.afterId, params.beforeId, params.limit)
    } catch (e) {
      errors.push(invokeErrorMessage(e))
    }
  }

  try {
    const q = new URLSearchParams()
    if (params.beforeId != null) q.set('before_id', String(params.beforeId))
    else if (params.afterId != null) q.set('after_id', String(params.afterId))
    if (params.limit != null) q.set('limit', String(params.limit))
    const qs = q.toString()
    return await apiFetch<RoutingLogsResponse>(`/v1/admin/routing-logs${qs ? `?${qs}` : ''}`)
  } catch (e) {
    errors.push(e instanceof Error ? e.message : String(e))
  }

  if (params.afterId == null && params.beforeId == null) {
    try {
      return await fetchRoutingLogsFromFile()
    } catch (e) {
      errors.push(e instanceof Error ? e.message : String(e))
    }
  }

  const detail = errors.filter(Boolean).join('; ') || 'routing logs unavailable'
  throw new Error(detail)
}

export function useRoutingLogsQuery(afterId: number | null, enabled: boolean) {
  const connected = useAppStore((s) => s.connected)
  return useQuery({
    queryKey: queryKeys.gatewayRoutingLogs(afterId),
    queryFn: () => fetchRoutingLogs({ afterId }),
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
      showToast('toast.upstreamSaved')
    },
    onError: (e: Error) => {
      const { key, vars } = toastErrorKey(e, 'toast.saveFail')
      showToast(key, false, vars)
    },
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
        const { key, vars } = toastErrorKey(e, 'toast.startFail')
        showToast(key, false, vars)
      }
      void qc.invalidateQueries({ queryKey: ['gateway'] })
    },
    onError: (e: Error) => {
      const { key, vars } = toastErrorKey(e, 'toast.startFail')
      useAppStore.getState().showToast(key, false, vars)
    },
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
    onError: (e: Error) => {
      const { key, vars } = toastErrorKey(e, 'toast.stopFail')
      useAppStore.getState().showToast(key, false, vars)
    },
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
        const { key, vars } = toastErrorKey(e, 'toast.restartFail')
        showToast(key, false, vars)
      }
      void qc.invalidateQueries({ queryKey: ['gateway'] })
    },
    onError: (e: Error) => {
      const { key, vars } = toastErrorKey(e, 'toast.restartFail')
      useAppStore.getState().showToast(key, false, vars)
    },
  })
  return { start, stop, restart }
}
