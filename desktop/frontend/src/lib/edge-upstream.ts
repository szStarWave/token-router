import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import {
  getEdgeStoreState,
  type ApiFetch,
  type EdgeDisplayItem,
  type HerdsmanModel,
  type HerdsmanStatusSnapshot,
  type ManualEdgeEntry,
  type SetupEdge,
} from '../stores/edgeStore'
import { useAppStore } from '../stores/appStore'
import { useSetupStore } from '../stores/setupStore'
import { apiFetch } from './gateway'
import { isTauri } from './tauri'
import type { UpstreamSetupView } from '../types/gateway'

export type {
  ApiFetch,
  EdgeDisplayItem,
  HerdsmanModel,
  HerdsmanStatusSnapshot,
  ManualEdgeEntry,
  SetupEdge,
  SetupEdgeSelection,
} from '../stores/edgeStore'

export interface EdgeSavePayload {
  edge: {
    base_url: string
    model: string | null
    api_key?: string
  }
  gateway?: {
    ctx_edge_max_tokens: number
  }
}

export interface EdgeReconcileResult {
  cleared: boolean
  deferred?: boolean
  response?: unknown
  error?: unknown
}

export interface EnsureEdgeUpstreamOptions {
  silent?: boolean
  currentModel?: string
  currentUrl?: string
}

export interface EnsureEdgeUpstreamResult {
  models: HerdsmanModel[]
  posted: boolean
  response?: unknown
  error?: unknown
}

export type EdgeModelChangeCallback = (value: string, item: EdgeDisplayItem | null) => void
export type EdgeUiChangeCallback = () => void
export type EdgeReconcileCompleteCallback = (result: EdgeReconcileResult) => void

const LOOPBACK_HOSTS = new Set(['localhost', '127.0.0.1', '0.0.0.0', '::1'])
export const HERDSMAN_INSTALL_URL = 'https://flowyaipc.cn/#ai-engine'
const EDGE_USER_CONFIGURED_KEY = 'tr-edge-user-configured'
const EDGE_MANUAL_ENTRIES_KEY = 'tr-edge-manual-entries'
const HERDSMAN_CONNECTION_POLL_MS = 12_000

function loadManualEntriesFromStorage(): ManualEdgeEntry[] {
  try {
    const raw = localStorage.getItem(EDGE_MANUAL_ENTRIES_KEY)
    if (!raw) return []
    const parsed = JSON.parse(raw) as unknown
    if (!Array.isArray(parsed)) return []
    return parsed.filter(
      (entry): entry is ManualEdgeEntry =>
        !!entry
        && typeof entry === 'object'
        && typeof (entry as ManualEdgeEntry).id === 'string'
        && typeof (entry as ManualEdgeEntry).name === 'string'
        && typeof (entry as ManualEdgeEntry).base_url === 'string'
        && typeof (entry as ManualEdgeEntry).model === 'string',
    )
  } catch {
    return []
  }
}

function persistManualEntries(entries: ManualEdgeEntry[]): void {
  try {
    localStorage.setItem(EDGE_MANUAL_ENTRIES_KEY, JSON.stringify(entries))
  } catch {
    // ignore quota / private mode
  }
}

let listenersInstalled = false
let unlistenFns: UnlistenFn[] = []
let connectionPollInterval: ReturnType<typeof setInterval> | null = null

const modelChangeListeners = new Set<EdgeModelChangeCallback>()
const uiChangeListeners = new Set<EdgeUiChangeCallback>()
const reconcileCompleteListeners = new Set<EdgeReconcileCompleteCallback>()

function isEdgeUserConfigured(): boolean {
  return localStorage.getItem(EDGE_USER_CONFIGURED_KEY) === '1'
}

function markEdgeUserConfigured(): void {
  localStorage.setItem(EDGE_USER_CONFIGURED_KEY, '1')
}

function clearEdgeUserConfigured(): void {
  localStorage.removeItem(EDGE_USER_CONFIGURED_KEY)
}

function updateHerdsmanConnectionPoll(): void {
  const state = getEdgeStoreState()
  const shouldPoll = isTauri() && state.herdsmanInstalled && !state.herdsmanConnected
  if (shouldPoll && !connectionPollInterval) {
    connectionPollInterval = setInterval(() => {
      void refreshHerdsmanStatus()
    }, HERDSMAN_CONNECTION_POLL_MS)
  } else if (!shouldPoll && connectionPollInterval) {
    clearInterval(connectionPollInterval)
    connectionPollInterval = null
  }
}

function stopHerdsmanConnectionPoll(): void {
  if (connectionPollInterval) {
    clearInterval(connectionPollInterval)
    connectionPollInterval = null
  }
}

function notifyModelChange(): void {
  const value = getEdgeModelValue()
  const item = getSelectedItem()
  for (const listener of modelChangeListeners) {
    listener(value, item)
  }
}

