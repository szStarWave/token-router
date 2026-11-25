/**
 * config-draft-persist 单测：allowlist 过滤（明文守卫）、身份匹配、隔离不覆盖。
 *
 * 本期持久化恒 disabled（服务身份不可靠），这里验证的是"如果启用"的安全框架。
 */
import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  toPersistablePayload,
  identityMatches,
  persistDraftIfEnabled,
  quarantineDraft,
  readStoredDraft,
  clearStoredDraft,
  STORAGE_KEYS,
} from './config-draft-persist'

const BASE_DRAFT = {
  schemaVersion: 1,
  defaultVirtualModel: 'auto',
  server: {
    listen: '127.0.0.1:17621',
    auth_enabled: true,
    auth_mode: 'inbound_key',
    admin_token: '',
    inbound_key: 'env:INBOUND',
    not_allowed_field: 'x',
  },
  profiles: {},
  candidates: [{ id: 'a', credential: 'env:KEY' }],
  virtualModels: [],
  passthrough: { judge: { service: { credential: 'env:JUDGE' } } },
  /** allowlist 外的字段（如未来误加的 prompt 输入）必须被剔掉。 */
  userPrompt: 'sensitive prompt content',
}

test('allowlist：顶层只留白名单字段（prompt 等绝不落盘）', () => {
  const payload = toPersistablePayload(BASE_DRAFT)
  assert.ok(!('userPrompt' in payload))
  assert.deepEqual(Object.keys(payload).sort(), [
    'candidates',
    'defaultVirtualModel',
    'passthrough',
    'profiles',
    'schemaVersion',
    'server',
    'virtualModels',
  ])
})

test('server 段剔除未知字段；引用形态凭据保留', () => {
  const payload = toPersistablePayload(BASE_DRAFT)
  const server = payload.server as Record<string, unknown>
  assert.deepEqual(Object.keys(server).sort(), [
    'admin_token',
    'auth_enabled',
    'auth_mode',
    'inbound_key',
    'listen',
  ])
  assert.equal(server.inbound_key, 'env:INBOUND')
})

test('明文凭据守卫：server 明文 key 剔除该字段', () => {
  const payload = toPersistablePayload({
    ...BASE_DRAFT,
    server: { ...BASE_DRAFT.server, inbound_key: 'sk-plaintext-secret' },
  })
  const server = payload.server as Record<string, unknown>
  assert.ok(!('inbound_key' in server), 'plaintext key must not enter storage')
})

test('明文凭据守卫：候选 credential 非引用 → 整条候选剔除', () => {
  const payload = toPersistablePayload({
    ...BASE_DRAFT,
    candidates: [
      { id: 'ok', credential: 'file:creds/a.key' },
      { id: 'bad', credential: 'sk-plaintext' },
    ],
  })
  assert.deepEqual(
    (payload.candidates as Array<{ id: string }>).map((candidate) => candidate.id),
    ['ok'],
  )
})

test('明文凭据守卫：组判断 credential 非引用 → 该 VM 的 judge 段剔除', () => {
  const payload = toPersistablePayload({
    ...BASE_DRAFT,
    virtualModels: [
      {
        id: 'auto',
        rules: [{ id: 'r1', intent: 'code', prefer: 'a' }],
        judge: { mode: 'active', service: { credential: 'sk-plaintext' } },
      },
      {
        id: 'safe',
        judge: { mode: 'active', service: { credential: 'env:JUDGE' } },
      },
    ],
  })
  const models = payload.virtualModels as Array<Record<string, unknown>>
  assert.ok(!('judge' in models[0]), 'plaintext judge credential must not enter storage')
  assert.deepEqual(models[0].rules, [{ id: 'r1', intent: 'code', prefer: 'a' }])
  assert.ok('judge' in models[1], 'reference credential is allowed')
})

test('明文凭据守卫：judge.service.credential 非引用 → passthrough 整段剔除 judge', () => {
  const payload = toPersistablePayload({
    ...BASE_DRAFT,
    passthrough: { judge: { service: { credential: 'sk-plaintext' } }, cost_gate: { mode: 'active' } },
  })
  const passthrough = payload.passthrough as Record<string, unknown>
  assert.ok(!('judge' in passthrough))
  assert.ok('cost_gate' in passthrough)
})

test('identityMatches：serviceId / baseRevision / schemaVersion 三者全等才匹配', () => {
  const identity = { serviceId: 'svc-1', baseRevision: 'rev1', schemaVersion: 1 }
  assert.ok(identityMatches({ identity }, identity))
  assert.ok(!identityMatches({ identity }, { ...identity, baseRevision: 'rev2' }))
  assert.ok(!identityMatches({ identity }, { ...identity, schemaVersion: 2 }))
  assert.ok(!identityMatches({ identity }, { ...identity, serviceId: 'svc-2' }))
})

test('persistDraftIfEnabled：本期恒 disabled，不写任何存储', () => {
  const result = persistDraftIfEnabled(
    { serviceId: 'svc', baseRevision: 'rev', schemaVersion: 1 },
    BASE_DRAFT,
  )
  assert.equal(result.persisted, false)
  assert.equal(readStoredDraft().active, null)
})

test('隔离：身份不匹配的草稿进隔离键；已有隔离内容不被覆盖', () => {
  clearStoredDraft()
  // node 测试环境无 window.localStorage：注入最小 stub 验证隔离逻辑。
  const backing = new Map<string, string>()
  const stub = {
    getItem: (key: string) => backing.get(key) ?? null,
    setItem: (key: string, value: string) => void backing.set(key, value),
    removeItem: (key: string) => void backing.delete(key),
    clear: () => void backing.clear(),
    key: (index: number) => Array.from(backing.keys())[index] ?? null,
    length: backing.size,
  } as Storage
  ;(globalThis as { window?: unknown }).window = { localStorage: stub }
  try {
    const first = {
      identity: { serviceId: 'svc-a', baseRevision: 'rev-a', schemaVersion: 1 },
      draft: { schemaVersion: 1 },
      savedAt: 1,
    }
    const second = {
      identity: { serviceId: 'svc-b', baseRevision: 'rev-b', schemaVersion: 1 },
      draft: { schemaVersion: 1 },
      savedAt: 2,
    }
    assert.ok(quarantineDraft(first))
    assert.ok(!quarantineDraft(second), 'existing quarantine must not be overwritten')
    const stored = readStoredDraft()
    assert.equal(stored.quarantined?.identity.serviceId, 'svc-a')
    assert.equal(stored.active, null)
  } finally {
    clearStoredDraft()
    delete (globalThis as { window?: unknown }).window
    assert.equal(readStoredDraft().quarantined, null)
  }
})

test('存储键命名不含明文语义（允许列表审计用）', () => {
  assert.equal(STORAGE_KEYS.DRAFT_KEY, 'tr2.configDraft.active')
  assert.equal(STORAGE_KEYS.QUARANTINE_KEY, 'tr2.configDraft.quarantine')
})
