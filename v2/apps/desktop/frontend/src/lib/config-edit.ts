/**
 * 配置编辑核心（§7.4）：文档 ↔ 草稿 ↔ TOML；validate、diff、apply。
 *
 * 规则：
 * - 草稿覆盖内核 `ConfigFile` 的全部字段；本模块不编辑的顶层段
 *   （judge/cost_gate/approvals/trust_tiers）原样透传，应用时不丢数据；
 * - `schema_version` 取自服务端文档，不写死（内核只接受 1）；
 * - TOML 由 smol-toml 序列化：手写行拼装会漏字段、漏转义；
 * - 保存前先 validate；无 issue 才允许 apply（CAS If-Match revision）；
 * - 候选删除前检查 virtual_models[].candidate_pool 引用。
 */
import { stringify as stringifyToml } from 'smol-toml'
import type { ConfigDocument, ConfigIssue } from '../api/types'

/** 服务端对明文密钥的占位符；回写等于把密钥改成这串字面量。 */
export const REDACTED_SECRET = 'redacted:literal'

/** 本模块可编辑的顶层键；其余顶层段原样透传。 */
const EDITABLE_TOP_LEVEL = new Set([
  'schema_version',
  'default_virtual_model',
  'server',
  'profiles',
  'candidates',
  'virtual_models',
])

export interface PriceDraft {
  currency: string
  input_micros_per_mtok: number
  output_micros_per_mtok: number
  cache_read_micros_per_mtok: number | null
  cache_write_micros_per_mtok: number | null
  request_micros: number
}

export interface HeaderDraft {
  name: string
  value: string
}

export interface CandidateDraft {
  id: string
  tier: string
  strength: number
  provider: string
  base_url: string
  model: string
  window: number
  credential: string
  proxy: string
  extra_headers: HeaderDraft[]
  input: string[]
  thinking_levels: string[]
  /** 候选级关闭思考：该模型请求一律不发推理参数。 */
  disable_thinking: boolean
  /** 候选级输出上限：请求 max_tokens 取 min(客户端值, 上限)，未给时填上限。 */
  max_tokens: number | null
  /** 能力标签（编译进候选 capability tags，供组判断能力模式参考）。 */
  caps: string[]
  price: PriceDraft | null
  enabled: boolean
}

export type GroupJudgeMode = 'off' | 'active'

/** 组级判断服务：覆盖 agent config `JudgeServiceFile` 全部字段，避免 round-trip 丢配置。 */
export interface JudgeServiceDraft {
  /** 本批前端只暴露 `llm`；旧配置的其他值原样保留，active 时由校验要求改为 llm。 */
  kind: string
  endpoint: string
  model: string
  service_id: string
  revision: number
  credential: string
  timeout_ms: number
  structured_output: string
  temperature: number
  max_output_tokens: number
  /** 判官关闭思考（默认 true；只对 `kind = "llm"` 有意义）。 */
  disable_thinking: boolean
  currency: string
  input_micros_per_mtok: number
  output_micros_per_mtok: number
  request_micros: number
}

/** `[virtual_models.judge]`：路由组自己的判断模型与预算。 */
export interface GroupJudgeDraft {
  mode: GroupJudgeMode
  max_calls_per_turn: number
  max_calls_per_minute: number
  max_calls_per_session: number
  deadline_ms: number
  service: JudgeServiceDraft
}

/** `[[virtual_models.rules]]`：判断模型读取规则与用户消息后选中的规则。 */
export interface GroupRuleDraft {
  id: string
  intent: string
  /** 命中规则后在组候选池中优先的候选 id。 */
  prefer: string
}

export type WindowMode = 'min' | 'max' | 'custom'

export interface VirtualModelDraft {
  id: string
  provider: string
  name: string
  profile: string
  /** 客户端可见/可路由；默认 true。 */
  enabled: boolean
  candidate_pool: string[]
  strategies: string[]
  thinking_levels: string[]
  /** 组级关闭思考：请求一律按 off 处理（剥离推理参数 + 已知厂商关闭开关）。 */
  disableThinking: boolean
  /** 窗口提示来源：最小/最大成员窗口，或显式值。 */
  contextWindowMode: WindowMode
  context_window: number | null
  max_tokens: number | null
  /** `/v1/models` 是否同时列出池内成员（`组/模型`）。 */
  listMembers: boolean
  input: string[]
  judge: GroupJudgeDraft
  rules: GroupRuleDraft[]
  /** 文档是否显式包含 `[virtual_models.judge]`（off 也是显式配置，保存时保留）。 */
  judgeConfigured: boolean
  /** 文档是否显式包含 rules 数组（显式空数组也保留）。 */
  rulesConfigured: boolean
}

