import { useEffect, useRef, useState } from 'react'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { useQueryClient } from '@tanstack/react-query'
import { apiFetch, normalizeClientGatewayBase, refreshGatewayStatusAfterRestart } from '../lib/gateway'
import {
  gatewayRestart,
  feedbackAppVersion,
  invokeErrorMessage,
  isTauri,
  isOtaDesktop,
  otaCheckNow,
  type OtaEventPayload,
} from '../lib/tauri'
import { useGatewayControlMutations } from '../queries/gateway'
import { useAppStore } from '../stores/appStore'
import { useSetupStore } from '../stores/setupStore'
import { useAuthStore } from '../stores/authStore'
import { useI18n, useTheme } from '../hooks/useI18n'
import { usePrefs } from '../hooks/usePrefs'
import { DEFAULT_GATEWAY_PORT } from '../constants/defaults'
import { GatewayAuthKeysPanel } from '../components/settings/GatewayAuthKeysPanel'
import { useOnboardingContext } from '../components/onboarding/OnboardingContext'
import { clearOnboardingSeen, ONBOARDING_ENABLED } from '../lib/onboarding'
import type { UpstreamSetupView } from '../types/gateway'

export function SettingsPage() {
  const { t } = useI18n()
  const { themePref, setThemePref } = useTheme()
  const locale = useAppStore((s) => s.locale)
  const setLocale = useAppStore((s) => s.setLocale)
  const { savePrefs } = usePrefs()
  const connected = useAppStore((s) => s.connected)
  const gatewayBase = useAppStore((s) => s.gatewayBase)
  const status = useAppStore((s) => s.status)
  const showToast = useAppStore((s) => s.showToast)
  const setGatewayBase = useAppStore((s) => s.setGatewayBase)
  const setup = useSetupStore((s) => s.setup)
  const setSetup = useSetupStore((s) => s.setSetup)
  const logout = useAuthStore((s) => s.logout)
  const { start, stop, restart } = useGatewayControlMutations()
  const qc = useQueryClient()
  const { restartTour } = useOnboardingContext()

  const g = setup?.gateway
  const [port, setPort] = useState(g?.listen_port ?? DEFAULT_GATEWAY_PORT)
  const [lan, setLan] = useState(!!g?.listen_lan)
  const [authEnabled, setAuthEnabled] = useState(!!g?.auth_enabled)

  useEffect(() => {
    setPort(g?.listen_port ?? DEFAULT_GATEWAY_PORT)
    setLan(!!g?.listen_lan)
    setAuthEnabled(!!g?.auth_enabled)
  }, [g])

  const onTheme = (pref: 'light' | 'dark' | 'system') => {
    setThemePref(pref)
    savePrefs()
  }

  const onLang = (lang: 'zh' | 'en') => {
    setLocale(lang)
    savePrefs()
  }

  const saveGatewayEndpoint = async () => {
    if (!connected) {
      showToast('conn.offline', false)
      return
    }
    if (!Number.isInteger(port) || port < 1024 || port > 65535) {
      showToast('toast.gatewayPortInvalid', false)
      return
    }
    try {
      const gateway = { listen_port: port, listen_lan: lan, auth_enabled: authEnabled }
      const res = await apiFetch<{ message?: string; upstream: typeof setup }>('/v1/admin/setup', {
        method: 'POST',
        body: JSON.stringify({ gateway }),
      })
      if (res.upstream) setSetup(res.upstream)
      if (isTauri()) {
        const url = await gatewayRestart()
        if (url) setGatewayBase(normalizeClientGatewayBase(url))
        await refreshGatewayStatusAfterRestart()
        showToast('toast.gatewayEndpointSaved')
      } else {
        showToast('toast.gatewayEndpointRestart')
      }
      const freshSetup = await apiFetch<UpstreamSetupView>('/v1/admin/setup')
      setSetup(freshSetup)
      void qc.invalidateQueries({ queryKey: ['gateway'] })
    } catch (e) {
      showToast('toast.saveFail', false, { msg: e instanceof Error ? e.message : String(e) })
    }
  }

  const handleLogout = () => {
    logout()
    location.reload()
  }

  const handleRestartOnboarding = () => {
    clearOnboardingSeen()
    restartTour()
    showToast('onboarding.restarted', true)
  }

  const gatewayRunning = status?.status === 'running'
  const controlBusy = start.isPending || stop.isPending || restart.isPending
  const canControl = isTauri()

  return (
    <section className="page active" id="page-settings">
      <div className="panel">
        <div className="panel-title">{t('settings.appearance')}</div>
        <div className="setting-row">
          <div className="setting-label">{t('settings.theme')}</div>
          <div className="segment-tabs" id="theme-tabs">
            {(['light', 'dark', 'system'] as const).map((pref) => (
              <button key={pref} type="button" className={`segment-tab${themePref === pref ? ' active' : ''}`} data-theme-pref={pref} onClick={() => onTheme(pref)}>
                {t(`theme.${pref}`)}
              </button>
            ))}
          </div>
        </div>
        <div className="setting-row">
          <div className="setting-label">{t('settings.language')}</div>
          <div className="segment-tabs" id="lang-tabs">
            {(['zh', 'en'] as const).map((lang) => (
              <button key={lang} type="button" className={`segment-tab${locale === lang ? ' active' : ''}`} data-lang={lang} onClick={() => onLang(lang)}>
                {t(`lang.${lang}`)}
              </button>
            ))}
          </div>
        </div>
      </div>

      <input type="hidden" id="gateway_base" value={gatewayBase} readOnly />

      <div className="panel">
        <div className="panel-title">{t('settings.gatewayListen')}</div>
        <div className="form-switches form-switches-stacked">
          <div className="switch-row">
            <span className="switch-row-label">{t('field.gatewayLanEnabled')}</span>
            <label className="switch">
              <input type="checkbox" id="gateway_lan_enabled" checked={lan} onChange={(e) => setLan(e.target.checked)} />
              <span className="switch-slider" />
            </label>
          </div>
          <div className="switch-row">
            <span className="switch-row-label">{t('field.gatewayAuthEnabled')}</span>
            <label className="switch">
              <input type="checkbox" id="gateway_auth_enabled" checked={authEnabled} onChange={(e) => setAuthEnabled(e.target.checked)} />
              <span className="switch-slider" />
            </label>
          </div>
        </div>
        <div className="gateway-endpoint-fields">
          <div className="form-row">
            <div>
              <label>{t('field.gatewayPort')}</label>
              <input id="gateway_port" type="number" min={1024} max={65535} step={1} value={port} onChange={(e) => setPort(Number(e.target.value))} />
            </div>
          </div>
        </div>
        <div className="upstream-actions">
          <button type="button" className="btn btn-primary" onClick={() => void saveGatewayEndpoint()}>
            {t('action.saveGatewayEndpoint')}
          </button>
        </div>
      </div>

      <GatewayAuthKeysPanel />

      <div className="panel">
        <div className="panel-title">{t('settings.gatewayControl')}</div>
        <div className="gateway-control-actions">
          <div className="gateway-control-group">
            <button type="button" className="btn btn-primary" id="btn-gateway-start" disabled={!canControl || gatewayRunning || controlBusy} onClick={() => start.mutate()}>
              {t('action.startGateway')}
            </button>
            <button type="button" className="btn btn-ghost" id="btn-gateway-stop" disabled={!canControl || !gatewayRunning || controlBusy} onClick={() => stop.mutate()}>
              {t('action.stopGateway')}
            </button>
          </div>
          <div className="gateway-control-group">
            <button type="button" className="btn btn-ghost" id="btn-gateway-restart" disabled={!canControl || !gatewayRunning || controlBusy} onClick={() => restart.mutate()}>
              {t('action.restartGateway')}
            </button>
          </div>
        </div>
      </div>

      {ONBOARDING_ENABLED ? (
        <div className="panel onboarding-settings-panel">
          <div className="panel-title">{t('settings.guide')}</div>
          <p className="setting-hint">{t('onboarding.settingsHint')}</p>
          <button type="button" className="btn btn-primary btn-sm" id="btn-restart-onboarding" onClick={handleRestartOnboarding}>
            {t('onboarding.restart')}
          </button>
        </div>
      ) : null}

      <div className="panel">
        <div className="panel-title">{t('settings.account')}</div>
        <button type="button" className="btn btn-ghost" id="btn-logout" onClick={handleLogout}>
          {t('action.logout')}
        </button>
      </div>

      <div className="panel">
        <div className="panel-title">{t('settings.dataDir')}</div>
        <div className="endpoint-box">
          <code id="data-dir">{status?.data_dir ?? '—'}</code>
        </div>
      </div>

      <AboutPanel />
    </section>
  )
}

