/**
 * 路由组视图的纯函数：模式/策略文案、成员与账号解析、profile 影响范围。
 *
 * 这些逻辑只读草稿和已拉取的供应商/账号数据，不写状态；页面与编辑器共用，
 * 单测覆盖边界（手写候选、缺失候选、账号前缀映射）。
 */
import type { AccountInfo, ProviderInfo } from '../../api/types'
import { providerOfCandidate, sanitizeCandidatePart } from '../../lib/candidate-source'
import type {
  CandidateDraft,
  ConfigDraft,
  GroupRuleDraft,
  VirtualModelDraft,
} from '../../lib/config-edit'
import { isSystemOneEndpoint } from '../../lib/config-edit'

// ---------------------------------------------------------------------------
// 文案
// ---------------------------------------------------------------------------

export function groupTitle(model: VirtualModelDraft): string {
  return model.name.trim() || model.id.trim() || '未命名组'
}

/** profile.mode 的用户词；不改语义，只换标签。 */
export function groupModeLabel(mode: string | undefined): string {
  switch ((mode ?? '').trim()) {
    case 'single':
      return '单次尝试'
    case 'cascade':
      return '级联'
    case 'verify_after':
      return '校验后升级'
    case '':
      return '未设置'
    default:
      return (mode ?? '').trim()
  }
}

const STRATEGY_LABEL: Record<string, string> = {
  baseline: '基础排序',
  failover: '故障转移',
  escalation: '能力升级',
  classify: '难度分类',
  'quota-reset': '到期前用完',
  fast: '速度优先',
}

/** 配置为空时 pipeline 用的默认策略集合（含自动追加的 quota-reset）。 */
export const DEFAULT_STRATEGIES = [
  'baseline',
  'failover',
  'escalation',
  'classify',
  'quota-reset',
]

/** 实际生效的策略 id：未显式配置时为默认集合。 */
export function effectiveStrategyIds(ids: string[]): string[] {
  return ids.length > 0 ? ids : DEFAULT_STRATEGIES
}

export function strategyLabel(id: string): string {
  return STRATEGY_LABEL[id] ?? id
}

/** 展示用策略清单；空配置时返回 pipeline 的默认集合。 */
export function strategyLabels(ids: string[]): string[] {
  return (ids.length > 0 ? ids : DEFAULT_STRATEGIES).map(strategyLabel)
}

/** 判断方式展示：off / LLM / JEV；旧配置的其他 kind 原样展示。 */
export function judgeModeText(model: VirtualModelDraft): string {
  if (model.judge.mode === 'off') return '不使用'
  const kind = model.judge.service.kind.trim()
  if (kind === '' || kind.toLowerCase() === 'llm') return 'LLM'
  if (kind.toLowerCase() === 'jev') return 'JEV'
  return kind
}

/** 供应商页为判断模型打的用途标签（编译时写入 caps）。 */
export function judgePurpose(candidate: CandidateDraft): 'llm' | 'jev' | null {
  if (candidate.caps.includes('judge:jev')) return 'jev'
  if (candidate.caps.includes('judge:llm')) return 'llm'
  return null
}

/**
 * 组判断可选模型：所有启用且 id 非空的编译候选。
 *
 * 判断模型不再依赖供应商页预设用途：在组里直接选模型 id 即可；
 * 只有 JEV 需要候选带了 jev 能力标记（选它时 kind 写 jev）。
 */
export function judgeSelectableCandidates(candidates: CandidateDraft[]): CandidateDraft[] {
  return candidates.filter(
    (candidate) => candidate.id.trim() !== '' && candidate.enabled,
  )
}

// ---------------------------------------------------------------------------
// 判断可用性 / 规则问题
// ---------------------------------------------------------------------------

function isCredentialRef(value: string): boolean {
  return value.startsWith('env:') || value.startsWith('file:') || value.startsWith('keyring:')
}

export interface JudgeAvailability {
  ok: boolean
  reason: string
  matched: CandidateDraft | undefined
}

