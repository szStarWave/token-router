/**
 * v2 app store（技术方案 §7.2 条款 5）：只保留 UI 偏好。
 *
 * 删除的服务端状态镜像：status/stats/connected/gatewayBase/authKey…
 * 服务端状态一律走 `src/api/client.ts` + TanStack Query（queries/admin.ts）。
 */
import { create } from 'zustand'
import type { Locale } from '../i18n/dict'

export type ThemePref = 'system' | 'light' | 'dark'

const THEME_KEY = 'tr2.theme'
const LOCALE_KEY = 'tr2.locale'

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

/** 主题偏好：默认亮色（用户可切深色或跟随系统）。 */
function storedThemePref(): ThemePref {
  try {
    const raw = window.localStorage.getItem(THEME_KEY)
    if (raw === 'light' || raw === 'dark' || raw === 'system') return raw
  } catch {
    /* ignore */
  }
  return 'light'
}

function initialLocale(): Locale {
  try {
    const raw = window.localStorage.getItem(LOCALE_KEY)
    if (raw === 'zh' || raw === 'en') return raw
  } catch {
    /* ignore */
  }
  if (typeof navigator !== 'undefined' && navigator.language.startsWith('zh')) return 'zh'
  return 'en'
}

export const useAppStore = create<AppState>((set) => ({
  themePref: storedThemePref(),
  locale: initialLocale(),
  siderNarrow: false,
  toasts: [],
  activePage: 'overview',
  activeNavId: 'overview',

  setThemePref: (themePref) => {
    try {
      window.localStorage.setItem(THEME_KEY, themePref)
    } catch {
      /* ignore */
    }
    applyThemePref(themePref)
    set({ themePref })
  },
  setLocale: (locale) => {
    try {
      window.localStorage.setItem(LOCALE_KEY, locale)
    } catch {
      /* ignore */
    }
    set({ locale })
  },
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

/** 主题 id（UnoCSS 预设按 data-theme 切换整套变量）。 */
const THEME_IDS: Record<'light' | 'dark', string> = {
  light: 'vercel-light',
  dark: 'vercel-dark',
}

/** 应用主题（写 <html data-theme>；token 层按此切换深/浅两套）。 */
export function applyThemePref(pref: ThemePref) {
  const effective = pref === 'system' ? (prefersDark() ? 'dark' : 'light') : pref
  document.documentElement.dataset.theme = THEME_IDS[effective]
  document.documentElement.dataset.effectiveTheme = effective
}

/** 系统主题变化时重算（仅 pref = system 生效）。 */
export function watchSystemTheme(): () => void {
  if (typeof window === 'undefined') return () => {}
  const media = window.matchMedia('(prefers-color-scheme: dark)')
  const onChange = () => {
    if (useAppStore.getState().themePref === 'system') applyThemePref('system')
  }
  media.addEventListener('change', onChange)
  return () => media.removeEventListener('change', onChange)
}
