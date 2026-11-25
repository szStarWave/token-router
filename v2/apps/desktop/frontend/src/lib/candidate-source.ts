/**
 * 候选来源判定：编译候选 vs 配置里手写的 `[[candidates]]`。
 *
 * 编译 id 由服务端拼成 `sanitize(provider)-sanitize(model)-account8`
 * （见 `v2/crates/control/src/provider_service.rs`）。前端没有单独的来源字段，
 * 只能按 provider id 前缀反解；匹配不到任何 provider 的即为手写候选。
 */
import type { ProviderInfo } from '../api/types'

/** 与服务端 `sanitize` 一致：非 `[A-Za-z0-9._-]` 一律替换成 `-`。 */
export function sanitizeCandidatePart(text: string): string {
  return text.replace(/[^A-Za-z0-9._-]/g, '-')
}

/**
 * 候选 id → 编译它的供应商。
 *
 * 按最长 provider id 前缀匹配，避免 `foo` 抢占 `foo-bar` 的候选。
 * 返回 `undefined` 表示这是配置里手写的候选。
 */
export function providerOfCandidate(
  providers: ProviderInfo[],
  candidateId: string,
): ProviderInfo | undefined {
  let match: ProviderInfo | undefined
  let matchedLength = -1
  for (const provider of providers) {
    const prefix = `${sanitizeCandidatePart(provider.id)}-`
    if (prefix.length > matchedLength && candidateId.startsWith(prefix)) {
      match = provider
      matchedLength = prefix.length
    }
  }
  return match
}
