/**
 * 一次决策模型（判官 / JEV）调用的完整记录：本次发送的任务与 prompt、选项、答案。
 *
 * 数据来自 `JudgeCallSummary.request/response/failure_message`（服务端落库摘要），
 * 与会话页「决策」行一一对应；不重新拼 prompt，也不回放请求。
 */
import type { JudgeCallSummary } from '../../api/types'
import { formatCompactNum } from '../../lib/format-number'
import { JudgeCallDetailBlock } from './RequestDetailPanel'
import {
  fmtClock,
  fmtCost,
  fmtDuration,
  judgeFailureLabel,
  judgeOutcomeLabel,
  judgeSourceLabel,
} from './shared'

export function JudgeCallPanel({ call }: { call: JudgeCallSummary }) {
  const source = judgeSourceLabel(call.effective_source)
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2 flex-wrap">
        <span className="text-ui-label font-semibold">决策模型调用</span>
        <span className="font-mono text-meta">{call.question_id}</span>
        <span className="font-mono text-meta">{call.service_id}</span>
        {source && <span className="app-pill">{source}</span>}
        <span title={`${call.outcome}${call.failure_code ? ` / ${call.failure_code}` : ''}`}>
          {judgeOutcomeLabel(call.outcome)}
          {call.failure_code ? ` · ${judgeFailureLabel(call.failure_code)}` : ''}
        </span>
        <span className="text-meta text-muted-foreground">
          {fmtClock(call.created_at)} · {fmtDuration(call.latency_ms)} ·{' '}
          {formatCompactNum(call.prompt_tokens ?? 0)} in /{' '}
          {formatCompactNum(call.completion_tokens ?? 0)} out ·{' '}
          {call.cost_micros === null ? '—' : fmtCost(call.cost_micros)}
        </span>
      </div>
      <JudgeCallDetailBlock call={call} />
    </div>
  )
}
