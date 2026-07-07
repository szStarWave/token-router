import { invokeErrorMessage, tauriInvoke } from './tauri'
import { resolveApiKey } from './agent-quick-setup'
import type { AgentKind } from './tauri'

export interface WslAgentDetectItem {
  agent: string
  initialized: boolean
  deployed: boolean
  configPath: string
}

export interface WslDistroInfo {
  name: string
  homePath?: string | null
  gatewayHost?: string | null
  gatewayV1Base?: string | null
  gatewayAnthropicBase?: string | null
  gatewayVerified?: boolean
  agents: WslAgentDetectItem[]
  message?: string | null
}

export interface WslDetectResult {
  available: boolean
  runningDistros: WslDistroInfo[]
  message?: string | null
}

export interface WslConfigureResult {
  distro: string
  gatewayHost: string
  configured: Array<{
    path: string
    model: string
    baseUrl: string
    agent: string
  }>
  skipped: string[]
}

export async function detectWslEnvironment(): Promise<WslDetectResult> {
  return tauriInvoke<WslDetectResult>('wsl_detect_environment')
}

export async function configureWslAgents(distro: string): Promise<WslConfigureResult> {
  const apiKey = await resolveApiKey('openclaw')
  return tauriInvoke<WslConfigureResult>('wsl_configure_agents', {
    distro,
    apiKey: apiKey ?? null,
  })
}

export function wslAgentLabel(agent: string): AgentKind | null {
  if (agent === 'openclaw' || agent === 'hermes' || agent === 'claude-code' || agent === 'codex' || agent === 'opencode') {
    return agent
  }
  return null
}

export function wslSetupErrorMessage(err: unknown): string {
  return invokeErrorMessage(err)
}
