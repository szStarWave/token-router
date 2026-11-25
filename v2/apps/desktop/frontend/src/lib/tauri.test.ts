/**
 * tauri 桥单测：supervisor external 语义（外壳动作边界）。
 *
 * - external 缺失（旧宿主）= 未知 → canManageLocally false，不显示本地启停；
 * - external === true（连接外部服务）→ 永不提供本地 start/stop/restart；
 * - external === false（壳托管）→ 提供本地动作；
 * - invokeErrorMessage 对 Tauri 的 string / Error 抛错都有稳定文案
 *   （supervisor_restart 在外部模式返回明确错误，走这条路径展示）。
 */
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { canManageLocally, invokeErrorMessage, type SupervisorStatus } from './tauri'

/** 在 Node 里伪造 Tauri WebView 环境：window.__TAURI_INTERNALS__。 */
function withTauri(internals: Record<string, unknown> | null): void {
  if (internals === null) {
    delete (globalThis as { window?: unknown }).window
    return
  }
  ;(globalThis as { window: Record<string, unknown> }).window = {
    __TAURI_INTERNALS__: internals,
  }
}

function status(overrides: Partial<SupervisorStatus> = {}): SupervisorStatus {
  return {
    state: 'ready',
    listen: '127.0.0.1:17621',
    data_dir: '/tmp/.token-router-v2',
    ...overrides,
  }
}

test('canManageLocally: 非 Tauri（浏览器）一律 false', () => {
  withTauri(null)
  assert.equal(canManageLocally(status({ external: false })), false)
  assert.equal(canManageLocally(null), false)
})

test('canManageLocally: 壳托管（external === false）→ true', () => {
  withTauri({ invoke: () => {} })
  assert.equal(canManageLocally(status({ external: false })), true)
  // stopped/failed 状态同样可管理（允许「启动服务」）。
  assert.equal(canManageLocally(status({ external: false, state: 'stopped' })), true)
})

test('canManageLocally: 外部服务（external === true）→ false，本地动作隐藏', () => {
  withTauri({ invoke: () => {} })
  assert.equal(canManageLocally(status({ external: true })), false)
})

test('canManageLocally: 旧宿主未返回 external（undefined）→ false（未知不动作）', () => {
  withTauri({ invoke: () => {} })
  assert.equal(canManageLocally(status()), false)
  assert.equal(canManageLocally(status({ external: undefined })), false)
})

test('invokeErrorMessage: Tauri 命令 Err(string)（如 external 模式 supervisor_restart）', () => {
  const error = 'restart unavailable in external mode: service is not managed by this shell'
  assert.equal(invokeErrorMessage(error), error)
})

test('invokeErrorMessage: Error 对象取 message', () => {
  assert.equal(
    invokeErrorMessage(new Error('external service unreachable: http://x')),
    'external service unreachable: http://x',
  )
})
