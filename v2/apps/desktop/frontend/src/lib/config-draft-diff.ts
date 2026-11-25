/**
 * 配置草稿 diff（product-design §8.5）：两份草稿之间的改动清单（文本形式）。
 *
 * 用途：
 * - 应用栏「有未保存的修改（N 处）」计数；
 * - CAS 冲突面板「载入最新前保留改动清单」；
 * - 过期草稿（隔离暂存）恢复前的对比预览。
 *
 * 纯函数：输入两份 ConfigDraft，输出人话改动列表；不触碰网络与存储。
 */
import type {
  CandidateDraft,
  ConfigDraft,
  GroupJudgeDraft,
  GroupRuleDraft,
  JudgeServiceDraft,
  ProfileDraft,
  ServerDraft,
  VirtualModelDraft,
} from './config-edit'

export interface DraftChange {
  /** 稳定键（React 列表 + 跳转定位用）。 */
  path: string
  kind: 'changed' | 'added' | 'removed'
  detail: string
}

const DETAIL_LIMIT = 100

function fmtValue(value: unknown): string {
  let text: string
  if (value === undefined || value === null) text = '（未设置）'
  else if (typeof value === 'string') text = value === '' ? '（空）' : value
  else text = JSON.stringify(value)
  return text.length > DETAIL_LIMIT ? `${text.slice(0, DETAIL_LIMIT - 1)}…` : text
}

export function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true
  if (typeof a !== 'object' || typeof b !== 'object' || a === null || b === null) return false
  if (Array.isArray(a) !== Array.isArray(b)) return false
  const recordA = a as Record<string, unknown>
  const recordB = b as Record<string, unknown>
  const keysA = Object.keys(recordA)
  const keysB = Object.keys(recordB)
  if (keysA.length !== keysB.length) return false
  return keysA.every((key) => key in recordB && deepEqual(recordA[key], recordB[key]))
}

/**
 * 「配置内容是否相同」的宽松比较：把 `null` 与 `undefined`（以及缺失键）视为等价。
 *
 * 为什么需要：TOML 不能表达 null——GET 文档里的 `null`（如 `judge.service.credential`）
 * 写回 TOML 后字段被省略，再解析成 `undefined`。若用严格 deepEqual 判断草稿是否干净，
 * 已提交的草稿会被误判为"仍有改动"，自动保存就会无限重写同一份配置。
 */
export function deepEqualLoose(a: unknown, b: unknown): boolean {
  if (a === b) return true
  if (a == null && b == null) return true
  if (a == null || b == null) return false
  if (typeof a !== 'object' || typeof b !== 'object') return false
  if (Array.isArray(a) !== Array.isArray(b)) return false
  if (Array.isArray(a) && Array.isArray(b)) {
    if (a.length !== b.length) return false
    return a.every((item, index) => deepEqualLoose(item, b[index]))
  }
  const recordA = a as Record<string, unknown>
  const recordB = b as Record<string, unknown>
  const keys = new Set([...Object.keys(recordA), ...Object.keys(recordB)])
  for (const key of keys) {
    if (!deepEqualLoose(recordA[key], recordB[key])) return false
  }
  return true
}

function changed(prefix: string, label: string, before: unknown, after: unknown): DraftChange {
  return {
    path: prefix,
    kind: 'changed',
    detail: `${label}：${fmtValue(before)} → ${fmtValue(after)}`,
  }
}

function diffRecordFields(
  out: DraftChange[],
  prefix: string,
  before: object,
  after: object,
  labels: ReadonlyArray<readonly [string, string]>,
): void {
  const recordBefore = before as Record<string, unknown>
  const recordAfter = after as Record<string, unknown>
  for (const [key, label] of labels) {
    const b = recordBefore[key]
    const a = recordAfter[key]
    if (!deepEqual(b, a)) out.push(changed(`${prefix}.${key}`, label, b, a))
  }
}

