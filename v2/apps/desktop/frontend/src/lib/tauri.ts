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
  /**
   * true = 壳连接的是外部服务（不托管该进程）；未知（旧宿主未返回）时为 undefined。
   * 本地 start/stop/restart 动作只在明确 external === false 时可用
   * （统一入口：`canManageLocally`）。
   */
  external?: boolean
}

/**
 * 本地生命周期管理（start/stop/restart）是否可用。
 *
 * 只有「壳托管进程」（external === false）才允许；
 * external === true 是外部服务（动作须隐藏），undefined = 旧宿主未上报，
 * 一律视为未知 → 不提供本地动作。
 */
export function canManageLocally(status: SupervisorStatus | null | undefined): boolean {
  return isTauri() && status?.external === false
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

/** 本机可用于局域网访问的 IPv4（不含回环）；探测不到时返回空数组。 */
export const hostLanAddresses = () => tauriInvoke<string[]>('host_lan_addresses')

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
// 退出前询问（设置页）
// ---------------------------------------------------------------------------

export const appSetExitGuard = (enabled: boolean) =>
  tauriInvoke<void>('app_set_exit_guard', { enabled })

export const appForceClose = () => tauriInvoke<void>('app_force_close')

export const appCancelExit = () => tauriInvoke<void>('app_cancel_exit')

// ---------------------------------------------------------------------------
// 托盘（关闭到托盘 / 托盘退出）
// ---------------------------------------------------------------------------

export const appSetCloseToTray = (enabled: boolean) =>
  tauriInvoke<void>('app_set_close_to_tray', { enabled })

export const appQuit = () => tauriInvoke<void>('app_quit')

// ---------------------------------------------------------------------------
// Agent onboarding（§8）：detect → plan → apply → verify
// ---------------------------------------------------------------------------

export interface DetectReport {
  agent: string
  target: string
  installed: boolean
  config_files: Array<{ path: string; exists: boolean; readable: boolean }>
  already_configured: boolean
  /** 现有配置里读回的模型 id；无法解析/未配置为 null。 */
  configured_model: string | null
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
  /** 显式切换入口：覆盖客户端已有 model；Onboarding 默认 false。 */
  force_model?: boolean
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