function notifyUiChange(): void {
  for (const listener of uiChangeListeners) {
    listener()
  }
}

function notifyReconcileComplete(result: EdgeReconcileResult): void {
  for (const listener of reconcileCompleteListeners) {
    listener(result)
  }
}

export function subscribeEdgeModelChange(callback: EdgeModelChangeCallback): () => void {
  modelChangeListeners.add(callback)
  return () => {
    modelChangeListeners.delete(callback)
  }
}

export function subscribeEdgeUpstreamUiChange(callback: EdgeUiChangeCallback): () => void {
  uiChangeListeners.add(callback)
  return () => {
    uiChangeListeners.delete(callback)
  }
}

export function subscribeEdgeReconcileComplete(callback: EdgeReconcileCompleteCallback): () => void {
  reconcileCompleteListeners.add(callback)
  return () => {
    reconcileCompleteListeners.delete(callback)
  }
}

function maybeMigrateLegacyEdgeConfirmation(edge: SetupEdge | null | undefined): void {
  const state = getEdgeStoreState()
  if (isEdgeUserConfigured() || !edge?.base_url || !edge?.model) return

  const model = edge.model.trim()
  const url = edge.base_url.trim()
  if (isAllowedHerdsmanEndpoint(url)) {
    if (state.herdsmanConnected && findHerdsmanItem(model, url)) {
      markEdgeUserConfigured()
    }
    return
  }
  markEdgeUserConfigured()
}

export function isAllowedHerdsmanEndpoint(endpoint: string | null | undefined): boolean {
  if (!endpoint) return false
  try {
    const url = new URL(endpoint)
    if (url.protocol !== 'http:' && url.protocol !== 'https:') return false
    return LOOPBACK_HOSTS.has(url.hostname)
  } catch {
    return false
  }
}

/** Map local Herdsman bind/advertised hosts to loopback for local HTTP clients. */
export function normalizeHerdsmanEndpoint(endpoint: string): string {
  const raw = (endpoint || '').trim()
  if (!raw) return endpoint

  let href = raw
  if (!/^[a-z][a-z0-9+.-]*:\/\//i.test(href)) {
    href = `http://${href}`
  }

  try {
    const url = new URL(href)
    if (shouldMapHerdsmanHostToLoopback(url.hostname)) {
      url.hostname = '127.0.0.1'
      return url.toString().replace(/\/+$/, '')
    }
    return href.replace(/\/+$/, '')
  } catch {
    return endpoint
  }
}

function shouldMapHerdsmanHostToLoopback(host: string): boolean {
  if (host === '0.0.0.0' || host === '::') return true
  if (host === 'localhost' || host === '127.0.0.1' || host === '::1') return false
  if (/^10\./.test(host)) return true
  if (/^192\.168\./.test(host)) return true
  if (/^172\.(1[6-9]|2\d|3[01])\./.test(host)) return true
  if (/^169\.254\./.test(host)) return true
  return false
}

function isHerdsmanEdgeSetup(edge: SetupEdge | null | undefined): boolean {
  const url = edge?.base_url?.trim()
  const model = edge?.model?.trim()
  if (!url || !model) return false
  return isAllowedHerdsmanEndpoint(url)
}

function shouldClearHerdsmanEdgeOnDisconnect(): boolean {
  const state = getEdgeStoreState()
  if (state.selectedKey?.startsWith('herdsman:')) return true

  const setupEdge = useSetupStore.getState().setup?.edge
  if (isHerdsmanEdgeSetup(setupEdge)) return true

  const pending = state.pendingSetupSelection
  if (pending && isAllowedHerdsmanEndpoint(pending.url) && isEdgeUserConfigured()) {
    return true
  }

  return false
}

function patchLocalSetupEdgeCleared(): void {
  const setup = useSetupStore.getState().setup
  if (!setup) return
  useSetupStore.getState().setSetup({
    ...setup,
    edge: {
      ...(setup.edge ?? { base_url: '' }),
      configured: false,
      base_url: '',
      model: null,
    },
  })
  const status = useAppStore.getState().status
  if (status) {
    useAppStore.getState().setStatus({ ...status, edge_configured: false })
  }
}