export interface ProfileDraft {
  mode: string
  stream_escalation: string
  strength_order: string[]
  difficulty_weight_micros: number
  hard_signal_weight_micros: number
  unknown_price_policy: string
}

export interface ServerDraft {
  listen: string
  auth_enabled: boolean
  auth_mode: string
  /** 服务端可能返回 `redacted:literal`；此时不可回写（见 localIssues）。 */
  admin_token: string
  inbound_key: string
}

export interface ConfigDraft {
  schemaVersion: number
  defaultVirtualModel: string
  server: ServerDraft
  candidates: CandidateDraft[]
  virtualModels: VirtualModelDraft[]
  profiles: Record<string, ProfileDraft>
  /** 未编辑的顶层段（judge/cost_gate/approvals/trust_tiers），原样保留。 */
  passthrough: Record<string, unknown>
}

// ---------------------------------------------------------------------------
// 文档 → 草稿
// ---------------------------------------------------------------------------

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function str(value: unknown, fallback: string): string {
  return typeof value === 'string' ? value : fallback
}

function num(value: unknown, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback
}

function optNum(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function bool(value: unknown, fallback: boolean): boolean {
  return typeof value === 'boolean' ? value : fallback
}

function strArray(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item) => typeof item === 'string') : []
}

function windowModeFrom(raw: Record<string, unknown>): WindowMode {
  const mode = str(raw.context_window_mode, '')
  if (mode === 'min' || mode === 'max' || mode === 'custom') return mode
  return optNum(raw.context_window) !== null ? 'custom' : 'min'
}

function headersFrom(value: unknown): HeaderDraft[] {
  if (!isRecord(value)) return []
  return Object.entries(value).map(([name, header]) => ({
    name,
    value: typeof header === 'string' ? header : String(header),
  }))
}

function priceFrom(value: unknown): PriceDraft | null {
  if (!isRecord(value)) return null
  return {
    currency: str(value.currency, 'USD'),
    input_micros_per_mtok: num(value.input_micros_per_mtok, 0),
    output_micros_per_mtok: num(value.output_micros_per_mtok, 0),
    cache_read_micros_per_mtok: optNum(value.cache_read_micros_per_mtok),
    cache_write_micros_per_mtok: optNum(value.cache_write_micros_per_mtok),
    request_micros: num(value.request_micros, 0),
  }
}

function candidateFrom(value: unknown): CandidateDraft {
  const raw = isRecord(value) ? value : {}
  return {
    id: str(raw.id, ''),
    tier: str(raw.tier, 'local'),
    strength: num(raw.strength, 20),
    provider: str(raw.provider, 'openai'),
    base_url: str(raw.base_url, ''),
    model: str(raw.model, ''),
    window: num(raw.window, 32768),
    credential: str(raw.credential, ''),
    proxy: str(raw.proxy, ''),
    extra_headers: headersFrom(raw.extra_headers),
    input: strArray(raw.input),
    thinking_levels: strArray(raw.thinking_levels),
    disable_thinking: bool(raw.disable_thinking, false),
    max_tokens: optNum(raw.max_tokens),
    caps: strArray(raw.caps),
    price: priceFrom(raw.price),
    enabled: bool(raw.enabled, true),
  }
}

function judgeServiceFrom(value: unknown): JudgeServiceDraft {
  const raw = isRecord(value) ? value : {}
  return {
    kind: str(raw.kind, 'llm'),
    endpoint: str(raw.endpoint, ''),
    model: str(raw.model, ''),
    service_id: str(raw.service_id, 'group-judge'),
    revision: num(raw.revision, 1),
    credential: str(raw.credential, ''),
    timeout_ms: num(raw.timeout_ms, 300),
    structured_output: str(raw.structured_output, 'json_mode'),
    temperature: num(raw.temperature, 0),
    max_output_tokens: num(raw.max_output_tokens, 256),
    disable_thinking: typeof raw.disable_thinking === 'boolean' ? raw.disable_thinking : true,
    currency: str(raw.currency, 'USD'),
    input_micros_per_mtok: num(raw.input_micros_per_mtok, 0),
    output_micros_per_mtok: num(raw.output_micros_per_mtok, 0),
    request_micros: num(raw.request_micros, 0),
  }
}

