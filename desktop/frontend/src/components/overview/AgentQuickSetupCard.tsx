import { useCallback, useEffect, useState } from 'react'
import flowyaipcIcon from '../../assets/flowyaipc.png?url'
import openclawIcon from '@lobehub/icons-static-svg/icons/openclaw-color.svg?url'
import hermesIcon from '@lobehub/icons-static-svg/icons/hermesagent.svg?url'
import claudecodeIcon from '@lobehub/icons-static-svg/icons/claudecode-color.svg?url'
import codexIcon from '@lobehub/icons-static-svg/icons/codex-color.svg?url'
import opencodeIcon from '@lobehub/icons-static-svg/icons/opencode.svg?url'
import { useAppStore } from '../../stores/appStore'
import { useI18n } from '../../hooks/useI18n'
import { isTauri } from '../../lib/tauri'
import {
  agentSetupErrorMessage,
  configureAgent,
  getAgentDeployStatus,
  parseAgentNotInitializedError,
  type AgentKind,
} from '../../lib/agent-quick-setup'

function BtnSpinner() {
  return <span className="btn-spinner" aria-hidden="true" />
}

type AgentNameKey =
  | 'overview.agentFlowyAipcName'
  | 'overview.agentOpenClawName'
  | 'overview.agentHermesName'
  | 'overview.agentClaudeCodeName'
  | 'overview.agentCodexName'
  | 'overview.agentOpenCodeName'

type AgentCardId = 'flowyaipc' | AgentKind

const AGENT_ACTIONS: Array<{
  id: AgentCardId
  kind: AgentKind
  nameKey: AgentNameKey
  icon: string
}> = [
  {
    id: 'flowyaipc',
    kind: 'openclaw',
    nameKey: 'overview.agentFlowyAipcName',
    icon: flowyaipcIcon,
  },
  {
    id: 'openclaw',
    kind: 'openclaw',
    nameKey: 'overview.agentOpenClawName',
    icon: openclawIcon,
  },
  {
    id: 'hermes',
    kind: 'hermes',
    nameKey: 'overview.agentHermesName',
    icon: hermesIcon,
  },
  {
    id: 'claude-code',
    kind: 'claude-code',
    nameKey: 'overview.agentClaudeCodeName',
    icon: claudecodeIcon,
  },
  {
    id: 'codex',
    kind: 'codex',
    nameKey: 'overview.agentCodexName',
    icon: codexIcon,
  },
  {
    id: 'opencode',
    kind: 'opencode',
    nameKey: 'overview.agentOpenCodeName',
    icon: opencodeIcon,
  },
]

export function AgentQuickSetupCard() {
  const { t } = useI18n()
  const connected = useAppStore((s) => s.connected)
  const showToast = useAppStore((s) => s.showToast)
  const desktopApp = isTauri()

  const [loadingId, setLoadingId] = useState<AgentCardId | null>(null)
  const [deployedMap, setDeployedMap] = useState<Partial<Record<AgentCardId, boolean>>>({})

  const disabled = !desktopApp || !connected || loadingId !== null
  const disabledHint = !desktopApp
    ? t('overview.desktopOnly')
    : !connected
      ? t('overview.gatewayRequired')
      : null

  const refreshDeployStatus = useCallback(async () => {
    if (!desktopApp) {
      setDeployedMap({})
      return
    }

    const uniqueKinds = [...new Set(AGENT_ACTIONS.map((card) => card.kind))]
    const kindStatus = new Map<AgentKind, boolean | null>()
    await Promise.all(
      uniqueKinds.map(async (kind) => {
        kindStatus.set(kind, await getAgentDeployStatus(kind))
      }),
    )

    setDeployedMap((prev) => {
      const next = { ...prev }
      for (const card of AGENT_ACTIONS) {
        const deployed = kindStatus.get(card.kind)
        if (deployed !== null && deployed !== undefined) {
          next[card.id] = deployed
        }
      }
      return next
    })
  }, [desktopApp])

  useEffect(() => {
    void refreshDeployStatus()
  }, [refreshDeployStatus])

  const runConfigure = async (card: (typeof AGENT_ACTIONS)[number]) => {
    if (disabled) return
    setLoadingId(card.id)
    const agentLabel = t(card.nameKey)
    try {
      const result = await configureAgent(card.kind)
      setDeployedMap((prev) => {
        const next = { ...prev }
        for (const item of AGENT_ACTIONS) {
          if (item.kind === card.kind) next[item.id] = true
        }
        return next
      })
      showToast('toast.agentConfigured', true, {
        agent: agentLabel,
        path: result.path,
        model: result.model,
      })
    } catch (err) {
      const notInit = parseAgentNotInitializedError(err)
      if (notInit) {
        showToast('toast.agentNotInitialized', false, {
          agent: agentLabel,
          path: notInit.configPath,
        })
        return
      }
      showToast('toast.agentConfigureFail', false, { msg: agentSetupErrorMessage(err) })
    } finally {
      setLoadingId(null)
    }
  }

  return (
    <div className="panel agent-quick-setup-card" id="agent-quick-setup-card">
      <div className="panel-title">{t('overview.agentQuickSetup')}</div>
      <p className="agent-quick-setup-hint">{t('overview.agentQuickSetupHint')}</p>
      {disabledHint && <p className="agent-quick-setup-note">{disabledHint}</p>}
      <div className="agent-card-grid">
        {AGENT_ACTIONS.map((card) => {
          const deployed = !!deployedMap[card.id]
          return (
            <article key={card.id} className="agent-card">
              <img className="agent-card-logo" src={card.icon} alt="" />
              <h3 className="agent-card-name">{t(card.nameKey)}</h3>
              <button
                type="button"
                className={`btn agent-card-deploy ${deployed ? 'btn-success' : 'btn-primary'}`}
                disabled={disabled}
                onClick={() => void runConfigure(card)}
              >
                {loadingId === card.id ? <BtnSpinner /> : null}
                {deployed ? t('overview.agentDeployed') : t('overview.deployAgent')}
              </button>
            </article>
          )
        })}
      </div>
    </div>
  )
}