async function clearEdgeUpstreamConfiguration(clearUserFlag?: boolean): Promise<EdgeReconcileResult> {
  const state = getEdgeStoreState()
  const hadEdgeOnServer = !!useSetupStore.getState().setup?.edge?.base_url?.trim()
  state.setPendingSetupSelection(null)
  state.setSelectedKey(null)
  if (clearUserFlag ?? (!state.manualEntries.length && !state.herdsmanConnected)) {
    clearEdgeUserConfigured()
  }
  patchLocalSetupEdgeCleared()
  notifyModelChange()
  notifyUiChange()

  const connected = useAppStore.getState().connected
  if (!connected || !hadEdgeOnServer) {
    return { cleared: true }
  }

  try {
    const res = await apiFetch<{ upstream?: UpstreamSetupView }>('/v1/admin/setup', {
      method: 'POST',
      body: JSON.stringify({ edge: { clear: true } }),
    })
    if (res?.upstream) {
      useSetupStore.getState().setSetup(res.upstream)
    }
    const status = useAppStore.getState().status
    if (status) {
      useAppStore.getState().setStatus({
        ...status,
        edge_configured: !!res?.upstream?.edge?.configured,
      })
    }
    notifyUiChange()
    notifyReconcileComplete({ cleared: true, response: res })
    return { cleared: true, response: res }
  } catch (error) {
    console.warn('[edge-upstream] clear edge failed', error)
    notifyUiChange()
    return { cleared: false, error }
  }
}

async function clearHerdsmanEdgeConfiguration(): Promise<EdgeReconcileResult> {
  return clearEdgeUpstreamConfiguration(true)
}

function shouldClearStaleHerdsmanEdge(): boolean {
  const state = getEdgeStoreState()
  if (!state.herdsmanConnected) return false

  const setupEdge = useSetupStore.getState().setup?.edge

  if (state.selectedKey?.startsWith('herdsman:') && !getSelectedItem()) {
    return true
  }

  if (isHerdsmanEdgeSetup(setupEdge) && !setupEdgeMatchesDisplayItems(setupEdge)) {
    return true
  }

  const pending = state.pendingSetupSelection
  if (pending && isAllowedHerdsmanEndpoint(pending.url) && isEdgeUserConfigured()) {
    if (!findHerdsmanItem(pending.model, pending.url)) {
      return true
    }
  }

  return false
}

function syncHerdsmanEdgeAvailability(): void {
  if (!shouldClearStaleHerdsmanEdge()) return
  void clearHerdsmanEdgeConfiguration()
}

function handleHerdsmanDisconnected(): void {
  if (!shouldClearHerdsmanEdgeOnDisconnect()) return
  void clearHerdsmanEdgeConfiguration()
}

export function isHerdsmanConnected(): boolean {
  return getEdgeStoreState().herdsmanConnected
}

export function getCachedEdgeModels(): HerdsmanModel[] {
  return getEdgeStoreState().cachedModels
}

function entryKey(type: 'herdsman' | 'manual', id: string): string {
  return `${type}:${id}`
}

function normalizeUrl(url: string | null | undefined): string {
  return (url || '').trim().replace(/\/+$/, '')
}

function filterHerdsmanModels(models: unknown): HerdsmanModel[] {
  if (!Array.isArray(models)) return []
  return models
    .filter(
      (model): model is HerdsmanModel =>
        !!model
        && typeof model === 'object'
        && typeof (model as HerdsmanModel).id === 'string',
    )
    .map((model) => ({
      ...model,
      endpoint: normalizeHerdsmanEndpoint(model.endpoint),
    }))
    .filter((model) => isAllowedHerdsmanEndpoint(model.endpoint))
}

function herdsmanModelSignature(baseUrl: string, modelId: string): string {
  return `${normalizeUrl(baseUrl)}|${(modelId || '').trim()}`
}

function findHerdsmanItem(modelId: string, baseUrl: string): HerdsmanModel | undefined {
  const model = (modelId || '').trim()
  const url = normalizeUrl(baseUrl)
  return getEdgeStoreState().cachedModels.find(
    (m) => m.id === model && normalizeUrl(m.endpoint) === url,
  )
}

function isDuplicateHerdsmanManualEntry(entry: ManualEdgeEntry | null | undefined): boolean {
  if (!entry) return false
  const sig = herdsmanModelSignature(entry.base_url, entry.model)
  if (!isAllowedHerdsmanEndpoint(entry.base_url)) return false
  return getEdgeStoreState().cachedModels.some(
    (m) => herdsmanModelSignature(m.endpoint, m.id) === sig,
  )
}

function pruneHerdsmanManualDuplicates(): void {
  const state = getEdgeStoreState()
  const manualEntries = state.manualEntries.filter((entry) => !isDuplicateHerdsmanManualEntry(entry))
  let selectedKey = state.selectedKey
  if (selectedKey?.startsWith('manual:')) {
    const id = selectedKey.slice('manual:'.length)
    if (!manualEntries.some((entry) => entry.id === id)) {
      selectedKey = null
    }
  }
  state.setManualEntries(manualEntries)
  persistManualEntries(manualEntries)
  if (selectedKey !== state.selectedKey) {
    state.setSelectedKey(selectedKey)
  }
}