function groupJudgeFrom(value: unknown): GroupJudgeDraft {
  const raw = isRecord(value) ? value : {}
  return {
    mode: str(raw.mode, 'off') === 'active' ? 'active' : 'off',
    max_calls_per_turn: num(raw.max_calls_per_turn, 1),
    max_calls_per_minute: num(raw.max_calls_per_minute, 30),
    max_calls_per_session: num(raw.max_calls_per_session, 200),
    deadline_ms: num(raw.deadline_ms, 300),
    service: judgeServiceFrom(raw.service),
  }
}

function groupRuleFrom(value: unknown): GroupRuleDraft {
  const raw = isRecord(value) ? value : {}
  return {
    id: str(raw.id, ''),
    intent: str(raw.intent, ''),
    prefer: str(raw.prefer, ''),
  }
}

function virtualModelFrom(value: unknown): VirtualModelDraft {
  const raw = isRecord(value) ? value : {}
  return {
    id: str(raw.id, ''),
    provider: str(raw.provider, 'token-router'),
    name: str(raw.name, ''),
    profile: str(raw.profile, 'default'),
    enabled: bool(raw.enabled, true),
    candidate_pool: strArray(raw.candidate_pool),
    strategies: strArray(raw.strategies),
    thinking_levels: strArray(raw.thinking_levels),
    disableThinking: bool(raw.disable_thinking, false),
    contextWindowMode: windowModeFrom(raw),
    context_window: optNum(raw.context_window),
    max_tokens: optNum(raw.max_tokens),
    listMembers: bool(raw.list_members, false),
    input: strArray(raw.input),
    judge: groupJudgeFrom(raw.judge),
    rules: Array.isArray(raw.rules) ? raw.rules.map(groupRuleFrom) : [],
    judgeConfigured: isRecord(raw.judge),
    rulesConfigured: Array.isArray(raw.rules),
  }
}

function profileFrom(value: unknown): ProfileDraft {
  const raw = isRecord(value) ? value : {}
  return {
    mode: str(raw.mode, 'cascade'),
    stream_escalation: str(raw.stream_escalation, 'next_turn'),
    strength_order: strArray(raw.strength_order),
    difficulty_weight_micros: num(raw.difficulty_weight_micros, 0),
    hard_signal_weight_micros: num(raw.hard_signal_weight_micros, 0),
    unknown_price_policy: str(raw.unknown_price_policy, 'quality_allow'),
  }
}

/** 从 ConfigDocument.document 提取可编辑草稿。 */
export function draftFromDocument(document: ConfigDocument): ConfigDraft {
  const doc = isRecord(document.document) ? document.document : {}
  const candidates = Array.isArray(doc.candidates) ? doc.candidates : []
  const virtualModels = Array.isArray(doc.virtual_models) ? doc.virtual_models : []
  const profilesRaw = isRecord(doc.profiles) ? doc.profiles : {}
  const serverRaw = isRecord(doc.server) ? doc.server : {}

  const profiles: Record<string, ProfileDraft> = {}
  for (const [id, profile] of Object.entries(profilesRaw)) {
    profiles[id] = profileFrom(profile)
  }

  const passthrough: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(doc)) {
    if (!EDITABLE_TOP_LEVEL.has(key)) passthrough[key] = value
  }

  return {
    schemaVersion: num(doc.schema_version, document.schema_version),
    // 缺失 ≠ "auto"：后端在缺省时会自动挑第一个启用组（config.rs），
    // 前端造一个 "auto" 会凭空产生 missing_virtual_model 校验错误。
    defaultVirtualModel: str(doc.default_virtual_model, ''),
    server: {
      listen: str(serverRaw.listen, '127.0.0.1:17621'),
      auth_enabled: bool(serverRaw.auth_enabled, false),
      auth_mode: str(serverRaw.auth_mode, 'inbound_key'),
      admin_token: str(serverRaw.admin_token, ''),
      inbound_key: str(serverRaw.inbound_key, ''),
    },
    profiles,
    candidates: candidates.map(candidateFrom),
    virtualModels: virtualModels.map(virtualModelFrom),
    passthrough,
  }
}

