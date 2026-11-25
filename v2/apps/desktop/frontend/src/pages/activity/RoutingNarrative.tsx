/**
 * 「最近一个请求是怎么路由的」：把一条请求的决策讲成人话。
 *
 * 参照 Magpie 的路由叙事（谁要、网关怎么选、落到谁、结果如何）。
 * 数据来自请求详情快照：入口 / 选择模型 / 派发模型与供应商 / 尝试序列 /
 * 策略贡献 / 原因码，不重算。
 */
import type { RequestDetail, RequestSummary } from '../../api/types'
import { fmtCost, fmtDuration, outcomeLabel, protocolLabel } from './shared'

export interface RoutingNarrativeProps {
  summary: RequestSummary | null
  detail: RequestDetail | null
}

export function RoutingNarrative({ summary, detail }: RoutingNarrativeProps) {
  if (!summary) {
    return <p className="text-meta text-muted-foreground">还没有请求记录。</p>
  }

  const contributions = detail && Array.isArray(detail.contributions)
    ? (detail.contributions as Array<Record<string, unknown>>)
    : []
  const strategies = [...new Set(contributions.map((entry) => String(entry.strategy_id ?? '')))]
    .filter(Boolean)
  const attempts = detail?.attempts ?? []

  return (
    <ul className="app-narrative">
      <li>
        <span className="text-ui-label">
          客户端通过 {protocolLabel(summary.ingress_protocol)} 请求了{' '}
          <span className="font-mono">{summary.selection_model}</span>
        </span>
      </li>
      <li>
        <span className="text-ui-label">
          {summary.dispatch_model ? (
            <>
              网关把它派发到{' '}
              <span className="font-mono">{summary.dispatch_model}</span>
              {detail?.dispatch_provider ? ` @ ${detail.dispatch_provider}` : ''}
              {summary.dispatch_source ? `（${summary.dispatch_source}）` : ''}
            </>
          ) : (
            '这次请求没有派发到任何上游'
          )}
          {strategies.length > 0 && (
            <>
              ，参与的策略：{strategies.join('、')}
            </>
          )}
        </span>
      </li>
      {attempts.length > 0 && (
        <li>
          <span className="text-ui-label">
            尝试 {attempts.length} 次：
            {attempts.map((attempt) => attempt.candidate_id).join(' → ')}
          </span>
        </li>
      )}
      <li>
        <span className="text-ui-label">
          结果 {outcomeLabel(summary.outcome)} · 耗时 {fmtDuration(summary.duration_ms)} · 首字节{' '}
          {fmtDuration(summary.ttft_ms)} ·{' '}
          {fmtCost(summary.cost_micros.answer + summary.cost_micros.routing)}
          {summary.reason_codes.length > 0 && ` · 原因码 ${summary.reason_codes.join(', ')}`}
        </span>
      </li>
    </ul>
  )
}
