import { create } from 'zustand'

export interface HerdsmanModel {
  id: string
  name: string
  endpoint: string
  context_window?: number
}

export interface ManualEdgeEntry {
  id: string
  name: string
  base_url: string
  model: string
  api_key?: string
  context_window?: number
  fromSetupRestore?: boolean
}

export interface EdgeDisplayItem {
  key: string
  type: 'herdsman' | 'manual'
  id: string
  name: string
  base_url: string
  model?: string
  api_key?: string
  context_window?: number
}

export interface SetupEdgeSelection {
  model: string
  url: string
}

export interface SetupEdge {
  base_url?: string | null
  model?: string | null
  api_key?: string | null
}

export type ApiFetch = (
  path: string,
  opts?: { method?: string; body?: string; headers?: Record<string, string> },
) => Promise<unknown>

export interface PendingEdgeReconcile {
  apiFetch: ApiFetch | null
  setupEdge: SetupEdge | null | undefined
}

export interface HerdsmanStatusSnapshot {
  connected: boolean
  installed: boolean
  models?: HerdsmanModel[]
  endpoint?: string | null
  openai_endpoint?: string | null
  launcher_path?: string | null
}

interface EdgeStoreState {
  cachedModels: HerdsmanModel[]
  manualEntries: ManualEdgeEntry[]
  selectedKey: string | null
  pendingSetupSelection: SetupEdgeSelection | null
  herdsmanConnected: boolean
  herdsmanInstalled: boolean
  edgeBootReconciled: boolean
  pendingEdgeReconcile: PendingEdgeReconcile | null

  setCachedModels: (models: HerdsmanModel[]) => void
  setManualEntries: (entries: ManualEdgeEntry[]) => void
  setSelectedKey: (key: string | null) => void
  setPendingSetupSelection: (selection: SetupEdgeSelection | null) => void
  setHerdsmanConnected: (connected: boolean) => void
  setHerdsmanInstalled: (installed: boolean) => void
  setEdgeBootReconciled: (reconciled: boolean) => void
  setPendingEdgeReconcile: (pending: PendingEdgeReconcile | null) => void
  applyHerdsmanSnapshot: (snapshot: HerdsmanStatusSnapshot) => void
  resetHerdsmanModels: () => void
}

export const useEdgeStore = create<EdgeStoreState>((set) => ({
  cachedModels: [],
  manualEntries: [],
  selectedKey: null,
  pendingSetupSelection: null,
  herdsmanConnected: false,
  herdsmanInstalled: false,
  edgeBootReconciled: false,
  pendingEdgeReconcile: null,

  setCachedModels: (models) => set({ cachedModels: models }),
  setManualEntries: (entries) => set({ manualEntries: entries }),
  setSelectedKey: (key) => set({ selectedKey: key }),
  setPendingSetupSelection: (selection) => set({ pendingSetupSelection: selection }),
  setHerdsmanConnected: (connected) => set({ herdsmanConnected: connected }),
  setHerdsmanInstalled: (installed) => set({ herdsmanInstalled: installed }),
  setEdgeBootReconciled: (reconciled) => set({ edgeBootReconciled: reconciled }),
  setPendingEdgeReconcile: (pending) => set({ pendingEdgeReconcile: pending }),

  applyHerdsmanSnapshot: (snapshot) =>
    set({
      herdsmanInstalled: !!snapshot.installed,
      herdsmanConnected: !!snapshot.connected,
      cachedModels:
        snapshot.connected && Array.isArray(snapshot.models) ? snapshot.models : [],
    }),

  resetHerdsmanModels: () => set({ cachedModels: [] }),
}))

export function getEdgeStoreState() {
  return useEdgeStore.getState()
}
