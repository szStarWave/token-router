import { useState } from 'react'
import { useAppStore } from '../../stores/appStore'
import { useI18n } from '../../hooks/useI18n'
import { isTauri } from '../../lib/tauri'
import {
  agentKindLabel,
  agentSetupErrorMessage,
  configureAgent,
  parseAgentNotInitializedError,
  type AgentKind,
} from '../../lib/agent-quick-setup'

function BtnSpinner() {
  return <span className="btn-spinner" aria-hidden="true" />
}

const AGENT_ACTIONS: Array<{
  kind: AgentKind
  labelKey:
    | 'overview.configureOpenClaw'
    | 'overview.configureHermes'
    | 'overview.configureHermesFlash'
    | 'overview.configureClaudeCode'
    | 'overview.configureCodex'
}> = [
  { kind: 'openclaw', labelKey: 'overview.configureOpenClaw' },
  { kind: 'hermes', labelKey: 'overview.configureHermes' },
  { kind: 'hermes-flash', labelKey: 'overview.configureHermesFlash' },
  { kind: 'claude-code', labelKey: 'overview.configureClaudeCode' },
  { kind: 'codex', labelKey: 'overview.configureCodex' },
]

export function AgentQuickSetupCard() {
  const { t } = useI18n()
  const connected = useAppStore((s) => s.connected)
  const showToast = useAppStore((s) => s.showToast)
  const desktopApp = isTauri()

  const [loading, setLoading] = useState<AgentKind | null>(null)

  const disabled = !desktopApp || !connected || loading !== null
  const disabledHint = !desktopApp
    ? t('overview.desktopOnly')
    : !connected
      ? t('overview.gatewayRequired')
      : null

  const runConfigure = async (kind: AgentKind) => {
    if (disabled) return
    setLoading(kind)
    try {
      const result = await configureAgent(kind)
      showToast('toast.agentConfigured', true, {
        agent: agentKindLabel(kind),
        path: result.path,
        model: result.model,
      })
    } catch (err) {
      const notInit = parseAgentNotInitializedError(err)
      if (notInit) {
        showToast('toast.agentNotInitialized', false, {
          agent: agentKindLabel(notInit.agent),
          path: notInit.configPath,
        })
        return
      }
      showToast('toast.agentConfigureFail', false, { msg: agentSetupErrorMessage(err) })
    } finally {
      setLoading(null)
    }
  }

  return (
    <div className="panel agent-quick-setup-card" id="agent-quick-setup-card">
      <div className="panel-title">{t('overview.agentQuickSetup')}</div>
      <p className="agent-quick-setup-hint">{t('overview.agentQuickSetupHint')}</p>
      {disabledHint && <p className="agent-quick-setup-note">{disabledHint}</p>}
      <div className="agent-quick-setup-actions">
        {AGENT_ACTIONS.map(({ kind, labelKey }) => (
          <button
            key={kind}
            type="button"
            className="btn btn-primary"
            disabled={disabled}
            onClick={() => void runConfigure(kind)}
          >
            {loading === kind ? <BtnSpinner /> : null}
            {t(labelKey)}
          </button>
        ))}
      </div>
    </div>
  )
}
