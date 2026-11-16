/**
 * v2 app store（技术方案 §7.2 条款 5）：只保留 UI 偏好。
 *
 * 删除的服务端状态镜像：status/stats/connected/gatewayBase/authKey…
 * 服务端状态一律走 `src/api/client.ts` + TanStack Query（queries/admin.ts）。
 */
import { create } from 'zustand'
import type { Locale } from '../i18n/dict'

export type ThemePref = 'system' | 'light' | 'dark'

export interface ToastItem {
  id: number
  key: string
  vars?: Record<string, string | number>
  ok: boolean
}

interface AppState {
  themePref: ThemePref
  locale: Locale
  siderNarrow: boolean
  toasts: ToastItem[]
  activePage: string
  activeNavId: string

  setThemePref: (pref: ThemePref) => void
  setLocale: (locale: Locale) => void
  setSiderNarrow: (narrow: boolean) => void
  showToast: (key: string, ok?: boolean, vars?: Record<string, string | number>) => void
  removeToast: (id: number) => void
  setActiveNav: (page: string, navId: string) => void
}

let toastSeq = 1

const prefersDark = (): boolean =>
  typeof window !== 'undefined' &&
  window.matchMedia('(prefers-color-scheme: dark)').matches

function initialLocale(): Locale {
  if (typeof navigator !== 'undefined' && navigator.language.startsWith('zh')) return 'zh'
  return 'en'
}

export const useAppStore = create<AppState>((set) => ({
  themePref: 'system',
  locale: initialLocale(),
  siderNarrow: false,
  toasts: [],
  activePage: 'overview',
  activeNavId: 'overview',

  setThemePref: (themePref) => set({ themePref }),
  setLocale: (locale) => set({ locale }),
  setSiderNarrow: (siderNarrow) => set({ siderNarrow }),
  showToast: (key, ok = true, vars) =>
    set((state) => ({
      toasts: [...state.toasts, { id: toastSeq++, key, ok, vars }],
    })),
  removeToast: (id) =>
    set((state) => ({ toasts: state.toasts.filter((toast) => toast.id !== id) })),
  setActiveNav: (activePage, activeNavId) => set({ activePage, activeNavId }),
}))

/** 命令式 toast（queries 层错误处理用）。 */
export function toast(key: string, ok = true, vars?: Record<string, string | number>) {
  useAppStore.getState().showToast(key, ok, vars)
}

/** 初始化主题（main.tsx 调用一次）。 */
export function applyThemePref(pref: ThemePref) {
  const effective = pref === 'system' ? (prefersDark() ? 'dark' : 'light') : pref
  document.documentElement.dataset.effectiveTheme = effective
}