/**
 * 判断模型是否可用（配置层面）。判断模型不必在本组，因此只按 judge 服务字段
 * 与供应商目录中的候选启用状态判定；真实请求是否可达不在这里判断。
 */
export function judgeAvailability(
  model: VirtualModelDraft,
  candidates: CandidateDraft[],
): JudgeAvailability {
  if (model.judge.mode !== 'active') return { ok: true, reason: '', matched: undefined }
  const service = model.judge.service
  const kind = service.kind.trim().toLowerCase()
  if (kind !== 'llm' && kind !== 'jev') {
    return { ok: false, reason: '判断服务类型只支持 llm / jev', matched: undefined }
  }
  if (!service.endpoint.trim()) {
    return { ok: false, reason: '请选择判断模型', matched: undefined }
  }
  if (kind === 'jev' && !isSystemOneEndpoint(service.endpoint)) {
    return {
      ok: false,
      reason: 'JEV endpoint 需为完整的 SystemOne 路径（/v1/systemone）',
      matched: undefined,
    }
  }
  if (!service.model.trim()) {
    return { ok: false, reason: '请选择判断模型', matched: undefined }
  }
  const credential = service.credential.trim()
  if (credential && !isCredentialRef(credential)) {
    return { ok: false, reason: '凭据需为 env: / file: / keyring: 引用', matched: undefined }
  }
  if (kind === 'jev') {
    // JEV 不要求判断模型出现在供应商目录里。
    return { ok: true, reason: '', matched: undefined }
  }
  const matched = candidates.find(
    (candidate) =>
      candidate.id.trim() !== '' &&
      candidate.base_url === service.endpoint &&
      candidate.model === service.model,
  )
  if (matched && !matched.enabled) {
    return { ok: false, reason: '判断模型候选已停用', matched }
  }
  return { ok: true, reason: '', matched }
}

/** 候选在配置/目录层面的不可用原因；空串表示可用。 */
export function candidateUnavailableReason(candidate: CandidateDraft | undefined): string {
  if (!candidate) return '候选不存在'
  if (!candidate.enabled) return '候选已停用'
  return ''
}

/** 规则 prefer 指向的候选是否可用；空串表示可用。 */
export function ruleTargetProblem(
  rule: GroupRuleDraft,
  pool: string[],
  candidates: CandidateDraft[],
): string {
  if (!rule.prefer) return '未选择优先模型'
  if (!pool.includes(rule.prefer)) return '优先模型不在本组'
  const candidate = candidates.find((item) => item.id === rule.prefer)
  if (!candidate) return '优先模型不存在'
  if (!candidate.enabled) return '优先模型已停用'
  return ''
}

/** 生成组内不重复的规则 id。 */
export function nextRuleId(rules: GroupRuleDraft[]): string {
  const used = new Set(rules.map((rule) => rule.id))
  let n = rules.length + 1
  while (used.has(`r${n}`)) n += 1
  return `r${n}`
}

/** 共用某个 profile 的路由组名字（模式是共享配置，改前要知道影响面）。 */
export function profileImpact(draft: ConfigDraft, profileId: string): string[] {
  return draft.virtualModels
    .filter((model) => model.profile === profileId)
    .map((model) => groupTitle(model))
}

// ---------------------------------------------------------------------------
// 成员与账号
// ---------------------------------------------------------------------------

export interface GroupMemberView {
  candidateId: string
  /** 配置里查不到时为 undefined（仍允许从本组移除）。 */
  candidate: CandidateDraft | undefined
  provider: ProviderInfo | undefined
  account: AccountInfo | undefined
  /** 成员行的账号说明；手写候选为空串。 */
  accountText: string
  /** 匹配不到任何供应商 = 配置里手写的 `[[candidates]]`。 */
  manual: boolean
  missing: boolean
  disabled: boolean
}

