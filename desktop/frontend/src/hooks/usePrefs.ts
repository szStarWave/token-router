import { useEffect } from 'react'
import { STORAGE_LOCALE, STORAGE_THEME } from '../constants/defaults'
import { useAppStore } from '../stores/appStore'
import type { Locale } from '../i18n/dict'
import type { ThemePref } from '../types/gateway'
import { useTheme } from './useI18n'

export function usePrefs() {
  const { setThemePref, setLocale } = useAppStore()
  const { applyTheme } = useTheme()

  useEffect(() => {
    const theme = (localStorage.getItem(STORAGE_THEME) || 'system') as ThemePref
    const locale = (localStorage.getItem(STORAGE_LOCALE) || 'zh') as Locale
    setThemePref(theme)
    setLocale(locale)
    applyTheme()
    document.documentElement.lang = locale === 'zh' ? 'zh-CN' : 'en'
  }, [setThemePref, setLocale, applyTheme])

  const savePrefs = () => {
    const { themePref, locale } = useAppStore.getState()
    localStorage.setItem(STORAGE_THEME, themePref)
    localStorage.setItem(STORAGE_LOCALE, locale)
    document.documentElement.lang = locale === 'zh' ? 'zh-CN' : 'en'
    applyTheme()
    window.dispatchEvent(new CustomEvent('app-locale-change'))
  }

  return { savePrefs }
}
