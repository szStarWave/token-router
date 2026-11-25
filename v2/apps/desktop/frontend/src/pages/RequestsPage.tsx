/**
 * 活动（§7.3）：路由视图 —— 网关现在怎么路由、刚才那次怎么路由、上下文用量与账号配额。
 *
 * 结构参照 Magpie 的「路由」页：
 *   1. 路由图（入口 → 路由组 → 候选账号）
 *   2. 指标条（请求 / 成功率 / 接力 / 模型数 / 缓存命中）
 *   3. 「最近一个请求是怎么路由的」叙事
 *   4. 上下文窗口与 token 分布
 *   5. 账号与配额
 *
 * 请求级明细（每轮消息、决策模型调用）在「活动 → 会话」：同一请求只在一个地方展开。
 *
 * 全部数字来自 admin 契约（账本 / 统计 / 配额观测）；没有的量不编。
 */
import { useEffect, useMemo, useState } from 'react'
import {
  useAllAccountsQuery,
  useConfigQuery,
  useQuotasQuery,
  useRequestDetailQuery,
  useRequestsQuery,
  useStatsSummaryQuery,
  useStatusQuery,
  useUsageQuery,
} from '../queries/admin'
import { usePrefsStore } from '../stores/prefsStore'
import { QueryState } from '../components/common/QueryState'
import { draftFromDocument } from '../lib/config-edit'
import { formatCompactNum } from '../lib/format-number'
import { RoutingGraph, type RoutingEntry } from './activity/RoutingGraph'
import { RoutingNarrative } from './activity/RoutingNarrative'
import { ContextPanel } from './activity/ContextPanel'
import { AccountQuotaPanel } from './activity/AccountQuotaPanel'

const REQUEST_LIMIT = 200

