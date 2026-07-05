import { generateGatewayAuthKey } from './gateway'
import {
  checkAgentDeployed,
  checkAgentInitialized,
  configureClaudeCodeAgent,
  configureCodexAgent,
  configureHermesAgent,
  configureHermesFlashAgent,
  configureOpenClawAgent,
  invokeErrorMessage,
  readDefaultAuthKey,
  readInboundAuthKey as readInboundAuthKeyCmd,
  type AgentKind,
} from './tauri'
import { useSetupStore } from '../stores/setupStore'

export type { AgentKind }

export interface AgentSetupResult {
  path: string
  model: string
  baseUrl: string
  agent: string
}

export class AgentNotInitializedError extends Error {
  readonly agent: AgentKind
  readonly configPath: string

  constructor(agent: AgentKind, configPath: string) {
    super(`agent_not_initialized:${agent}:${configPath}`)
    this.name = 'AgentNotInitializedError'
    this.agent = agent
    this.configPath = configPath
  }
}

const AGENT_KIND_PATTERN =
  'openclaw|hermes-flash|hermes|claude-code|codex'

export function parseAgentNotInitializedError(err: unknown): AgentNotInitializedError | null {
  const msg = invokeErrorMessage(err)
  const match = new RegExp(`^agent_not_initialized:(${AGENT_KIND_PATTERN}):(.+)$`).exec(msg)
  if (!match) return null
  return new AgentNotInitializedError(match[1] as AgentKind, match[2])
}

function isAuthEnabled(): boolean {
  return !!useSetupStore.getState().setup?.gateway?.auth_enabled
}

export async function resolveApiKey(_agentName: AgentKind): Promise<string | null> {
  if (!isAuthEnabled()) {
    return generateGatewayAuthKey()
  }

  const defaultKey = await readDefaultAuthKey()
  if (defaultKey?.trim()) return defaultKey.trim()

  const fallback = await readInboundAuthKeyCmd()
  return fallback?.trim() || null
}

export async function getAgentDeployStatus(agent: AgentKind): Promise<boolean | null> {
  try {
    const status = await checkAgentDeployed(agent)
    return status.deployed
  } catch {
    return null
  }
}

async function ensureAgentInitialized(agent: AgentKind) {
  const status = await checkAgentInitialized(agent)
  if (!status.initialized) {
    throw new AgentNotInitializedError(agent, status.configPath)
  }
}

const CONFIGURE_HANDLERS: Record<
  AgentKind,
  (apiKey: string | null) => Promise<AgentSetupResult>
> = {
  openclaw: (apiKey) => configureOpenClawAgent(apiKey),
  hermes: (apiKey) => configureHermesAgent(apiKey),
  'hermes-flash': (apiKey) => configureHermesFlashAgent(apiKey),
  'claude-code': (apiKey) => configureClaudeCodeAgent(apiKey),
  codex: (apiKey) => configureCodexAgent(apiKey),
}

export async function configureAgent(agent: AgentKind): Promise<AgentSetupResult> {
  await ensureAgentInitialized(agent)
  const apiKey = await resolveApiKey(agent)
  return CONFIGURE_HANDLERS[agent](apiKey)
}

export function configureOpenClaw() {
  return configureAgent('openclaw')
}

export function configureHermes() {
  return configureAgent('hermes')
}

export function configureHermesFlash() {
  return configureAgent('hermes-flash')
}

export function configureClaudeCode() {
  return configureAgent('claude-code')
}

export function configureCodex() {
  return configureAgent('codex')
}

export function agentKindLabel(agent: AgentKind): string {
  switch (agent) {
    case 'openclaw':
      return 'OpenClaw'
    case 'hermes-flash':
      return 'Hermes Flash'
    case 'hermes':
      return 'Hermes Agent'
    case 'claude-code':
      return 'Claude Code'
    case 'codex':
      return 'Codex'
  }
}

export { invokeErrorMessage as agentSetupErrorMessage }