export function buildDisplayItems(): EdgeDisplayItem[] {
  const { cachedModels, herdsmanConnected, manualEntries } = getEdgeStoreState()
  const items: EdgeDisplayItem[] = []
  const seen = new Set<string>()

  if (herdsmanConnected) {
    for (const model of cachedModels) {
      const key = entryKey('herdsman', model.id)
      const sig = herdsmanModelSignature(model.endpoint, model.id)
      seen.add(sig)
      items.push({
        key,
        type: 'herdsman',
        id: model.id,
        name: model.name || model.id,
        base_url: model.endpoint,
        context_window: model.context_window,
      })
    }
  }

  for (const entry of manualEntries) {
    const sig = herdsmanModelSignature(entry.base_url, entry.model)
    if (seen.has(sig)) continue
    if (isDuplicateHerdsmanManualEntry(entry)) continue
    seen.add(sig)
    items.push({
      key: entryKey('manual', entry.id),
      type: 'manual',
      id: entry.id,
      name: entry.name || entry.model,
      base_url: entry.base_url,
      model: entry.model,
      api_key: entry.api_key,
      context_window: entry.context_window,
    })
  }

  return items
}

export function getSelectedItem(): EdgeDisplayItem | null {
  const { selectedKey } = getEdgeStoreState()
  if (!selectedKey) return null
  return buildDisplayItems().find((item) => item.key === selectedKey) ?? null
}

function applyPendingSetupSelection(): void {
  const { pendingSetupSelection } = getEdgeStoreState()
  if (!pendingSetupSelection || !isEdgeUserConfigured()) return
  ensureSelectedKey(pendingSetupSelection.model, pendingSetupSelection.url)
}

function selectedItemHasEndpoint(item: EdgeDisplayItem): boolean {
  const url = item.base_url?.trim()
  const model = (item.type === 'manual' ? item.model : item.id)?.trim()
  return !!(url && model)
}

function edgeSelectionMatchesSetup(
  item: EdgeDisplayItem,
  setupEdge: SetupEdge | null | undefined,
): boolean {
  const model = setupEdge?.model?.trim()
  const url = setupEdge?.base_url?.trim()
  if (!model || !url) return false
  const itemModel = item.type === 'manual' ? item.model : item.id
  return (
    normalizeUrl(item.base_url) === normalizeUrl(url)
    && (itemModel === model || item.id === model)
  )
}

function setupEdgeMatchesDisplayItems(setupEdge: SetupEdge | null | undefined): boolean {
  const url = setupEdge?.base_url?.trim()
  const model = setupEdge?.model?.trim()
  if (!url || !model) return false
  if (isAllowedHerdsmanEndpoint(url) && !getEdgeStoreState().herdsmanConnected) return false
  return buildDisplayItems().some((item) => edgeSelectionMatchesSetup(item, setupEdge))
}

export function isEdgeModelUiConfigured(setupEdge: SetupEdge | null | undefined): boolean {
  const item = getSelectedItem()
  if (item) {
    if (item.type === 'herdsman' && !getEdgeStoreState().herdsmanConnected) return false
    return selectedItemHasEndpoint(item)
  }

  return setupEdgeMatchesDisplayItems(setupEdge)
}

/** Whether edge upstream should show as configured in sidebar / upstream pages. */
export function isEdgeUpstreamConfigured(
  setupEdge: (SetupEdge & { configured?: boolean }) | null | undefined,
): boolean {
  return isEdgeModelUiConfigured(setupEdge)
}

export function getEdgeModelValue(): string {
  const item = getSelectedItem()
  if (!item) return ''
  return item.type === 'manual' ? (item.model || '') : (item.id || '')
}

export function getEdgeModelDisplayName(setupEdge?: SetupEdge | null): string {
  const item = getSelectedItem()
  if (item) return item.name || item.model || item.id || ''
  const edge = setupEdge ?? useSetupStore.getState().setup?.edge
  if (!setupEdgeMatchesDisplayItems(edge)) return ''
  const matched = buildDisplayItems().find((entry) => edgeSelectionMatchesSetup(entry, edge))
  return matched?.name || matched?.model || edge?.model?.trim() || ''
}

export function resolveEdgeModelLabel(setupEdge: SetupEdge | null | undefined): string {
  const item = getSelectedItem()
  if (item) return item.name || item.model || item.id || ''
  if (!setupEdgeMatchesDisplayItems(setupEdge)) return ''
  const matched = buildDisplayItems().find((entry) => edgeSelectionMatchesSetup(entry, setupEdge))
  return matched?.name || matched?.model || setupEdge?.model?.trim() || ''
}

export function resolveEdgeModelSourceType(
  setupEdge: SetupEdge | null | undefined,
): 'herdsman' | 'manual' | null {
  if (!isEdgeModelUiConfigured(setupEdge)) return null
  const item = getSelectedItem()
  if (item) return item.type
  if (setupEdge?.model?.trim() && isAllowedHerdsmanEndpoint(setupEdge?.base_url)) {
    return 'herdsman'
  }
  return setupEdge?.model?.trim() ? 'manual' : null
}

