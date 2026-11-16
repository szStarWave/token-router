/**
 * 配置编辑核心（§7.4）：文档 ↔ TOML 双向、validate、diff、apply。
 *
 * 规则：
 * - 表单字段与 ConfigDocument 一一对应，不维护第二套模型；
 * - 保存前先 validate；无 issue 才允许 apply（CAS If-Match revision）；
 * - 候选删除前检查 virtual_models[].candidate_pool 引用。
 */
import type { ConfigDocument, ConfigIssue } from '../api/types'

export interface CandidateDraft {
  id: string
  tier: string
  strength: number
  provider: string
  base_url: string
  model: string
  window: number
  credential: string
  enabled: boolean
}

export interface VirtualModelDraft {
  id: string
  provider: string
  name: string
  profile: string
  candidate_pool: string[]
  strategies: string[]
}

export interface ProfileDraft {
  mode: string
  stream_escalation: string
  strength_order: string[]
  unknown_price_policy: string
}

export interface ConfigDraft {
  defaultVirtualModel: string
  server: { listen: string; auth_enabled: boolean; auth_mode: string }
  candidates: CandidateDraft[]
  virtualModels: VirtualModelDraft[]
  profiles: Record<string, ProfileDraft>
}

/** 从 ConfigDocument.document 提取可编辑草稿。 */
export function draftFromDocument(document: ConfigDocument): ConfigDraft {
  const doc = document.document as Record<string, unknown>
  const candidates = Array.isArray(doc.candidates) ? doc.candidates : []
  const virtualModels = Array.isArray(doc.virtual_models) ? doc.virtual_models : []
  const profilesRaw = (doc.profiles ?? {}) as Record<string, Record<string, unknown>>
  const serverRaw = (doc.server ?? {}) as Record<string, unknown>

  const profiles: Record<string, ProfileDraft> = {}
  for (const [id, profile] of Object.entries(profilesRaw)) {
    profiles[id] = {
      mode: String(profile.mode ?? 'cascade'),
      stream_escalation: String(profile.stream_escalation ?? 'next_turn'),
      strength_order: Array.isArray(profile.strength_order)
        ? profile.strength_order.map(String)
        : [],
      unknown_price_policy: String(profile.unknown_price_policy ?? 'quality_allow'),
    }
  }

  const draft: ConfigDraft = {
    defaultVirtualModel: String(doc.default_virtual_model ?? 'auto'),
    server: {
      listen: String(serverRaw.listen ?? '127.0.0.1:17621'),
      auth_enabled: Boolean(serverRaw.auth_enabled ?? false),
      auth_mode: String(serverRaw.auth_mode ?? 'token_store'),
    },
    profiles,
    candidates: candidates.map((candidate) => fromUnknown<CandidateDraft>(candidate, {
      id: '',
      tier: 'local',
      strength: 20,
      provider: 'openai',
      base_url: '',
      model: '',
      window: 32768,
      credential: '',
      enabled: true,
    })),
    virtualModels: virtualModels.map((model) => fromUnknown<VirtualModelDraft>(model, {
      id: '',
      provider: 'router',
      name: '',
      profile: 'default',
      candidate_pool: [],
      strategies: ['baseline'],
    })),
  }
  return draft
}

function fromUnknown<T extends object>(value: unknown, fallback: T): T {
  if (typeof value !== 'object' || value === null) return fallback
  const merged = { ...fallback } as Record<string, unknown>
  for (const key of Object.keys(fallback) as Array<keyof T & string>) {
    const fallbackValue = fallback[key]
    const provided = (value as Record<string, unknown>)[key]
    if (provided === undefined || provided === null) continue
    if (Array.isArray(fallbackValue) && Array.isArray(provided)) {
      merged[key] = provided.map(String)
    } else if (typeof fallbackValue === 'boolean') {
      merged[key] = Boolean(provided)
    } else if (typeof fallbackValue === 'number') {
      merged[key] = Number(provided)
    } else {
      merged[key] = String(provided)
    }
  }
  return merged as T
}

