import { getCurrentFlowyServerBase } from './flowy/server'
import { FLOWY_HOSTS } from './flowy/config'
import { getAuthToken } from '../stores/authStore'
import { getAvailableModelList, type CloudModel } from './flowy/api'
import { CLOUD_BUDGET_MIN } from '../constants/defaults'
import type { ApiFetch } from '../stores/edgeStore'
import {
  getCloudStoreState,
  type CloudDisplayItem,
  type ManualCloudEntry,
  type SetupCloud,
} from '../stores/cloudStore'
import { useSetupStore } from '../stores/setupStore'
import { useAppStore } from '../stores/appStore'
import { apiFetch } from './gateway'
import type { UpstreamSetupView } from '../types/gateway'

export type { CloudDisplayItem, ManualCloudEntry, SetupCloud } from '../stores/cloudStore'

export const AUTO_MODEL_ID = 'auto'
export const AUTO_MODEL_DISPLAY_NAME = 'Auto'

/** Default max context for custom cloud models (1M). */
export const DEFAULT_CLOUD_CONTEXT_WINDOW = 1_000_000

const CLOUD_USER_CONFIGURED_KEY = 'tr-cloud-user-configured'
const CLOUD_MANUAL_ENTRIES_KEY = 'tr-cloud-manual-entries'

const uiChangeListeners = new Set<() => void>()

function loadManualEntriesFromStorage(): ManualCloudEntry[] {
  try {
    const raw = localStorage.getItem(CLOUD_MANUAL_ENTRIES_KEY)
    if (!raw) return []
    const parsed = JSON.parse(raw) as unknown
    if (!Array.isArray(parsed)) return []
    return parsed.filter(
      (entry): entry is ManualCloudEntry =>
        !!entry
        && typeof entry === 'object'
        && typeof (entry as ManualCloudEntry).id === 'string'
        && typeof (entry as ManualCloudEntry).name === 'string'
        && typeof (entry as ManualCloudEntry).base_url === 'string'
        && typeof (entry as ManualCloudEntry).model === 'string',
    )
  } catch {
    return []
  }
}

function persistManualEntries(entries: ManualCloudEntry[]): void {
  try {
    localStorage.setItem(CLOUD_MANUAL_ENTRIES_KEY, JSON.stringify(entries))
  } catch {
    // ignore quota / private mode
  }
}

function isCloudUserConfigured(): boolean {
  return localStorage.getItem(CLOUD_USER_CONFIGURED_KEY) === '1'
}

function markCloudUserConfigured(): void {
  localStorage.setItem(CLOUD_USER_CONFIGURED_KEY, '1')
}

function clearCloudUserConfigured(): void {
  localStorage.removeItem(CLOUD_USER_CONFIGURED_KEY)
}

function notifyUiChange(): void {
  for (const listener of uiChangeListeners) {
    listener()
  }
}

export function subscribeCloudUiChange(listener: () => void): () => void {
  uiChangeListeners.add(listener)
  return () => uiChangeListeners.delete(listener)
}

export function getCloudBaseUrl() {
  return getCurrentFlowyServerBase('/claw/v1')
}

function normalizeUrl(url: string | null | undefined): string {
  return (url || '').trim().replace(/\/+$/, '')
}

function normalizeModelId(id: string | null | undefined) {
  if (!id) return ''
  if (id === 'Auto') return AUTO_MODEL_ID
  return id
}

function entryKey(type: 'flowy' | 'manual', id: string): string {
  return `${type}:${id}`
}

function resolveCloudContextWindow(value: number | null | undefined): number {
  const n = Number(value)
  if (!Number.isFinite(n) || n <= 0) return DEFAULT_CLOUD_CONTEXT_WINDOW
  return Math.min(Math.max(Math.floor(n), 4096), 2_000_000)
}

function cloudModelSignature(baseUrl: string, modelId: string): string {
  return `${normalizeUrl(baseUrl)}|${(modelId || '').trim()}`
}

