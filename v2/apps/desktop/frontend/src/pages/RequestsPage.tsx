/**
 * Requests（§7.3）：账本列表 + 详情。
 * 过滤：会话/模型/outcome/协议/原因码；详情含 plan/judge/contributions。
 */
import { useState } from 'react'
import { useRequestsQuery, useRequestDetailQuery } from '../queries/admin'
import { QueryState } from '../components/common/QueryState'
import { Icon } from '../components/common/Icon'
import { outcomePillClass } from '../lib/status'

const OUTCOMES = ['', 'success', 'upstream_error', 'quality_fail', 'cancelled', 'rejected']
const PROTOCOLS = ['', 'chat', 'anthropic_messages', 'responses']

function fmtTime(ms: number): string {
  return new Date(ms).toLocaleString()
}

export function RequestsPage() {
  const [outcome, setOutcome] = useState('')
  const [protocol, setProtocol] = useState('')
  const [model, setModel] = useState('')
  const [limit, setLimit] = useState(20)
  const [selectedId, setSelectedId] = useState<string | null>(null)

  const listQuery = useRequestsQuery({
    outcome: outcome || undefined,
    protocol: protocol || undefined,
    model: model || undefined,
    limit,
  })
  const detailQuery = useRequestDetailQuery(selectedId)

  return (
    <div className="app-page app-page-split">
      <section className="app-card" aria-label="请求列表">
        <div className="app-filter-bar">
          <label className="app-filter-label">
            结果
            <select
              className="app-select-sm"
              value={outcome}
              onChange={(e) => setOutcome(e.target.value)}
            >
              {OUTCOMES.map((value) => (
                <option key={value} value={value}>
                  {value || '全部'}
                </option>
              ))}
            </select>
          </label>
          <label className="app-filter-label">
            协议
            <select
              className="app-select-sm"
              value={protocol}
              onChange={(e) => setProtocol(e.target.value)}
            >
              {PROTOCOLS.map((value) => (
                <option key={value} value={value}>
                  {value || '全部'}
                </option>
              ))}
            </select>
          </label>
          <label className="app-filter-label">
            模型
            <input
              type="text"
              className="app-field-sm h-7 w-36"
              value={model}
              placeholder="如 glm-5.3-flash"
              onChange={(e) => setModel(e.target.value)}
            />
          </label>
          <label className="app-filter-label">
            条数
            <select
              className="app-select-sm"
              value={limit}
              onChange={(e) => setLimit(Number(e.target.value))}
            >
              {[20, 50, 100].map((value) => (
                <option key={value} value={value}>
                  {value}
                </option>
              ))}
            </select>
          </label>
        </div>

        <QueryState
          loading={listQuery.isLoading}
          error={listQuery.error}
          empty={(listQuery.data?.items.length ?? 0) === 0}
          onRetry={() => void listQuery.refetch()}
        >
          <table className="app-table app-table-clickable">
            <thead>
              <tr>
                <th>时间</th>
                <th>模型</th>
                <th>结果</th>
                <th>耗时</th>
                <th>tokens</th>
                <th>成本</th>
              </tr>
            </thead>
            <tbody>
              {listQuery.data?.items.map((row) => (
                <tr
                  key={row.request_id}
                  className={selectedId === row.request_id ? 'app-row-selected' : undefined}
                  onClick={() => setSelectedId(row.request_id)}
                >
                  <td className="font-mono">{fmtTime(row.created_at)}</td>
                  <td className="font-mono">{row.dispatch_model ?? row.selection_model}</td>
                  <td>
                    <span className={outcomePillClass(row.outcome)}>{row.outcome}</span>
                  </td>
                  <td>{row.duration_ms}ms</td>
                  <td>
                    {row.tokens.input}/{row.tokens.output}
                  </td>
                  <td>{row.cost_micros.answer + row.cost_micros.routing}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {listQuery.data?.next_cursor && (
            <p className="app-card-sub">还有更多（游标分页，v1 先用条数过滤）。</p>
          )}
        </QueryState>
      </section>

      <section className="app-card" aria-label="请求详情">
        {selectedId === null ? (
          <p className="app-card-sub">选择左侧一行查看详情。</p>
        ) : (
          <QueryState
            loading={detailQuery.isLoading}
            error={detailQuery.error}
            onRetry={() => void detailQuery.refetch()}
          >
            {detailQuery.data && (
              <>
                <h2 className="app-card-title">请求详情</h2>
                <div className="app-detail-copy">
                  <span className="font-mono text-muted-foreground">
                    {detailQuery.data.request_id}
                  </span>
                  <button
                    type="button"
                    className="oc-btn-default oc-btn-sm"
                    onClick={() => void navigator.clipboard.writeText(detailQuery.data.request_id)}
                  >
                    <Icon name="copy" />
                    复制 ID
                  </button>
                </div>
                <dl className="app-kv">
                  <div className="app-kv-item">
                    <dt className="app-kv-key">时间</dt>
                    <dd className="app-kv-value">{fmtTime(detailQuery.data.created_at)}</dd>
                  </div>
                  <div className="app-kv-item">
                    <dt className="app-kv-key">会话</dt>
                    <dd className="app-kv-value font-mono">{detailQuery.data.session_key}</dd>
                  </div>
                  <div className="app-kv-item">
                    <dt className="app-kv-key">分支</dt>
                    <dd className="app-kv-value font-mono">{detailQuery.data.branch_id}</dd>
                  </div>
                  <div className="app-kv-item">
                    <dt className="app-kv-key">协议</dt>
                    <dd className="app-kv-value">{detailQuery.data.ingress_protocol}</dd>
                  </div>
                  <div className="app-kv-item">
                    <dt className="app-kv-key">选择模型</dt>
                    <dd className="app-kv-value font-mono">{detailQuery.data.selection_model}</dd>
                  </div>
                  <div className="app-kv-item">
                    <dt className="app-kv-key">派发模型</dt>
                    <dd className="app-kv-value font-mono">
                      {detailQuery.data.dispatch_model ?? '—'}
                    </dd>
                  </div>
                  <div className="app-kv-item">
                    <dt className="app-kv-key">派发来源</dt>
                    <dd className="app-kv-value">{detailQuery.data.dispatch_source ?? '—'}</dd>
                  </div>
                  <div className="app-kv-item">
                    <dt className="app-kv-key">结果</dt>
                    <dd className="app-kv-value">
                      {detailQuery.data.outcome} (HTTP {detailQuery.data.http_status ?? '—'})
                    </dd>
                  </div>
                  <div className="app-kv-item">
                    <dt className="app-kv-key">TTFT</dt>
                    <dd className="app-kv-value">{detailQuery.data.ttft_ms ?? '—'}ms</dd>
                  </div>
                  <div className="app-kv-item">
                    <dt className="app-kv-key">总耗时</dt>
                    <dd className="app-kv-value">{detailQuery.data.duration_ms}ms</dd>
                  </div>
                  <div className="app-kv-item">
                    <dt className="app-kv-key">tokens</dt>
                    <dd className="app-kv-value">
                      {detailQuery.data.tokens.input} / cache_read{' '}
                      {detailQuery.data.tokens.cache_read} / out {detailQuery.data.tokens.output}
                    </dd>
                  </div>
                  <div className="app-kv-item">
                    <dt className="app-kv-key">成本 (micros)</dt>
                    <dd className="app-kv-value">
                      answer {detailQuery.data.cost_micros.answer} · routing{' '}
                      {detailQuery.data.cost_micros.routing}
                    </dd>
                  </div>
                  <div className="app-kv-item">
                    <dt className="app-kv-key">计划模式</dt>
                    <dd className="app-kv-value">{detailQuery.data.plan_mode}</dd>
                  </div>
                  <div className="app-kv-item">
                    <dt className="app-kv-key">切换次数</dt>
                    <dd className="app-kv-value">{detailQuery.data.switches}</dd>
                  </div>
                  <div className="app-kv-item">
                    <dt className="app-kv-key">plan_id</dt>
                    <dd className="app-kv-value font-mono">{detailQuery.data.plan_id}</dd>
                  </div>
                  <div className="app-kv-item">
                    <dt className="app-kv-key">原因码</dt>
                    <dd className="app-kv-value">
                      {detailQuery.data.reason_codes.length === 0
                        ? '—'
                        : detailQuery.data.reason_codes.join(', ')}
                    </dd>
                  </div>
                </dl>

                <h3 className="text-ui-label font-medium text-muted-foreground">路由时间线</h3>
                <RoutingTimeline detail={detailQuery.data} />

                <h3 className="text-ui-label font-medium text-muted-foreground">尝试</h3>
                {detailQuery.data.attempts.length === 0 ? (
                  <p className="app-card-sub">无尝试记录。</p>
                ) : (
                  <table className="app-table">
                    <thead>
                      <tr>
                        <th>候选</th>
                        <th>结果</th>
                        <th>耗时</th>
                        <th>错误类</th>
                      </tr>
                    </thead>
                    <tbody>
                      {detailQuery.data.attempts.map((attempt, index) => (
                        <tr key={index}>
                          <td className="font-mono">{attempt.candidate_id}</td>
                          <td>{attempt.outcome}</td>
                          <td>{attempt.latency_ms ?? '—'}ms</td>
                          <td>{attempt.error_class ?? '—'}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                )}

                <h3 className="text-ui-label font-medium text-muted-foreground">策略贡献</h3>
                <pre className="app-code-block">
                  {JSON.stringify(detailQuery.data.contributions, null, 2)}
                </pre>
              </>
            )}
          </QueryState>
        )}
      </section>
    </div>
  )
}

/**
 * 路由时间线（v3 P5-C）：Selection → Strategies → Plan → Dispatch → (Failover/Judge) → Outcome。
 * 数据全部来自决策快照（contributions/preference/failover/judge），不重算。
 */
function RoutingTimeline({
  detail,
}: {
  detail: {
    selection_model: string
    selection_level: string
    dispatch_model: string | null
    dispatch_provider: string | null
    dispatch_source: string | null
    dispatch_level: string | null
    plan_mode: string
    preference: unknown
    failover: unknown
    switches: number
    contributions: unknown
    judge: unknown
    outcome: string
    reason_codes: string[]
    ttft_ms: number | null
    duration_ms: number
  }
}) {
  const contributions = Array.isArray(detail.contributions)
    ? (detail.contributions as Array<Record<string, unknown>>)
    : []
  const preference = Array.isArray(detail.preference)
    ? (detail.preference as Array<Record<string, unknown>>)
    : []
  const failover = Array.isArray(detail.failover)
    ? (detail.failover as Array<Record<string, unknown>>)
    : []
  const judge = detail.judge as Record<string, unknown> | null

  const steps: Array<{ label: string; body: React.ReactNode; tone?: string }> = [
    {
      label: '选择（Selection）',
      body: (
        <span className="font-mono">
          {detail.selection_model} @ {detail.selection_level}
        </span>
      ),
    },
    {
      label: `策略（${contributions.length} 条贡献）`,
      body:
        contributions.length === 0 ? (
          <span className="text-muted-foreground">无贡献记录</span>
        ) : (
          <ul className="flex flex-col gap-1">
            {contributions.map((entry, index) => {
              const strategy = String(entry.strategy_id ?? '?')
              const contribution = (entry.contribution ?? {}) as Record<string, unknown>
              const kind = Object.keys(contribution)[0] ?? 'note'
              const payload = contribution[kind] as Record<string, unknown> | undefined
              const reason =
                payload && 'reason' in payload
                  ? String((payload.reason as Record<string, unknown> | undefined)?.code ?? '')
                  : ''
              return (
                <li key={index} className="text-meta flex items-center gap-2 flex-wrap">
                  <span className="app-pill">{strategy}</span>
                  <span className="text-muted-foreground">{kind}</span>
                  {reason && <span className="font-mono">{reason}</span>}
                </li>
              )
            })}
          </ul>
        ),
    },
    {
      label: '计划（Plan）',
      body:
        preference.length === 0 ? (
          <span className="text-muted-foreground">无偏好记录</span>
        ) : (
          <span className="font-mono">
            {preference
              .map((slot) => String(slot.candidate_id ?? slot.id ?? '?'))
              .join(' → ')}
          </span>
        ),
    },
    {
      label: '派发（Dispatch）',
      body: (
        <span className="font-mono">
          {detail.dispatch_model ?? '—'}
          {detail.dispatch_provider ? ` @ ${detail.dispatch_provider}` : ''}
          {detail.dispatch_source ? ` · ${detail.dispatch_source}` : ''}
        </span>
      ),
    },
  ]

  if (failover.length > 0) {
    steps.push({
      label: '接力（Failover）',
      body: (
        <span className="font-mono">
          {failover
            .map((slot) => String(slot.candidate_id ?? slot.id ?? '?'))
            .join(' → ')}
        </span>
      ),
    })
  }
  if (judge) {
    const verdicts = judge.verdicts ?? judge.answers ?? null
    steps.push({
      label: '判官（Judge）',
      body: (
        <pre className="app-code-block max-h-40 overflow-auto">
          {JSON.stringify(verdicts ?? judge, null, 2)}
        </pre>
      ),
    })
  }
  steps.push({
    label: '结果（Outcome）',
    tone:
      detail.outcome === 'success'
        ? 'app-pill app-pill-ok'
        : 'app-pill app-pill-error',
    body: (
      <span className="text-meta">
        {detail.outcome} · {detail.plan_mode} · 切换 {detail.switches} 次 · TTFT{' '}
        {detail.ttft_ms ?? '—'}ms · 耗时 {detail.duration_ms}ms
        {detail.reason_codes.length > 0 ? ` · ${detail.reason_codes.join(', ')}` : ''}
      </span>
    ),
  })

  return (
    <ol className="app-plan-entry">
      {steps.map((step, index) => (
        <li key={index} className="flex gap-3 py-1.5">
          <span className="text-meta text-muted-foreground w-40 shrink-0">{step.label}</span>
          <span className="min-w-0 flex-1">{step.body}</span>
        </li>
      ))}
    </ol>
  )
}