export function getEdgeModelSourceType(): 'herdsman' | 'manual' | null {
  return getSelectedItem()?.type ?? null
}

export function selectEdgeModel(key: string | null): void {
  getEdgeStoreState().setSelectedKey(key)
  if (key) markEdgeUserConfigured()
  notifyModelChange()
  notifyUiChange()
}

/** After a custom edge model is removed, prefer Herdsman, then another custom entry. */
function fallbackEdgeModelAfterManualDelete(): boolean {
  const state = getEdgeStoreState()
  const items = buildDisplayItems()

  const firstHerdsman = items.find((item) => item.type === 'herdsman')
  if (firstHerdsman) {
    state.setSelectedKey(firstHerdsman.key)
    markEdgeUserConfigured()
    return true
  }

  const firstManual = items.find((item) => item.type === 'manual')
  if (firstManual) {
    state.setSelectedKey(firstManual.key)
    markEdgeUserConfigured()
    return true
  }

  state.setSelectedKey(null)
  clearEdgeUserConfigured()
  return false
}

function ensureSelectedKey(preferredModel?: string | null, preferredUrl?: string | null): void {
  const state = getEdgeStoreState()
  const items = buildDisplayItems()
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
    const herdsman = matches.find((item) => item.type === 'herdsman')
    if (herdsman) {
      state.setSelectedKey(herdsman.key)
      return
    }
    if (matches[0]) {
      state.setSelectedKey(matches[0].key)
      return
    }
    state.setSelectedKey(null)
    return
  }

  if (model) {
    const match = items.find((item) => item.id === model || item.model === model)
    if (match) {
      state.setSelectedKey(match.key)
      return
    }
    state.setSelectedKey(null)
    return
  }

  if (state.selectedKey && !items.some((item) => item.key === state.selectedKey)) {
    state.setSelectedKey(null)
  }
}

export function formatContextWindow(value: number | null | undefined): string {
  const n = Number(value)
  if (!Number.isFinite(n) || n <= 0) return ''
  if (n >= 1_000_000) {
    const m = n / 1_000_000
    return `${Number.isInteger(m) ? m : m.toFixed(1)}M`
  }
  if (n >= 1000) return `${Math.round(n / 1000)}K`
  return String(n)
}

function normalizeContextWindow(value: number | null | undefined): number | undefined {
  const n = Number(value)
  if (!Number.isFinite(n) || n <= 0) return undefined
  return Math.min(Math.max(Math.floor(n), 4096), 2_000_000)
}

function newManualId(): string {
  return `m-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`
}

export function upsertManualEntry(entry: ManualEdgeEntry): void {
  const state = getEdgeStoreState()
  const normalized: ManualEdgeEntry = {
    ...entry,
    fromSetupRestore: false,
    context_window: normalizeContextWindow(entry.context_window),
  }
  const manualEntries = [...state.manualEntries]
  const idx = manualEntries.findIndex((e) => e.id === normalized.id)
  if (idx >= 0) manualEntries[idx] = normalized
  else manualEntries.push(normalized)
  state.setManualEntries(manualEntries)
  persistManualEntries(manualEntries)
  markEdgeUserConfigured()
  ensureSelectedKey(normalized.model, normalized.base_url)
  notifyModelChange()
  notifyUiChange()
}

export function deleteManualEntry(id: string): void {
  const state = getEdgeStoreState()
  const deleted = state.manualEntries.find((entry) => entry.id === id)
  const setupEdge = useSetupStore.getState().setup?.edge
  const deletedActiveSetup = deleted && setupEdge
    ? edgeSelectionMatchesSetup(
        {
          key: entryKey('manual', deleted.id),
          type: 'manual',
          id: deleted.id,
          name: deleted.name,
          base_url: deleted.base_url,
          model: deleted.model,
        },
        setupEdge,
      )
    : false
  const wasSelected = state.selectedKey === entryKey('manual', id)

  const manualEntries = state.manualEntries.filter((entry) => entry.id !== id)
  state.setManualEntries(manualEntries)
  persistManualEntries(manualEntries)

  if (wasSelected || deletedActiveSetup) {
    if (!fallbackEdgeModelAfterManualDelete()) {
      patchLocalSetupEdgeCleared()
    }
  } else if (!manualEntries.length && !state.selectedKey?.startsWith('herdsman:')) {
    clearEdgeUserConfigured()
  }

  if (
    !getSelectedItem()
    && !setupEdgeMatchesDisplayItems(useSetupStore.getState().setup?.edge)
  ) {
    patchLocalSetupEdgeCleared()
  }
  notifyModelChange()
  notifyUiChange()
}

