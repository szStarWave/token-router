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
  emptyVirtualModel,
  defaultVirtualModelOf,
  virtualModelSummaries,
  systemOneEndpoint,
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
        caps: ['快速', '低成本'],
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
  assert.deepEqual(draft.candidates[0].caps, ['快速', '低成本'])
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

test('候选级关闭思考与输出上限可往返', () => {
  const draft = draftFromDocument(SAMPLE)
  const candidate = draft.candidates[0]
  candidate.disable_thinking = true
  candidate.max_tokens = 512

  const toml = draftToToml(draft)
  assert.match(toml, /disable_thinking = true/)
  assert.match(toml, /max_tokens = 512/)

  const reparsed = reparse(toml)
  const candidates = reparsed.candidates as Array<Record<string, unknown>>
  assert.equal(candidates[0].disable_thinking, true)
  assert.equal(candidates[0].max_tokens, 512)

  candidate.disable_thinking = false
  candidate.max_tokens = null
  const clean = draftToToml(draft)
  assert.ok(!clean.includes('disable_thinking'), '关闭后不写字段')
  assert.ok(!/^max_tokens = /m.test(clean), '清空后不写字段')
})

test('候选的 price/proxy/headers/input/thinking_levels 应用后不丢', () => {
  const draft = draftFromDocument(SAMPLE)
  const reparsed = reparse(draftToToml(draft))
  const candidates = reparsed.candidates as Array<Record<string, unknown>>

  assert.deepEqual(candidates[0].input, ['text'])
  assert.deepEqual(candidates[0].thinking_levels, ['low'])
  assert.deepEqual(candidates[0].caps, ['快速', '低成本'])

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
    ...emptyVirtualModel(),
    id: 'broken',
    profile: 'nonexistent',
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

test('default_virtual_model 缺失按“未设置”处理：不报假错、不写空串', () => {
  const copy = structuredClone(SAMPLE) as unknown as { document: Record<string, unknown> }
  delete copy.document.default_virtual_model
  const draft = draftFromDocument(copy as unknown as ConfigDocument)
  assert.equal(draft.defaultVirtualModel, '')
  assert.ok(
    !localIssues(draft).some((issue) => issue.path === 'default_virtual_model'),
    '缺失时后端会自动挑第一个启用组，前端不应报错',
  )
  const toml = draftToToml(draft)
  assert.ok(!toml.includes('default_virtual_model'), '空串必须省略，不能写成 ""')
})

test('default_virtual_model 置空后保存不写出空串', () => {
  const draft = draftFromDocument(SAMPLE)
  draft.defaultVirtualModel = ''
  const reparsed = reparse(draftToToml(draft))
  assert.ok(!('default_virtual_model' in reparsed))
})

test('virtualModelSummaries / defaultVirtualModelOf 读只读文档', () => {
  const summaries = virtualModelSummaries(SAMPLE.document)
  assert.deepEqual(summaries, [
    { id: 'auto', name: 'Auto', candidateCount: 2, enabled: true },
  ])
  assert.equal(defaultVirtualModelOf(SAMPLE.document), 'auto')
  assert.equal(defaultVirtualModelOf({}), null)
  assert.deepEqual(virtualModelSummaries(null), [])
})

test('组级关闭思考写进配置并可往返', () => {
  const draft = draftFromDocument(SAMPLE)
  draft.virtualModels[0].disableThinking = true
  const toml = draftToToml(draft)
  assert.ok(toml.includes('disable_thinking = true'), toml)
  const reparsed = reparse(toml)
  const models = reparsed.virtual_models as Array<Record<string, unknown>>
  assert.equal(models[0].disable_thinking, true)
  const roundTrip = draftFromDocument({ document: reparsed } as unknown as ConfigDocument)
  assert.equal(roundTrip.virtualModels[0].disableThinking, true)

  // 关闭再打开：字段消失，不写 `disable_thinking = false`。
  roundTrip.virtualModels[0].disableThinking = false
  assert.ok(!draftToToml(roundTrip).includes('disable_thinking'))
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

// ---------------------------------------------------------------------------
// 路由组判断 / 规则（[virtual_models.judge] / [[virtual_models.rules]]）
// ---------------------------------------------------------------------------

const GROUP_JUDGE = {
  mode: 'active',
  max_calls_per_turn: 2,
  max_calls_per_minute: 30,
  max_calls_per_session: 200,
  deadline_ms: 350,
  service: {
    kind: 'llm',
    endpoint: 'https://judge.example.com/v1',
    model: 'judge-small',
    service_id: 'group-judge',
    revision: 3,
    credential: 'env:JUDGE_KEY',
    timeout_ms: 500,
    structured_output: 'json_schema',
    temperature: 0.1,
    max_output_tokens: 128,
    disable_thinking: true,
    currency: 'USD',
    input_micros_per_mtok: 150,
    output_micros_per_mtok: 600,
    request_micros: 7,
  },
}

const GROUP_RULES = [
  { id: 'r1', intent: '代码生成', prefer: 'local-fast' },
  { id: 'r2', intent: '长文分析', prefer: 'cloud' },
]

/** 在 SAMPLE 的第一个虚拟模型上挂组判断与规则。 */
function documentWithGroup(judge: Record<string, unknown> = GROUP_JUDGE, rules: unknown[] = GROUP_RULES): ConfigDocument {
  const copy = structuredClone(SAMPLE) as unknown as { document: Record<string, unknown> }
  const models = copy.document.virtual_models as Array<Record<string, unknown>>
  models[0].judge = judge
  models[0].rules = rules
  return copy as unknown as ConfigDocument
}

test('旧配置没有 judge/rules：默认 off、空规则，输出不回写空表', () => {
  const draft = draftFromDocument(SAMPLE)
  const model = draft.virtualModels[0]
  assert.equal(model.judge.mode, 'off')
  assert.equal(model.judge.service.timeout_ms, 300)
  assert.equal(model.judge.service.kind, 'llm')
  assert.deepEqual(model.rules, [])
  assert.equal(model.judgeConfigured, false)
  assert.equal(model.rulesConfigured, false)

  const toml = draftToToml(draft)
  assert.ok(!toml.includes('[virtual_models.judge]'))
  assert.ok(!toml.includes('[virtual_models.judge.service]'))
  assert.ok(!toml.includes('rules = []'))
})

test('judge/service 全字段与 rules 可解析且 round-trip 不丢', () => {
  const draft = draftFromDocument(documentWithGroup())
  const model = draft.virtualModels[0]
  assert.equal(model.judgeConfigured, true)
  assert.equal(model.rulesConfigured, true)
  assert.deepEqual(model.rules, GROUP_RULES)

  const toml = draftToToml(draft)
  assert.match(toml, /\[virtual_models\.judge\]/)
  assert.match(toml, /\[virtual_models\.judge\.service\]/)
  assert.match(toml, /\[\[virtual_models\.rules\]\]/)

  const reparsed = reparse(toml)
  const models = reparsed.virtual_models as Array<Record<string, unknown>>
  assert.deepEqual(models[0].judge, GROUP_JUDGE)
  assert.deepEqual(models[0].rules, GROUP_RULES)
})

test('JEV SystemOne：kind=jev 可 round-trip，endpoint 必须是 /v1/systemone', () => {
  const jevJudge = {
    ...structuredClone(GROUP_JUDGE),
    service: {
      ...structuredClone(GROUP_JUDGE.service),
      kind: 'jev',
      endpoint: 'https://api.typesafe.ai/v1/systemone',
    },
  }
  const draft = draftFromDocument(documentWithGroup(jevJudge))
  const model = draft.virtualModels[0]
  assert.equal(model.judge.service.kind, 'jev')
  assert.ok(
    !localIssues(draft).some((issue) => issue.path.includes('judge.service')),
    '合法 JEV 服务不应报错',
  )

  const toml = draftToToml(draft)
  assert.match(toml, /kind = "jev"/)
  const reparsed = reparse(toml)
  const models = reparsed.virtual_models as Array<Record<string, unknown>>
  assert.equal(
    ((models[0].judge as Record<string, unknown>).service as Record<string, unknown>).kind,
    'jev',
  )

  model.judge.service.endpoint = 'https://api.typesafe.ai/v1'
  assert.ok(
    localIssues(draft).some((issue) => issue.code === 'invalid_systemone_endpoint'),
    '非 SystemOne 路径应报错',
  )
})

test('systemOneEndpoint 把供应商 base_url 归一化为 /v1/systemone', () => {
  assert.equal(
    systemOneEndpoint('https://api.siliconflow.cn/v1'),
    'https://api.siliconflow.cn/v1/systemone',
  )
  assert.equal(
    systemOneEndpoint('https://api.siliconflow.cn/v1/'),
    'https://api.siliconflow.cn/v1/systemone',
  )
  assert.equal(
    systemOneEndpoint('https://api.siliconflow.cn/v1/systemone'),
    'https://api.siliconflow.cn/v1/systemone',
  )
  assert.equal(
    systemOneEndpoint('https://api.example.com'),
    'https://api.example.com/v1/systemone',
  )
  assert.equal(systemOneEndpoint('  '), '')
})

test('显式 off 的 judge 表保存时保留', () => {
  const explicitOff = {
    ...structuredClone(GROUP_JUDGE),
    mode: 'off',
  }
  const draft = draftFromDocument(documentWithGroup(explicitOff, []))
  const toml = draftToToml(draft)
  assert.match(toml, /\[virtual_models\.judge\]/)
  assert.match(toml, /mode = "off"/)
  const reparsed = reparse(toml)
  const models = reparsed.virtual_models as Array<Record<string, unknown>>
  assert.equal((models[0].judge as Record<string, unknown>).mode, 'off')
})

test('有规则但判断 off 报错；active 校验服务；prefer 越界与重复 id 报错', () => {
  const draft = draftFromDocument(SAMPLE)
  const model = draft.virtualModels[0]
  model.rules = [{ id: 'r1', intent: '代码生成', prefer: 'local-fast' }]
  model.rulesConfigured = true

  const offIssues = localIssues(draft)
  assert.ok(offIssues.some((issue) => issue.code === 'rules_require_judge'))
  assert.ok(offIssues.some((issue) => issue.path === 'virtual_models.0.judge.mode'))

  model.judge.mode = 'active'
  assert.ok(!localIssues(draft).some((issue) => issue.code === 'rules_require_judge'))
  assert.ok(localIssues(draft).some((issue) => issue.code === 'missing_judge_model'))

  model.judge.service.endpoint = 'https://judge.example.com/v1'
  model.judge.service.model = 'judge-small'
  assert.ok(
    !localIssues(draft).some((issue) => issue.path.startsWith('virtual_models.0.judge.service.')),
    'active 且 endpoint/model 齐全时判断服务本身不应报错',
  )

  model.rules.push({ id: 'r1', intent: '', prefer: 'ghost' })
  const issues = localIssues(draft)
  assert.ok(issues.some((issue) => issue.path === 'virtual_models.0.rules.1.prefer' && issue.code === 'prefer_not_in_pool'))
  assert.ok(issues.some((issue) => issue.path === 'virtual_models.0.rules.1.id' && issue.code === 'duplicate'))
  assert.ok(issues.some((issue) => issue.path === 'virtual_models.0.rules.1.intent' && issue.code === 'empty'))

  model.judge.service.credential = 'sk-secret'
  assert.ok(
    localIssues(draft).some(
      (issue) => issue.path === 'virtual_models.0.judge.service.credential' && issue.code === 'literal_secret',
    ),
  )
})

test('无规则 + active 合法（判断模型读候选能力信息）', () => {
  const draft = draftFromDocument(SAMPLE)
  const model = draft.virtualModels[0]
  model.judge = {
    ...model.judge,
    mode: 'active',
    service: {
      ...model.judge.service,
      endpoint: 'https://judge.example.com/v1',
      model: 'judge-small',
    },
  }
  const issues = localIssues(draft).filter((issue) => issue.path.startsWith('virtual_models.0.judge'))
  assert.deepEqual(issues, [])
})