export function isFlowyCloudUrl(url: string | null | undefined): boolean {
  const normalized = normalizeUrl(url).toLowerCase()
  if (!normalized) return false
  const flowyBase = normalizeUrl(getCloudBaseUrl()).toLowerCase()
  if (normalized === flowyBase) return true
  const hosts = [
    FLOWY_HOSTS.productionDomesticHost,
    FLOWY_HOSTS.productionInternationalHost,
    FLOWY_HOSTS.testHost,
  ]
  try {
    const parsed = new URL(normalized.startsWith('http') ? normalized : `https://${normalized}`)
    return hosts.some((host) => parsed.hostname === host || parsed.hostname.endsWith(`.${host}`))
      && parsed.pathname.includes('/claw')
  } catch {
    return false
  }
}

export function withAutoModelOption(models: CloudModel[]) {
  const list = models.filter((m) => m.id !== 'Auto' && m.id !== AUTO_MODEL_ID)
  list.unshift({
    id: AUTO_MODEL_ID,
    name: AUTO_MODEL_DISPLAY_NAME,
    icon: '',
    context_window: DEFAULT_CLOUD_CONTEXT_WINDOW,
  })
  return list
}

export function getCachedCloudModels() {
  return getCloudStoreState().flowyModels
}

export function buildCloudDisplayItems(): CloudDisplayItem[] {
  const { flowyModels, manualEntries } = getCloudStoreState()
  const items: CloudDisplayItem[] = []
  const seen = new Set<string>()
  const flowyBase = getCloudBaseUrl()

  for (const model of flowyModels) {
    const modelId = normalizeModelId(model.id)
    const sig = cloudModelSignature(flowyBase, modelId)
    if (seen.has(sig)) continue
    seen.add(sig)
    items.push({
      key: entryKey('flowy', modelId),
      type: 'flowy',
      id: modelId,
      name: modelId === AUTO_MODEL_ID ? AUTO_MODEL_DISPLAY_NAME : (model.name || modelId),
      base_url: flowyBase,
      model: modelId,
      icon: model.icon,
      context_window: resolveCloudContextWindow(model.context_window),
    })
  }

  for (const entry of manualEntries) {
    const sig = cloudModelSignature(entry.base_url, entry.model)
    if (seen.has(sig)) continue
    if (isFlowyCloudUrl(entry.base_url)) continue
    seen.add(sig)
    items.push({
      key: entryKey('manual', entry.id),
      type: 'manual',
      id: entry.id,
      name: entry.name || entry.model,
      base_url: entry.base_url,
      model: entry.model,
      api_key: entry.api_key,
      context_window: resolveCloudContextWindow(entry.context_window),
    })
  }

  return items
}

export function getSelectedCloudItem(): CloudDisplayItem | null {
  const { selectedKey } = getCloudStoreState()
  if (!selectedKey) return null
  return buildCloudDisplayItems().find((item) => item.key === selectedKey) ?? null
}

function selectedCloudItemHasEndpoint(item: CloudDisplayItem): boolean {
  const url = item.base_url?.trim()
  const model = (item.type === 'manual' ? item.model : item.id)?.trim()
  return !!(url && model)
}

function cloudSelectionMatchesSetup(
  item: CloudDisplayItem,
  setupCloud: SetupCloud | null | undefined,
): boolean {
  const model = setupCloud?.model?.trim()
  const url = setupCloud?.base_url?.trim()
  if (!model || !url) return false
  const itemModel = item.type === 'manual' ? item.model : item.id
  return (
    normalizeUrl(item.base_url) === normalizeUrl(url)
    && (itemModel === model || item.id === model)
  )
}

function setupCloudMatchesDisplayItems(setupCloud: SetupCloud | null | undefined): boolean {
  const url = setupCloud?.base_url?.trim()
  const model = setupCloud?.model?.trim()
  if (!url || !model) return false
  return buildCloudDisplayItems().some((item) => cloudSelectionMatchesSetup(item, setupCloud))
}

export function isCloudModelUiConfigured(setupCloud: SetupCloud | null | undefined): boolean {
  const item = getSelectedCloudItem()
  if (item) return selectedCloudItemHasEndpoint(item)
  return setupCloudMatchesDisplayItems(setupCloud)
}

