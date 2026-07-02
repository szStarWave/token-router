#!/usr/bin/env node
/**
 * Bump Token Router version across manifest files.
 * Usage: node scripts/bump_version.mjs <x.y.z>
 */

import { readFileSync, writeFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const REPO_ROOT = join(__dirname, '..')
const SEMVER_RE = /^\d+\.\d+\.\d+$/

const FILES = {
  cargoRoot: join(REPO_ROOT, 'Cargo.toml'),
  cargoDesktop: join(REPO_ROOT, 'desktop', 'src-tauri', 'Cargo.toml'),
  packageJson: join(REPO_ROOT, 'desktop', 'package.json'),
  tauriConf: join(REPO_ROOT, 'desktop', 'src-tauri', 'tauri.conf.json'),
  desktopReadme: join(REPO_ROOT, 'desktop', 'README.md'),
  otaNotes: join(REPO_ROOT, 'docs', 'ota-release-notes.json'),
}

function readCurrentVersion() {
  const text = readFileSync(FILES.cargoRoot, 'utf8')
  const match = text.match(/^version = "([^"]+)"/m)
  if (!match) throw new Error('Could not read current version from Cargo.toml')
  return match[1]
}

function replaceCargoVersion(path, newVersion) {
  const text = readFileSync(path, 'utf8')
  const updated = text.replace(/^version = "[^"]+"/m, `version = "${newVersion}"`)
  if (updated === text) throw new Error(`Failed to update version in ${path}`)
  writeFileSync(path, updated, 'utf8')
}

function replaceJsonVersion(path, newVersion) {
  const text = readFileSync(path, 'utf8')
  const updated = text.replace(/"version"\s*:\s*"[^"]+"/, `"version": "${newVersion}"`)
  if (updated === text) throw new Error(`Failed to update version in ${path}`)
  writeFileSync(path, updated, 'utf8')
}

function replaceReadmeExamples(path, oldVersion, newVersion) {
  const text = readFileSync(path, 'utf8')
  const updated = text
    .replaceAll(oldVersion, newVersion)
    .replaceAll(`v${oldVersion}`, `v${newVersion}`)
  writeFileSync(path, updated, 'utf8')
}

function ensureOtaNotes(path, newVersion) {
  const doc = JSON.parse(readFileSync(path, 'utf8'))
  const versions = doc.versions ?? (doc.versions = {})
  const placeholder = {
    'zh-CN': ['版本更新'],
    'en-US': ['Version update'],
  }
  for (const key of [newVersion, `v${newVersion}`]) {
    if (!versions[key]) versions[key] = placeholder
  }
  writeFileSync(path, `${JSON.stringify(doc, null, 2)}\n`, 'utf8')
}

function refreshLockfiles() {
  for (const cwd of [REPO_ROOT, join(REPO_ROOT, 'desktop', 'src-tauri')]) {
    const result = spawnSync('cargo', ['generate-lockfile'], { cwd, stdio: 'inherit' })
    if (result.status !== 0) process.exit(result.status ?? 1)
  }
}

function main() {
  const arg = process.argv[2]?.trim().replace(/^v/, '')
  if (!arg || !SEMVER_RE.test(arg)) {
    console.error('Usage: bump_version.mjs <x.y.z>')
    process.exit(1)
  }

  const oldVersion = readCurrentVersion()
  if (oldVersion === arg) {
    console.log(`Version already ${arg}`)
    return
  }

  console.log(`Bumping version: ${oldVersion} -> ${arg}`)

  replaceCargoVersion(FILES.cargoRoot, arg)
  replaceCargoVersion(FILES.cargoDesktop, arg)
  replaceJsonVersion(FILES.packageJson, arg)
  replaceJsonVersion(FILES.tauriConf, arg)
  replaceReadmeExamples(FILES.desktopReadme, oldVersion, arg)
  ensureOtaNotes(FILES.otaNotes, arg)
  refreshLockfiles()

  console.log('Updated:')
  for (const path of Object.values(FILES)) {
    console.log(`  - ${path.replace(`${REPO_ROOT}\\`, '').replace(`${REPO_ROOT}/`, '')}`)
  }
  console.log('  - Cargo.lock')
  console.log('  - desktop/src-tauri/Cargo.lock')
}

main()
