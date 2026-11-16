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
