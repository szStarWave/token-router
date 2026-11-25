/**
 * 路由图（活动页头部）：入口 → 路由组 → 候选账号。
 *
 * 参照 Magpie 的「路由」视图：左边是进来的客户端入口，中间是网关自己的
 * 路由节点，右边是这个路由组当前可以落到哪些上游账号。
 *
 * 口径：候选来自运行中的配置（config 文档），配额来自 quota_windows 观测，
 * 高亮的候选是最近一次请求实际派发到的那个；没有数据就显示「未观测」，
 * 不编配额数字。
 */
import type { VirtualModelDraft, CandidateDraft } from '../../lib/config-edit'
import type { AccountInfo, ProviderInfo, QuotaWindowInfo } from '../../api/types'
import { formatCompactNum } from '../../lib/format-number'
import {
  fmtRelative,
  protocolLabel,
  quotaState,
  quotaStateLabel,
  quotaStatePill,
} from './shared'

export interface RoutingEntry {
  protocol: string
  count: number
}

export interface RoutingGraphProps {
  group: VirtualModelDraft | null
  candidates: CandidateDraft[]
  quotas: QuotaWindowInfo[]
  providers: ProviderInfo[]
  accounts: AccountInfo[]
  entries: RoutingEntry[]
  activeCandidateId: string | null
  nowMs: number
}

export function RoutingGraph({
  group,
  candidates,
  quotas,
  providers,
  accounts,
  entries,
  activeCandidateId,
  nowMs,
}: RoutingGraphProps) {
  const quotaByCandidate = new Map(quotas.map((item) => [item.candidate_id, item]))
  const providerById = new Map(providers.map((item) => [item.id, item]))
  const accountById = new Map(accounts.map((item) => [item.id, item]))
  const candidateById = new Map(candidates.map((item) => [item.id, item]))

  const pool = (group?.candidate_pool ?? [])
    .map((id) => candidateById.get(id))
    .filter((item): item is CandidateDraft => item !== undefined)

  const entriesSorted = [...entries].sort((a, b) => b.count - a.count)

  return (
    <div className="app-route-graph">
      {/* 入口列 */}
      <div className="app-route-col">
        {entriesSorted.length === 0 ? (
          <p className="text-meta text-muted-foreground">还没有请求。</p>
        ) : (
          entriesSorted.map((entry) => (
            <div key={entry.protocol} className="app-route-entry-row">
              <div className="app-route-entry">
                <span className="app-status-dot bg-status-success" aria-hidden />
                <span className="text-ui-label truncate">{protocolLabel(entry.protocol)}</span>
                <span className="ml-auto text-meta text-muted-foreground tabular-nums">
                  {formatCompactNum(entry.count)} 次
                </span>
              </div>
              <span className="app-route-link" aria-hidden />
            </div>
          ))
        )}
      </div>

      {/* 路由节点 */}
      <div className="app-route-col">
        <div className="app-route-node">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="app-route-dot bg-primary" aria-hidden />
            <span className="text-ui-label font-semibold truncate">
              {group?.name || group?.id || '（未配置路由组）'}
            </span>
            <span className="app-pill">路由</span>
          </div>
          <p className="truncate font-mono text-micro text-muted-foreground">
            {group?.id ?? '—'}
          </p>
          <p className="text-meta text-muted-foreground">
            {group?.context_window
              ? `上下文 ${formatCompactNum(group.context_window)} token`
              : '上下文窗口未设置'}
          </p>
        </div>
        <span className="app-route-link" aria-hidden />
      </div>

      {/* 候选列 */}
      <div className="app-route-col">
        {pool.length === 0 ? (
          <p className="text-meta text-muted-foreground">还没有候选模型。</p>
        ) : (
          pool.map((candidate) => {
            const quota = quotaByCandidate.get(candidate.id)
            const state = quotaState(quota)
            const provider = providerById.get(candidate.provider)
            const accountId = quota?.account_id ?? null
            const account = accountId ? accountById.get(accountId) : undefined
            const active = candidate.id === activeCandidateId
            const used = quota && quota.limit !== null ? Math.max(quota.limit - (quota.remaining ?? 0), 0) : null
            const ratio =
              quota && quota.limit !== null && quota.limit > 0 && quota.remaining !== null
                ? Math.min(Math.max(quota.remaining / quota.limit, 0), 1)
                : null
            return (
              <div key={candidate.id} className="app-route-entry-row">
                <span className="app-route-link" aria-hidden />
                <div className={active ? 'app-route-card app-route-card-active' : 'app-route-card'}>
                  <div className="flex items-center gap-2 flex-wrap min-w-0">
                    <span className="text-ui-label font-semibold truncate">
                      {provider?.display_name ?? candidate.provider}
                    </span>
                    <span className="font-mono text-meta truncate">{candidate.model}</span>
                    {active && <span className="app-pill app-pill-ok">当前</span>}
                  </div>
                  <div className="flex items-center gap-2 flex-wrap min-w-0">
                    <span className="app-pill">tier {candidate.tier}</span>
                    <span className="app-pill">强度 {candidate.strength}</span>
                    <span className={quotaStatePill(state)}>{quotaStateLabel(state)}</span>
                  </div>
                  <p className="text-meta text-muted-foreground truncate">
                    {account ? `${account.label} · 尾号 ${account.fingerprint || '—'}` : '账号未关联'}
                  </p>
                  {quota && quota.remaining !== null && quota.limit !== null ? (
                    <div className="flex flex-col gap-1">
                      <div className="flex items-center gap-2 text-meta text-muted-foreground">
                        <span className="tabular-nums">
                          {formatCompactNum(quota.remaining)} / {formatCompactNum(quota.limit)}
                        </span>
                        {quota.reset_at && (
                          <span className="ml-auto">
                            {fmtRelative(quota.reset_at, nowMs)}重置
                          </span>
                        )}
                      </div>
                      <div className="app-route-bar">
                        <div
                          className="app-route-bar-fill"
                          style={{ width: `${Math.round((ratio ?? 0) * 100)}%` }}
                        />
                      </div>
                      {used !== null && used > 0 && (
                        <span className="text-micro text-muted-foreground">
                          已用 {formatCompactNum(used)}
                        </span>
                      )}
                    </div>
                  ) : null}
                </div>
              </div>
            )
          })
        )}
      </div>
    </div>
  )
}