// ---------------------------------------------------------------------------
// 草稿 → TOML
// ---------------------------------------------------------------------------

function headersTo(headers: HeaderDraft[]): Record<string, string> {
  const out: Record<string, string> = {}
  for (const header of headers) {
    const name = header.name.trim()
    if (name) out[name] = header.value
  }
  return out
}

function priceTo(price: PriceDraft): Record<string, unknown> {
  const out: Record<string, unknown> = {
    currency: price.currency,
    input_micros_per_mtok: price.input_micros_per_mtok,
    output_micros_per_mtok: price.output_micros_per_mtok,
  }
  if (price.cache_read_micros_per_mtok !== null) {
    out.cache_read_micros_per_mtok = price.cache_read_micros_per_mtok
  }
  if (price.cache_write_micros_per_mtok !== null) {
    out.cache_write_micros_per_mtok = price.cache_write_micros_per_mtok
  }
  out.request_micros = price.request_micros
  return out
}

function candidateTo(candidate: CandidateDraft): Record<string, unknown> {
  const out: Record<string, unknown> = {
    id: candidate.id,
    tier: candidate.tier,
    strength: candidate.strength,
    provider: candidate.provider,
    base_url: candidate.base_url,
    model: candidate.model,
  }
  if (candidate.credential) out.credential = candidate.credential
  if (candidate.proxy.trim()) out.proxy = candidate.proxy.trim()
  const headers = headersTo(candidate.extra_headers)
  if (Object.keys(headers).length > 0) out.extra_headers = headers
  out.window = candidate.window
  if (candidate.input.length > 0) out.input = candidate.input
  if (candidate.thinking_levels.length > 0) out.thinking_levels = candidate.thinking_levels
  if (candidate.disable_thinking) out.disable_thinking = true
  if (candidate.max_tokens !== null) out.max_tokens = candidate.max_tokens
  if (candidate.caps.length > 0) out.caps = candidate.caps
  if (candidate.price) out.price = priceTo(candidate.price)
  out.enabled = candidate.enabled
  return out
}

function judgeServiceTo(service: JudgeServiceDraft): Record<string, unknown> {
  const out: Record<string, unknown> = {
    kind: service.kind,
    endpoint: service.endpoint,
    model: service.model,
    service_id: service.service_id,
    revision: service.revision,
    timeout_ms: service.timeout_ms,
    structured_output: service.structured_output,
    temperature: service.temperature,
    max_output_tokens: service.max_output_tokens,
    disable_thinking: service.disable_thinking,
    currency: service.currency,
    input_micros_per_mtok: service.input_micros_per_mtok,
    output_micros_per_mtok: service.output_micros_per_mtok,
    request_micros: service.request_micros,
  }
  // Option<String>：空引用与缺省等价，不输出空串。
  if (service.credential) out.credential = service.credential
  return out
}

function groupJudgeTo(judge: GroupJudgeDraft): Record<string, unknown> {
  return {
    mode: judge.mode,
    max_calls_per_turn: judge.max_calls_per_turn,
    max_calls_per_minute: judge.max_calls_per_minute,
    max_calls_per_session: judge.max_calls_per_session,
    deadline_ms: judge.deadline_ms,
    service: judgeServiceTo(judge.service),
  }
}

function groupRuleTo(rule: GroupRuleDraft): Record<string, unknown> {
  return {
    id: rule.id,
    intent: rule.intent,
    prefer: rule.prefer,
  }
}