export function RequestsPage() {
  const [nowMs, setNowMs] = useState(() => Date.now())

  // 相对时间（配额重置倒计时）每 30 秒走一格。
  useEffect(() => {
    const timer = window.setInterval(() => setNowMs(Date.now()), 30_000)
    return () => window.clearInterval(timer)
  }, [])

  const configQuery = useConfigQuery()
  // 轻量模式没有 SSE，用轮询兜底（其他页面同理）。
  const liteMode = usePrefsStore((s) => s.liteMode)
  const statusQuery = useStatusQuery()
  const requestsQuery = useRequestsQuery(
    { limit: REQUEST_LIMIT },
    true,
    liteMode ? 10_000 : false,
  )
  const quotasQuery = useQuotasQuery()
  const accountsQuery = useAllAccountsQuery()
  const statsQuery = useStatsSummaryQuery()

  // 窗口必须稳定：Date.now() 进查询键会导致每次渲染都生成新查询（无限重取）。
  const windowFrom = 0
  const usageModelQuery = useUsageQuery('model', windowFrom, nowMs)
  const usageSessionQuery = useUsageQuery('session', windowFrom, nowMs)

  const requests = requestsQuery.data?.items ?? []
  const newest = requests.length > 0 ? requests[0] : null

  const newestDetailQuery = useRequestDetailQuery(newest?.request_id ?? null)

  const draft = useMemo(
    () => (configQuery.data ? draftFromDocument(configQuery.data) : null),
    [configQuery.data],
  )

  const group = useMemo(() => {
    if (!draft) return null
    // 默认组已停用时不要展示它，否则“当前路由”与运行时不一致。
    const byId = draft.virtualModels.find(
      (item) => item.id === draft.defaultVirtualModel && item.enabled,
    )
    if (byId) return byId
    return draft.virtualModels.find((item) => item.enabled) ?? draft.virtualModels[0] ?? null
  }, [draft])

  // 入口分布：按协议统计本次拉到的请求。
  const entries = useMemo<RoutingEntry[]>(() => {
    const counts = new Map<string, number>()
    for (const row of requests) {
      counts.set(row.ingress_protocol, (counts.get(row.ingress_protocol) ?? 0) + 1)
    }
    return [...counts.entries()].map(([protocol, count]) => ({ protocol, count }))
  }, [requests])

  // 高亮候选：最近一次请求实际派发到的候选（尝试序列里的最后一个）。
  const activeCandidateId = useMemo(() => {
    const detail = newestDetailQuery.data
    if (!detail || detail.attempts.length === 0) return null
    const successful = [...detail.attempts].reverse().find((item) => item.outcome === 'success')
    return (successful ?? detail.attempts[detail.attempts.length - 1]).candidate_id
  }, [newestDetailQuery.data])

  const stats = statsQuery.data
  const usageTotals = usageModelQuery.data?.totals
  const tokens = {
    input: stats?.tokens.input ?? usageTotals?.input_tokens ?? 0,
    output: stats?.tokens.output ?? usageTotals?.output_tokens ?? 0,
    cacheRead: stats?.tokens.cache_read ?? usageTotals?.cache_read_tokens ?? 0,
    cacheWrite: stats?.tokens.cache_write ?? usageTotals?.cache_write_tokens ?? 0,
  }
  const cacheHit =
    usageTotals?.cache_hit_ratio ??
    (tokens.input + tokens.cacheRead > 0
      ? tokens.cacheRead / (tokens.input + tokens.cacheRead)
      : null)

  const loading = requestsQuery.isLoading && requests.length === 0
  const error = requestsQuery.error
  // 路由图来自磁盘配置；与运行中 digest 不一致时，先把话说清楚。
  const configPendingRestart =
    configQuery.data !== undefined &&
    statusQuery.data !== undefined &&
    configQuery.data.revision !== statusQuery.data.config_digest

  return (
    <div className="app-page-wide">
      {/* 1. 路由图 */}
      <section className="app-card" aria-label="路由">
        <div className="flex items-center gap-3 flex-wrap">
          <h2 className="app-card-title">路由</h2>
          <span
            className={
              statusQuery.isSuccess
                ? 'app-status-dot bg-status-success'
                : 'app-status-dot bg-status-warning'
            }
            aria-hidden
          />
          <span className="text-meta text-muted-foreground">
            {statusQuery.isSuccess
              ? '实时 · 关注网关正在做的决定'
              : '未连接 · 显示最近一次读到的数据'}
          </span>
        </div>
        {configPendingRestart && (
          <div className="app-banner app-banner-warn text-meta">
            网关尚未重启，实际派发仍按旧配置进行。
          </div>
        )}
        <QueryState loading={loading} error={error} onRetry={() => void requestsQuery.refetch()}>
          <RoutingGraph
            group={group}
            candidates={draft?.candidates ?? []}
            quotas={quotasQuery.data?.items ?? []}
            providers={accountsQuery.providers}
            accounts={accountsQuery.accounts}
            entries={entries}
            activeCandidateId={activeCandidateId}
            nowMs={nowMs}
          />
        </QueryState>
      </section>

      {/* 2. 指标条 */}
      <section className="app-metric-grid" aria-label="指标">
        <div className="app-metric-card">
          <span className="app-metric-key">请求</span>
          <span className="app-metric-value">{formatCompactNum(stats?.requests ?? 0)}</span>
        </div>
        <div className="app-metric-card">
          <span className="app-metric-key">成功率</span>
          <span className="app-metric-value">
            {stats ? `${(stats.success_rate * 100).toFixed(1)}%` : '—'}
          </span>
          <span className="app-metric-note">
            成功 {formatCompactNum(stats?.successes ?? 0)} · 失败{' '}
            {formatCompactNum(
              (stats?.upstream_errors ?? 0) + (stats?.quality_fails ?? 0) + (stats?.cancelled ?? 0),
            )}
          </span>
        </div>
        <div className="app-metric-card">
          <span className="app-metric-key">接力切换</span>
          <span className="app-metric-value">{formatCompactNum(stats?.switches ?? 0)}</span>
        </div>
        <div className="app-metric-card">
          <span className="app-metric-key">决策模型</span>
          <span className="app-metric-value">{formatCompactNum(stats?.judge.calls ?? 0)}</span>
          <span
            className={
              stats && stats.judge.failed > 0
                ? 'app-metric-note text-status-warning'
                : 'app-metric-note'
            }
          >
            失败 {formatCompactNum(stats?.judge.failed ?? 0)}
          </span>
        </div>
        <div className="app-metric-card">
          <span className="app-metric-key">涉及模型</span>
          <span className="app-metric-value">
            {formatCompactNum(usageModelQuery.data?.rows.length ?? 0)}
          </span>
          <span className="app-metric-note">
            会话 {formatCompactNum(usageSessionQuery.data?.rows.length ?? 0)} 个
          </span>
        </div>
        <div className="app-metric-card">
          <span className="app-metric-key">缓存命中</span>
          <span className="app-metric-value">
            {cacheHit === null ? '—' : `${(cacheHit * 100).toFixed(1)}%`}
          </span>
        </div>
      </section>

      {/* 3. 最近一次请求的路由叙事 */}
      <section className="app-card" aria-label="最近一次请求">
        <h2 className="app-card-title">最近一个请求是怎么路由的</h2>
        <RoutingNarrative
          summary={newest}
          detail={newestDetailQuery.data ?? null}
        />
      </section>

      {/* 4. 上下文窗口 */}
      <section className="app-card" aria-label="上下文窗口">
        <h2 className="app-card-title">上下文窗口</h2>
        <ContextPanel
          contextWindow={group?.context_window ?? null}
          tokens={tokens}
          cacheHitRatio={cacheHit}
          sessions={usageSessionQuery.data?.rows.length ?? 0}
        />
      </section>

      {/* 5. 账号与配额 */}
      <section className="app-card" aria-label="账号与配额">
        <h2 className="app-card-title">账号与配额</h2>
        <AccountQuotaPanel
          providers={accountsQuery.providers}
          accounts={accountsQuery.accounts}
          quotas={quotasQuery.data?.items ?? []}
          nowMs={nowMs}
        />
      </section>
    </div>
  )
}