function AboutPanel() {
  const { t } = useI18n()
  const showToast = useAppStore((s) => s.showToast)
  const [version, setVersion] = useState('—')
  const [checkingUpdate, setCheckingUpdate] = useState(false)
  const manualCheckRef = useRef(false)

  useEffect(() => {
    if (isTauri()) {
      void feedbackAppVersion()
        .then(setVersion)
        .catch(() => setVersion('—'))
    }
  }, [])

  useEffect(() => {
    let unlisten: UnlistenFn | undefined
    void listen<OtaEventPayload>('ota:event', (event) => {
      if (!manualCheckRef.current) return

      const { message, data } = event.payload
      switch (message) {
        case 'ota.upToDate':
          manualCheckRef.current = false
          showToast('toast.checkUpdateUpToDate', true)
          break
        case 'ota.newVersion': {
          manualCheckRef.current = false
          const newVersion = typeof data?.new_version === 'string' ? data.new_version : ''
          showToast('toast.checkUpdateNewVersion', true, { version: newVersion })
          break
        }
        case 'ota.checkFailed':
        case 'ota.compareFailed': {
          manualCheckRef.current = false
          const msg =
            typeof data?.error === 'string'
              ? data.error
              : t(message === 'ota.checkFailed' ? 'ota.checkFailed' : 'ota.compareFailed')
          showToast('toast.checkUpdateFail', false, { msg })
          break
        }
      }
    }).then((fn) => {
      unlisten = fn
    })
    return () => {
      void unlisten?.()
    }
  }, [showToast, t])

  const checkUpdate = async () => {
    if (checkingUpdate) return
    if (!isOtaDesktop()) {
      showToast('ota.installFailed', false)
      return
    }
    manualCheckRef.current = true
    setCheckingUpdate(true)
    try {
      await otaCheckNow()
    } catch (err) {
      manualCheckRef.current = false
      showToast('toast.checkUpdateFail', false, { msg: invokeErrorMessage(err) })
    } finally {
      setCheckingUpdate(false)
    }
  }

  return (
    <div className="panel">
      <div className="panel-title">{t('settings.about')}</div>
      <div className="about-version-row">
        <span className="about-version-label">{t('settings.appVersion')}</span>
        <code className="about-version-code">{formatAppVersion(version)}</code>
        {isOtaDesktop() && (
          <button
            type="button"
            className="btn btn-ghost btn-sm"
            disabled={checkingUpdate}
            aria-busy={checkingUpdate}
            onClick={() => void checkUpdate()}
          >
            {checkingUpdate && <span className="btn-spinner" aria-hidden="true" />}
            {checkingUpdate ? t('settings.checkUpdateStarted') : t('settings.checkUpdate')}
          </button>
        )}
      </div>
    </div>
  )
}

function formatAppVersion(version: string) {
  if (!version || version === '—') return version
  return version.startsWith('v') ? version : `v${version}`
}