function virtualModelTo(model: VirtualModelDraft): Record<string, unknown> {
  const out: Record<string, unknown> = {
    provider: model.provider,
    id: model.id,
  }
  if (model.name) out.name = model.name
  if (model.thinking_levels.length > 0) out.thinking_levels = model.thinking_levels
  if (model.disableThinking) out.disable_thinking = true
  if (model.contextWindowMode === 'max') {
    out.context_window_mode = 'max'
  } else if (model.contextWindowMode === 'custom' && model.context_window !== null) {
    out.context_window_mode = 'custom'
    out.context_window = model.context_window
  } else if (model.context_window !== null) {
    // 旧语义：显式值即自定义（不写 mode，保持旧配置写法）。
    out.context_window = model.context_window
  }
  if (model.listMembers) out.list_members = true
  if (model.max_tokens !== null) out.max_tokens = model.max_tokens
  if (model.input.length > 0) out.input = model.input
  out.profile = model.profile
  if (!model.enabled) out.enabled = false
  out.strategies = model.strategies
  out.candidate_pool = model.candidate_pool
  // off + 无规则的新草稿不写空表；显式存在的 judge/rules 原样保留。
  if (model.judgeConfigured || model.judge.mode === 'active') out.judge = groupJudgeTo(model.judge)
  if (model.rulesConfigured || model.rules.length > 0) out.rules = model.rules.map(groupRuleTo)
  return out
}

function profileTo(profile: ProfileDraft): Record<string, unknown> {
  return {
    mode: profile.mode,
    stream_escalation: profile.stream_escalation,
    strength_order: profile.strength_order,
    difficulty_weight_micros: profile.difficulty_weight_micros,
    hard_signal_weight_micros: profile.hard_signal_weight_micros,
    unknown_price_policy: profile.unknown_price_policy,
  }
}

/** 草稿 → TOML（覆盖内核 `ConfigFile` 全字段；未编辑段原样透传）。 */
export function draftToToml(draft: ConfigDraft): string {
  const profiles: Record<string, unknown> = {}
  for (const [id, profile] of Object.entries(draft.profiles)) {
    profiles[id] = profileTo(profile)
  }
  const document: Record<string, unknown> = {
    schema_version: draft.schemaVersion,
    // 空串 = 未设置：必须省略而不是写 `default_virtual_model = ""`
    //（后端会把它当作 Some("") 并报“不存在”）。
    ...(draft.defaultVirtualModel.trim() !== ''
      ? { default_virtual_model: draft.defaultVirtualModel }
      : {}),
    server: {
      listen: draft.server.listen,
      auth_enabled: draft.server.auth_enabled,
      auth_mode: draft.server.auth_mode,
      admin_token: draft.server.admin_token,
      inbound_key: draft.server.inbound_key,
    },
    profiles,
    candidates: draft.candidates.map(candidateTo),
    virtual_models: draft.virtualModels.map(virtualModelTo),
    ...draft.passthrough,
  }
  return `${stringifyToml(document)}\n`
}

// ---------------------------------------------------------------------------
// 本地预检
// ---------------------------------------------------------------------------

/** 候选被哪些虚拟模型引用。 */
export function referencesOf(draft: ConfigDraft, candidateId: string): string[] {
  return draft.virtualModels
    .filter((model) => model.candidate_pool.includes(candidateId))
    .map((model) => model.id)
}

/** 明文密钥占位符无法回写：列出所有出现位置。 */
function redactedSecretPaths(draft: ConfigDraft): string[] {
  const paths: string[] = []
  if (draft.server.admin_token === REDACTED_SECRET) paths.push('server.admin_token')
  if (draft.server.inbound_key === REDACTED_SECRET) paths.push('server.inbound_key')
  for (const candidate of draft.candidates) {
    if (candidate.credential === REDACTED_SECRET) {
      paths.push(`candidates[${candidate.id}].credential`)
    }
  }
  const judge = draft.passthrough.judge
  if (isRecord(judge) && isRecord(judge.service) && judge.service.credential === REDACTED_SECRET) {
    paths.push('judge.service.credential')
  }
  for (const [index, model] of draft.virtualModels.entries()) {
    if (model.judge.service.credential === REDACTED_SECRET) {
      paths.push(`virtual_models.${index}.judge.service.credential`)
    }
  }
  return paths
}

function isCredentialRef(value: string): boolean {
  return value.startsWith('env:') || value.startsWith('file:') || value.startsWith('keyring:')
}

/** JEV 的 SystemOne endpoint 必须是完整路径（允许 query/fragment）。 */
export function isSystemOneEndpoint(value: string): boolean {
  const trimmed = value.trim()
  if (trimmed === '') return false
  try {
    const path = new URL(trimmed).pathname.replace(/\/+$/, '')
    return path.endsWith('/v1/systemone')
  } catch {
    return false
  }
}

