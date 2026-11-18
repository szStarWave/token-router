/**
 * config-edit 单测（§7.4）：draft 往返、未编辑段透传、引用检查、本地校验、TOML 生成。
 *
 * 重点覆盖 P0 回归：
 * - schema_version 取服务端值（内核只接受 1）；
 * - judge/cost_gate/approvals/trust_tiers 与候选 price/proxy/headers 应用后不丢；
 * - 明文密钥占位符不回写。
 */
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { parse as parseToml } from 'smol-toml'
import type { ConfigDocument } from '../api/types'
import {
  draftFromDocument,
  draftToToml,
  referencesOf,
  localIssues,
  issuesByPath,
  emptyCandidate,
} from './config-edit'

/** smol-toml 返回 null-prototype 对象，deepEqual 前转回普通对象。 */
function plain<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

function reparse(toml: string): Record<string, unknown> {
  return plain(parseToml(toml) as Record<string, unknown>)
}

const SAMPLE: ConfigDocument = {
  schema_version: 1,
  revision: 'abc123',
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
        strength_order: ['local', 'cloud'],
        difficulty_weight_micros: 0,
        hard_signal_weight_micros: 0,
        unknown_price_policy: 'quality_allow',
      },
    },
    judge: {
      mode: 'shadow',
      max_calls_per_turn: 1,
      max_calls_per_minute: 30,
      max_calls_per_session: 200,
      deadline_ms: 300,
      service: {
        kind: 'llm',
        endpoint: 'https://judge.example.com/v1',
        model: 'judge-small',
        service_id: 'remote-judge',
        revision: 3,
        credential: 'env:JUDGE_KEY',
        timeout_ms: 500,
        structured_output: 'json_schema',
        temperature: 0.1,
        max_output_tokens: 128,
        currency: 'USD',
        input_micros_per_mtok: 150,
        output_micros_per_mtok: 600,
        request_micros: 0,
      },
    },
    cost_gate: { mode: 'active' },
    approvals: { mode: 'enforce', ttl_secs: 600, min_risk: 'high', fail_closed: true },
    trust_tiers: { trial: 'default' },
    candidates: [
      {
        id: 'local-fast',
        tier: 'local',
        strength: 20,
        provider: 'openai',
        base_url: 'http://127.0.0.1:11434/v1',
        model: 'qwen',
        window: 32768,
        input: ['text'],
        thinking_levels: ['low'],
        enabled: true,
      },
      {
        id: 'cloud',
        tier: 'cloud',
        strength: 80,
        provider: 'openai',
        base_url: 'https://api.example.com/v1',
        model: 'gpt',
        window: 128000,
        credential: 'env:CLOUD_KEY',
        proxy: 'http://127.0.0.1:7890',
        extra_headers: { 'HTTP-Referer': 'https://example.com' },
        price: {
          currency: 'USD',
          input_micros_per_mtok: 1000,
          output_micros_per_mtok: 3000,
          cache_read_micros_per_mtok: 100,
          cache_write_micros_per_mtok: 1500,
          request_micros: 0,
        },
        enabled: true,
      },
    ],
    virtual_models: [
      {
        provider: 'token-router',
        id: 'auto',
        name: 'Auto',
        profile: 'default',
        candidate_pool: ['local-fast', 'cloud'],
        strategies: ['baseline'],
      },
    ],
  },
  secrets: [],
  warnings: [],
} as unknown as ConfigDocument

test('draftFromDocument 提取候选与引用', () => {
  const draft = draftFromDocument(SAMPLE)
  assert.equal(draft.schemaVersion, 1)
  assert.equal(draft.candidates.length, 2)
  assert.equal(draft.candidates[0].id, 'local-fast')
  assert.equal(draft.candidates[1].credential, 'env:CLOUD_KEY')
  assert.equal(draft.candidates[1].proxy, 'http://127.0.0.1:7890')
  assert.deepEqual(draft.candidates[1].extra_headers, [
    { name: 'HTTP-Referer', value: 'https://example.com' },
  ])
  assert.equal(draft.candidates[1].price?.cache_write_micros_per_mtok, 1500)
  assert.equal(draft.virtualModels[0].candidate_pool.length, 2)
  assert.ok(draft.profiles.default)
})