const SERVER_LABELS = [
  ['listen', '监听地址'],
  ['auth_enabled', '鉴权开关'],
  ['auth_mode', '鉴权模式'],
  ['admin_token', 'admin token（引用）'],
  ['inbound_key', '网关入站 key（引用）'],
] as const satisfies ReadonlyArray<readonly [keyof ServerDraft, string]>

const CANDIDATE_LABELS = [
  ['tier', '档位'],
  ['strength', '强度'],
  ['provider', 'provider'],
  ['base_url', 'base_url'],
  ['model', 'model'],
  ['window', '上下文窗口'],
  ['credential', '凭据（引用）'],
  ['proxy', '代理'],
  ['extra_headers', '自定义 header'],
  ['input', '输入模态'],
  ['thinking_levels', '思考档'],
  ['disable_thinking', '关闭思考'],
  ['max_tokens', '输出上限'],
  ['price', '价格'],
  ['enabled', '启用'],
] as const satisfies ReadonlyArray<readonly [keyof CandidateDraft, string]>

const VIRTUAL_MODEL_LABELS = [
  ['provider', 'provider'],
  ['name', '名称'],
  ['profile', 'profile'],
  ['enabled', '启用'],
  ['candidate_pool', '候选池'],
  ['strategies', '策略'],
  ['thinking_levels', '思考档'],
  ['disableThinking', '关闭思考'],
  ['contextWindowMode', '窗口来源'],
  ['context_window', '上下文窗口'],
  ['listMembers', '模型列表'],
  ['max_tokens', '最大输出'],
  ['input', '输入模态'],
] as const satisfies ReadonlyArray<readonly [keyof VirtualModelDraft, string]>

const GROUP_JUDGE_LABELS = [
  ['mode', '判断方式'],
  ['max_calls_per_turn', '每轮判断上限'],
  ['max_calls_per_minute', '每分钟判断上限'],
  ['max_calls_per_session', '每会话判断上限'],
  ['deadline_ms', '判断超时'],
] as const satisfies ReadonlyArray<readonly [keyof GroupJudgeDraft, string]>

const JUDGE_SERVICE_LABELS = [
  ['kind', '服务类型'],
  ['endpoint', 'endpoint'],
  ['model', 'model'],
  ['service_id', 'service_id'],
  ['revision', 'revision'],
  ['credential', '凭据（引用）'],
  ['timeout_ms', '超时'],
  ['structured_output', '结构化输出'],
  ['temperature', 'temperature'],
  ['max_output_tokens', '最大输出 token'],
  ['currency', '币种'],
  ['input_micros_per_mtok', '输入价'],
  ['output_micros_per_mtok', '输出价'],
  ['request_micros', '请求价'],
] as const satisfies ReadonlyArray<readonly [keyof JudgeServiceDraft, string]>

const GROUP_RULE_LABELS = [
  ['intent', '意图'],
  ['prefer', '优先候选'],
] as const satisfies ReadonlyArray<readonly [keyof GroupRuleDraft, string]>

const PROFILE_LABELS = [
  ['mode', '路由模式'],
  ['stream_escalation', '流式升级'],
  ['strength_order', '档位排序'],
  ['difficulty_weight_micros', '难度权重'],
  ['hard_signal_weight_micros', '硬信号权重'],
  ['unknown_price_policy', '未知价格策略'],
] as const satisfies ReadonlyArray<readonly [keyof ProfileDraft, string]>