export function getCloudModelValue(): string {
  const item = getSelectedCloudItem()
  if (!item) return ''
  return item.type === 'manual' ? (item.model || '') : (item.id || '')
}

export function getCloudModelDisplayName(modelId?: string | null): string {
  const item = getSelectedCloudItem()
  if (item) {
    if (normalizeModelId(item.id) === AUTO_MODEL_ID) return AUTO_MODEL_DISPLAY_NAME
    return item.name || item.model || item.id || ''
  }

  const setup = useSetupStore.getState().setup?.cloud
  if (setupCloudMatchesDisplayItems(setup)) {
    const matched = buildCloudDisplayItems().find((entry) => cloudSelectionMatchesSetup(entry, setup))
    if (matched) {
      if (normalizeModelId(matched.id) === AUTO_MODEL_ID) return AUTO_MODEL_DISPLAY_NAME
      return matched.name || matched.model || matched.id || ''
    }
  }

  if (!modelId) return ''
  const id = normalizeModelId(modelId)
  if (id === AUTO_MODEL_ID) return AUTO_MODEL_DISPLAY_NAME
  const found = getCloudStoreState().flowyModels.find((m) => normalizeModelId(m.id) === id)
  if (found) {
    if (normalizeModelId(found.id) === AUTO_MODEL_ID) return AUTO_MODEL_DISPLAY_NAME
    return found.name || id
  }
  const manual = getCloudStoreState().manualEntries.find((e) => e.model === id)
  return manual?.name || id
}

export function resolveCloudModelLabel(setupCloud: SetupCloud | null | undefined): string {
  const item = getSelectedCloudItem()
  if (item) {
    if (normalizeModelId(item.id) === AUTO_MODEL_ID) return AUTO_MODEL_DISPLAY_NAME
    return item.name || item.model || item.id || ''
  }
  if (!setupCloudMatchesDisplayItems(setupCloud)) return ''
  const matched = buildCloudDisplayItems().find((entry) => cloudSelectionMatchesSetup(entry, setupCloud))
  if (matched && normalizeModelId(matched.id) === AUTO_MODEL_ID) return AUTO_MODEL_DISPLAY_NAME
  return matched?.name || matched?.model || setupCloud?.model?.trim() || ''
}

export function selectCloudModel(key: string | null): void {
  getCloudStoreState().setSelectedKey(key)
  if (key) markCloudUserConfigured()
  notifyUiChange()
}

function autoFlowyModelKey(): string {
  return entryKey('flowy', AUTO_MODEL_ID)
}

function fallbackToAutoFlowyModel(): boolean {
  const state = getCloudStoreState()
  const autoKey = autoFlowyModelKey()
  const items = buildCloudDisplayItems()
  if (!items.some((item) => item.key === autoKey)) {
    state.setSelectedKey(null)
    return false
  }
  state.setSelectedKey(autoKey)
  markCloudUserConfigured()
  return true
}

function ensureSelectedKey(preferredModel?: string | null, preferredUrl?: string | null): void {
  const state = getCloudStoreState()
  const items = buildCloudDisplayItems()
  if (!items.length) {
    state.setSelectedKey(null)
    return
  }

  const model = (preferredModel || '').trim()
  const url = normalizeUrl(preferredUrl)

  if (model && url) {
    const matches = items.filter(
      (item) => (item.model === model || item.id === model)
        && normalizeUrl(item.base_url) === url,
    )
    const flowy = matches.find((item) => item.type === 'flowy')
    if (flowy) {
      state.setSelectedKey(flowy.key)
      return
    }
    if (matches[0]) {
      state.setSelectedKey(matches[0].key)
      return
    }
    fallbackToAutoFlowyModel()
    return
  }

  if (model) {
    const match = items.find((item) => item.id === model || item.model === model)
    if (match) {
      state.setSelectedKey(match.key)
      return
    }
    fallbackToAutoFlowyModel()
    return
  }

  if (state.selectedKey && !items.some((item) => item.key === state.selectedKey)) {
    fallbackToAutoFlowyModel()
  }
}

