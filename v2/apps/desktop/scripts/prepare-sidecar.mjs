/**
 * prepare-sidecar：把构建好的 token-router-v2 复制为 Tauri externalBin
 * 要求的 per-triple 命名（§12）。
 *
 * 用法：node scripts/prepare-sidecar.mjs [--release]
 */
import { existsSync, mkdirSync, copyFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

const args = process.argv.slice(2)
const profile = args.includes('--release') ? 'release' : 'debug'

const v2Root = resolve(import.meta.dirname, '../../..')
const src =
  profile === 'release'
    ? join(v2Root, 'target', 'release', 'token-router-v2.exe')
    : join(v2Root, 'target', 'debug', 'token-router-v2.exe')

// 目标 triple：优先 TAURI_ENV_TARGET_TRIPLE，否则 Windows x64 默认。
const triple =
  process.env.TAURI_ENV_TARGET_TRIPLE ?? 'x86_64-pc-windows-msvc'

const destDir = resolve(import.meta.dirname, '../src-tauri/sidecar')
const dest = join(destDir, `token-router-v2-${triple}.exe`)

if (!existsSync(src)) {
  console.error(`sidecar source not found: ${src}`)
  console.error('先运行 cargo build -p tr-server')
  process.exit(1)
}

mkdirSync(destDir, { recursive: true })
copyFileSync(src, dest)
console.log(`sidecar ready: ${dest}`)
