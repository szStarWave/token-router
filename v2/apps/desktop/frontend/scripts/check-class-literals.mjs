/**
 * 类名必须是完整字面量：UnoCSS 靠静态扫描抽类名，
 * `className={`app-x${cond ? ' app-y' : ''}`}` 这类拼接抽不到（app-x 会变成
 * "app-x${cond"），样式在运行时静默丢失。
 *
 * 正确写法：整串分支写成完整字面量。
 *   className={cond ? 'app-x app-y' : 'app-x'}
 *
 *   node scripts/check-class-literals.mjs
 */
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(fileURLToPath(new URL('.', import.meta.url)), '..')
const srcDir = join(root, 'src')

function walk(dir) {
  const out = []
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry)
    if (statSync(path).isDirectory()) out.push(...walk(path))
    else if (/\.(tsx|ts|jsx|js)$/.test(entry)) out.push(path)
  }
  return out
}

const violations = []
for (const file of walk(srcDir)) {
  const lines = readFileSync(file, 'utf8').split(/\r?\n/)
  lines.forEach((line, index) => {
    // className={`…`} —— 模板字面量
    if (/className=\{`/.test(line)) {
      violations.push(`${relative(root, file)}:${index + 1}  ${line.trim()}`)
    }
  })
}

if (violations.length > 0) {
  console.error('类名不能是模板字面量（UnoCSS 抽不到）：')
  for (const v of violations) console.error('  ' + v)
  process.exit(1)
}
console.log('className 字面量检查通过')
