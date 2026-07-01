import { useAppStore } from '../stores/appStore'
import { t, type Locale } from '../i18n/dict'

export function useI18n() {
  const locale = useAppStore((s) => s.locale)
  return {
    locale,
    t: (key: string, vars?: Record<string, string | number>) => t(locale as Locale, key, vars),
  }
}

export function useTheme() {
  const themePref = useAppStore((s) => s.themePref)
  const setThemePref = useAppStore((s) => s.setThemePref)

  function systemTheme(): 'light' | 'dark' {
    return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
  }

  function effectiveTheme(): 'light' | 'dark' {
    return themePref === 'system' ? systemTheme() : themePref
  }

  function applyTheme() {
    document.documentElement.dataset.effectiveTheme = effectiveTheme()
  }

  return { themePref, setThemePref, effectiveTheme, applyTheme }
}
