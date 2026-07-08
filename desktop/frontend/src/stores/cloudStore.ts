import { create } from 'zustand'
import type { CloudModel } from '../lib/flowy/api'

export interface ManualCloudEntry {
  id: string
  name: string
  base_url: string
  model: string
  api_key?: string
  context_window?: number
  fromSetupRestore?: boolean
}

export interface CloudDisplayItem {
  key: string
  type: 'flowy' | 'manual'
  id: string
  name: string
  base_url: string
  model?: string
  api_key?: string
  icon?: string
  context_window?: number
}

export interface SetupCloudSelection {
  model: string
  url: string
}

export interface SetupCloud {
  base_url?: string | null
  model?: string | null
  api_key?: string | null
}

interface CloudStoreState {
  flowyModels: CloudModel[]
  manualEntries: ManualCloudEntry[]
  selectedKey: string | null
  pendingSetupSelection: SetupCloudSelection | null

  setFlowyModels: (models: CloudModel[]) => void
  setManualEntries: (entries: ManualCloudEntry[]) => void
  setSelectedKey: (key: string | null) => void
  setPendingSetupSelection: (selection: SetupCloudSelection | null) => void
}

export const useCloudStore = create<CloudStoreState>((set) => ({
  flowyModels: [],
  manualEntries: [],
  selectedKey: null,
  pendingSetupSelection: null,

  setFlowyModels: (models) => set({ flowyModels: models }),
  setManualEntries: (entries) => set({ manualEntries: entries }),
  setSelectedKey: (key) => set({ selectedKey: key }),
  setPendingSetupSelection: (selection) => set({ pendingSetupSelection: selection }),
}))

export function getCloudStoreState() {
  return useCloudStore.getState()
}
