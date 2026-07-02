import { useCallback, useEffect, useRef } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { apiFetch } from '../lib/gateway'
import { normalizeClientGatewayBase } from '../lib/gateway'
import {
  bootstrapHerdsmanStatus,
  ensureEdgeUpstreamConfigured,
  initEdgeUpstream,
  reconcileEdgeOnBoot,
} from '../lib/edge-upstream'
import { ensureCloudUpstreamConfigured } from '../lib/cloud-upstream'
import { isTauri, gatewayStatus, gatewayStart } from '../lib/tauri'
import { useAppStore } from '../stores/appStore'
import { useSetupStore } from '../stores/setupStore'
import type { GatewayStatus, StatsSnapshot } from '../types/gateway'
import { queryKeys } from '../queries/keys'
import { syncLocalUsageFromStats } from './useLocalUsageSync'
import { usePrefs } from './usePrefs'
import { useTheme } from './useI18n'

export function useBootstrap(enabled: boolean) {
  const qc = useQueryClient()
  const booted = useRef(false)
  usePrefs()
  const { applyTheme } = useTheme()

  const {
    setConnected,
    setGatewayBase,
    setStatus,
    setStats,
    setGlobalStats,
    setUptimeAnchor,
    showToast,
    setIsTauriApp,
  } = useAppStore()

  const setSetup = useSetupStore((s) => s.setSetup)

  const tryConnect = useCallback(async () => {
    try {
      const status = await apiFetch<GatewayStatus>('/v1/admin/status')
      setStatus(status)
      setConnected(true)
      setUptimeAnchor({ secs: status.uptime_secs, at: Date.now() })
      const setup = await apiFetch<import('../types/gateway').UpstreamSetupView>('/v1/admin/setup')
      setSetup(setup)
      const [sessionStats, globalStats] = await Promise.all([
        apiFetch<StatsSnapshot>('/v1/admin/stats?scope=session'),
        apiFetch<StatsSnapshot>('/v1/admin/stats?scope=global'),
      ])
      const scope = useAppStore.getState().scope
      setStats(scope === 'global' ? globalStats : sessionStats)
      setGlobalStats(globalStats)
      void syncLocalUsageFromStats(globalStats, {
        scope: 'global',
        modelId: setup?.edge?.model?.trim() || undefined,
      })
      void qc.invalidateQueries({ queryKey: ['gateway'] })
      showToast('toast.connected')
    } catch {
      setConnected(false)
      setStatus(null)
      setStats(null)
      setGlobalStats(null)
      setUptimeAnchor(null)
    }
  }, [qc, setConnected, setGatewayBase, setGlobalStats, setSetup, setStats, setStatus, setUptimeAnchor, showToast])

  const afterBoot = useCallback(async () => {
    const setup = useSetupStore.getState().setup
    const connected = useAppStore.getState().connected
    const api = connected ? apiFetch : null
    try {
      const cloudResult = await ensureCloudUpstreamConfigured(api, {
        currentModel: setup?.cloud?.model,
        tokenBudget: setup?.cloud?.token_budget ?? undefined,
        silent: true,
      })
      if (cloudResult.response && typeof cloudResult.response === 'object' && 'upstream' in cloudResult.response) {
        setSetup((cloudResult.response as { upstream: import('../types/gateway').UpstreamSetupView }).upstream)
      }
    } catch (e) {
      console.warn('bootstrapCloudUpstream', e)
    }
    try {
      await bootstrapHerdsmanStatus()
      const edge = setup?.edge
      const edgeResult = await ensureEdgeUpstreamConfigured(api, {
        currentModel: edge?.model ?? undefined,
        currentUrl: edge?.base_url ?? undefined,
        silent: true,
      })
      if (edgeResult.response && typeof edgeResult.response === 'object' && 'upstream' in edgeResult.response) {
        setSetup((edgeResult.response as { upstream: import('../types/gateway').UpstreamSetupView }).upstream)
      }
      const reconciled = await reconcileEdgeOnBoot(api, setup?.edge)
      if (reconciled.response && typeof reconciled.response === 'object' && 'upstream' in reconciled.response) {
        setSetup((reconciled.response as { upstream: import('../types/gateway').UpstreamSetupView }).upstream)
      }
    } catch (e) {
      console.warn('bootstrapEdgeUpstream', e)
    }
    void qc.invalidateQueries({ queryKey: ['flowy'] })
  }, [qc, setSetup])

  useEffect(() => {
    if (!enabled || booted.current) return
    booted.current = true

    const tauri = isTauri()
    setIsTauriApp(tauri)
    if (tauri) document.body.classList.add('tauri-app')

    applyTheme()
    void initEdgeUpstream()

    const run = async () => {
      if (tauri) {
        try {
          const status = await gatewayStatus()
          if (!status.running) {
            const url = await gatewayStart()
            if (url) setGatewayBase(normalizeClientGatewayBase(url))
          } else if (status.url) {
            setGatewayBase(normalizeClientGatewayBase(status.url))
          }
        } catch (e) {
          console.error('Tauri bootstrap failed', e)
        }
      }
      await tryConnect()
      await afterBoot()
    }
    void run()

    const poll = setInterval(async () => {
      if (!useAppStore.getState().connected) return
      try {
        const status = await apiFetch<GatewayStatus>('/v1/admin/status')
        setStatus(status)
        setUptimeAnchor({ secs: status.uptime_secs, at: Date.now() })
        const scope = useAppStore.getState().scope
        const stats = await apiFetch<StatsSnapshot>(`/v1/admin/stats?scope=${scope}`)
        setStats(stats)
        if (scope === 'session') {
          const globalStats = await apiFetch<StatsSnapshot>('/v1/admin/stats?scope=global')
          setGlobalStats(globalStats)
          void syncLocalUsageFromStats(globalStats, {
            scope: 'global',
            modelId: useSetupStore.getState().setup?.edge?.model?.trim() || undefined,
          })
        } else {
          setGlobalStats(stats)
          void syncLocalUsageFromStats(stats, {
            scope: 'global',
            modelId: useSetupStore.getState().setup?.edge?.model?.trim() || undefined,
          })
        }
        void qc.invalidateQueries({ queryKey: queryKeys.gatewayStatus })
      } catch (e) {
        console.warn('status poll', e)
      }
    }, 10_000)

    return () => clearInterval(poll)
  }, [
    enabled,
    afterBoot,
    applyTheme,
    setGatewayBase,
    setGlobalStats,
    setIsTauriApp,
    setStats,
    setStatus,
    setUptimeAnchor,
    tryConnect,
    qc,
  ])
}