/**
 * 把候选池展开成可渲染的成员行。
 *
 * 账号来自候选 id 末尾的 account8（`sanitize(account.id)` 前 8 位）；
 * 供应商来自最长前缀反解。手写候选没有供应商/账号信息。
 */
export function groupMemberViews(
  group: VirtualModelDraft,
  candidates: CandidateDraft[],
  providers: ProviderInfo[],
  accounts: AccountInfo[],
): GroupMemberView[] {
  const accountBySuffix = new Map<string, AccountInfo>()
  for (const account of accounts) {
    accountBySuffix.set(sanitizeCandidatePart(account.id).slice(0, 8), account)
  }
  return group.candidate_pool.map((candidateId) => {
    const candidate = candidates.find((item) => item.id === candidateId)
    const provider = providerOfCandidate(providers, candidateId)
    const account = provider ? accountBySuffix.get(candidateId.slice(-8)) : undefined
    const accountText = account
      ? `${account.label.trim() || '账号'} · …${account.fingerprint}`
      : provider
        ? `账号 …${candidateId.slice(-8)}`
        : ''
    return {
      candidateId,
      candidate,
      provider,
      account,
      accountText,
      manual: provider === undefined,
      missing: candidate === undefined,
      disabled: candidate !== undefined && !candidate.enabled,
    }
  })
}

export interface GroupModelChip {
  key: string
  /** 模型名；缺失候选回落到候选 id。 */
  label: string
  /** 同一模型下的候选数（≈账号数）。 */
  count: number
  manual: boolean
  disabled: boolean
}

/** 收起行/摘要里的模型 chips：同一 provider+model 的多个账号合并。 */
export function groupModelChips(views: GroupMemberView[]): GroupModelChip[] {
  const chips = new Map<string, GroupModelChip>()
  for (const view of views) {
    const key = `${view.provider?.id ?? '__manual__'}::${view.candidate?.model ?? view.candidateId}`
    const entry = chips.get(key) ?? {
      key,
      label: view.candidate?.model?.trim() || view.candidateId,
      count: 0,
      manual: view.manual,
      disabled: true,
    }
    entry.count += 1
    entry.disabled = entry.disabled && view.disabled
    chips.set(key, entry)
  }
  return [...chips.values()]
}

/**
 * 成员账号摘要：同一供应商下，若某模型覆盖了该供应商全部启用账号，
 * 显示「任一账号」，否则列出具体账号标签。
 */
export interface GroupAccountSummary {
  provider: ProviderInfo
  label: string
  detail: string
  /** 该供应商所有启用账号都在本组。 */
  allAccounts: boolean
}

export function groupAccountSummaries(
  views: GroupMemberView[],
  providers: ProviderInfo[],
  accounts: AccountInfo[],
): GroupAccountSummary[] {
  const byProvider = new Map<string, GroupMemberView[]>()
  for (const view of views) {
    if (!view.provider) continue
    const list = byProvider.get(view.provider.id) ?? []
    list.push(view)
    byProvider.set(view.provider.id, list)
  }
  const summaries: GroupAccountSummary[] = []
  for (const provider of providers) {
    const members = byProvider.get(provider.id)
    if (!members || members.length === 0) continue
    const enabledAccounts = accounts.filter(
      (account) => account.provider_id === provider.id && account.enabled,
    )
    const used = new Set(
      members.map((member) => member.account?.id ?? member.candidateId.slice(-8)),
    )
    const allAccounts =
      enabledAccounts.length > 0 &&
      enabledAccounts.every((account) =>
        used.has(account.id) || used.has(sanitizeCandidatePart(account.id).slice(0, 8)),
      )
    const labels = members
      .map((member) => member.accountText)
      .filter((text) => text !== '')
    const unique = [...new Set(labels)]
    summaries.push({
      provider,
      label: provider.display_name,
      detail: allAccounts
        ? `全部 ${enabledAccounts.length} 个启用账号`
        : unique.join('、') || '账号未知',
      allAccounts,
    })
  }
  return summaries
}
