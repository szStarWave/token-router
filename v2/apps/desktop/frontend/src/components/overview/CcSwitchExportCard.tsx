import { useState } from 'react'
import { useAppStore } from '../../stores/appStore'
import { useI18n } from '../../hooks/useI18n'
import { isTauri } from '../../lib/tauri'
import {
  CC_SWITCH_RELEASES_URL,
  ccSwitchExportErrorMessage,
  exportToCcSwitch,
  type CcSwitchApp,
} from '../../lib/cc-switch-export'
import { openExternalUrl } from '../../lib/open-external'

function BtnSpinner() {
  return <span className="btn-spinner" aria-hidden="true" />
}

const CC_SWITCH_ACTIONS: Array<{
  app: CcSwitchApp
  labelKey:
    | 'overview.ccSwitchExportClaude'
    | 'overview.ccSwitchExportCodex'
    | 'overview.ccSwitchExportOpenClaw'
    | 'overview.ccSwitchExportHermes'
    | 'overview.ccSwitchExportGemini'
    | 'overview.ccSwitchExportOpenCode'
}> = [
  { app: 'claude', labelKey: 'overview.ccSwitchExportClaude' },
  { app: 'codex', labelKey: 'overview.ccSwitchExportCodex' },
  { app: 'openclaw', labelKey: 'overview.ccSwitchExportOpenClaw' },
  { app: 'hermes', labelKey: 'overview.ccSwitchExportHermes' },
  { app: 'gemini', labelKey: 'overview.ccSwitchExportGemini' },
  { app: 'opencode', labelKey: 'overview.ccSwitchExportOpenCode' },
]

export function CcSwitchExportCard() {
  const { t } = useI18n()
  const connected = useAppStore((s) => s.connected)
  const showToast = useAppStore((s) => s.showToast)
  const desktopApp = isTauri()

  const [loading, setLoading] = useState<CcSwitchApp | null>(null)

  const disabled = !desktopApp || !connected || loading !== null
  const disabledHint = !desktopApp
    ? t('overview.desktopOnly')
    : !connected
      ? t('overview.gatewayRequired')
      : null

  const runExport = async (app: CcSwitchApp) => {
    if (disabled) return
    setLoading(app)
    try {
      const deeplink = await exportToCcSwitch(app)
      await openExternalUrl(deeplink)
      showToast('toast.ccSwitchExportOpened', true)
    } catch (err) {
      const msg = ccSwitchExportErrorMessage(err)
      if (/not allowed|scope|denied|permission/i.test(msg)) {
        showToast('toast.ccSwitchNotInstalled', false)
        try {
          await openExternalUrl(CC_SWITCH_RELEASES_URL)
        } catch {
          // ignore secondary open failure
        }
        return
      }
      showToast('toast.ccSwitchExportFail', false, { msg })
    } finally {
      setLoading(null)
    }
  }

  return (
    <div className="panel agent-quick-setup-card cc-switch-export-card">
      <div className="panel-title">{t('overview.ccSwitchExport')}</div>
      <p className="agent-quick-setup-hint">{t('overview.ccSwitchExportHint')}</p>
      {disabledHint && <p className="agent-quick-setup-note">{disabledHint}</p>}
      <div className="agent-quick-setup-actions">
        {CC_SWITCH_ACTIONS.map(({ app, labelKey }) => (
          <button
            key={app}
            type="button"
            className="btn btn-primary"
            disabled={disabled}
            onClick={() => void runExport(app)}
          >
            {loading === app ? <BtnSpinner /> : null}
            {t(labelKey)}
          </button>
        ))}
      </div>
    </div>
  )
}
