/**
 * OpenChamber 设计规范的 UnoCSS 预设（vendored）。
 *
 * 上游：https://github.com/me9rez/openchamber-design-system
 *   packages/unocss-preset/src/index.ts —— MIT，未发布到 npm，故随仓 vendored。
 *   主题 JSON 源：openchamber/openchamber（MIT）。
 * 归属与许可见 docs/dev/frontend-design-system.zh.md §2。
 *
 * 颜色、圆角、字号、三级阴影全部走 CSS 变量，所以 `data-theme` 一换整套跟着变；
 * 组件尺寸来自上游 cva 变体（按钮 32/28/24/36、触发器 28、分段项 28、步进器 32）。
 */
import type { Preset } from '@unocss/core'
import { presetWind4 } from '@unocss/preset-wind4'
import {
  colorTheme,
  defaultTheme,
  themeIds,
  themeVars,
  themeVariants,
  type ThemeId,
} from './generated'

export { themeIds, themeVars, themeVariants, colorTheme, defaultTheme }
export type { ThemeId }
export type ThemeName = ThemeId

export interface PresetOpenChamberOptions {
  /** 生成哪些主题的变量块，默认全部 */
  themes?: ThemeId[]
  /** `:root` 上默认生效的主题，默认 `openchamber-dark` */
  defaultTheme?: ThemeId
  /** 是否注入 body 的底色/字色/字体，默认 true；关掉就只剩变量和工具类 */
  base?: boolean
}

/** tile 色阶：把文字色混进下层，深浅主题与任意父级上都成立 */
const tone = (pct: number) =>
  `color-mix(in srgb,var(--surface-foreground) ${pct}%,transparent)`

/**
 * 每套主题的变量块；末尾带上 color-scheme，
 * 让原生控件（下拉、滚动条、复选框）跟着主题走。
 */
const varsBlock = (id: ThemeId) =>
  [
    ...Object.entries(themeVars[id]).map(([name, value]) => `${name}:${value}`),
    `color-scheme:${themeVariants[id] ?? 'light'}`,
  ].join(';')