/** Reconcile UI selection when the model list or setup no longer contains the current choice. */
export function reconcileCloudModelSelection(): boolean {
  const state = getCloudStoreState()
  const previousKey = state.selectedKey
  const items = buildCloudDisplayItems()

  if (!items.length) {
    if (previousKey !== null) state.setSelectedKey(null)
    return previousKey !== null
  }

  if (previousKey && items.some((item) => item.key === previousKey)) {
    return false
  }

  applyPendingSetupSelection()
  const afterPending = getCloudStoreState().selectedKey
  if (afterPending && buildCloudDisplayItems().some((item) => item.key === afterPending)) {
    return previousKey !== afterPending
  }

  const setup = useSetupStore.getState().setup?.cloud
  if (setup?.model?.trim() && setup.base_url?.trim()) {
    ensureSelectedKey(setup.model, setup.base_url)
    const afterSetup = getCloudStoreState().selectedKey
    if (afterSetup && buildCloudDisplayItems().some((item) => item.key === afterSetup)) {
      return previousKey !== afterSetup
    }
  }

  fallbackToAutoFlowyModel()
  return previousKey !== getCloudStoreState().selectedKey
}

function applyPendingSetupSelection(): void {
  const { pendingSetupSelection } = getCloudStoreState()
  if (!pendingSetupSelection || !isCloudUserConfigured()) return
  ensureSelectedKey(pendingSetupSelection.model, pendingSetupSelection.url)
}

function newManualId(): string {
  return `c-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`
}

export function upsertManualCloudEntry(entry: ManualCloudEntry): void {
  const state = getCloudStoreState()
  const normalized: ManualCloudEntry = {
    ...entry,
    fromSetupRestore: false,
    context_window: resolveCloudContextWindow(entry.context_window),
  }
  const manualEntries = [...state.manualEntries]
  const idx = manualEntries.findIndex((e) => e.id === normalized.id)
  if (idx >= 0) manualEntries[idx] = normalized
  else manualEntries.push(normalized)
  state.setManualEntries(manualEntries)
  persistManualEntries(manualEntries)
  markCloudUserConfigured()
  ensureSelectedKey(normalized.model, normalized.base_url)
  notifyUiChange()
}

export function deleteManualCloudEntry(id: string): void {
  const state = getCloudStoreState()
  const deleted = state.manualEntries.find((entry) => entry.id === id)
  if (!deleted) return

  const deletedKey = entryKey('manual', id)
  const wasSelected = state.selectedKey === deletedKey
  const setupCloud = useSetupStore.getState().setup?.cloud
  const deletedActiveSetup = setupCloud
    ? cloudSelectionMatchesSetup(
        {
          key: deletedKey,
          type: 'manual',
          id: deleted.id,
          name: deleted.name,
          base_url: deleted.base_url,
          model: deleted.model,
        },
        setupCloud,
      )
    : false

  const manualEntries = state.manualEntries.filter((entry) => entry.id !== id)
  state.setManualEntries(manualEntries)
  persistManualEntries(manualEntries)

  if (wasSelected || deletedActiveSetup) {
    fallbackToAutoFlowyModel()
  } else {
    const selectedKey = state.selectedKey
    if (!manualEntries.length && !selectedKey?.startsWith('flowy:')) {
      clearCloudUserConfigured()
    }
  }

  if (
    (deletedActiveSetup || !setupCloudMatchesDisplayItems(setupCloud))
    && !getSelectedCloudItem()
  ) {
    patchLocalSetupCloudCleared()
  }

  notifyUiChange()
}

function patchLocalSetupCloudCleared(): void {
  const setup = useSetupStore.getState().setup
  if (!setup?.cloud) return
  useSetupStore.getState().setSetup({
    ...setup,
    cloud: {
      ...setup.cloud,
      configured: false,
      base_url: '',
      model: null,
    },
  })
}

