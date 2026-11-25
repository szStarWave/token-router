/**
 * 活动页（路由视图）的共享小工具。
 *
 * 只做纯格式换算，不含数据获取；口径全部来自 admin 契约字段，
 * 不做推断（没有的量就不显示，不编）。
 */
import type { QuotaWindowInfo, RequestSummary } from '../../api/types'
import type { ModelNamePref } from '../../stores/prefsStore'

/** HH:MM:SS。 */
export function fmtClock(ms: number): string {
  const date = new Date(ms)
  return date.toLocaleTimeString('zh-CN', { hour12: false })
}

/** YYYY-MM-DD。 */
export function fmtDay(ms: number): string {
  const date = new Date(ms)
  const month = String(date.getMonth() + 1).padStart(2, '0')
  const day = String(date.getDate()).padStart(2, '0')
  return `${date.getFullYear()}-${month}-${day}`
}

/** 相对现在的人话时间（用于配额重置倒计时）。 */
export function fmtRelative(targetMs: number, nowMs: number): string {
  const delta = targetMs - nowMs
  const abs = Math.abs(delta)
  const minute = 60_000
  const hour = 60 * minute
  const day = 24 * hour
  let text: string
  if (abs < minute) text = '不到 1 分钟'
  else if (abs < hour) text = `${Math.round(abs / minute)} 分钟`
  else if (abs < day) text = `${Math.round(abs / hour)} 小时`
  else text = `${Math.round(abs / day)} 天`
  return delta >= 0 ? `${text}后` : `${text}前`
}

/** 美元金额（micros → $）。 */
export function fmtCost(micros: number): string {
  const dollars = micros / 1_000_000
  if (dollars === 0) return '$0'
  if (dollars < 0.01) return `$${dollars.toFixed(5)}`
  return `$${dollars.toFixed(4)}`
}

/** 毫秒 → 人话（1.2 秒 / 340 毫秒）。 */
export function fmtDuration(ms: number | null): string {
  if (ms === null || ms === undefined) return '—'
  if (ms < 1000) return `${Math.round(ms)} 毫秒`
  return `${(ms / 1000).toFixed(1)} 秒`
}

/** 配额状态：耗尽 / 估算 / 正常 / 未知。 */
export type QuotaState = 'exhausted' | 'estimated' | 'ok' | 'unknown'

export function quotaState(quota: QuotaWindowInfo | undefined): QuotaState {
  if (!quota) return 'unknown'
  if (quota.consecutive_429 > 0) return 'exhausted'
  if (quota.remaining === null) return 'unknown'
  if (quota.remaining <= 0) return 'exhausted'
  return quota.estimated ? 'estimated' : 'ok'
}

export function quotaStateLabel(state: QuotaState): string {
  switch (state) {
    case 'exhausted':
      return '已耗尽'
    case 'estimated':
      return '本地估算'
    case 'ok':
      return '配额正常'
    default:
      return '无观测'
  }
}

export function quotaStatePill(state: QuotaState): string {
  switch (state) {
    case 'exhausted':
      return 'app-pill app-pill-error'
    case 'estimated':
      return 'app-pill app-pill-warn'
    case 'ok':
      return 'app-pill app-pill-ok'
    default:
      return 'app-pill'
  }
}

/** 请求的一次「token 合计」（输入 + 缓存读 + 缓存写 + 输出）。 */
export function requestTokens(row: RequestSummary): number {
  return row.tokens.input + row.tokens.cache_read + row.tokens.output
}

/** 缓存命中率（cache_read / (input + cache_read)）；无输入返回 null。 */
export function cacheHitRatio(input: number, cacheRead: number): number | null {
  const total = input + cacheRead
  if (total <= 0) return null
  return cacheRead / total
}

/** 活动页的时间范围（客户端过滤；服务端只有固定窗口）。 */
export type RangeKey = 'today' | 'yesterday' | 'week' | 'all'

export const RANGES: Array<{ key: RangeKey; label: string }> = [
  { key: 'today', label: '今天' },
  { key: 'yesterday', label: '昨天' },
  { key: 'week', label: '近 7 天' },
  { key: 'all', label: '全部' },
]

function startOfDay(ms: number): number {
  const date = new Date(ms)
  date.setHours(0, 0, 0, 0)
  return date.getTime()
}

export function inRange(at: number, range: RangeKey, nowMs: number): boolean {
  if (range === 'all') return true
  const today = startOfDay(nowMs)
  if (range === 'today') return at >= today
  if (range === 'yesterday') return at >= today - 86_400_000 && at < today
  return at >= today - 6 * 86_400_000
}

/** 入口协议 → 人话。 */
export function protocolLabel(protocol: string): string {
  switch (protocol) {
    case 'chat':
      return 'OpenAI 兼容'
    case 'anthropic_messages':
      return 'Anthropic'
    case 'responses':
      return 'OpenAI Responses'
    default:
      return protocol || '未知入口'
  }
}

/** 结果 → 简短标签。 */
export function outcomeLabel(outcome: string): string {
  switch (outcome) {
    case 'success':
      return '成功'
    case 'upstream_error':
      return '上游错误'
    case 'quality_fail':
      return '质量未过'
    case 'cancelled':
      return '已取消'
    case 'rejected':
      return '已拒绝'
    default:
      return outcome
  }
}

/** 模型名的显示形态（设置页「模型名显示」）。
 *
 * 客户端可填的 id 是 `group/member`（member 自身可能还带 `/`，
 * 如 `auto/meta-llama/Llama-3.1-8B`）。
 *
 * - full：原样；
 * - short：只去掉最前面的路由组段（`auto/meta-llama/Llama…` → `meta-llama/Llama…`）。
 */
export function formatModelLabel(model: string, pref: ModelNamePref): string {
  if (pref === 'full') return model
  const index = model.indexOf('/')
  return index >= 0 ? model.slice(index + 1) : model
}

/**
 * 判官调用结果（`judge_calls.outcome`）→ 显示文案。
 *
 * `answered / selected / no_match` 是拿到结论；`unavailable / failed / budget_exceeded`
 * 是没拿到结论（失败计数口径同后端）。未知值原样返回（机器可消费）。
 */
export function judgeOutcomeLabel(outcome: string): string {
  switch (outcome) {
    case 'answered':
      return '已回答'
    case 'selected':
      return '已选'
    case 'no_match':
      return '无匹配'
    case 'unavailable':
      return '不可用'
    case 'failed':
      return '失败'
    case 'budget_exceeded':
      return '预算用尽'
    default:
      return outcome
  }
}

/** 判官失败码 → 显示文案；未知码原样返回。 */
export function judgeFailureLabel(code: string): string {
  switch (code) {
    case 'Timeout':
      return '超时'
    case 'InputRejected':
      return '输入被拒绝'
    case 'NoModel':
      return '未配置模型'
    case 'Canceled':
    case 'Cancelled':
      return '已取消'
    default:
      return code
  }
}

/**
 * 判官调用来源 → 短标签（`judge_calls.effective_source`）。
 *
 * JEV 与 LLM 传输不同但都是外部决策模型；`disabled` 表示预算/配置导致没真正调用。
 */
export function judgeSourceLabel(source: string): string {
  switch (source) {
    case 'external_jev':
      return 'JEV'
    case 'external_llm':
      return 'LLM'
    case 'external_service':
      return '外部'
    case 'fallback':
      return '回退'
    case 'heuristic':
      return '启发式'
    case 'disabled':
      return '未调用'
    default:
      return ''
  }
}