/** 候选数组按 id 对齐（空 id 用位置兜底）；id 改动表现为「删除 + 新增」。 */
function diffCandidates(out: DraftChange[], before: CandidateDraft[], after: CandidateDraft[]): void {
  const keyOf = (candidate: CandidateDraft, index: number): string => candidate.id || `#${index}`
  const beforeIds = new Set(before.map(keyOf))
  const afterIds = new Set(after.map(keyOf))

  for (const candidate of after) {
    const key = keyOf(candidate, after.indexOf(candidate))
    if (!beforeIds.has(key)) {
      out.push({
        path: `candidates[${key}]`,
        kind: 'added',
        detail: `新增候选 "${candidate.id || '(未命名)'}"（${candidate.tier} / ${candidate.model || '无 model'}）`,
      })
    }
  }
  for (const candidate of before) {
    const key = keyOf(candidate, before.indexOf(candidate))
    if (!afterIds.has(key)) {
      out.push({ path: `candidates[${key}]`, kind: 'removed', detail: `删除候选 "${candidate.id || '(未命名)'}"` })
    }
  }
  for (const [index, candidate] of after.entries()) {
    const key = keyOf(candidate, index)
    if (!beforeIds.has(key)) continue
    const counterpart = before.find((item, i) => keyOf(item, i) === key)
    if (!counterpart || deepEqual(counterpart, candidate)) continue
    diffRecordFields(out, `candidates[${key}]`, counterpart, candidate, CANDIDATE_LABELS)
  }
}

/** 组判断服务：先列 judge 自身字段，再列 service 字段（避免整块 JSON 被截断）。 */
function diffGroupJudge(out: DraftChange[], prefix: string, before: GroupJudgeDraft, after: GroupJudgeDraft): void {
  diffRecordFields(out, prefix, before, after, GROUP_JUDGE_LABELS)
  if (!deepEqual(before.service, after.service)) {
    diffRecordFields(out, `${prefix}.service`, before.service, after.service, JUDGE_SERVICE_LABELS)
  }
}

/** 组规则按 id 对齐（空 id 用位置兜底）；id 改动表现为「删除 + 新增」。 */
function diffGroupRules(out: DraftChange[], prefix: string, before: GroupRuleDraft[], after: GroupRuleDraft[]): void {
  const keyOf = (rule: GroupRuleDraft, index: number): string => rule.id || `#${index}`
  const beforeIds = new Set(before.map(keyOf))
  const afterIds = new Set(after.map(keyOf))

  for (const [index, rule] of after.entries()) {
    const key = keyOf(rule, index)
    if (!beforeIds.has(key)) {
      out.push({
        path: `${prefix}[${key}]`,
        kind: 'added',
        detail: `新增路由规则 "${rule.id || '(未命名)'}"（意图：${rule.intent || '空'}）`,
      })
    }
  }
  for (const [index, rule] of before.entries()) {
    const key = keyOf(rule, index)
    if (!afterIds.has(key)) {
      out.push({ path: `${prefix}[${key}]`, kind: 'removed', detail: `删除路由规则 "${rule.id || '(未命名)'}"` })
    }
  }
  for (const [index, rule] of after.entries()) {
    const key = keyOf(rule, index)
    if (!beforeIds.has(key)) continue
    const counterpart = before.find((item, i) => keyOf(item, i) === key)
    if (!counterpart || deepEqual(counterpart, rule)) continue
    diffRecordFields(out, `${prefix}[${key}]`, counterpart, rule, GROUP_RULE_LABELS)
  }
}