/**
 * 目录模型的 `base_url` → SystemOne 决策模型 endpoint。
 *
 * 供应商 base 一般指到 `/v1`，JEV 判官要求完整 `/v1/systemone`；
 * 选中决策模型时用这里归一化，避免把 `/v1` 写进 `judge.service.endpoint`
 * 后被后端当成配置错误。
 */
export function systemOneEndpoint(baseUrl: string): string {
  const base = baseUrl.trim().replace(/\/+$/, '')
  if (base === '') return base
  const [path, suffix] = base.split(/(?=[?#])/, 2)
  if (isSystemOneEndpoint(path)) return base
  const normalized = path.endsWith('/v1') ? `${path}/systemone` : `${path}/v1/systemone`
  return `${normalized}${suffix ?? ''}`
}

/** 本地预检（服务端 validate 之前的第一道）。 */
export function localIssues(draft: ConfigDraft): ConfigIssue[] {
  const issues: ConfigIssue[] = []

  for (const path of redactedSecretPaths(draft)) {
    issues.push({
      path,
      code: 'literal_secret_roundtrip',
      message: '该字段是明文密钥，服务端不回传明文、无法回写；请改成 env:NAME 或 file:path 引用后再保存',
    })
  }

  const candidateIds = new Set<string>()
  for (const candidate of draft.candidates) {
    if (!candidate.id.trim()) {
      issues.push({ path: 'candidates[].id', code: 'empty', message: '候选 id 不能为空' })
    }
    if (candidateIds.has(candidate.id)) {
      issues.push({ path: `candidates[${candidate.id}].id`, code: 'duplicate', message: '候选 id 重复' })
    }
    candidateIds.add(candidate.id)
    if (!candidate.base_url.startsWith('http://') && !candidate.base_url.startsWith('https://')) {
      issues.push({
        path: `candidates[${candidate.id}].base_url`,
        code: 'invalid_url',
        message: 'base_url 必须是 http/https',
      })
    }
    if (candidate.window <= 0 || candidate.window > 0xffff_ffff) {
      issues.push({
        path: `candidates[${candidate.id}].window`,
        code: 'out_of_range',
        message: 'window 必须在 1..=4294967295',
      })
    }
    if (candidate.strength < 0 || candidate.strength > 255) {
      issues.push({
        path: `candidates[${candidate.id}].strength`,
        code: 'out_of_range',
        message: 'strength 0..=255',
      })
    }
    const credential = candidate.credential.trim()
    if (
      credential &&
      credential !== REDACTED_SECRET &&
      !credential.startsWith('env:') &&
      !credential.startsWith('file:')
    ) {
      issues.push({
        path: `candidates[${candidate.id}].credential`,
        code: 'literal_secret',
        message: '凭据只能填引用：env:NAME 或 file:path（密钥值不进配置）',
      })
    }
  }

  if (draft.server.auth_enabled && draft.server.inbound_key.trim() === '') {
    issues.push({
      path: 'server.inbound_key',
      code: 'missing_inbound_key',
      message: 'auth_enabled = true 时必须设置 inbound_key',
    })
  }

  const profileIds = new Set(Object.keys(draft.profiles))
  const virtualModelIds = new Set<string>()
  for (const [index, model] of draft.virtualModels.entries()) {
    if (!model.id.trim()) {
      issues.push({ path: 'virtual_models[].id', code: 'empty', message: '虚拟模型 id 不能为空' })
    }
    if (virtualModelIds.has(model.id)) {
      issues.push({
        path: `virtual_models[${model.id}].id`,
        code: 'duplicate',
        message: '虚拟模型 id 重复',
      })
    }
    virtualModelIds.add(model.id)
    if (!profileIds.has(model.profile)) {
      issues.push({
        path: `virtual_models[${model.id}].profile`,
        code: 'missing_profile',
        message: `profile "${model.profile}" 不存在`,
      })
    }
    if (
      model.contextWindowMode === 'custom' &&
      (model.context_window === null || model.context_window <= 0)
    ) {
      issues.push({
        path: `virtual_models[${model.id}].context_window`,
        code: 'out_of_range',
        message: '自定义上下文窗口必须大于 0',
      })
    }
    if (model.candidate_pool.length === 0) {
      issues.push({
        path: `virtual_models[${model.id}].candidate_pool`,
        code: 'empty_pool',
        message: '候选池不能为空',
      })
    }
    for (const candidateId of model.candidate_pool) {
      if (!candidateIds.has(candidateId)) {
        issues.push({
          path: `virtual_models[${model.id}].candidate_pool`,
          code: 'missing_candidate',
          message: `候选 "${candidateId}" 不存在`,
        })
      }
    }

    // 组级判断：有规则必须 active；active 时判断服务必须可用。
    if (model.rules.length > 0 && model.judge.mode !== 'active') {
      issues.push({
        path: `virtual_models.${index}.judge.mode`,
        code: 'rules_require_judge',
        message: '有路由规则时必须启用判断模型（active）',
      })
    }
    if (model.judge.mode === 'active') {
      const service = model.judge.service
      const kind = service.kind.trim().toLowerCase()
      if (kind !== 'llm' && kind !== 'jev') {
        issues.push({
          path: `virtual_models.${index}.judge.service.kind`,
          code: 'invalid_judge_kind',
          message: '判断服务类型只支持 llm / jev',
        })
      }
      if (!service.endpoint.trim() || !service.model.trim()) {
        issues.push({
          path: `virtual_models.${index}.judge.service.model`,
          code: 'missing_judge_model',
          message: '请选择判断模型（在供应商页配置用途后编译）',
        })
      } else if (kind === 'jev' && !isSystemOneEndpoint(service.endpoint)) {
        issues.push({
          path: `virtual_models.${index}.judge.service.endpoint`,
          code: 'invalid_systemone_endpoint',
          message: 'JEV 的 endpoint 必须是完整的 SystemOne 路径（以 /v1/systemone 结尾）',
        })
      }
      const credential = service.credential.trim()
      if (credential && credential !== REDACTED_SECRET && !isCredentialRef(credential)) {
        issues.push({
          path: `virtual_models.${index}.judge.service.credential`,
          code: 'literal_secret',
          message: '判断服务凭据只能填引用：env:NAME / file:path / keyring:NAME',
        })
      }
    }

    const ruleIds = new Set<string>()
    model.rules.forEach((rule, ruleIndex) => {
      const rulePath = `virtual_models.${index}.rules.${ruleIndex}`
      if (!rule.id.trim()) {
        issues.push({ path: `${rulePath}.id`, code: 'empty', message: '规则 id 不能为空' })
      } else if (ruleIds.has(rule.id)) {
        issues.push({ path: `${rulePath}.id`, code: 'duplicate', message: '规则 id 在组内重复' })
      }
      ruleIds.add(rule.id)
      if (!rule.intent.trim()) {
        issues.push({ path: `${rulePath}.intent`, code: 'empty', message: '规则 intent 不能为空' })
      }
      if (!model.candidate_pool.includes(rule.prefer)) {
        issues.push({
          path: `${rulePath}.prefer`,
          code: 'prefer_not_in_pool',
          message: 'prefer 必须是本组候选池中的候选',
        })
      }
    })

    // 内核要求 strength_order 覆盖池内候选实际用到的 tier，否则排序静默落空。
    const profile = draft.profiles[model.profile]
    if (profile && profile.strength_order.length > 0) {
      const known = new Set(profile.strength_order)
      for (const candidateId of model.candidate_pool) {
        const candidate = draft.candidates.find((item) => item.id === candidateId)
        if (candidate && !known.has(candidate.tier)) {
          issues.push({
            path: `profiles.${model.profile}.strength_order`,
            code: 'missing_tier',
            message: `strength_order 缺少候选档位 "${candidate.tier}"（候选 ${candidateId}）`,
          })
        }
      }
    }
  }

  if (
    draft.defaultVirtualModel !== '' &&
    draft.virtualModels.every((model) => model.id !== draft.defaultVirtualModel)
  ) {
    issues.push({
      path: 'default_virtual_model',
      code: 'missing_virtual_model',
      message: `默认虚拟模型 "${draft.defaultVirtualModel}" 不存在`,
    })
  }

  const defaultModel = draft.virtualModels.find(
    (model) => model.id === draft.defaultVirtualModel,
  )
  if (defaultModel && !defaultModel.enabled) {
    issues.push({
      path: 'default_virtual_model',
      code: 'default_virtual_model_disabled',
      message: `默认虚拟模型 "${draft.defaultVirtualModel}" 已停用`,
    })
  }
  if (draft.virtualModels.length > 0 && !draft.virtualModels.some((model) => model.enabled)) {
    issues.push({
      path: 'virtual_models',
      code: 'no_enabled_virtual_model',
      message: '至少需要一个启用的虚拟模型作为默认入口',
    })
  }

  return issues
}

/** 候选项的内存草稿工厂。 */
export function emptyCandidate(): CandidateDraft {
  return {
    id: '',
    tier: 'local',
    strength: 20,
    provider: 'openai',
    base_url: 'http://127.0.0.1:11434/v1',
    model: '',
    window: 32768,
    credential: '',
    proxy: '',
    extra_headers: [],
    input: [],
    thinking_levels: [],
    disable_thinking: false,
    max_tokens: null,
    caps: [],
    price: null,
    enabled: true,
  }
}

export function emptyVirtualModel(): VirtualModelDraft {
  return {
    id: '',
    provider: 'token-router',
    name: '',
    profile: 'default',
    enabled: true,
    candidate_pool: [],
    strategies: ['baseline'],
    thinking_levels: [],
    disableThinking: false,
    contextWindowMode: 'min',
    context_window: null,
    max_tokens: null,
    listMembers: false,
    input: [],
    judge: {
      mode: 'off',
      max_calls_per_turn: 1,
      max_calls_per_minute: 30,
      max_calls_per_session: 200,
      deadline_ms: 300,
      service: {
        kind: 'llm',
        endpoint: '',
        model: '',
        service_id: 'group-judge',
        revision: 1,
        credential: '',
        timeout_ms: 300,
        structured_output: 'json_mode',
        temperature: 0,
        max_output_tokens: 256,
        disable_thinking: true,
        currency: 'USD',
        input_micros_per_mtok: 0,
        output_micros_per_mtok: 0,
        request_micros: 0,
      },
    },
    rules: [],
    judgeConfigured: false,
    rulesConfigured: false,
  }
}

// ---------------------------------------------------------------------------
// 只读配置摘要（Apps / Onboarding / Playground 共用，避免各自解析）
// ---------------------------------------------------------------------------

export interface VirtualModelSummary {
  id: string
  name: string
  /** 候选数；配置里没有 candidate_pool 时为 null（不猜）。 */
  candidateCount: number | null
  /** 显式 false 才是停用（配置默认 true 且 true 时不序列化）。 */
  enabled: boolean
}

/** 从 config 文档（`ConfigDocument.document`）读出虚拟模型摘要。 */
export function virtualModelSummaries(document: unknown): VirtualModelSummary[] {
  if (!isRecord(document) || !Array.isArray(document.virtual_models)) return []
  const out: VirtualModelSummary[] = []
  for (const item of document.virtual_models) {
    if (!isRecord(item)) continue
    const id = typeof item.id === 'string' ? item.id.trim() : ''
    if (id === '') continue
    out.push({
      id,
      name: typeof item.name === 'string' ? item.name : '',
      candidateCount: Array.isArray(item.candidate_pool) ? item.candidate_pool.length : null,
      enabled: item.enabled !== false,
    })
  }
  return out
}

/** 配置声明的默认虚拟模型；缺失/空串返回 null（后端会自行挑第一个启用组）。 */
export function defaultVirtualModelOf(document: unknown): string | null {
  if (!isRecord(document)) return null
  const value = document.default_virtual_model
  return typeof value === 'string' && value.trim() !== '' ? value.trim() : null
}

/** 把 issues 按字段路径分组（表单标红用）。 */
export function issuesByPath(issues: ConfigIssue[]): Map<string, ConfigIssue[]> {
  const grouped = new Map<string, ConfigIssue[]>()
  for (const issue of issues) {
    const list = grouped.get(issue.path) ?? []
    list.push(issue)
    grouped.set(issue.path, list)
  }
  return grouped
}