export function presetOpenChamber(options: PresetOpenChamberOptions = {}): Preset<object> {
  const themes = options.themes?.length ? options.themes : (themeIds as unknown as ThemeId[])
  // 默认主题必须在生成列表里，否则 :root 会拿到一套没被导出的变量
  const rootTheme =
    options.defaultTheme && themes.includes(options.defaultTheme)
      ? options.defaultTheme
      : themes.includes(defaultTheme)
        ? defaultTheme
        : themes[0]

  const themeCss = [
    `:root{${varsBlock(rootTheme)}}`,
    ...themes
      .filter((id) => id !== rootTheme)
      .map((id) => `[data-theme="${id}"]{${varsBlock(id)}}`),
    `[data-theme="${rootTheme}"]{${varsBlock(rootTheme)}}`,
  ].join('\n')

  // 预设不带 reset；边界盒是这类控件尺寸能成立的前提（否则 h-8 会变成 32+padding）
  // 字体族也在这里定一份：正文与等宽各一个变量，body / 工具类 / 行内 code 共用，
  // 免得行内 code 与代码块落到两套字体上（前者会掉到 Consolas）。
  const baseCss = `
*,::before,::after{box-sizing:border-box}
:root{--oc-font-sans:"Inter",-apple-system,BlinkMacSystemFont,"Segoe UI",system-ui,"Microsoft YaHei UI",sans-serif;--oc-font-mono:"Geist Mono",ui-monospace,SFMono-Regular,Menlo,"Cascadia Mono","Microsoft YaHei UI",monospace}
body{background:var(--surface-background);color:var(--surface-foreground);font-family:var(--oc-font-sans);font-weight:400;line-height:1.45}
`

  return {
    name: 'preset-openchamber',
    // 主题值必须挂在排在 presetWind4 之后的预设上，否则 wind4 自带的 radius /
    // shadow 默认档会盖掉我们的值（上游实测结论）
    presets: [
      presetWind4(),
      {
        name: 'preset-openchamber-theme',
        theme: {
          colors: colorTheme as unknown as Record<string, never>,
          // wind4 的主题键与 mini 不同：radius / text / shadow / font
          radius: {
            none: '0',
            sm: '4px',
            md: '7px',
            DEFAULT: '9px',
            lg: '9px',
            xl: '12px',
            full: '9999px',
          },
          text: {
            'ui-label': { fontSize: '13.5px', lineHeight: '1.2' },
            'group-title': { fontSize: '13.5px', lineHeight: '1.2' },
            meta: { fontSize: '12.5px', lineHeight: '1.5' },
            micro: { fontSize: '12px', lineHeight: '1.45' },
            header: { fontSize: '14px', lineHeight: '1.2' },
            section: { fontSize: '15px', lineHeight: '1.45' },
            markdown: { fontSize: '14px', lineHeight: '1.45' },
            code: { fontSize: '12px', lineHeight: '1.45' },
            'page-title': { fontSize: '17px', lineHeight: '1.25' },
          },
          shadow: {
            button: 'var(--oc-elevation-button)',
            raised: 'var(--oc-elevation-raised)',
            floating: 'var(--oc-elevation-floating)',
          },
          font: {
            sans: 'var(--oc-font-sans)',
            mono: 'var(--oc-font-mono)',
          },
        },
      },
    ],
    preflights: [
      { layer: 'theme', getCSS: () => themeCss },
      ...(options.base === false ? [] : [{ layer: 'base' as const, getCSS: () => baseCss }]),
    ],
    rules: [
      // 多属性的面：一条规则写清，避免在 shortcuts 里堆任意值
      ['oc-raised', { 'box-shadow': 'var(--oc-elevation-raised)' }],
      ['oc-tile', { background: tone(4) }],
      ['oc-tile-inset', { background: tone(6) }],
      ['oc-recessed', { 'box-shadow': 'inset 0 1px 2px rgba(0,0,0,.16)', background: tone(4) }],
      [
        'oc-glass-popover',
        {
          background: 'color-mix(in srgb,var(--surface-elevated) 97%,transparent)',
          'backdrop-filter': 'blur(40px) saturate(1.3)',
          '-webkit-backdrop-filter': 'blur(40px) saturate(1.3)',
          'box-shadow': 'var(--oc-elevation-floating)',
          'border-radius': '12px',
        },
      ],
    ] as unknown as Preset<object>['rules'],
    shortcuts: [
      // tile 的 hover 档：把文字色混进下层再深一档
      [
        'oc-tile-hover',
        'hover:bg-[color-mix(in_srgb,var(--surface-foreground)_7%,transparent)]',
      ],
      // ---- 按钮：default 32 / sm 28 / xs 24 / lg 36 / icon 32，圆角 7 ----
      [
        'oc-btn',
        'inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-ui-label font-medium cursor-pointer select-none transition-colors disabled:opacity-50 disabled:cursor-not-allowed',
      ],
      ['oc-btn-default', 'oc-btn h-8 px-3 bg-background text-foreground shadow-button hover:bg-interactive-hover'],
      ['oc-btn-primary', 'oc-btn h-8 px-3 bg-primary text-primary-foreground hover:bg-primary-hover'],
      ['oc-btn-outline', 'oc-btn h-8 px-3 bg-background text-foreground shadow-button'],
      [
        'oc-btn-ghost',
        'oc-btn h-8 px-3 bg-transparent text-muted-foreground hover:bg-interactive-hover hover:text-foreground',
      ],
      [
        'oc-btn-destructive',
        'oc-btn h-8 px-3 text-status-error bg-[color-mix(in_srgb,var(--status-error)_13%,var(--surface-background))] shadow-button',
      ],
      ['oc-btn-sm', 'h-7 px-2.5'],
      ['oc-btn-xs', 'h-6 px-2 rounded-[5px] text-micro'],
      ['oc-btn-lg', 'h-9 px-4 rounded-lg'],
      ['oc-btn-icon', 'size-8 px-0'],

      // ---- 下拉触发器：28 / r7 / 横向 10，浅色画布色、深色 elevated ----
      [
        'oc-trigger',
        'inline-flex items-center justify-between gap-1.5 h-7 min-h-7 px-2.5 rounded-md bg-[var(--oc-control-surface)] text-foreground text-ui-label whitespace-nowrap shadow-button outline-none hover:bg-interactive-hover data-[popup-open]:bg-interactive-active',
      ],

      // ---- 输入框：32 / r7 / elevated 底 ----
      [
        'oc-field',
        'h-8 w-full min-w-0 rounded-md px-2.5 bg-surface-elevated text-foreground text-ui-label shadow-raised outline-none placeholder:text-muted-foreground [&[aria-invalid]]:shadow-[0_0_0_1px_var(--status-error)]',
      ],

      // ---- 分段选择：轨道 9 + 2px 内距/间距 + fg 4%，选中项回画布色再抬起 ----
      ['oc-seg', 'oc-recessed inline-flex flex-wrap items-center gap-0.5 rounded-lg p-0.5'],
      [
        'oc-seg-item',
        'inline-flex items-center justify-center gap-1.5 h-7 px-2.5 rounded-md text-ui-label font-normal text-muted-foreground hover:text-foreground cursor-pointer',
      ],
      [
        'oc-seg-item-active',
        'inline-flex items-center justify-center gap-1.5 h-7 px-2.5 rounded-md text-ui-label text-foreground bg-[var(--oc-control-surface)] shadow-button',
      ],

      // ---- 步进器：32 / r9 / ± 各 36 / 数值区 min-width ----
      [
        'oc-stepper',
        'inline-flex items-stretch h-8 rounded-lg bg-surface-elevated shadow-[0_0_0_1px_var(--interactive-border)] overflow-hidden select-none',
      ],
      [
        'oc-stepper-btn',
        'w-9 flex-none grid place-items-center bg-transparent text-muted-foreground cursor-pointer hover:text-foreground',
      ],
      [
        'oc-stepper-value',
        'flex-1 grid place-items-center px-1.5 text-ui-label [font-variant-numeric:tabular-nums] [min-width:calc(5.75rem+5ch)]',
      ],
      ['oc-unit', 'text-meta text-muted-foreground flex-none'],

      // ---- 设置页文字：字段标签用正文色 13.5/400，不是 muted ----
      ['oc-label', 'text-ui-label font-normal text-foreground'],
      ['oc-section-title', 'text-header font-semibold'],
      ['oc-group-title', 'text-group-title font-medium'],
      ['oc-meta', 'text-meta text-muted-foreground'],

      // ---- 浮层 / 面板 ----
      ['oc-glass', 'oc-glass-popover'],
      ['oc-panel-edge', 'rounded-lg shadow-raised'],
    ],
  }
}
