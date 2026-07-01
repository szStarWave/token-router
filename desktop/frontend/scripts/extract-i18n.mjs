import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const html = fs.readFileSync(path.join(__dirname, '../../index.html'), 'utf8')
const m = html.match(/const I18N = (\{[\s\S]*?\n  \});/)
if (!m) throw new Error('I18N not found')
// eslint-disable-next-line no-eval
const dict = eval(`(${m[1]})`)
const out = `export type Locale = 'zh' | 'en'
export type I18nDict = Record<string, string>
export const DICT: Record<Locale, I18nDict> = ${JSON.stringify(dict, null, 2)}
export const DEFAULT_LOCALE: Locale = 'zh'
export function t(locale: Locale, key: string, vars?: Record<string, string | number>): string {
  let s = DICT[locale]?.[key] ?? DICT.zh[key] ?? key
  if (vars) {
    for (const [k, v] of Object.entries(vars)) {
      s = s.replace(new RegExp(\`{\${k}}\`, 'g'), String(v))
    }
  }
  return s
}
`
fs.mkdirSync(path.join(__dirname, '../src/i18n'), { recursive: true })
fs.writeFileSync(path.join(__dirname, '../src/i18n/dict.ts'), out)
console.log('written', Object.keys(dict.zh).length, 'keys')
