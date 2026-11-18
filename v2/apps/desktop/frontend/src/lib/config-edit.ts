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
  price: PriceDraft | null
  enabled: boolean
}

export interface VirtualModelDraft {
  id: string
  provider: string
  name: string
  profile: string
  candidate_pool: string[]
  strategies: string[]
  thinking_levels: string[]
  context_window: number | null
  max_tokens: number | null
  input: string[]
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
    price: priceFrom(raw.price),
    enabled: bool(raw.enabled, true),
  }
}

function virtualModelFrom(value: unknown): VirtualModelDraft {
  const raw = isRecord(value) ? value : {}
  return {
    id: str(raw.id, ''),
    provider: str(raw.provider, 'token-router'),
    name: str(raw.name, ''),
    profile: str(raw.profile, 'default'),
    candidate_pool: strArray(raw.candidate_pool),
    strategies: strArray(raw.strategies),
    thinking_levels: strArray(raw.thinking_levels),
    context_window: optNum(raw.context_window),
    max_tokens: optNum(raw.max_tokens),
    input: strArray(raw.input),
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
    defaultVirtualModel: str(doc.default_virtual_model, 'auto'),
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
  if (candidate.price) out.price = priceTo(candidate.price)
  out.enabled = candidate.enabled
  return out
}

function virtualModelTo(model: VirtualModelDraft): Record<string, unknown> {
  const out: Record<string, unknown> = {
    provider: model.provider,
    id: model.id,
  }
  if (model.name) out.name = model.name
  if (model.thinking_levels.length > 0) out.thinking_levels = model.thinking_levels
  if (model.context_window !== null) out.context_window = model.context_window
  if (model.max_tokens !== null) out.max_tokens = model.max_tokens
  if (model.input.length > 0) out.input = model.input
  out.profile = model.profile
  out.strategies = model.strategies
  out.candidate_pool = model.candidate_pool
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
    default_virtual_model: draft.defaultVirtualModel,
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
  return paths
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
  for (const model of draft.virtualModels) {
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
    candidate_pool: [],
    strategies: ['baseline'],
    thinking_levels: [],
    context_window: null,
    max_tokens: null,
    input: [],
  }
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