export function syncEdgeFromSetup(edge: SetupEdge | null | undefined): void {
  const state = getEdgeStoreState()
  state.setPendingSetupSelection(null)
  if (!edge?.base_url || !edge?.model) {
    notifyUiChange()
    return
  }

  maybeMigrateLegacyEdgeConfirmation(edge)

  const url = edge.base_url.trim()
  const model = edge.model.trim()
  state.setPendingSetupSelection({ model, url })
  const herdsmanMatch = findHerdsmanItem(model, url)

  if (herdsmanMatch || isAllowedHerdsmanEndpoint(url)) {
    pruneHerdsmanManualDuplicates()
    if (isEdgeUserConfigured()) {
      ensureSelectedKey(model, url)
    }
    notifyUiChange()
    return
  }

  let entry = state.manualEntries.find(
    (e) => normalizeUrl(e.base_url) === normalizeUrl(url) && e.model === model,
  )

  if (!isEdgeUserConfigured()) {
    notifyUiChange()
    return
  }

  if (!entry) {
    entry = {
      id: newManualId(),
      name: model,
      base_url: url,
      model,
      api_key: edge.api_key || undefined,
      fromSetupRestore: true,
    }
    const manualEntries = [...state.manualEntries, entry]
    state.setManualEntries(manualEntries)
    persistManualEntries(manualEntries)
  } else if (edge.api_key) {
    const manualEntries = state.manualEntries.map((e) =>
      e.id === entry!.id ? { ...e, api_key: edge.api_key || undefined } : e,
    )
    state.setManualEntries(manualEntries)
    persistManualEntries(manualEntries)
  }

  ensureSelectedKey(model, url)
  notifyUiChange()
}

export async function persistEdgeSelection(
  saveSetup?: (body: import('../types/gateway').UpstreamSetupUpdate) => void,
): Promise<void> {
  const built = buildEdgeSavePayload()
  if (!built?.edge?.base_url || !built?.edge?.model) {
    if (saveSetup && useAppStore.getState().connected) {
      saveSetup({ edge: { clear: true } })
    } else {
      await clearEdgeUpstreamConfiguration(false)
    }
    return
  }

  if (saveSetup && useAppStore.getState().connected) {
    saveSetup({ edge: built.edge, gateway: built.gateway })
    return
  }

  if (useAppStore.getState().connected) {
    try {
      const res = await apiFetch<{ upstream?: UpstreamSetupView }>('/v1/admin/setup', {
        method: 'POST',
        body: JSON.stringify({ edge: built.edge, gateway: built.gateway }),
      })
      if (res?.upstream) useSetupStore.getState().setSetup(res.upstream)
    } catch (error) {
      console.warn('[edge-upstream] persist edge selection failed', error)
    }
  }
}

export function populateEdgeModelSelect(models: unknown, selectedId?: string | null): void {
  const state = getEdgeStoreState()
  state.setCachedModels(filterHerdsmanModels(models))
  if (selectedId) {
    ensureSelectedKey(selectedId, state.pendingSetupSelection?.url)
  } else {
    applyPendingSetupSelection()
  }
  notifyUiChange()
}

export function initEdgeModelSelect(onChange?: EdgeModelChangeCallback): {
  getValue: typeof getEdgeModelValue
  unsubscribe?: () => void
} {
  notifyUiChange()
  const unsubscribe = onChange ? subscribeEdgeModelChange(onChange) : undefined
  return { getValue: getEdgeModelValue, unsubscribe }
}

export function fetchEdgeModels(): HerdsmanModel[] {
  return getEdgeStoreState().cachedModels
}

export function buildEdgeSavePayload(modelId?: string | null): EdgeSavePayload {
  const preferred = modelId || getEdgeModelValue()
  const items = buildDisplayItems()
  const item =
    items.find((i) => i.id === preferred || i.model === preferred) ?? getSelectedItem()
  if (!item) {
    return { edge: { base_url: '', model: null } }
  }

  const payload: EdgeSavePayload['edge'] = {
    base_url: normalizeHerdsmanEndpoint(item.base_url),
    model: item.type === 'manual' ? (item.model ?? null) : item.id,
  }

  if (item.type === 'manual' && item.api_key) {
    payload.api_key = item.api_key
  }

  const ctxMax = normalizeContextWindow(item.context_window)
  const gateway = ctxMax ? { ctx_edge_max_tokens: ctxMax } : undefined

  return {
    edge: payload,
    gateway,
  }
}

export function updateHerdsmanConnectionUi(connected: boolean): void {
  const state = getEdgeStoreState()
  state.setHerdsmanConnected(connected)
  if (!connected && state.selectedKey?.startsWith('herdsman:')) {
    state.setSelectedKey(null)
  }
  updateHerdsmanConnectionPoll()
  notifyUiChange()
}

