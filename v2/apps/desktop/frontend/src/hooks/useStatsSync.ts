import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { apiFetch } from '../lib/gateway'
import { useAppStore } from '../stores/appStore'
import { useSetupStore } from '../stores/setupStore'
import type { GatewayStatus, StatsSnapshot } from '../types/gateway'
import { queryKeys } from '../queries/keys'
import { syncLocalUsageFromStats } from './useLocalUsageSync'

const STATS_POLL_MS = 5_000

/** Keep sidebar / store stats fresh while the app shell is mounted. */
export function useStatsSync() {
  const connected = useAppStore((s) => s.connected)
  const scope = useAppStore((s) => s.scope)
  const qc = useQueryClient()

  useEffect(() => {
    if (!connected) return

    let cancelled = false

    const sync = async () => {
      try {
        const status = await apiFetch<GatewayStatus>('/v1/admin/status')
        if (cancelled) return

        const { setStatus, setUptimeAnchor, setStats, setGlobalStats } = useAppStore.getState()
        setStatus(status)
        setUptimeAnchor({ secs: status.uptime_secs, at: Date.now() })

        const activeScope = useAppStore.getState().scope
        const modelId = useSetupStore.getState().setup?.edge?.model?.trim() || undefined
        const [sessionStats, globalStats] = await Promise.all([
          apiFetch<StatsSnapshot>('/v1/admin/stats?scope=session'),
          apiFetch<StatsSnapshot>('/v1/admin/stats?scope=global'),
        ])
        if (cancelled) return

        setStats(activeScope === 'global' ? globalStats : sessionStats)
        setGlobalStats(globalStats)
        void syncLocalUsageFromStats(sessionStats, { scope: 'session', modelId })
        void syncLocalUsageFromStats(globalStats, { scope: 'global', modelId })
        void qc.invalidateQueries({ queryKey: queryKeys.gatewayStats('session') })
        void qc.invalidateQueries({ queryKey: queryKeys.gatewayStats('global') })
        void qc.invalidateQueries({ queryKey: queryKeys.gatewayStatus })
      } catch (e) {
        console.warn('[stats-sync]', e)
      }
    }

    void sync()
    const id = setInterval(() => void sync(), STATS_POLL_MS)
    return () => {
      cancelled = true
      clearInterval(id)
    }
  }, [connected, scope, qc])
}