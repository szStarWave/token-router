import { useCallback, useEffect, useMemo, useState } from 'react'
import { useAppStore } from '../../stores/appStore'
import { useI18n } from '../../hooks/useI18n'
import { isWindowsTauri } from '../../lib/tauri'
import {
  configureWslAgent,
  detectWslEnvironment,
  getWslDetectSessionCache,
  wslSetupErrorMessage,
  type WslDetectResult,
  type WslDistroInfo,
} from '../../lib/wsl-setup'
import { agentKindLabel } from '../../lib/agent-quick-setup'
import type { AgentKind } from '../../lib/tauri'

function BtnSpinner() {
  return <span className="btn-spinner" aria-hidden="true" />
}

const AGENT_NAME_KEYS: Record<string, string> = {
  openclaw: 'overview.agentOpenClawName',
  hermes: 'overview.agentHermesName',
  'claude-code': 'overview.agentClaudeCodeName',
  codex: 'overview.agentCodexName',
  opencode: 'overview.agentOpenCodeName',
  codebuddy: 'overview.agentCodeBuddyName',
  workbuddy: 'overview.agentWorkBuddyName',
}

function pickDefaultDistro(distros: WslDistroInfo[], current: string | null): string | null {
  if (!distros.length) return null
  if (current && distros.some((d) => d.name === current)) return current
  if (distros.length === 1) return distros[0].name
  return null
}