export function refreshEdgeUpstreamUi(
  selectedModel?: string | null,
  selectedUrl?: string | null,
): HerdsmanModel[] {
  const state = getEdgeStoreState()
  const url = selectedUrl || state.pendingSetupSelection?.url
  if (selectedModel || url) {
    ensureSelectedKey(selectedModel, url)
  } else {
    applyPendingSetupSelection()
  }
  updateHerdsmanConnectionUi(state.herdsmanConnected)
  return state.cachedModels
}

export async function reconcileEdgeOnBoot(
  apiFetch: ApiFetch | null,
  setupEdge: SetupEdge | null | undefined,
): Promise<EdgeReconcileResult> {
  const state = getEdgeStoreState()
  state.setPendingEdgeReconcile({ apiFetch, setupEdge })

  applyPendingSetupSelection()
  ensureSelectedKey()
  maybeMigrateLegacyEdgeConfirmation(setupEdge)

  if (isEdgeUserConfigured()) {
    const herdsmanStale =
      isHerdsmanEdgeSetup(setupEdge) && !getEdgeStoreState().herdsmanConnected
    if (!herdsmanStale) {
      applyPendingSetupSelection()
      ensureSelectedKey()
      state.setEdgeBootReconciled(true)
      state.setPendingEdgeReconcile(null)
      return { cleared: false }
    }
  }

  const url = setupEdge?.base_url?.trim()
  const model = setupEdge?.model?.trim()
  if (url && model && isAllowedHerdsmanEndpoint(url) && state.herdsmanInstalled && !state.herdsmanConnected) {
    setTimeout(() => {
      const current = getEdgeStoreState()
      if (!current.edgeBootReconciled && current.pendingEdgeReconcile) {
        void finishEdgeReconcile().then((result) => {
          notifyReconcileComplete(result)
        })
      }
    }, 15_000)
    return { cleared: false, deferred: true }
  }

  return finishEdgeReconcile()
}

async function finishEdgeReconcile(): Promise<EdgeReconcileResult> {
  const state = getEdgeStoreState()
  if (state.edgeBootReconciled) return { cleared: false }

  const pending = state.pendingEdgeReconcile
  const { apiFetch, setupEdge } = pending || {}
  state.setPendingEdgeReconcile(null)
  state.setEdgeBootReconciled(true)

  applyPendingSetupSelection()
  ensureSelectedKey()
  maybeMigrateLegacyEdgeConfirmation(setupEdge)

  if (isEdgeUserConfigured()) {
    const herdsmanStale =
      isHerdsmanEdgeSetup(setupEdge) && !getEdgeStoreState().herdsmanConnected
    if (!herdsmanStale) {
      applyPendingSetupSelection()
      ensureSelectedKey()
      return { cleared: false }
    }
  }

  state.setPendingSetupSelection(null)
  state.setSelectedKey(null)
  state.setManualEntries(state.manualEntries.filter((entry) => !entry.fromSetupRestore))

  const hasStaleServerEdge = !!setupEdge?.base_url?.trim()
  if (!hasStaleServerEdge || !apiFetch) {
    notifyUiChange()
    return { cleared: false }
  }

  try {
    const res = await apiFetch('/v1/admin/setup', {
      method: 'POST',
      body: JSON.stringify({ edge: { clear: true } }),
    })
    notifyUiChange()
    return { cleared: true, response: res }
  } catch (error) {
    console.warn('[edge-upstream] clear stale edge failed', error)
    notifyUiChange()
    return { cleared: false, error }
  }
}

export async function ensureEdgeUpstreamConfigured(
  apiFetch: ApiFetch | null,
  options: EnsureEdgeUpstreamOptions = {},
): Promise<EnsureEdgeUpstreamResult> {
  const { silent = true, currentModel, currentUrl } = options
  refreshEdgeUpstreamUi(currentModel, currentUrl)

  const state = getEdgeStoreState()
  if (!apiFetch || !state.herdsmanConnected || !state.cachedModels.length) {
    return { models: state.cachedModels, posted: false }
  }

  const modelId = getEdgeModelValue()
  if (!modelId) {
    return { models: state.cachedModels, posted: false }
  }

  const { edge, gateway } = buildEdgeSavePayload(modelId)
  if (!edge?.base_url || !edge?.model) {
    return { models: state.cachedModels, posted: false }
  }

  const body: { edge: EdgeSavePayload['edge']; gateway?: EdgeSavePayload['gateway'] } = { edge }
  if (gateway) body.gateway = gateway

  try {
    const res = await apiFetch('/v1/admin/setup', {
      method: 'POST',
      body: JSON.stringify(body),
    })
    return { models: state.cachedModels, posted: true, response: res }
  } catch (error) {
    if (!silent) throw error
    console.warn('[edge-upstream] auto setup failed', error)
    return { models: state.cachedModels, posted: false, error }
  }
}