export function syncCloudFromSetup(cloud: SetupCloud | null | undefined): void {
  const state = getCloudStoreState()
  state.setPendingSetupSelection(null)
  if (!cloud?.base_url || !cloud?.model) {
    notifyUiChange()
    return
  }

  const url = cloud.base_url.trim()
  const model = cloud.model.trim()
  state.setPendingSetupSelection({ model, url })

  if (isFlowyCloudUrl(url)) {
    markCloudUserConfigured()
    ensureSelectedKey(model, url)
    notifyUiChange()
    return
  }

  let entry = state.manualEntries.find(
    (e) => normalizeUrl(e.base_url) === normalizeUrl(url) && e.model === model,
  )

  if (!entry) {
    entry = {
      id: newManualId(),
      name: model,
      base_url: url,
      model,
      api_key: cloud.api_key || undefined,
      context_window: DEFAULT_CLOUD_CONTEXT_WINDOW,
      fromSetupRestore: true,
    }
    const manualEntries = [...state.manualEntries, entry]
    state.setManualEntries(manualEntries)
    persistManualEntries(manualEntries)
  } else if (cloud.api_key) {
    const manualEntries = state.manualEntries.map((e) =>
      e.id === entry!.id ? { ...e, api_key: cloud.api_key || undefined } : e,
    )
    state.setManualEntries(manualEntries)
    persistManualEntries(manualEntries)
  }

  markCloudUserConfigured()
  ensureSelectedKey(model, url)
  notifyUiChange()
}

export function initCloudUpstream(): void {
  const state = getCloudStoreState()
  state.setManualEntries(loadManualEntriesFromStorage())
  syncCloudFromSetup(useSetupStore.getState().setup?.cloud)
  applyPendingSetupSelection()
  reconcileCloudModelSelection()
}

export async function fetchCloudModels() {
  const token = getAuthToken()
  if (!token) throw new Error('未登录')
  const models = withAutoModelOption(await getAvailableModelList(token))
  getCloudStoreState().setFlowyModels(models)
  const selectionChanged = reconcileCloudModelSelection()
  notifyUiChange()
  if (selectionChanged && useAppStore.getState().connected) {
    const cloud = useSetupStore.getState().setup?.cloud
    const tokenBudget =
      cloud?.token_quota_enabled && cloud.token_budget != null
        ? cloud.token_budget
        : undefined
    void persistCloudSelection(tokenBudget)
  }
  return models
}

export function normalizeCloudTokenBudget(tokenBudget: unknown) {
  if (tokenBudget === undefined || tokenBudget === null || tokenBudget === '') return 0
  const n = Number(tokenBudget)
  if (!Number.isFinite(n) || n <= 0) return 0
  return Math.floor(n)
}

export interface CloudSavePayload {
  base_url: string
  model: string
  api_key?: string
  token_budget?: number
}

export function buildCloudSavePayload(
  modelId?: string | null,
  tokenBudget?: number | null,
): CloudSavePayload | null {
  const preferred = modelId ?? getCloudModelValue()
  const items = buildCloudDisplayItems()
  const item =
    items.find((i) => i.id === preferred || i.model === preferred)
    ?? getSelectedCloudItem()
    ?? items.find((i) => i.type === 'flowy' && i.id === AUTO_MODEL_ID)
    ?? null
  if (!item) return null

  if (item.type === 'flowy') {
    const token = getAuthToken()
    if (!token) throw new Error('未登录')
    const payload: CloudSavePayload = {
      base_url: getCloudBaseUrl(),
      model: normalizeModelId(item.id) || AUTO_MODEL_ID,
      api_key: token,
    }
    if (tokenBudget != null) payload.token_budget = tokenBudget
    return payload
  }

  const payload: CloudSavePayload = {
    base_url: normalizeUrl(item.base_url),
    model: item.model || item.id,
  }
  if (item.api_key) payload.api_key = item.api_key
  if (tokenBudget != null) payload.token_budget = tokenBudget
  return payload
}

