import { create } from 'zustand'
import type { Locale } from '../i18n/dict'
import type { GatewayStatus, StatsSnapshot, ThemePref } from '../types/gateway'
import { DEFAULT_GATEWAY_BASE } from '../constants/defaults'

export interface ToastItem {
  id: number
  message: string
  ok: boolean
}

interface AppState {
  themePref: ThemePref
  locale: Locale
  gatewayBase: string
  connected: boolean
  siderWidth: number
  siderNarrow: boolean
  toasts: ToastItem[]
  status: GatewayStatus | null
  stats: StatsSnapshot | null
  globalStats: StatsSnapshot | null
  scope: 'session' | 'global'
  uptimeAnchor: { secs: number; at: number } | null
  gatewayAuthKeyPending: string | null
  savedPoints: number | null
  activePage: string
  activeNavId: string
  routeTab: string
  isTauriApp: boolean

  setThemePref: (pref: ThemePref) => void
  setLocale: (locale: Locale) => void
  setGatewayBase: (base: string) => void
  setConnected: (connected: boolean) => void
  setSiderWidth: (width: number) => void
  setSiderNarrow: (narrow: boolean) => void
  showToast: (message: string, ok?: boolean) => void
  removeToast: (id: number) => void
  setStatus: (status: GatewayStatus | null) => void
  setStats: (stats: StatsSnapshot | null) => void
  setGlobalStats: (stats: StatsSnapshot | null) => void
  setScope: (scope: 'session' | 'global') => void
  setUptimeAnchor: (anchor: { secs: number; at: number } | null) => void
  setGatewayAuthKeyPending: (key: string | null) => void
  setSavedPoints: (n: number | null) => void
  setActiveNav: (page: string, navId: string) => void
  setRouteTab: (route: string) => void
  setIsTauriApp: (v: boolean) => void
}

let toastId = 0

export const useAppStore = create<AppState>()((set, get) => ({
  themePref: 'system',
  locale: 'zh',
  gatewayBase: DEFAULT_GATEWAY_BASE,
  connected: false,
  siderWidth: 330,
  siderNarrow: false,
  toasts: [],
  status: null,
  stats: null,
  globalStats: null,
  scope: 'global',
  uptimeAnchor: null,
  gatewayAuthKeyPending: null,
  savedPoints: null,
  activePage: 'overview',
  activeNavId: 'overview',
  routeTab: 'auto',
  isTauriApp: false,

  setThemePref: (pref) => set({ themePref: pref }),
  setLocale: (locale) => set({ locale }),
  setGatewayBase: (base) => set({ gatewayBase: base }),
  setConnected: (connected) => set({ connected }),
  setSiderWidth: (width) => set({ siderWidth: width }),
  setSiderNarrow: (narrow) => set({ siderNarrow: narrow }),
  showToast: (message, ok = true) => {
    const id = ++toastId
    set({ toasts: [...get().toasts, { id, message, ok }] })
    setTimeout(() => get().removeToast(id), 3000)
  },
  removeToast: (id) => set({ toasts: get().toasts.filter((t) => t.id !== id) }),
  setStatus: (status) => set({ status }),
  setStats: (stats) => set({ stats }),
  setGlobalStats: (stats) => set({ globalStats: stats }),
  setScope: (scope) => set({ scope }),
  setUptimeAnchor: (anchor) => set({ uptimeAnchor: anchor }),
  setGatewayAuthKeyPending: (key) => set({ gatewayAuthKeyPending: key }),
  setSavedPoints: (n) => set({ savedPoints: n }),
  setActiveNav: (page, navId) => set({ activePage: page, activeNavId: navId }),
  setRouteTab: (route) => set({ routeTab: route }),
  setIsTauriApp: (v) => set({ isTauriApp: v }),
}))
