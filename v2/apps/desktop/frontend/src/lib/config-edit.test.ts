/**
 * config-edit 单测（§7.4）：draft 往返、引用检查、本地校验、TOML 生成。
 */
import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { ConfigDocument } from '../api/types'
import {
  draftFromDocument,
  draftToToml,
  referencesOf,
  localIssues,
  issuesByPath,
  emptyCandidate,
} from './config-edit'

const SAMPLE: ConfigDocument = {
  schema_version: 2,
  revision: 'abc123',
  editable: true,
  document: {
    default_virtual_model: 'auto',
    server: { listen: '127.0.0.1:17621', auth_enabled: false, auth_mode: 'token_store' },
    profiles: {
      default: { mode: 'cascade', strength_order: ['local'] },
    },
    candidates: [
      { id: 'local-fast', tier: 'local', strength: 20, provider: 'openai', base_url: 'http://127.0.0.1:11434/v1', model: 'qwen', window: 32768, enabled: true },
      { id: 'cloud', tier: 'cloud', strength: 80, provider: 'openai', base_url: 'https://api.example.com/v1', model: 'gpt', window: 128000, credential: 'env:CLOUD_KEY', enabled: true },
    ],
    virtual_models: [
      { provider: 'router', id: 'auto', name: 'Auto', profile: 'default', candidate_pool: ['local-fast', 'cloud'], strategies: ['baseline'] },
    ],
  },
  secrets: [],
  warnings: [],
} as unknown as ConfigDocument

test('draftFromDocument 提取候选与引用', () => {
  const draft = draftFromDocument(SAMPLE)
  assert.equal(draft.candidates.length, 2)
  assert.equal(draft.candidates[0].id, 'local-fast')
  assert.equal(draft.candidates[1].credential, 'env:CLOUD_KEY')
  assert.equal(draft.virtualModels[0].candidate_pool.length, 2)
  assert.ok(draft.profiles.default)
})

test('referencesOf 找出引用候选的虚拟模型', () => {
  const draft = draftFromDocument(SAMPLE)
  assert.deepEqual(referencesOf(draft, 'cloud'), ['auto'])
  assert.deepEqual(referencesOf(draft, 'nonexistent'), [])
})

test('localIssues 拒绝字面量凭据与坏 URL', () => {
  const draft = draftFromDocument(SAMPLE)
  draft.candidates.push({ ...emptyCandidate(), id: 'bad', base_url: 'ftp://x', credential: 'sk-secret' })
  const issues = localIssues(draft)
  const codes = issues.map((issue) => issue.code)
  assert.ok(codes.includes('invalid_url'), 'bad base_url must be flagged')
  assert.ok(codes.includes('literal_secret'), 'literal credential must be flagged')
})

test('localIssues 缺 profile / 空候选池', () => {
  const draft = draftFromDocument(SAMPLE)
  draft.virtualModels.push({
    id: 'broken',
    provider: 'router',
    name: '',
    profile: 'nonexistent',
    candidate_pool: [],
    strategies: ['baseline'],
  })
  const codes = localIssues(draft).map((issue) => issue.code)
  assert.ok(codes.includes('missing_profile'))
  assert.ok(codes.includes('empty_pool'))
})

test('issuesByPath 分组', () => {
  const grouped = issuesByPath([
    { path: 'candidates[0].id', code: 'duplicate', message: 'dup' },
    { path: 'candidates[0].id', code: 'empty', message: 'empty' },
    { path: 'server.listen', code: 'invalid', message: 'bad' },
  ])
  assert.equal(grouped.get('candidates[0].id')?.length, 2)
  assert.equal(grouped.get('server.listen')?.length, 1)
})

test('draftToToml 生成可解析的 TOML（凭据引用保留）', () => {
  const draft = draftFromDocument(SAMPLE)
  const toml = draftToToml(draft)
  assert.match(toml, /schema_version = 2/)
  assert.match(toml, /credential = "env:CLOUD_KEY"/)
  assert.match(toml, /candidate_pool = \["local-fast", "cloud"\]/)
  assert.ok(!toml.includes('sk-'), 'no secret values in output')
})