export async function persistCloudSelection(
  tokenBudget?: number | null,
  saveSetup?: (body: import('../types/gateway').UpstreamSetupUpdate) => void,
): Promise<void> {
  let cloud: CloudSavePayload | null
  try {
    cloud = buildCloudSavePayload(undefined, tokenBudget)
  } catch {
    cloud = null
  }

  if (!cloud?.base_url || !cloud?.model) {
    if (saveSetup && useAppStore.getState().connected) {
      saveSetup({ cloud: { clear: true } })
    }
    return
  }

  if (saveSetup && useAppStore.getState().connected) {
    saveSetup({ cloud })
    return
  }

  if (useAppStore.getState().connected) {
    try {
      const res = await apiFetch<{ upstream?: UpstreamSetupView }>('/v1/admin/setup', {
        method: 'POST',
        body: JSON.stringify({ cloud }),
      })
      if (res?.upstream) useSetupStore.getState().setSetup(res.upstream)
    } catch (error) {
      console.warn('[cloud-upstream] persist cloud selection failed', error)
    }
  }
}

export function sliderFromCloudBudget(budget: number) {
  const min = CLOUD_BUDGET_MIN
  const max = 1_000_000_000_000
  if (budget <= 0) return 0
  const clamped = Math.max(min, Math.min(max, budget))
  const logMin = Math.log10(min)
  const logMax = Math.log10(max)
  const logVal = Math.log10(clamped)
  return Math.round(((logVal - logMin) / (logMax - logMin)) * 1000)
}

export function budgetFromSlider(sliderVal: number) {
  const min = CLOUD_BUDGET_MIN
  const max = 1_000_000_000_000
  if (sliderVal <= 0) return 0
  const logMin = Math.log10(min)
  const logMax = Math.log10(max)
  const logVal = logMin + (sliderVal / 1000) * (logMax - logMin)
  return Math.round(Math.pow(10, logVal))
}

function isCustomCloudConfigured(setupCloud?: SetupCloud | null): boolean {
  const url = setupCloud?.base_url?.trim()
  const model = setupCloud?.model?.trim()
  if (!url || !model) return false
  return !isFlowyCloudUrl(url)
}

export async function ensureCloudUpstreamConfigured(
  fetch: ApiFetch | null,
  options: {
    currentModel?: string | null
    currentUrl?: string | null
    tokenBudget?: number
    silent?: boolean
  } = {},
) {
  const setupCloud: SetupCloud = {
    base_url: options.currentUrl,
    model: options.currentModel,
  }

  let models: CloudModel[] = []
  const token = getAuthToken()
  if (token) {
    try {
      models = await fetchCloudModels()
    } catch (e) {
      if (!options.silent) throw e
      console.warn('[cloud-upstream] fetch models', e)
    }
  }

  if (isCustomCloudConfigured(setupCloud) || getSelectedCloudItem()?.type === 'manual') {
    syncCloudFromSetup(setupCloud)
    return { models, posted: false }
  }

  if (!fetch) return { models, posted: false }

  if (!token) {
    syncCloudFromSetup(setupCloud)
    return { models, posted: false }
  }

  reconcileCloudModelSelection()
  const modelId =
    getCloudModelValue()
    || normalizeModelId(options.currentModel)
    || AUTO_MODEL_ID
  const tokenBudget =
    options.tokenBudget === undefined
      ? undefined
      : normalizeCloudTokenBudget(options.tokenBudget)

  let cloud: CloudSavePayload | null
  try {
    cloud = buildCloudSavePayload(modelId, tokenBudget)
  } catch (e) {
    if (!options.silent) throw e
    console.warn('[cloud-upstream]', e)
    return { models, posted: false, error: e }
  }

  if (!cloud) {
    syncCloudFromSetup(setupCloud)
    return { models, posted: false }
  }

  try {
    const res = (await fetch('/v1/admin/setup', {
      method: 'POST',
      body: JSON.stringify({ cloud }),
    })) as { upstream?: unknown }
    syncCloudFromSetup({
      base_url: cloud.base_url,
      model: cloud.model,
    })
    return { models, posted: true, response: res }
  } catch (e) {
    if (!options.silent) throw e
    console.warn('[cloud-upstream]', e)
    return { models, posted: false, error: e }
  }
}
