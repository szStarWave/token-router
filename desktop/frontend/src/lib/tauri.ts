import { invoke } from '@tauri-apps/api/core'

export interface GatewayStatusPayload {
  running: boolean
  url?: string | null
  version?: string
}

export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI__' in window
}

export async function tauriInvoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  return invoke<T>(cmd, args)
}

export function invokeErrorMessage(err: unknown): string {
  if (typeof err === 'string') return err
  if (err && typeof err === 'object' && 'message' in err) return String((err as Error).message)
  return String(err)
}

export async function gatewayStatus() {
  return tauriInvoke<GatewayStatusPayload>('gateway_status')
}

export async function gatewayStart() {
  return tauriInvoke<string>('gateway_start')
}

export async function gatewayStop() {
  return tauriInvoke<void>('gateway_stop')
}

export async function gatewayRestart() {
  return tauriInvoke<string>('gateway_restart')
}

export async function gatewayReadLogs(
  offset?: number | null,
  beforeOffset?: number | null,
) {
  return tauriInvoke<{
    offset: number
    next_offset: number
    reset: boolean
    lines: Array<{ level: string; text?: string; msg?: string }>
  }>('gateway_read_logs', {
    offset: offset ?? null,
    before_offset: beforeOffset ?? null,
  })
}

export async function gatewayOpenLogsDir() {
  return tauriInvoke<void>('gateway_open_logs_dir')
}

export async function herdsmanGetStatus() {
  return tauriInvoke<{
    connected: boolean
    install_detected?: boolean
    models?: unknown[]
  }>('herdsman_get_status')
}

export async function herdsmanStart() {
  return tauriInvoke<void>('herdsman_start')
}

export async function herdsmanOpenOrInstall() {
  return tauriInvoke<void>('herdsman_open_or_install')
}

export async function feedbackAppVersion() {
  return tauriInvoke<string>('feedback_app_version')
}

export async function feedbackSubmit(content: string, category?: string) {
  return tauriInvoke<void>('feedback_submit', { content, category: category ?? null })
}

export interface AgentSetupResultPayload {
  path: string
  model: string
  baseUrl: string
  agent: string
}

export interface AgentInitStatusPayload {
  initialized: boolean
  configPath: string
  agent: string
}

export interface AgentDeployStatusPayload {
  deployed: boolean
  configPath: string
  agent: string
}

export type AgentKind =
  | 'openclaw'
  | 'hermes'
  | 'hermes-flash'
  | 'claude-code'
  | 'codex'

export async function checkAgentInitialized(agent: AgentKind) {
  return tauriInvoke<AgentInitStatusPayload>('check_agent_initialized', { agent })
}

export async function checkAgentDeployed(agent: AgentKind) {
  return tauriInvoke<AgentDeployStatusPayload>('check_agent_deployed', { agent })
}

export async function configureOpenClawAgent(apiKey?: string | null) {
  return tauriInvoke<AgentSetupResultPayload>('configure_openclaw_agent', {
    apiKey: apiKey ?? null,
  })
}

export async function configureHermesAgent(apiKey?: string | null) {
  return tauriInvoke<AgentSetupResultPayload>('configure_hermes_agent', {
    apiKey: apiKey ?? null,
  })
}

export async function configureHermesFlashAgent(apiKey?: string | null) {
  return tauriInvoke<AgentSetupResultPayload>('configure_hermes_flash_agent', {
    apiKey: apiKey ?? null,
  })
}

export async function configureClaudeCodeAgent(apiKey?: string | null) {
  return tauriInvoke<AgentSetupResultPayload>('configure_claude_code_agent', {
    apiKey: apiKey ?? null,
  })
}

export async function configureCodexAgent(apiKey?: string | null) {
  return tauriInvoke<AgentSetupResultPayload>('configure_codex_agent', {
    apiKey: apiKey ?? null,
  })
}

export async function readInboundAuthKey(preferredName?: string | null) {
  return tauriInvoke<string | null>('read_inbound_auth_key_cmd', {
    preferredName: preferredName ?? null,
  })
}

export interface OtaEventPayload {
  message: string
  data?: Record<string, unknown> | null
}

export interface PostOtaRestartNotice {
  show: boolean
  version: string
  release_notes?: Record<string, string[]>
}

export async function otaAppVersion() {
  return tauriInvoke<string>('ota_app_version')
}

export async function otaCheckNow() {
  return tauriInvoke<void>('ota_check_now')
}

export async function otaDownloadUpdate() {
  return tauriInvoke<void>('ota_download_update')
}

export async function otaDoUpdate() {
  return tauriInvoke<void>('ota_do_update')
}

export async function otaGetPostRestartNotice() {
  return tauriInvoke<PostOtaRestartNotice>('ota_get_post_restart_notice')
}

export function isWindowsTauri(): boolean {
  if (!isTauri()) return false
  return typeof navigator !== 'undefined' && /Windows/i.test(navigator.userAgent)
}
