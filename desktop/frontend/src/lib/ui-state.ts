import { tauriInvoke } from './tauri'
import { isTauri } from './tauri'
export { isTauri }

export interface UiState {
  schemaVersion: number
  theme: string
  locale: string
  hasSeenOnboarding: boolean
  edgeUserConfigured: boolean
  edgeManualEntries: unknown[]
  cloudUserConfigured: boolean
  cloudManualEntries: unknown[]
}

const DEFAULT_STATE: UiState = {
  schemaVersion: 1,
  theme: 'system',
  locale: 'zh',
  hasSeenOnboarding: false,
  edgeUserConfigured: false,
  edgeManualEntries: [],
  cloudUserConfigured: false,
  cloudManualEntries: [],
}

/** Legacy localStorage keys that must move to ~/.token-router-desktop/ui-state.json */
const LEGACY_UI_KEYS = [
  'tr-theme',
  'tr-locale',
  'flowyRouterHasSeenOnboarding',
  'tr-edge-user-configured',
  'tr-edge-manual-entries',
  'tr-cloud-user-configured',
  'tr-cloud-manual-entries',
  'tr-gateway-auth-key',
] as const

let cache: UiState | null = null

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v)
}

function isValidString(v: unknown): v is string {
  return typeof v === 'string'
}

function hasLegacyUiKeys(): boolean {
  try {
    return LEGACY_UI_KEYS.some((key) => localStorage.getItem(key) !== null)
  } catch {
    return false
  }
}

function clearLegacyUiKeys(): void {
  try {
    for (const key of LEGACY_UI_KEYS) {
      localStorage.removeItem(key)
    }
  } catch {
    /* ignore */
  }
}

function parseManualEntries(raw: string | null): Record<string, unknown>[] {
  if (raw === null) return []
  try {
    const parsed = JSON.parse(raw) as unknown
    if (!Array.isArray(parsed)) return []
    return parsed.filter(
      (e): e is Record<string, unknown> =>
        isPlainObject(e)
        && isValidString(e.id)
        && isValidString(e.name)
        && isValidString(e.base_url)
        && isValidString(e.model),
    )
  } catch {
    return []
  }
}

function mergeManualEntries(
  disk: unknown[],
  local: Record<string, unknown>[],
): Record<string, unknown>[] {
  const diskEntries = (disk as Record<string, unknown>[]).filter(isPlainObject)
  const signatures = new Set(
    diskEntries.map((e) => `${String(e.base_url ?? '')}|${String(e.model ?? '')}`),
  )
  const extras = local.filter(
    (e) => !signatures.has(`${String(e.base_url)}|${String(e.model)}`),
  )
  return [...diskEntries, ...extras]
}

/** Merge legacy localStorage into disk state. Does not mutate localStorage. */
export function migrateFromLocalStorage(disk: UiState): { state: UiState; didMigrate: boolean } {
  if (!hasLegacyUiKeys()) {
    return { state: disk, didMigrate: false }
  }

  const merged: UiState = { ...disk, schemaVersion: disk.schemaVersion || 1 }
  let didMigrate = false

  try {
    const theme = localStorage.getItem('tr-theme')
    if (theme !== null) {
      didMigrate = true
      if (
        (theme === 'system' || theme === 'light' || theme === 'dark')
        && disk.theme === 'system'
        && theme !== 'system'
      ) {
        merged.theme = theme
      }
    }

    const locale = localStorage.getItem('tr-locale')
    if (locale !== null) {
      didMigrate = true
      if (
        (locale === 'zh' || locale === 'en')
        && disk.locale === 'zh'
        && locale !== 'zh'
      ) {
        merged.locale = locale
      }
    }

    const onboarding = localStorage.getItem('flowyRouterHasSeenOnboarding')
    if (onboarding !== null) {
      didMigrate = true
      merged.hasSeenOnboarding = merged.hasSeenOnboarding || onboarding === 'true'
    }

    const edgeConfigured = localStorage.getItem('tr-edge-user-configured')
    if (edgeConfigured !== null) {
      didMigrate = true
      merged.edgeUserConfigured = merged.edgeUserConfigured || edgeConfigured === '1'
    }

    const edgeRaw = localStorage.getItem('tr-edge-manual-entries')
    if (edgeRaw !== null) {
      didMigrate = true
      merged.edgeManualEntries = mergeManualEntries(
        merged.edgeManualEntries,
        parseManualEntries(edgeRaw),
      )
    }

    const cloudConfigured = localStorage.getItem('tr-cloud-user-configured')
    if (cloudConfigured !== null) {
      didMigrate = true
      merged.cloudUserConfigured = merged.cloudUserConfigured || cloudConfigured === '1'
    }

    const cloudRaw = localStorage.getItem('tr-cloud-manual-entries')
    if (cloudRaw !== null) {
      didMigrate = true
      merged.cloudManualEntries = mergeManualEntries(
        merged.cloudManualEntries,
        parseManualEntries(cloudRaw),
      )
    }

    if (localStorage.getItem('tr-gateway-auth-key') !== null) {
      didMigrate = true
    }
  } catch {
    /* private mode / quota — treat as no migration */
  }

  return { state: merged, didMigrate }
}

export function isUiStateLoaded(): boolean {
  return cache !== null
}

export async function loadUiState(): Promise<void> {
  if (!isTauri()) {
    cache = { ...DEFAULT_STATE }
    return
  }

  let disk: UiState = { ...DEFAULT_STATE }
  try {
    disk = await tauriInvoke<UiState>('ui_state_load')
  } catch (e) {
    console.warn('[ui-state] load failed, using defaults', e)
  }

  const { state: migrated, didMigrate } = migrateFromLocalStorage(disk)
  cache = migrated

  if (didMigrate) {
    try {
      await tauriInvoke('ui_state_save', { state: migrated })
      clearLegacyUiKeys()
    } catch (e) {
      console.warn('[ui-state] migrate save failed; will retry next launch', e)
    }
  }
}

export function getUiState(): UiState {
  if (!cache) {
    cache = { ...DEFAULT_STATE }
  }
  return cache
}

async function persistUiState(): Promise<void> {
  if (!isTauri() || !cache) return
  try {
    await tauriInvoke('ui_state_save', { state: cache })
  } catch (e) {
    console.warn('[ui-state] persist failed', e)
  }
}

export function setUiTheme(value: string): void {
  getUiState().theme = value
  void persistUiState()
}

export function setUiLocale(value: string): void {
  getUiState().locale = value
  void persistUiState()
}

export function setUiHasSeenOnboarding(value: boolean): void {
  getUiState().hasSeenOnboarding = value
  void persistUiState()
}

export function setUiEdgeUserConfigured(value: boolean): void {
  getUiState().edgeUserConfigured = value
  void persistUiState()
}

export function setUiEdgeManualEntries(entries: unknown[]): void {
  getUiState().edgeManualEntries = entries
  void persistUiState()
}

export function setUiCloudUserConfigured(value: boolean): void {
  getUiState().cloudUserConfigured = value
  void persistUiState()
}

export function setUiCloudManualEntries(entries: unknown[]): void {
  getUiState().cloudManualEntries = entries
  void persistUiState()
}