function handleHerdsmanModels(models: unknown): void {
  const state = getEdgeStoreState()
  state.setCachedModels(filterHerdsmanModels(models))
  pruneHerdsmanManualDuplicates()
  applyPendingSetupSelection()
  ensureSelectedKey()
  syncHerdsmanEdgeAvailability()
  updateHerdsmanConnectionUi(state.herdsmanConnected)
  if (!state.edgeBootReconciled && state.pendingEdgeReconcile) {
    void finishEdgeReconcile().then((result) => {
      notifyReconcileComplete(result)
    })
  }
}

function applyHerdsmanSnapshot(snapshot: HerdsmanStatusSnapshot): void {
  if (!snapshot || typeof snapshot !== 'object') return
  const state = getEdgeStoreState()
  state.setHerdsmanInstalled(!!snapshot.installed)
  state.setHerdsmanConnected(!!snapshot.connected)
  if (snapshot.connected && Array.isArray(snapshot.models)) {
    state.setCachedModels(filterHerdsmanModels(snapshot.models))
  } else {
    state.resetHerdsmanModels()
    if (!snapshot.connected) {
      handleHerdsmanDisconnected()
    }
  }
  pruneHerdsmanManualDuplicates()
  applyPendingSetupSelection()
  ensureSelectedKey()
  syncHerdsmanEdgeAvailability()
  updateHerdsmanConnectionUi(!!snapshot.connected)
}

export async function listenHerdsmanEvents(): Promise<void> {
  if (listenersInstalled || !isTauri()) return
  listenersInstalled = true

  unlistenFns.push(
    await listen<boolean>('herdsman-connected', (event) => {
      const connected = !!event.payload
      const state = getEdgeStoreState()
      const shouldClear = !connected && shouldClearHerdsmanEdgeOnDisconnect()
      state.setHerdsmanConnected(connected)
      if (!connected) {
        state.resetHerdsmanModels()
        if (shouldClear) {
          void clearHerdsmanEdgeConfiguration()
        }
      }
      updateHerdsmanConnectionUi(connected)
      if (connected && !state.edgeBootReconciled && state.pendingEdgeReconcile) {
        void finishEdgeReconcile().then((result) => {
          notifyReconcileComplete(result)
        })
      }
    }),
  )

  unlistenFns.push(
    await listen<unknown>('herdsman-models', (event) => {
      handleHerdsmanModels(event.payload)
    }),
  )

  unlistenFns.push(
    await listen<{ installed?: boolean }>('herdsman-install-detected', (event) => {
      const payload = event.payload
      if (!payload || typeof payload !== 'object') return
      getEdgeStoreState().setHerdsmanInstalled(!!payload.installed)
      updateHerdsmanConnectionPoll()
      void bootstrapHerdsmanStatus()
    }),
  )
}

export async function unlistenHerdsmanEvents(): Promise<void> {
  await Promise.all(unlistenFns.map((unlisten) => unlisten()))
  unlistenFns = []
  listenersInstalled = false
  stopHerdsmanConnectionPoll()
}

export async function bootstrapHerdsmanStatus(): Promise<void> {
  if (!isTauri()) return
  try {
    const snapshot = await invoke<HerdsmanStatusSnapshot>('herdsman_get_status')
    applyHerdsmanSnapshot(snapshot)
  } catch (error) {
    console.warn('[edge-upstream] herdsman_get_status failed', error)
  }
}

export async function refreshHerdsmanStatus(): Promise<void> {
  if (!isTauri()) return
  try {
    const snapshot = await invoke<HerdsmanStatusSnapshot>('herdsman_refresh_status')
    applyHerdsmanSnapshot(snapshot)
  } catch (error) {
    console.warn('[edge-upstream] herdsman_refresh_status failed', error)
  }
}

export async function startHerdsman(): Promise<void> {
  if (!isTauri()) return
  return invoke('herdsman_start')
}

export async function openHerdsmanOrInstall(): Promise<void> {
  if (!isTauri()) {
    window.open(HERDSMAN_INSTALL_URL, '_blank', 'noopener,noreferrer')
    return
  }
  return invoke('herdsman_open_or_install')
}

export async function initEdgeUpstream(): Promise<void> {
  const storedManualEntries = loadManualEntriesFromStorage()
  if (storedManualEntries.length) {
    getEdgeStoreState().setManualEntries(storedManualEntries)
  }
  await listenHerdsmanEvents()
  await bootstrapHerdsmanStatus()
  updateHerdsmanConnectionPoll()
  syncEdgeFromSetup(useSetupStore.getState().setup?.edge)
  initEdgeModelSelect()
}

export function refreshEdgeI18n(): void {
  updateHerdsmanConnectionUi(getEdgeStoreState().herdsmanConnected)
}

export { useEdgeStore, getEdgeStoreState } from '../stores/edgeStore'
