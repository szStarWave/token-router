/**
 * 会话详情：一个 session 的**完整调用**——客户端请求（每一轮）+ 决策模型（判官）调用。
 *
 * 汇总来自服务端会话聚合（`GET /v2/admin/sessions`，不受请求列表分页影响）；
 * 明细来自按会话过滤的请求列表（翻页拉全）与 `GET /v2/admin/judge-calls?session=…`。
 * 决策模型调用按 `request_id` 关联到那一轮请求，点开可看输入摘要/答案。
 */
import type { JudgeCallSummary, RequestSummary, SessionSummary } from '../../api/types'
import { formatCompactNum } from '../../lib/format-number'
import {
  fmtClock,
  fmtCost,
  fmtDuration,
  judgeFailureLabel,
  judgeOutcomeLabel,
  judgeSourceLabel,
  outcomeLabel,
} from './shared'

export interface SessionPanelProps {
  sessionKey: string
  session: SessionSummary | null
  requests: RequestSummary[]
  judgeCalls: JudgeCallSummary[]
  /** 点请求行：看该轮请求详情。 */
  onSelectRequest?: (id: string) => void
  /** 点决策行：看这次决策模型调用的输入与答案。 */
  onSelectJudge?: (call: JudgeCallSummary) => void
}

interface CallRow {
  at: number
  request?: RequestSummary
  judge?: JudgeCallSummary
}

export function SessionPanel({
  sessionKey,
  session,
  requests,
  judgeCalls,
  onSelectRequest,
  onSelectJudge,
}: SessionPanelProps) {
  const rows: CallRow[] = [
    ...requests.map((request) => ({ at: request.created_at, request })),
    ...judgeCalls.map((judge) => ({ at: judge.created_at, judge })),
  ].sort((a, b) => b.at - a.at)

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2 flex-wrap">
        <span className="text-ui-label font-semibold">会话</span>
        <span className="font-mono text-meta min-w-0 truncate">{sessionKey}</span>
        {session && (
          <>
            <span className="app-pill">{session.requests} 个请求</span>
            {session.failures > 0 && (
              <span className="app-pill app-pill-warn">{session.failures} 个失败</span>
            )}
            <span className="app-pill">{session.judge_calls} 次判官调用</span>
            {session.judge_failures > 0 && (
              <span className="app-pill app-pill-warn">{session.judge_failures} 次判官失败</span>
            )}
            <span className="text-meta text-muted-foreground">
              Token {formatCompactNum(session.tokens.input)} in /{' '}
              {formatCompactNum(session.tokens.output)} out · 成本{' '}
              {fmtCost(session.cost_micros.answer + session.cost_micros.routing)}
              {session.judge_cost_micros > 0
                ? `（其中判官 ${fmtCost(session.judge_cost_micros)}）`
                : ''}
            </span>
          </>
        )}
      </div>

      {rows.length === 0 ? (
        <p className="text-meta text-muted-foreground">这个会话没有调用记录。</p>
      ) : (
        <table className="app-table">
          <thead>
            <tr>
              <th>时间</th>
              <th>类型</th>
              <th>对象</th>
              <th>状态</th>
              <th>耗时</th>
              <th>Token</th>
              <th>成本</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) =>
              row.request ? (
                <tr
                  key={row.request.request_id}
                  className={onSelectRequest ? 'cursor-pointer' : undefined}
                  onClick={
                    onSelectRequest
                      ? () => onSelectRequest(row.request?.request_id ?? '')
                      : undefined
                  }
                >
                  <td className="font-mono tabular-nums">{fmtClock(row.at)}</td>
                  <td>
                    <span className="app-pill">请求</span>
                  </td>
                  <td className="font-mono">
                    {row.request.selection_model} → {row.request.dispatch_model ?? '未派发'}
                  </td>
                  <td>{outcomeLabel(row.request.outcome)}</td>
                  <td>{fmtDuration(row.request.duration_ms)}</td>
                  <td>
                    {formatCompactNum(row.request.tokens.input)} in /{' '}
                    {formatCompactNum(row.request.tokens.output)} out
                  </td>
                  <td>
                    {fmtCost(row.request.cost_micros.answer + row.request.cost_micros.routing)}
                  </td>
                </tr>
              ) : (
                row.judge && (
                  <tr
                    key={`${row.judge.request_id}-${row.judge.created_at}-${row.judge.question_id}`}
                    className={onSelectJudge ? 'cursor-pointer' : undefined}
                    onClick={
                      onSelectJudge ? () => onSelectJudge(row.judge as JudgeCallSummary) : undefined
                    }
                  >
                    <td className="font-mono tabular-nums">{fmtClock(row.at)}</td>
                    <td>
                      <span className="app-pill">决策</span>
                    </td>
                    <td>
                      <span className="font-mono">
                        {row.judge.question_id} · {row.judge.service_id}
                      </span>
                      {judgeSourceLabel(row.judge.effective_source) && (
                        <span className="app-pill ml-1.5">
                          {judgeSourceLabel(row.judge.effective_source)}
                        </span>
                      )}
                    </td>
                    <td
                      title={`${row.judge.outcome}${row.judge.failure_code ? ` / ${row.judge.failure_code}` : ''}`}
                    >
                      {judgeOutcomeLabel(row.judge.outcome)}
                      {row.judge.failure_code ? ` · ${judgeFailureLabel(row.judge.failure_code)}` : ''}
                    </td>
                    <td>{fmtDuration(row.judge.latency_ms)}</td>
                    <td>
                      {formatCompactNum(row.judge.prompt_tokens ?? 0)} in /{' '}
                      {formatCompactNum(row.judge.completion_tokens ?? 0)} out
                    </td>
                    <td>{row.judge.cost_micros === null ? '—' : fmtCost(row.judge.cost_micros)}</td>
                  </tr>
                )
              ),
            )}
          </tbody>
        </table>
      )}
    </div>
  )
}