function diffVirtualModels(out: DraftChange[], before: VirtualModelDraft[], after: VirtualModelDraft[]): void {
  const keyOf = (model: VirtualModelDraft, index: number): string => model.id || `#${index}`
  const beforeIds = new Set(before.map(keyOf))
  const afterIds = new Set(after.map(keyOf))

  for (const model of after) {
    const key = keyOf(model, after.indexOf(model))
    if (!beforeIds.has(key)) {
      out.push({ path: `virtual_models[${key}]`, kind: 'added', detail: `新增虚拟模型 "${model.id || '(未命名)'}"` })
    }
  }
  for (const model of before) {
    const key = keyOf(model, before.indexOf(model))
    if (!afterIds.has(key)) {
      out.push({ path: `virtual_models[${key}]`, kind: 'removed', detail: `删除虚拟模型 "${model.id || '(未命名)'}"` })
    }
  }
  for (const [index, model] of after.entries()) {
    const key = keyOf(model, index)
    if (!beforeIds.has(key)) continue
    const counterpart = before.find((item, i) => keyOf(item, i) === key)
    if (!counterpart || deepEqual(counterpart, model)) continue
    const prefix = `virtual_models[${key}]`
    diffRecordFields(out, prefix, counterpart, model, VIRTUAL_MODEL_LABELS)
    const judgeEqual = deepEqual(counterpart.judge, model.judge)
    if (!judgeEqual) {
      diffGroupJudge(out, `${prefix}.judge`, counterpart.judge, model.judge)
    } else if (counterpart.judgeConfigured !== model.judgeConfigured) {
      out.push({
        path: `${prefix}.judge`,
        kind: model.judgeConfigured ? 'added' : 'removed',
        detail: model.judgeConfigured ? '显式保存组判断配置（off）' : '移除组判断配置',
      })
    }
    const rulesEqual = deepEqual(counterpart.rules, model.rules)
    if (!rulesEqual) {
      diffGroupRules(out, `${prefix}.rules`, counterpart.rules, model.rules)
    } else if (counterpart.rulesConfigured !== model.rulesConfigured) {
      out.push({
        path: `${prefix}.rules`,
        kind: model.rulesConfigured ? 'added' : 'removed',
        detail: model.rulesConfigured ? '显式保存空规则数组' : '移除空规则数组',
      })
    }
  }
}

function diffProfiles(
  out: DraftChange[],
  before: Record<string, ProfileDraft>,
  after: Record<string, ProfileDraft>,
): void {
  const keys = new Set([...Object.keys(before), ...Object.keys(after)])
  for (const key of keys) {
    const b = before[key]
    const a = after[key]
    if (b && !a) {
      out.push({ path: `profiles.${key}`, kind: 'removed', detail: `删除 profile "${key}"` })
    } else if (!b && a) {
      out.push({ path: `profiles.${key}`, kind: 'added', detail: `新增 profile "${key}"（${a.mode}）` })
    } else if (b && a && !deepEqual(b, a)) {
      diffRecordFields(out, `profiles.${key}`, b, a, PROFILE_LABELS)
    }
  }
}

function diffPassthrough(
  out: DraftChange[],
  before: Record<string, unknown>,
  after: Record<string, unknown>,
): void {
  const keys = new Set([...Object.keys(before), ...Object.keys(after)])
  for (const key of keys) {
    const b = before[key]
    const a = after[key]
    if (deepEqual(b, a)) continue
    if (b === undefined) {
      out.push({ path: key, kind: 'added', detail: `新增顶层段 "${key}"` })
    } else if (a === undefined) {
      out.push({ path: key, kind: 'removed', detail: `删除顶层段 "${key}"` })
    } else {
      out.push({ path: key, kind: 'changed', detail: `顶层段 "${key}"：${fmtValue(b)} → ${fmtValue(a)}` })
    }
  }
}

/** 列出 before → after 的全部改动（人话清单；顺序：server → 默认入口 → 候选 → 虚拟模型 → profile → 透传段）。 */
export function diffDrafts(before: ConfigDraft, after: ConfigDraft): DraftChange[] {
  const out: DraftChange[] = []

  diffRecordFields(out, 'server', before.server, after.server, SERVER_LABELS)
  if (before.defaultVirtualModel !== after.defaultVirtualModel) {
    out.push(
      changed('default_virtual_model', '默认虚拟模型', before.defaultVirtualModel, after.defaultVirtualModel),
    )
  }

  diffCandidates(out, before.candidates, after.candidates)
  diffVirtualModels(out, before.virtualModels, after.virtualModels)
  diffProfiles(out, before.profiles, after.profiles)
  diffPassthrough(out, before.passthrough, after.passthrough)

  return out
}
