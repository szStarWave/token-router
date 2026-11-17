/**
 * 诊断脚本：确认源码里出现的类名都被 UnoCSS 命中。
 *
 * 重点盯两类：
 *   1. 设计系统自己的类（app-* / oc-*）—— 拼接写法最容易抽不到；
 *   2. 常用原子类（font-mono / text-* 等）—— 白名单核对。
 *
 *   node --import tsx scripts/check-uno-tokens.ts
 */
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { createGenerator } from 'unocss'
import config from '../uno.config'
import { themeVars } from '../src/styles/uno/generated'

const srcDir = join(import.meta.dirname, '..', 'src')

/** 允许出现在 class 里的原子类白名单（前缀式）。 */
const ATOMIC = [
  'font-mono', 'text-muted-foreground', 'text-status-', 'text-ui-label', 'text-meta',
  'text-micro', 'text-code', 'text-section', 'text-header', 'text-foreground',
  'bg-background', 'bg-surface-', 'bg-primary', 'bg-status-', 'accent-primary',
  'rounded-md', 'rounded-lg', 'rounded-xl', 'rounded-full', 'rounded-sm',
  'shadow-button', 'shadow-raised', 'shadow-floating', 'border-border',
  'flex', 'grid', 'block', 'inline-flex', 'hidden', 'truncate', 'items-center',
  'justify-center', 'gap-', 'p-', 'px-', 'py-', 'pt-', 'pb-', 'm-', 'mt-', 'mb-',
  'w-', 'h-', 'size-', 'max-w-', 'min-w-', 'shrink-0', 'flex-1', 'flex-none',
  'overflow-hidden', 'overflow-y-auto', 'overflow-x-auto', 'whitespace-nowrap',
  'list-none', 'list-disc', 'no-underline', 'cursor-pointer', 'tabular-nums',
  'break-all', 'break-words', 'leading-', 'font-medium', 'font-semibold',
  'self-start', 'place-content-center', 'place-items-center', 'uppercase',
  'transition-colors', 'transition-all', 'duration-', 'opacity-', 'absolute',
  'relative', 'fixed', 'z-', 'top-', 'right-', 'bottom-', 'left-', 'col-span-',
]

function walk(dir: string): string[] {
  const out: string[] = []
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry)
    if (statSync(path).isDirectory()) out.push(...walk(path))
    else if (/\.(tsx|ts|jsx|js)$/.test(entry)) out.push(path)
  }
  return out
}

/** 收集源码里出现的类名（字面量字符串里的类列表）。 */
function collectClassNames(): Set<string> {
  const found = new Set<string>()
  for (const file of walk(srcDir)) {
    const text = readFileSync(file, 'utf8')
    // 引号里的字符串（类名都是字面量，模板字面量由 check-class-literals.mjs 负责）
    for (const match of text.matchAll(/'([^'\n]*)'|"([^"\n]*)"/g)) {
      const value = match[1] ?? match[2] ?? ''
      if (!value || value.length > 160) continue
      if (!/[a-z]/.test(value)) continue
      for (const token of value.split(/\s+/)) {
        if (!token) continue
        if (token.startsWith('app-') || token.startsWith('oc-') || token.startsWith('drag-')) {
          found.add(token)
          continue
        }
        if (ATOMIC.some((a) => (a.endsWith('-') ? token.startsWith(a) : token === a))) {
          found.add(token)
        }
      }
    }
  }
  return found
}

const tokens = [...collectClassNames()].sort()

const uno = await createGenerator(config)
const { css, matched } = await uno.generate(new Set(tokens), { preflights: false })
const missing = tokens.filter((t) => !matched.has(t))

console.log(`类名命中 ${matched.size}/${tokens.length}`)
if (missing.length > 0) {
  console.error('未命中（样式会丢失）：')
  for (const t of missing) console.error('  -', t)
  process.exit(1)
}

// 变量体检：产物里引用的产品命名空间变量必须真实存在于主题 token 表，
// 否则（例如换了主题后变量改名）会静默回退成继承色。
const themeVarNames = new Set(
  Object.values(themeVars).flatMap((vars) => Object.keys(vars)),
)
const NAMESPACES = [
  '--surface-',
  '--interactive-',
  '--primary-',
  '--status-',
  '--syntax-',
  '--markdown-',
  '--tools-',
  '--chat-',
  '--pr-',
  '--oc-',
]
const referenced = new Set(
  [...css.matchAll(/var\((--[a-z0-9-]+)/gi)].map((m) => m[1]),
)
const dangling = [...referenced].filter(
  (name) =>
    NAMESPACES.some((ns) => name.startsWith(ns)) && !themeVarNames.has(name),
)
if (dangling.length > 0) {
  console.error('引用了主题里不存在的变量：')
  for (const name of dangling) console.error('  -', name)
  process.exit(1)
}
console.log(`主题变量体检通过（引用 ${referenced.size} 个）`)