export function WslSetupCard() {
  const { t } = useI18n()
  const connected = useAppStore((s) => s.connected)
  const showToast = useAppStore((s) => s.showToast)
  const desktopApp = isWindowsTauri()

  const [detecting, setDetecting] = useState(false)
  const [detect, setDetect] = useState<WslDetectResult | null>(null)
  const [selectedDistro, setSelectedDistro] = useState<string | null>(null)
  const [configuringAgents, setConfiguringAgents] = useState<Record<string, boolean>>({})

  const runningDistros = detect?.runningDistros ?? []
  const activeDistro = useMemo(
    () => runningDistros.find((d) => d.name === selectedDistro) ?? null,
    [runningDistros, selectedDistro],
  )

  const disabled = !desktopApp || !connected || detecting
  const disabledHint = !desktopApp
    ? t('overview.wslWindowsOnly')
    : !connected
      ? t('overview.gatewayRequired')
      : null

  const refreshDetect = useCallback(async (force = false) => {
    if (!desktopApp) {
      setDetect(null)
      setSelectedDistro(null)
      return
    }
    setDetecting(true)
    try {
      const result = await detectWslEnvironment({ force })
      setDetect(result)
      setSelectedDistro((current) => pickDefaultDistro(result.runningDistros, current))
    } catch (err) {
      const failed: WslDetectResult = {
        available: false,
        runningDistros: [],
        message: wslSetupErrorMessage(err),
      }
      setDetect(failed)
      setSelectedDistro(null)
    } finally {
      setDetecting(false)
    }
  }, [desktopApp])

  useEffect(() => {
    if (!desktopApp) return

    const cached = getWslDetectSessionCache()
    if (cached) {
      setDetect(cached)
      setSelectedDistro((current) => pickDefaultDistro(cached.runningDistros, current))
    }
  }, [desktopApp])

  const initializedAgents = useMemo(
    () => activeDistro?.agents.filter((a) => a.initialized) ?? [],
    [activeDistro?.agents],
  )

  const statusLine = useMemo(() => {
    if (detecting) return t('overview.wslDetecting')
    if (!detect) return t('overview.wslDetectPrompt')
    if (!detect.available) return detect.message || t('overview.wslUnavailable')
    if (!runningDistros.length) return detect.message || t('overview.wslNoRunningDistros')
    if (runningDistros.length > 1 && !selectedDistro) {
      return t('overview.wslMultipleRunning', { n: runningDistros.length })
    }
    if (!activeDistro) return t('overview.wslSelectDistroPrompt')
    const parts = [t('overview.wslDistro', { name: activeDistro.name })]
    if (activeDistro.gatewayHost) {
      parts.push(t('overview.wslGatewayHost', { host: activeDistro.gatewayHost }))
      if (activeDistro.gatewayVerified === false && activeDistro.message) {
        parts.push(t('overview.wslGatewayUnverified'))
      }
    } else if (activeDistro.message) {
      parts.push(activeDistro.message)
    }
    if (initializedAgents.length) {
      parts.push(t('overview.wslAgentsFound', { n: initializedAgents.length }))
    } else {
      parts.push(t('overview.wslNoAgents'))
    }
    return parts.join(' · ')
  }, [activeDistro, detect, detecting, initializedAgents.length, runningDistros.length, selectedDistro, t])

  const handleConfigureAgent = async (agent: string) => {
    const distro = selectedDistro ?? runningDistros[0]?.name
    if (!distro) {
      showToast('toast.agentConfigureFail', false, { msg: t('overview.wslSelectDistroPrompt') })
      return
    }

    setConfiguringAgents((prev) => ({ ...prev, [agent]: true }))
    try {
      const result = await configureWslAgent(distro, agent)
      await refreshDetect(true)
      showToast('toast.agentConfigured', true, {
        agent: result.agent,
        model: result.model,
        path: result.path,
      })
    } catch (err) {
      const msg = wslSetupErrorMessage(err)
      if (msg.includes('agent_not_initialized')) {
        const parts = msg.split(':')
        const ag = parts[1] || agent
        const path = parts.slice(2).join(':') || ''
        showToast('toast.agentNotInitialized', false, { agent: ag, path })
      } else {
        showToast('toast.agentConfigureFail', false, { msg })
      }
    } finally {
      setConfiguringAgents((prev) => ({ ...prev, [agent]: false }))
    }
  }

  if (!desktopApp) return null

  return (
    <>
      <div className="panel agent-quick-setup-card wsl-setup-card" id="wsl-setup-card">
        <div className="panel-title">{t('overview.wslSetup')}</div>
        <p className="agent-quick-setup-hint">{t('overview.wslSetupHint')}</p>
        {disabledHint && <p className="agent-quick-setup-note">{disabledHint}</p>}
        <p className="agent-quick-setup-note" id="wsl-setup-status">{statusLine}</p>

        {runningDistros.length > 1 && (
          <div className="wsl-distro-picker-inline">
            <label htmlFor="wsl-distro-select">{t('overview.wslSelectedDistro')}</label>
            <select
              id="wsl-distro-select"
              value={selectedDistro ?? ''}
              onChange={(e) => setSelectedDistro(e.target.value || null)}
            >
              <option value="">{t('overview.wslSelectDistroPlaceholder')}</option>
              {runningDistros.map((distro) => (
                <option key={distro.name} value={distro.name}>{distro.name}</option>
              ))}
            </select>
          </div>
        )}

        {!!initializedAgents.length && (
          <ul className="wsl-agent-detect-list">
            {initializedAgents.map((item) => {
              const key = AGENT_NAME_KEYS[item.agent]
              const label = key ? t(key) : agentKindLabel(item.agent as AgentKind) || item.agent
              const configuring = configuringAgents[item.agent] ?? false
              const distroReady = !!(selectedDistro ?? runningDistros[0]?.name)
              return (
                <li key={item.agent}>
                  <span>{label}</span>
                  <button
                    type="button"
                    className={`btn btn-sm ${item.deployed ? 'btn-ghost' : 'btn-primary'}`}
                    disabled={!distroReady || disabled || configuring}
                    onClick={() => void handleConfigureAgent(item.agent)}
                  >
                    {configuring ? <BtnSpinner /> : null}
                    {item.deployed ? t('overview.agentDeployed') : t('overview.deployAgent')}
                  </button>
                </li>
              )
            })}
          </ul>
        )}

        <div className="agent-quick-setup-actions">
          <button
            type="button"
            className="btn btn-ghost"
            id="btn-wsl-refresh"
            disabled={detecting}
            onClick={() => void refreshDetect(true)}
          >
            {detecting ? <BtnSpinner /> : null}
            {t('overview.wslRefreshDetect')}
          </button>
        </div>
      </div>
    </>
  )
}
