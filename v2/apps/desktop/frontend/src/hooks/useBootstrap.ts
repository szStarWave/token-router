import { useCallback, useEffect, useRef } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { apiFetch } from '../lib/gateway'
import { normalizeClientGatewayBase } from '../lib/gateway'
import { loadUiState } from '../lib/ui-state'
import {
  bootstrapHerdsmanStatus,
  ensureEdgeUpstreamConfigured,
  initEdgeUpstream,
  reconcileEdgeOnBoot,
} from '../lib/edge-upstream'
import { ensureCloudUpstreamConfigured, initCloudUpstream } from '../lib/cloud-upstream'
import { isTauri, gatewayStart } from '../lib/tauri'
import { useAppStore } from '../stores/appStore'
import { useSetupStore } from '../stores/setupStore'
import type { GatewayStatus, StatsSnapshot } from '../types/gateway'
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
      const modelId = setup?.edge?.model?.trim() || undefined
      void syncLocalUsageFromStats(sessionStats, { scope: 'session', modelId })
      void syncLocalUsageFromStats(globalStats, { scope: 'global', modelId })
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
    const connected = useAppStore.getState().connected
    const api = connected ? apiFetch : null
    const setup = useSetupStore.getState().setup
    try {
      const cloud = setup?.cloud
      const cloudResult = await ensureCloudUpstreamConfigured(api, {
        currentModel: cloud?.model,
        currentUrl: cloud?.base_url,
        tokenBudget:
          cloud?.token_quota_enabled && cloud.token_budget != null
            ? cloud.token_budget
            : undefined,
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

    const run = async () => {
      if (tauri) {
        try {
          await loadUiState()
        } catch (e) {
          console.warn('loadUiState failed', e)
        }
      }
      void initEdgeUpstream()
      initCloudUpstream()
      if (tauri) {
        try {
          const url = await gatewayStart()
          if (url) setGatewayBase(normalizeClientGatewayBase(url))
        } catch (e) {
          console.error('Tauri bootstrap failed', e)
        }
      }
      await tryConnect()
      await afterBoot()
    }
    void run()
  }, [
    enabled,
    afterBoot,
    applyTheme,
    setGatewayBase,
    setIsTauriApp,
    tryConnect,
  ])
}