test('未编辑的顶层段原样透传（judge/cost_gate/approvals/trust_tiers）', () => {
  const draft = draftFromDocument(SAMPLE)
  assert.deepEqual(Object.keys(draft.passthrough).sort(), [
    'approvals',
    'cost_gate',
    'judge',
    'trust_tiers',
  ])

  const reparsed = reparse(draftToToml(draft))
  const judge = reparsed.judge as Record<string, unknown>
  const service = judge.service as Record<string, unknown>
  assert.equal(judge.mode, 'shadow')
  assert.equal(service.kind, 'llm')
  assert.equal(service.endpoint, 'https://judge.example.com/v1')
  assert.equal(service.credential, 'env:JUDGE_KEY')
  assert.equal(service.revision, 3)
  assert.equal(service.temperature, 0.1)
  assert.deepEqual(reparsed.cost_gate, { mode: 'active' })
  assert.deepEqual(reparsed.approvals, {
    mode: 'enforce',
    ttl_secs: 600,
    min_risk: 'high',
    fail_closed: true,
  })
  assert.deepEqual(reparsed.trust_tiers, { trial: 'default' })
})

test('候选的 price/proxy/headers/input/thinking_levels 应用后不丢', () => {
  const draft = draftFromDocument(SAMPLE)
  const reparsed = reparse(draftToToml(draft))
  const candidates = reparsed.candidates as Array<Record<string, unknown>>

  assert.deepEqual(candidates[0].input, ['text'])
  assert.deepEqual(candidates[0].thinking_levels, ['low'])

  assert.equal(candidates[1].proxy, 'http://127.0.0.1:7890')
  assert.deepEqual(candidates[1].extra_headers, { 'HTTP-Referer': 'https://example.com' })
  assert.deepEqual(candidates[1].price, {
    currency: 'USD',
    input_micros_per_mtok: 1000,
    output_micros_per_mtok: 3000,
    cache_read_micros_per_mtok: 100,
    cache_write_micros_per_mtok: 1500,
    request_micros: 0,
  })
})

test('schema_version 跟随服务端文档，不写死', () => {
  const draft = draftFromDocument(SAMPLE)
  assert.match(draftToToml(draft), /^schema_version = 1$/m)

  draft.schemaVersion = 7
  assert.match(draftToToml(draft), /^schema_version = 7$/m)
})

test('referencesOf 找出引用候选的虚拟模型', () => {
  const draft = draftFromDocument(SAMPLE)
  assert.deepEqual(referencesOf(draft, 'cloud'), ['auto'])
  assert.deepEqual(referencesOf(draft, 'nonexistent'), [])
})

test('localIssues 拒绝字面量凭据与坏 URL', () => {
  const draft = draftFromDocument(SAMPLE)
  draft.candidates.push({ ...emptyCandidate(), id: 'bad', base_url: 'ftp://x', credential: 'sk-secret' })
  const codes = localIssues(draft).map((issue) => issue.code)
  assert.ok(codes.includes('invalid_url'), 'bad base_url must be flagged')
  assert.ok(codes.includes('literal_secret'), 'literal credential must be flagged')
})

test('localIssues 拒绝明文密钥占位符回写', () => {
  const redacted = structuredClone(SAMPLE) as unknown as { document: Record<string, unknown> }
  const candidates = redacted.document.candidates as Array<Record<string, unknown>>
  candidates[1].credential = 'redacted:literal'
  const draft = draftFromDocument(redacted as unknown as ConfigDocument)
  const issues = localIssues(draft)
  assert.ok(
    issues.some((issue) => issue.code === 'literal_secret_roundtrip'),
    'redacted placeholder must block save',
  )
})

test('localIssues 缺 profile / 空候选池 / 缺档位', () => {
  const draft = draftFromDocument(SAMPLE)
  draft.virtualModels.push({
    id: 'broken',
    provider: 'token-router',
    name: '',
    profile: 'nonexistent',
    candidate_pool: [],
    strategies: ['baseline'],
    thinking_levels: [],
    context_window: null,
    max_tokens: null,
    input: [],
  })
  const codes = localIssues(draft).map((issue) => issue.code)
  assert.ok(codes.includes('missing_profile'))
  assert.ok(codes.includes('empty_pool'))

  draft.profiles.default.strength_order = ['local']
  assert.ok(localIssues(draft).some((issue) => issue.code === 'missing_tier'))
})

test('localIssues 默认虚拟模型不存在', () => {
  const draft = draftFromDocument(SAMPLE)
  draft.defaultVirtualModel = 'ghost'
  assert.ok(localIssues(draft).some((issue) => issue.code === 'missing_virtual_model'))
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
  assert.match(toml, /schema_version = 1/)
  assert.match(toml, /credential = "env:CLOUD_KEY"/)
  assert.ok(!toml.includes('sk-'), 'no secret values in output')
})
