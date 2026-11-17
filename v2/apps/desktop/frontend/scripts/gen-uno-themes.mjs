/**
 * 把 src/styles/uno/tokens/*.json 编译成 src/styles/uno/generated.ts。
 *
 * 单一数据源：tokens 目录（取自 OpenChamber 主题 JSON，MIT）。
 * 产物是纯 TS 常量，UnoCSS 预设据此生成 CSS 变量与语义色板：
 *   - themeVars：每套主题一份变量表（颜色 + 三级阴影 + 控件底色）
 *   - colorTheme：所有主题共用的扁平颜色映射，值一律 var()，换主题只换变量
 *
 *   node scripts/gen-uno-themes.mjs
 */
import { readFileSync, writeFileSync, readdirSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const tokensDir = join(root, 'src', 'styles', 'uno', 'tokens')
const kebab = (s) => s.replace(/([a-z0-9])([A-Z])/g, '$1-$2').toLowerCase()

/** 三级抬起：深色主题在亮环外再加一圈暗环，否则单一环在暗底上要么消失要么发光 */
const ELEVATION = {
  dark: {
    '--oc-lit': 'rgba(255,255,255,.07)',
    '--oc-elevation-button':
      'inset 0 1px 0 var(--oc-lit), 0 0 0 .5px var(--oc-edge), 0 0 0 1px rgba(0,0,0,.6), 0 1px 1.5px rgba(0,0,0,.45)',
    '--oc-elevation-raised':
      'inset 0 1px 0 var(--oc-lit), 0 0 0 .5px var(--oc-edge), 0 0 0 1px rgba(0,0,0,.6), 0 1px 2px -.5px rgba(0,0,0,.5), 0 4px 10px -2px rgba(0,0,0,.55)',
    '--oc-elevation-floating':
      'inset 0 1px 0 var(--oc-lit), 0 0 0 .5px var(--oc-edge), 0 0 0 1px rgba(0,0,0,.6), 0 1px 2px -.5px rgba(0,0,0,.5), 0 4px 10px -2px rgba(0,0,0,.55), 0 14px 28px -6px rgba(0,0,0,.6)',
  },
  light: {
    '--oc-lit': 'rgba(255,255,255,.9)',
    '--oc-elevation-button':
      'inset 0 1px 0 var(--oc-lit), 0 0 0 .5px rgba(0,0,0,.10), 0 1px 1.5px rgba(0,0,0,.08)',
    '--oc-elevation-raised':
      'inset 0 1px 0 var(--oc-lit), 0 0 0 .5px rgba(0,0,0,.10), 0 1px 2px -.5px rgba(0,0,0,.06), 0 4px 10px -2px rgba(0,0,0,.07)',
    '--oc-elevation-floating':
      'inset 0 1px 0 var(--oc-lit), 0 0 0 .5px rgba(0,0,0,.10), 0 1px 2px -.5px rgba(0,0,0,.08), 0 4px 10px -2px rgba(0,0,0,.11), 0 14px 28px -6px rgba(0,0,0,.12)',
  },
}

const files = readdirSync(tokensDir)
  .filter((f) => f.endsWith('.json'))
  .sort()
const themeVars = {}
const themeVariants = {}
const colorPaths = new Set()

for (const file of files) {
  const theme = JSON.parse(readFileSync(join(tokensDir, file), 'utf8'))
  const id = theme.metadata.id
  const vars = {}

  // 颜色：叶子路径 → --family-role
  const walk = (node, path) => {
    for (const [key, value] of Object.entries(node)) {
      if (value && typeof value === 'object') {
        walk(value, [...path, key])
        continue
      }
      const name = [...path, key].map(kebab).join('-')
      vars[`--${name}`] = value
      colorPaths.add(name)
    }
  }
  walk(theme.colors, [])

  // hairline 颜色：前景的 16%（深）/ 14%（浅）
  const variant = theme.metadata.variant === 'light' ? 'light' : 'dark'
  vars['--oc-edge'] =
    variant === 'dark'
      ? 'color-mix(in srgb, var(--surface-foreground) 16%, transparent)'
      : 'color-mix(in srgb, var(--surface-foreground) 14%, transparent)'
  Object.assign(vars, ELEVATION[variant])

  // 主色上的文字：深色主题取画布色，浅色主题取白
  vars['--primary-foreground'] =
    variant === 'light' ? '#ffffff' : theme.colors.surface.background
  colorPaths.add('primary-foreground')

  // 控件底：浅色取画布色（比 elevated 浅），深色取 elevated
  vars['--oc-control-surface'] =
    variant === 'light' ? theme.colors.surface.background : theme.colors.surface.elevated

  themeVars[id] = vars
  themeVariants[id] = variant
}

/**
 * 共用颜色映射：family → 扁平子键。
 * 只保留一层嵌套，因为 'muted' 与 'muted-foreground' 这类前缀关系在深层嵌套里会撞车；
 * base/tokens/highlights/edit 是上游的分组壳，压掉一层。
 *
 * 注意（本地修正）：桶键压平，但**变量名保留分组层级**——上游脚本两处都用压平后的路径，
 * 于是 colorTheme 指向 --syntax-punctuation，而 themeVars 里实际是 --syntax-tokens-punctuation，
 * 引用必然落空。变量名以 themeVars 为准。
 */
const GROUPS = new Set(['base', 'tokens', 'highlights', 'edit'])
const kebabKey = (segments) => segments.map(kebab).join('-')

const nested = {}
for (const file of files) {
  const theme = JSON.parse(readFileSync(join(tokensDir, file), 'utf8'))
  for (const [family, value] of Object.entries(theme.colors)) {
    const bucket = (nested[family] ??= {})
    const walkFamily = (node, keyPath, varPath) => {
      for (const [key, v] of Object.entries(node)) {
        if (v && typeof v === 'object') {
          if (!GROUPS.has(key)) walkFamily(v, [...keyPath, key], [...varPath, key])
          else walkFamily(v, keyPath, [...varPath, key])
          continue
        }
        bucket[kebabKey([...keyPath, key])] = `var(--${kebabKey([...varPath, key])})`
      }
    }
    if (value && typeof value === 'object') walkFamily(value, [], [family])
    else bucket.DEFAULT = `var(--${kebabKey([family])})`
  }
}
if (nested.primary) {
  nested.primary.DEFAULT = 'var(--primary-base)'
  nested.primary.foreground = 'var(--primary-foreground)'
}
nested.background = 'var(--surface-background)'
nested.foreground = 'var(--surface-foreground)'
nested.muted = 'var(--surface-muted)'
nested['muted-foreground'] = 'var(--surface-muted-foreground)'
nested.border = 'var(--interactive-border)'

const ids = Object.keys(themeVars)
const out = `/* 由 scripts/gen-uno-themes.mjs 生成，勿手改。源：src/styles/uno/tokens/*.json（OpenChamber 主题，MIT） */
export const themeIds = ${JSON.stringify(ids)} as const;
export type ThemeId = (typeof themeIds)[number];

export const defaultTheme: ThemeId = ${JSON.stringify(ids.find((id) => id.endsWith('-dark')) ?? ids[0])};

export const themeVars: Record<ThemeId, Record<string, string>> = ${JSON.stringify(themeVars, null, 2)};

export const themeVariants: Record<ThemeId, 'light' | 'dark'> = ${JSON.stringify(themeVariants, null, 2)};

export const colorTheme = ${JSON.stringify(nested, null, 2)} as const;
`
writeFileSync(join(root, 'src', 'styles', 'uno', 'generated.ts'), out, 'utf8')

const total = Object.values(themeVars).reduce((n, v) => n + Object.keys(v).length, 0)
console.log(
  `themes: ${ids.join(', ')}`,
)
console.log(
  `颜色变量 ${colorPaths.size} 个 × ${ids.length} 套 = ${total} 条；generated.ts ${out.length} 字节`,
)
