import { create } from 'zustand'
import type { UpstreamSetupView } from '../types/gateway'

interface SetupStoreState {
  setup: UpstreamSetupView | null
  setSetup: (setup: UpstreamSetupView | null) => void
  patchSetup: (patch: Partial<UpstreamSetupView>) => void
}

export const useSetupStore = create<SetupStoreState>()((set, get) => ({
  setup: null,
  setSetup: (setup) => set({ setup }),
  patchSetup: (patch) => {
    const current = get().setup
    if (!current) {
      set({ setup: patch as UpstreamSetupView })
      return
    }
    set({ setup: { ...current, ...patch } })
  },
}))