/** 草稿 → TOML（仅覆盖 P4 可编辑字段；未编辑的原字段由服务端 deny_unknown_fields 保护）。 */
export function draftToToml(draft: ConfigDraft): string {
  const lines: string[] = []
  lines.push('schema_version = 2')
  lines.push(`default_virtual_model = "${escape(draft.defaultVirtualModel)}"`)
  lines.push('')
  lines.push('[server]')
  lines.push(`listen = "${escape(draft.server.listen)}"`)
  lines.push(`auth_enabled = ${draft.server.auth_enabled}`)
  lines.push(`auth_mode = "${escape(draft.server.auth_mode)}"`)
  for (const [id, profile] of Object.entries(draft.profiles)) {
    lines.push('')
    lines.push(`[profiles.${escape(id)}]`)
    lines.push(`mode = "${escape(profile.mode)}"`)
    lines.push(`stream_escalation = "${escape(profile.stream_escalation)}"`)
    lines.push(`strength_order = [${profile.strength_order.map((tier) => `"${escape(tier)}"`).join(', ')}]`)
    lines.push(`unknown_price_policy = "${escape(profile.unknown_price_policy)}"`)
  }
  for (const candidate of draft.candidates) {
    lines.push('')
    lines.push('[[candidates]]')
    lines.push(`id = "${escape(candidate.id)}"`)
    lines.push(`tier = "${escape(candidate.tier)}"`)
    lines.push(`strength = ${candidate.strength}`)
    lines.push(`provider = "${escape(candidate.provider)}"`)
    lines.push(`base_url = "${escape(candidate.base_url)}"`)
    lines.push(`model = "${escape(candidate.model)}"`)
    lines.push(`window = ${candidate.window}`)
    if (candidate.credential) {
      lines.push(`credential = "${escape(candidate.credential)}"`)
    }
    lines.push(`enabled = ${candidate.enabled}`)
  }
  for (const model of draft.virtualModels) {
    lines.push('')
    lines.push('[[virtual_models]]')
    lines.push(`provider = "${escape(model.provider)}"`)
    lines.push(`id = "${escape(model.id)}"`)
    lines.push(`name = "${escape(model.name)}"`)
    lines.push(`profile = "${escape(model.profile)}"`)
    lines.push(`candidate_pool = [${model.candidate_pool.map((id) => `"${escape(id)}"`).join(', ')}]`)
    lines.push(`strategies = [${model.strategies.map((strategy) => `"${escape(strategy)}"`).join(', ')}]`)
  }
  return lines.join('\n') + '\n'
}

function escape(text: string): string {
  return text.replace(/\\/g, '\\\\').replace(/"/g, '\\"')
}

/** 候选被哪些虚拟模型引用。 */
export function referencesOf(draft: ConfigDraft, candidateId: string): string[] {
  return draft.virtualModels
    .filter((model) => model.candidate_pool.includes(candidateId))
    .map((model) => model.id)
}

/** 本地预检（服务端 validate 之前的第一道）。 */
export function localIssues(draft: ConfigDraft): ConfigIssue[] {
  const issues: ConfigIssue[] = []
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
    if (candidate.window <= 0) {
      issues.push({ path: `candidates[${candidate.id}].window`, code: 'out_of_range', message: 'window 必须 > 0' })
    }
    if (candidate.strength < 0 || candidate.strength > 100) {
      issues.push({ path: `candidates[${candidate.id}].strength`, code: 'out_of_range', message: 'strength 0..=100' })
    }
    const credential = candidate.credential.trim()
    if (credential && !credential.startsWith('env:') && !credential.startsWith('file:')) {
      issues.push({
        path: `candidates[${candidate.id}].credential`,
        code: 'literal_secret',
        message: '凭据只能填引用：env:NAME 或 file:path（密钥值不进配置）',
      })
    }
  }
  const profileIds = new Set(Object.keys(draft.profiles))
  for (const model of draft.virtualModels) {
    if (!model.id.trim()) {
      issues.push({ path: 'virtual_models[].id', code: 'empty', message: '虚拟模型 id 不能为空' })
    }
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
    enabled: true,
  }
}

export function emptyVirtualModel(): VirtualModelDraft {
  return {
    id: '',
    provider: 'router',
    name: '',
    profile: 'default',
    candidate_pool: [],
    strategies: ['baseline'],
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
