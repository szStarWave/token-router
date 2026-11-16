/** Tauri 桥的最小封装：supervisor / host 命令（admin 数据走 api/client.ts）。 */
import { invoke } from '@tauri-apps/api/core'

export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
}

export async function tauriInvoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  return invoke<T>(cmd, args)
}

export function invokeErrorMessage(err: unknown): string {
  if (typeof err === 'string') return err
  if (err && typeof err === 'object' && 'message' in err) return String((err as Error).message)
  return String(err)
}

// ---------------------------------------------------------------------------
// supervisor（服务生命周期；§6.2）
// ---------------------------------------------------------------------------

export interface SupervisorStatus {
  state: 'stopped' | 'starting' | 'ready' | 'restarting' | 'failed'
  listen: string | null
  data_dir: string
}

export interface SpawnOutcome {
  state: SupervisorStatus['state']
  listen: string | null
  message?: string
}

export const supervisorStatus = () =>
  tauriInvoke<SupervisorStatus>('supervisor_status')

export const supervisorStart = () =>
  tauriInvoke<SpawnOutcome>('supervisor_start')

export const supervisorRestart = () =>
  tauriInvoke<SpawnOutcome>('supervisor_restart')

export const supervisorStop = () =>
  tauriInvoke<SpawnOutcome>('supervisor_stop')

// ---------------------------------------------------------------------------
// host 命令（§6.4）
// ---------------------------------------------------------------------------

export interface AppInfo {
  version: string
  platform: string
  data_dir: string
}

export const hostAppInfo = () => tauriInvoke<AppInfo>('host_app_info')

export const hostOpenPath = (path?: string) =>
  tauriInvoke<void>('host_open_path', { path: path ?? null })

export interface HostUiState {
  theme?: string | null
  language?: string | null
  sidebar_collapsed?: boolean | null
  tour_done?: boolean | null
}

export const hostUiStateLoad = () => tauriInvoke<HostUiState>('host_ui_state_load')

export const hostUiStateSave = (uiState: HostUiState) =>
  tauriInvoke<void>('host_ui_state_save', { uiState })

// ---------------------------------------------------------------------------
// Agent onboarding（§8）：detect → plan → apply → verify
// ---------------------------------------------------------------------------

export interface DetectReport {
  agent: string
  target: string
  installed: boolean
  config_files: Array<{ path: string; exists: boolean; readable: boolean }>
  already_configured: boolean
}

export interface PlanEntry {
  path: string
  format: string
  action: string
  before_digest: string
  after_preview: string
}

export interface SetupPlan {
  agent: string
  entries: PlanEntry[]
}

export interface ApplyReport {
  agent: string
  written: Array<{ path: string; bytes: number; backup_path: string | null }>
  backups: string[]
}

export interface VerifyReport {
  agent: string
  ok: boolean
  messages: string[]
}

export interface WslReport {
  wsl_available: boolean
  distros: string[]
  default_distro: string | null
  error: string | null
}

export interface OnboardSetupArgs {
  agent: string
  wsl_distro?: string | null
  gateway_url?: string | null
  /** 凭据引用：env:NAME / file:path（明文不进 UI）。 */
  credential_ref: string
  model: string
}

export const onboardDetect = (args: { agent: string; wsl_distro?: string | null }) =>
  tauriInvoke<DetectReport>('onboard_detect', { args })

export const onboardPlan = (args: OnboardSetupArgs) =>
  tauriInvoke<SetupPlan>('onboard_plan', { args })

export const onboardApply = (args: OnboardSetupArgs) =>
  tauriInvoke<ApplyReport>('onboard_apply', { args })

export const onboardVerify = (args: { agent: string; wsl_distro?: string | null }) =>
  tauriInvoke<VerifyReport>('onboard_verify', { args })

export const onboardWslDetect = () =>
  tauriInvoke<WslReport>('onboard_wsl_detect')
