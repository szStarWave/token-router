/**
 * 桌面偏好（设置页）：外观与行为类偏好。
 *
 * 持久化在 localStorage，改动立即生效（写入 <html> 的 data-* 与 CSS 变量）。
 * 这里只放**前端**偏好；服务端配置（监听、鉴权、候选）走 config 草稿与 apply。
 */
import { create } from 'zustand'

export type AccentKey = 'default' | 'indigo' | 'teal' | 'green' | 'orange' | 'magenta' | 'ink'
export type MotionPref = 'full' | 'reduced'
export type FontScaleKey = '100' | '110' | '115' | '120' | '125' | '150'
export type UiFontKey = 'system' | 'inter'
export type MonoFontKey = 'system' | 'geist'
/** 模型名显示：`full` 原样（`group/member`），`short` 去掉前缀（`member`）。 */
export type ModelNamePref = 'full' | 'short'

interface Accent {
  base: string
  hover: string
  active: string
}

export const ACCENTS: Record<Exclude<AccentKey, 'default'>, Accent> = {
  indigo: { base: '#4f46e5', hover: '#4338ca', active: '#3730a3' },
  teal: { base: '#0d9488', hover: '#0f766e', active: '#115e59' },
  green: { base: '#16a34a', hover: '#15803d', active: '#166534' },
  orange: { base: '#ea580c', hover: '#c2410c', active: '#9a3412' },
  magenta: { base: '#c026d3', hover: '#a21caf', active: '#86198f' },
  ink: { base: '#334155', hover: '#1e293b', active: '#0f172a' },
}

export const ACCENT_OPTIONS: Array<{ key: AccentKey; label: string }> = [
  { key: 'default', label: '主题默认' },
  { key: 'indigo', label: '靛蓝' },
  { key: 'teal', label: '青碧' },
  { key: 'green', label: '松绿' },
  { key: 'orange', label: '橘' },
  { key: 'magenta', label: '品红' },
  { key: 'ink', label: '墨蓝' },
]

export const FONT_SCALES: Array<{ key: FontScaleKey; label: string }> = [
  { key: '100', label: '100%' },
  { key: '110', label: '110%' },
  { key: '115', label: '115%' },
  { key: '120', label: '120%' },
  { key: '125', label: '125%' },
  { key: '150', label: '150%' },
]

export const UI_FONTS: Array<{ key: UiFontKey; label: string }> = [
  { key: 'system', label: '系统默认' },
  { key: 'inter', label: 'Inter' },
]

export const MONO_FONTS: Array<{ key: MonoFontKey; label: string }> = [
  { key: 'system', label: '系统默认' },
  { key: 'geist', label: 'Geist Mono' },
]

export const MODEL_NAME_OPTIONS: Array<{ key: ModelNamePref; label: string }> = [
  { key: 'full', label: '完整' },
  { key: 'short', label: '去掉前缀' },
]

const SYSTEM_SANS =
  '-apple-system, BlinkMacSystemFont, "Segoe UI", "Microsoft YaHei UI", "PingFang SC", sans-serif'
const SYSTEM_MONO =
  'ui-monospace, SFMono-Regular, "Cascadia Mono", Consolas, "Microsoft YaHei Mono", monospace'

const KEY_PREFIX = 'tr2.pref.'

function read(key: string, fallback: string): string {
  try {
    const raw = window.localStorage.getItem(KEY_PREFIX + key)
    if (raw !== null) return raw
  } catch {
    /* ignore */
  }
  return fallback
}

function write(key: string, value: string): void {
  try {
    window.localStorage.setItem(KEY_PREFIX + key, value)
  } catch {
    /* ignore */
  }
}

/** 强调色：覆盖主题的 --primary-* 变量（默认 = 不覆盖，跟随主题）。 */
export function applyAccent(key: AccentKey): void {
  const root = document.documentElement
  const accent = key === 'default' ? undefined : ACCENTS[key]
  if (!accent) {
    for (const name of ['--primary-base', '--primary-hover', '--primary-active', '--primary-muted']) {
      root.style.removeProperty(name)
    }
    return
  }
  root.style.setProperty('--primary-base', accent.base)
  root.style.setProperty('--primary-hover', accent.hover)
  root.style.setProperty('--primary-active', accent.active)
  root.style.setProperty('--primary-muted', `${accent.base}80`)
}

/** 减弱动态效果：html[data-motion=reduced] 由 preflight 规则接管。 */
export function applyMotion(pref: MotionPref): void {
  document.documentElement.dataset.motion = pref
}

