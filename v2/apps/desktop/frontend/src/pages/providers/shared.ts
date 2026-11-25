/** Playground/Providers 共用的无 React 小工具（fast refresh：非组件导出单独放）。 */
import type {
  ModelPurposeDto,
  ProviderInfo,
  ProviderKindDto,
  ProviderStatus,
} from '../../api/types'

/** `Name: Value` 多行文本 → header 数组。 */
export function parseHeaders(text: string): Array<{ name: string; value: string }> {
  return text
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const index = line.indexOf(':')
      return index > 0
        ? { name: line.slice(0, index).trim(), value: line.slice(index + 1).trim() }
        : null
    })
    .filter((item): item is { name: string; value: string } => item !== null)
}

/** 错误消息提取（errorMessage 之外的轻量版）。 */
export function errorMessageText(error: unknown): string {
  if (error instanceof Error) return error.message
  return String(error)
}

/** 列表状态口径：`ready` 只说明最近一次目录刷新成功，不代表 key 已验证。 */
export const PROVIDER_STATUS_LABEL: Record<ProviderStatus, string> = {
  not_configured: '未配置',
  awaiting_catalog: '待刷新目录',
  ready: '目录已更新',
  catalog_error: '目录失败',
}

export const PROVIDER_STATUS_PILL: Record<ProviderStatus, string> = {
  not_configured: 'app-pill app-pill-warn',
  awaiting_catalog: 'app-pill app-pill-warn',
  ready: 'app-pill app-pill-ok',
  catalog_error: 'app-pill app-pill-error',
}

export const PROVIDER_KIND_LABEL: Record<ProviderKindDto, string> = {
  open_ai_compatible: 'OpenAI 兼容',
  anthropic: 'Anthropic',
  responses: 'OpenAI Responses',
}

/** 目录模型的用途标签（选择器分组元数据，不改已编译候选）。 */
export const MODEL_PURPOSE_LABEL: Record<ModelPurposeDto, string> = {
  chat: '回答',
  judge_llm: '判断 LLM',
  jev: '决策模型（SystemOne）',
}

/** chips / 行内用的短标签：只有判断类模型需要额外标记。 */
export function modelPurposeTag(purpose: ModelPurposeDto): string | null {
  if (purpose === 'judge_llm') return '判断'
  if (purpose === 'jev') return '决策'
  return null
}

/** 厂商列表状态：无账号 = 未配置；其余取目录状态。 */
export function statusOf(provider: ProviderInfo): ProviderStatus {
  if (provider.account_count === 0) return 'not_configured'
  return provider.status
}

export function providerKindLabel(kind: ProviderKindDto): string {
  return PROVIDER_KIND_LABEL[kind]
}

/** 相对时间（列表里只给粗粒度，避免每秒刷新）。 */
export function formatRelativeTime(timestamp: number, now = Date.now()): string {
  const diff = Math.max(0, now - timestamp)
  if (diff < 60_000) return '刚刚'
  const minutes = Math.floor(diff / 60_000)
  if (minutes < 60) return `${minutes} 分钟前`
  const hours = Math.floor(minutes / 60)
  if (hours < 48) return `${hours} 小时前`
  return `${Math.floor(hours / 24)} 天前`
}

/** 最近目录刷新 / 错误摘要（列表行右侧）。 */
export function providerCatalogSummary(provider: ProviderInfo): string {
  if (provider.catalog_error) return `目录错误：${provider.catalog_error}`
  if (provider.catalog_refreshed_at !== null) {
    return `目录刷新：${formatRelativeTime(provider.catalog_refreshed_at)}`
  }
  return '目录未刷新'
}
