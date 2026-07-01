import { useEffect, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { apiFetch, generateGatewayAuthKey, maskGatewayAuthKey, normalizeClientGatewayBase, persistGatewayAuthKeyFull, readGatewayAuthKeyFromStorage, refreshGatewayStatusAfterRestart } from '../lib/gateway'
import { gatewayRestart, feedbackAppVersion, invokeErrorMessage, isTauri, isWindowsTauri, otaCheckNow } from '../lib/tauri'
import { useGatewayControlMutations } from '../queries/gateway'
import { useAppStore } from '../stores/appStore'
import { useSetupStore } from '../stores/setupStore'
import { useAuthStore } from '../stores/authStore'
import { useI18n, useTheme } from '../hooks/useI18n'
import { usePrefs } from '../hooks/usePrefs'
import { DEFAULT_GATEWAY_PORT } from '../constants/defaults'
import type { GatewayConfigView } from '../types/gateway'

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
  const gatewayAuthKeyPending = useAppStore((s) => s.gatewayAuthKeyPending)
  const setGatewayAuthKeyPending = useAppStore((s) => s.setGatewayAuthKeyPending)
  const setup = useSetupStore((s) => s.setup)
  const setSetup = useSetupStore((s) => s.setSetup)
  const logout = useAuthStore((s) => s.logout)
  const { start, stop, restart } = useGatewayControlMutations()
  const qc = useQueryClient()

  const g = setup?.gateway
  const [port, setPort] = useState(g?.listen_port ?? DEFAULT_GATEWAY_PORT)
  const [lan, setLan] = useState(!!g?.listen_lan)
  const [authKeyDisplay, setAuthKeyDisplay] = useState('')

  useEffect(() => {
    setPort(g?.listen_port ?? DEFAULT_GATEWAY_PORT)
    setLan(!!g?.listen_lan)
    refreshAuthKeyDisplay(g)
  }, [g])

  const refreshAuthKeyDisplay = (gateway?: GatewayConfigView | null) => {
    const pending = gatewayAuthKeyPending ?? readGatewayAuthKeyFromStorage()
    if (pending) {
      setAuthKeyDisplay(pending)
      return
    }
    if (gateway?.api_key_set && gateway.api_key_preview) {
      setAuthKeyDisplay(gateway.api_key_preview)
    } else {
      setAuthKeyDisplay('')
    }
  }

  const onTheme = (pref: 'light' | 'dark' | 'system') => {
    setThemePref(pref)
    savePrefs()
  }

  const onLang = (lang: 'zh' | 'en') => {
    setLocale(lang)
    savePrefs()
  }

  const generateKey = async () => {
    const key = generateGatewayAuthKey()
    setGatewayAuthKeyPending(key)
    persistGatewayAuthKeyFull(key)
    setAuthKeyDisplay(key)
    if (connected) {
      try {
        await apiFetch('/v1/admin/setup', { method: 'POST', body: JSON.stringify({ gateway: { api_key: key } }) })
        showToast(t('toast.gatewayAuthKeySaved'))
      } catch {
        /* saved locally */
      }
    }
  }

  const copyKey = async () => {
    const key = gatewayAuthKeyPending ?? readGatewayAuthKeyFromStorage()
    if (!key) {
      showToast(t('toast.gatewayAuthKeyEmpty'), false)
      return
    }
    await navigator.clipboard.writeText(key)
    showToast(t('toast.copied'))
  }

  const saveGatewayEndpoint = async () => {
    if (!connected) {
      showToast(t('conn.offline'), false)
      return
    }
    if (!Number.isInteger(port) || port < 1024 || port > 65535) {
      showToast(t('toast.gatewayPortInvalid'), false)
      return
    }
    try {
      const gateway: Record<string, unknown> = { listen_port: port, listen_lan: lan }
      const key = gatewayAuthKeyPending ?? readGatewayAuthKeyFromStorage()
      if (key) {
        gateway.api_key = key
        persistGatewayAuthKeyFull(key)
      }
      const res = await apiFetch<{ message?: string; upstream: typeof setup }>('/v1/admin/setup', {
        method: 'POST',
        body: JSON.stringify({ gateway }),
      })
      if (res.upstream) setSetup(res.upstream)
      if (isTauri()) {
        const url = await gatewayRestart()
        if (url) setGatewayBase(normalizeClientGatewayBase(url))
        await refreshGatewayStatusAfterRestart()
        showToast(res.message || t('toast.gatewayEndpointSaved'))
      } else {
        showToast(t('toast.gatewayEndpointRestart'))
      }
      void qc.invalidateQueries({ queryKey: ['gateway'] })
    } catch (e) {
      showToast(t('toast.saveFail', { msg: e instanceof Error ? e.message : String(e) }), false)
    }
  }

  const handleLogout = () => {
    logout()
    location.reload()
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
        <div className="form-switches">
          <div className="switch-row">
            <label htmlFor="gateway_lan_enabled">{t('field.gatewayLanEnabled')}</label>
            <label className="switch" aria-hidden="true">
              <input type="checkbox" id="gateway_lan_enabled" checked={lan} onChange={(e) => setLan(e.target.checked)} />
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
          <div className="form-row">
            <div>
              <label>{t('field.gatewayAuthKey')}</label>
              <div className="input-action-row">
                <input
                  id="gateway_api_key"
                  type="text"
                  readOnly
                  autoComplete="off"
                  spellCheck={false}
                  placeholder={t('ph.gatewayAuthKey')}
                  value={authKeyDisplay ? (authKeyDisplay.length > 12 && !authKeyDisplay.startsWith('token-') ? authKeyDisplay : maskGatewayAuthKey(authKeyDisplay)) : ''}
                />
                <button type="button" className="btn btn-ghost btn-sm" id="btn-gateway-auth-key-generate" onClick={() => void generateKey()}>
                  {t('action.generateAuthKey')}
                </button>
                <button type="button" className="btn btn-ghost btn-sm" id="btn-gateway-auth-key-copy" onClick={() => void copyKey()}>
                  {t('action.copy')}
                </button>
              </div>
            </div>
          </div>
        </div>
        <div className="upstream-actions">
          <button type="button" className="btn btn-primary" onClick={() => void saveGatewayEndpoint()}>
            {t('action.saveGatewayEndpoint')}
          </button>
        </div>
      </div>

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

  useEffect(() => {
    if (isTauri()) {
      void feedbackAppVersion()
        .then(setVersion)
        .catch(() => setVersion('—'))
    }
  }, [])

  const checkUpdate = async () => {
    if (!isWindowsTauri()) {
      showToast(t('ota.installFailed'), false)
      return
    }
    try {
      await otaCheckNow()
      showToast(t('settings.checkUpdateStarted'), true)
    } catch (err) {
      showToast(`${t('ota.checkFailed')}: ${invokeErrorMessage(err)}`, false)
    }
  }

  return (
    <div className="panel">
      <div className="panel-title">{t('settings.about')}</div>
      <div className="about-version-row">
        <span className="about-version-label">{t('settings.appVersion')}</span>
        <code className="about-version-code">{formatAppVersion(version)}</code>
        {isWindowsTauri() && (
          <button type="button" className="btn btn-ghost btn-sm" onClick={() => void checkUpdate()}>
            {t('settings.checkUpdate')}
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