/**
 * 文字大小：只放大页面内容（.app-page-content），侧栏与标题栏不变。
 * 主题字号是 px 固定值（不是 rem），所以用 zoom 而不是 root font-size。
 */
export function applyFontScale(key: FontScaleKey): void {
  const scale = Number(key) / 100
  document.documentElement.style.setProperty('--tr-text-scale', String(scale))
}

/** 界面字体 / 代码字体：覆盖主题的字体变量。 */
export function applyFonts(ui: UiFontKey, mono: MonoFontKey): void {
  const root = document.documentElement
  root.style.setProperty(
    '--oc-font-sans',
    ui === 'inter' ? `Inter, ${SYSTEM_SANS}` : SYSTEM_SANS,
  )
  root.style.setProperty(
    '--oc-font-mono',
    mono === 'geist' ? `"Geist Mono", ${SYSTEM_MONO}` : SYSTEM_MONO,
  )
}

export interface PrefsState {
  accent: AccentKey
  motion: MotionPref
  fontScale: FontScaleKey
  uiFont: UiFontKey
  monoFont: MonoFontKey
  modelName: ModelNamePref
  liteMode: boolean
  confirmOnExit: boolean
  /** 关闭到托盘：窗口关闭 = 隐藏到托盘（壳侧默认开，偏好持久化后覆盖）。 */
  closeToTray: boolean
  /** 账号打码：只显示密钥指纹；关掉则显示凭据引用（env:/file:）。明文永远不显示。 */
  maskAccounts: boolean
  setAccent: (value: AccentKey) => void
  setMotion: (value: MotionPref) => void
  setFontScale: (value: FontScaleKey) => void
  setUiFont: (value: UiFontKey) => void
  setMonoFont: (value: MonoFontKey) => void
  setModelName: (value: ModelNamePref) => void
  setLiteMode: (value: boolean) => void
  setConfirmOnExit: (value: boolean) => void
  setCloseToTray: (value: boolean) => void
  setMaskAccounts: (value: boolean) => void
}

/** 旧值迁移：off / on → full（原样），plain → short（去前缀）。 */
function initialModelName(): ModelNamePref {
  const raw = read('modelName', 'full')
  return raw === 'plain' || raw === 'short' ? 'short' : 'full'
}

export const usePrefsStore = create<PrefsState>((set, get) => ({
  accent: read('accent', 'default') as AccentKey,
  motion: read('motion', 'full') as MotionPref,
  fontScale: read('fontScale', '100') as FontScaleKey,
  uiFont: read('uiFont', 'system') as UiFontKey,
  monoFont: read('monoFont', 'system') as MonoFontKey,
  modelName: initialModelName(),
  liteMode: read('liteMode', 'false') === 'true',
  confirmOnExit: read('confirmOnExit', 'false') === 'true',
  closeToTray: read('closeToTray', 'true') !== 'false',
  maskAccounts: read('maskAccounts', 'true') !== 'false',

  setAccent: (accent) => {
    write('accent', accent)
    applyAccent(accent)
    set({ accent })
  },
  setMotion: (motion) => {
    write('motion', motion)
    applyMotion(motion)
    set({ motion })
  },
  setFontScale: (fontScale) => {
    write('fontScale', fontScale)
    applyFontScale(fontScale)
    set({ fontScale })
  },
  setUiFont: (uiFont) => {
    write('uiFont', uiFont)
    // 必须应用新值：get() 在 set 之前仍是旧值，先 apply 会让这次切换不生效。
    applyFonts(uiFont, get().monoFont)
    set({ uiFont })
  },
  setMonoFont: (monoFont) => {
    write('monoFont', monoFont)
    applyFonts(get().uiFont, monoFont)
    set({ monoFont })
  },
  setModelName: (modelName) => {
    write('modelName', modelName)
    set({ modelName })
  },
  setLiteMode: (liteMode) => {
    write('liteMode', String(liteMode))
    set({ liteMode })
  },
  setConfirmOnExit: (confirmOnExit) => {
    write('confirmOnExit', String(confirmOnExit))
    set({ confirmOnExit })
  },
  setCloseToTray: (closeToTray) => {
    write('closeToTray', String(closeToTray))
    set({ closeToTray })
  },
  setMaskAccounts: (maskAccounts) => {
    write('maskAccounts', String(maskAccounts))
    set({ maskAccounts })
  },
}))

/** 首屏应用一次（主题由 appStore 负责）。 */
export function applyStoredPrefs(): void {
  const state = usePrefsStore.getState()
  applyAccent(state.accent)
  applyMotion(state.motion)
  applyFontScale(state.fontScale)
  applyFonts(state.uiFont, state.monoFont)
}
