/**
 * config-draft-diff 单测：改动清单（应用栏计数 / 冲突面板 / 跨服务对比共用）。
 */
import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { ConfigDocument } from '../api/types'
import { draftFromDocument } from './config-edit'
import { diffDrafts, deepEqual, deepEqualLoose, type DraftChange } from './config-draft-diff'

function docFrom(partial: Record<string, unknown>, revision = 'rev-base'): ConfigDocument {
  return {
    schema_version: 1,
    revision,
    editable: true,
    document: {
      schema_version: 1,
      default_virtual_model: 'auto',
      server: {
        listen: '127.0.0.1:17621',
        auth_enabled: false,
        auth_mode: 'inbound_key',
        admin_token: '',
        inbound_key: '',
      },
      profiles: {
        default: {
          mode: 'cascade',
          stream_escalation: 'next_turn',
          strength_order: [],
          difficulty_weight_micros: 0,
          hard_signal_weight_micros: 0,
          unknown_price_policy: 'quality_allow',
        },
      },
      candidates: [],
      virtual_models: [
        {
          provider: 'token-router',
          id: 'auto',
          name: 'Auto',
          profile: 'default',
          candidate_pool: [],
          strategies: ['baseline'],
        },
      ],
      ...partial,
    },
    secrets: [],
    warnings: [],
  } as unknown as ConfigDocument
}

const KINDS = (changes: DraftChange[]) => changes.map((change) => `${change.kind}:${change.path}`)

test('相同草稿返回空清单（deepEqual 内容级比较）', () => {
  const document = docFrom({})
  const a = draftFromDocument(document)
  const b = draftFromDocument(docFrom({}, 'rev-other'))
  assert.deepEqual(diffDrafts(a, b), [])
  assert.ok(deepEqual(a, b))
})

test('server.listen 与默认入口的改动被列出', () => {
  const before = draftFromDocument(docFrom({}))
  const after = draftFromDocument(docFrom({}))
  after.server.listen = '127.0.0.1:17777'
  after.defaultVirtualModel = 'coder'
  const kinds = KINDS(diffDrafts(before, after))
  assert.ok(kinds.includes('changed:server.listen'))
  assert.ok(kinds.includes('changed:default_virtual_model'))
})

test('候选增删改：按 id 对齐，id 不变只列字段级改动', () => {
  const before = draftFromDocument(
    docFrom({
      candidates: [
        { id: 'a', tier: 'local', strength: 20, provider: 'openai', base_url: 'http://x/v1', model: 'm', window: 32768 },
      ],
    }),
  )
  const after = draftFromDocument(
    docFrom({
      candidates: [
        { id: 'a', tier: 'local', strength: 90, provider: 'openai', base_url: 'http://x/v1', model: 'm', window: 32768 },
        { id: 'b', tier: 'cloud', strength: 10, provider: 'openai', base_url: 'http://y/v1', model: 'n', window: 8192 },
      ],
    }),
  )
  const kinds = KINDS(diffDrafts(before, after))
  assert.ok(kinds.includes('added:candidates[b]'))
  assert.ok(kinds.includes('changed:candidates[a].strength'))
  assert.ok(!kinds.some((kind) => kind.startsWith('removed')))

  const removed = draftFromDocument(docFrom({ candidates: [] }))
  const kindsRemoved = KINDS(diffDrafts(after, removed))
  assert.ok(kindsRemoved.includes('removed:candidates[a]'))
  assert.ok(kindsRemoved.includes('removed:candidates[b]'))
})

test('候选凭据引用改动被列出（引用可见，不含明文语义）', () => {
  const before = draftFromDocument(
    docFrom({
      candidates: [
        { id: 'a', tier: 'local', strength: 20, provider: 'openai', base_url: 'http://x/v1', model: 'm', window: 32768 },
      ],
    }),
  )
  const after = draftFromDocument(docFrom({}))
  after.candidates = [
    {
      ...before.candidates[0],
      credential: 'env:NEW_KEY',
      price: null,
    },
  ]
  const kinds = KINDS(diffDrafts(before, after))
  assert.ok(kinds.includes('changed:candidates[a].credential'))
})

test('虚拟模型候选池改动被列出', () => {
  const before = draftFromDocument(docFrom({}))
  const after = draftFromDocument(docFrom({}))
  after.virtualModels[0].candidate_pool = ['a', 'b']
  const kinds = KINDS(diffDrafts(before, after))
  assert.ok(kinds.includes('changed:virtual_models[auto].candidate_pool'))
})

test('虚拟模型的组判断与规则改动被列出', () => {
  const before = draftFromDocument(docFrom({}))
  const after = draftFromDocument(docFrom({}))
  const model = after.virtualModels[0]
  model.judge = {
    ...model.judge,
    mode: 'active',
    deadline_ms: 500,
    service: { ...model.judge.service, endpoint: 'https://judge.example.com/v1', model: 'judge-small' },
  }
  model.rules = [{ id: 'r1', intent: '代码生成', prefer: 'a' }]

  const kinds = KINDS(diffDrafts(before, after))
  assert.ok(kinds.includes('changed:virtual_models[auto].judge.mode'))
  assert.ok(kinds.includes('changed:virtual_models[auto].judge.deadline_ms'))
  assert.ok(kinds.includes('changed:virtual_models[auto].judge.service.endpoint'))
  assert.ok(kinds.includes('changed:virtual_models[auto].judge.service.model'))
  assert.ok(kinds.includes('added:virtual_models[auto].rules[r1]'))
})

test('profile 与透传段改动被列出', () => {
  const before = draftFromDocument(docFrom({ judge: { mode: 'shadow' } }))
  const after = draftFromDocument(docFrom({ judge: { mode: 'enforce' } }))
  after.profiles.default.mode = 'single'
  const kinds = KINDS(diffDrafts(before, after))
  assert.ok(kinds.includes('changed:profiles.default.mode'))
  assert.ok(kinds.includes('changed:judge'))
})

test('deepEqualLoose：null 与 undefined / 缺失键等价，其余差异仍能识别', () => {
  assert.equal(deepEqualLoose({ a: null }, { a: undefined }), true)
  assert.equal(deepEqualLoose({ a: 1 }, { a: 1, b: undefined }), true)
  assert.equal(deepEqualLoose({ a: 1 }, { a: 2 }), false)
  assert.equal(deepEqualLoose([1, 2], [1, 2]), true)
  assert.equal(deepEqualLoose([1, 2], [1]), false)
})
