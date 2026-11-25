/**
 * 单条请求的详情：路由叙事 + 决策快照 + 尝试序列 + 这一轮的消息。
 *
 * 全部字段来自 admin 契约的 `RequestDetail`（决策快照）与 `RequestTranscript`（消息），
 * 不重算路由。
 */
import { useState } from 'react'
import type { JudgeCallSummary, RequestDetail, TranscriptMessage } from '../../api/types'
import { useRequestTranscriptQuery } from '../../queries/admin'
import { Icon } from '../../components/common/Icon'
import { formatCompactNum } from '../../lib/format-number'
import { outcomePillClass } from '../../lib/status'
import {
  fmtClock,
  fmtCost,
  fmtDuration,
  judgeFailureLabel,
  judgeOutcomeLabel,
  judgeSourceLabel,
  outcomeLabel,
  protocolLabel,
} from './shared'

export interface RequestDetailPanelProps {
  detail: RequestDetail
}

/** preference / failover 项：契约里是候选 id 字符串数组；兼容对象形态（旧快照）。 */
function slotLabel(slot: unknown): string {
  if (typeof slot === 'string') return slot
  if (slot && typeof slot === 'object') {
    const record = slot as Record<string, unknown>
    return String(record.candidate_id ?? record.id ?? '?')
  }
  return '?'
}

export function RequestDetailPanel({ detail }: RequestDetailPanelProps) {
  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center gap-2 flex-wrap">
        <span className="text-ui-label font-semibold">路由经过</span>
        <span className="font-mono text-meta">{protocolLabel(detail.ingress_protocol)}</span>
        <span className="text-muted-foreground">→</span>
        <span className="font-mono text-meta">{detail.selection_model}</span>
        <span className="text-muted-foreground">→</span>
        <span className="font-mono text-meta">{detail.dispatch_model ?? '未派发'}</span>
        <span className={outcomePillClass(detail.outcome)}>{outcomeLabel(detail.outcome)}</span>
      </div>

      <div className="app-detail-copy">
        <span className="font-mono text-muted-foreground">{detail.request_id}</span>
        <button
          type="button"
          className="oc-btn-default oc-btn-sm"
          onClick={() => void navigator.clipboard.writeText(detail.request_id)}
        >
          <Icon name="copy" />
          复制 ID
        </button>
      </div>

      <dl className="app-kv">
        <div className="app-kv-item">
          <dt className="app-kv-key">时间</dt>
          <dd className="app-kv-value">{fmtClock(detail.created_at)}</dd>
        </div>
        <div className="app-kv-item">
          <dt className="app-kv-key">会话</dt>
          <dd className="app-kv-value font-mono truncate">{detail.session_key}</dd>
        </div>
        <div className="app-kv-item">
          <dt className="app-kv-key">分支</dt>
          <dd className="app-kv-value font-mono truncate">{detail.branch_id}</dd>
        </div>
        <div className="app-kv-item">
          <dt className="app-kv-key">选择层级</dt>
          <dd className="app-kv-value">{detail.selection_level}</dd>
        </div>
        <div className="app-kv-item">
          <dt className="app-kv-key">派发来源</dt>
          <dd className="app-kv-value">{detail.dispatch_source ?? '—'}</dd>
        </div>
        <div className="app-kv-item">
          <dt className="app-kv-key">TTFT</dt>
          <dd className="app-kv-value">{fmtDuration(detail.ttft_ms)}</dd>
        </div>
        <div className="app-kv-item">
          <dt className="app-kv-key">总耗时</dt>
          <dd className="app-kv-value">{fmtDuration(detail.duration_ms)}</dd>
        </div>
        <div className="app-kv-item">
          <dt className="app-kv-key">tokens</dt>
          <dd className="app-kv-value">
            in {detail.tokens.input} · cache_read {detail.tokens.cache_read} · out{' '}
            {detail.tokens.output} · cache_write {detail.cache_write_tokens}
          </dd>
        </div>
        <div className="app-kv-item">
          <dt className="app-kv-key">成本</dt>
          <dd className="app-kv-value">
            {fmtCost(detail.cost_micros.answer + detail.cost_micros.routing)}
          </dd>
        </div>
        <div className="app-kv-item">
          <dt className="app-kv-key">计划模式</dt>
          <dd className="app-kv-value">{detail.plan_mode}</dd>
        </div>
        <div className="app-kv-item">
          <dt className="app-kv-key">切换次数</dt>
          <dd className="app-kv-value">{detail.switches}</dd>
        </div>
        <div className="app-kv-item">
          <dt className="app-kv-key">原因码</dt>
          <dd className="app-kv-value">
            {detail.reason_codes.length === 0 ? '—' : detail.reason_codes.join(', ')}
          </dd>
        </div>
      </dl>

      <section className="app-detail-section">
        <h3 className="text-ui-label font-medium text-muted-foreground">路由时间线</h3>
        <RoutingTimeline detail={detail} />
      </section>

      <MessageSection requestId={detail.request_id} />

      <section className="app-detail-section">
        <h3 className="text-ui-label font-medium text-muted-foreground">尝试</h3>
        {detail.attempts.length === 0 ? (
          <p className="text-meta text-muted-foreground">无尝试记录。</p>
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
              {detail.attempts.map((attempt, index) => (
                <tr key={index}>
                  <td className="font-mono">{attempt.candidate_id}</td>
                  <td>{outcomeLabel(attempt.outcome)}</td>
                  <td>{attempt.latency_ms === null ? '—' : `${attempt.latency_ms}ms`}</td>
                  <td>{attempt.error_class ?? '—'}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </div>
  )
}

/**
 * 这一轮的消息：发给上游的输入 + 模型回复。
 *
 * 正文由服务端展平（图片/音频只留占位符，工具调用/结果展开为文本）；
 * 是否记录由配置 `[recording] transcript` 控制，历史请求可能没有记录。
 */
function MessageSection({ requestId }: { requestId: string }) {
  const query = useRequestTranscriptQuery(requestId)
  const transcript = query.data

  return (
    <section className="app-detail-section">
      <h3 className="text-ui-label font-medium text-muted-foreground">消息</h3>
      {query.isLoading ? (
        <p className="text-meta text-muted-foreground">加载中…</p>
      ) : query.error ? (
        <p className="text-meta text-status-error">{query.error.message}</p>
      ) : transcript && transcript.recorded ? (
        <div className="flex flex-col gap-3">
          <MessageGroup label="输入" messages={transcript.input} />
          <MessageGroup label="回复" messages={transcript.output} />
          {transcript.truncated && (
            <span
              className="app-pill app-pill-warn self-start"
              title="单条超过 20,000 字符或单轮超过 200,000 字符的部分已截断"
            >
              已截断
            </span>
          )}
        </div>
      ) : (
        <p
          className="text-meta text-muted-foreground"
          title="配置 [recording] transcript = false 时不记录；早于该能力的请求也没有记录。"
        >
          没有消息记录。
        </p>
      )}
    </section>
  )
}

function MessageGroup({ label, messages }: { label: string; messages: TranscriptMessage[] }) {
  // 只去首尾空白：模型常在正文前带 "\n\n"，按原样展示会出现整块空行。
  const visible = messages
    .map((message) => ({ ...message, text: message.text.trim() }))
    .filter((message) => message.text.length > 0)
  if (visible.length === 0) return null
  return (
    <div className="flex flex-col gap-2">
      <span className="text-meta text-muted-foreground">{label}</span>
      <ol className="flex flex-col gap-2.5">
        {visible.map((message, index) => (
          <li key={index} className="flex flex-col items-stretch gap-1">
            <span className="app-pill self-start" title={message.role}>
              {messageRoleLabel(message.role)}
              {message.name ? ` · ${message.name}` : ''}
            </span>
            <div className="app-message-text">{message.text}</div>
          </li>
        ))}
      </ol>
    </div>
  )
}

function messageRoleLabel(role: string): string {
  switch (role) {
    case 'system':
      return '系统'
    case 'user':
      return '用户'
    case 'assistant':
      return '助手'
    case 'tool':
      return '工具'
    default:
      return role
  }
}

/**
 * 路由时间线：Selection → Strategies → Plan → Dispatch → (Failover/Judge) → Outcome。
 * 数据全部来自决策快照（contributions/preference/failover/judge），不重算。
 */
function RoutingTimeline({ detail }: { detail: RequestDetail }) {
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

  const steps: Array<{ label: string; body: React.ReactNode }> = [
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
              // 契约：`PlanContribution` 是 `#[serde(tag = "contribution")]` 的内部标记枚举，
              // 形状是 `{contribution:"prefer", order:[…], reason:{code,…}}`——reason 与
              // 变体字段在同一层，不是 `{kind: payload}` 包装。
              const payload =
                entry.contribution && typeof entry.contribution === 'object'
                  ? (entry.contribution as Record<string, unknown>)
                  : {}
              const kind = String(payload.contribution ?? 'note')
              const reason = String((payload.reason as { code?: string } | undefined)?.code ?? '')
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
            {preference.map((slot) => slotLabel(slot)).join(' → ')}
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
          {failover.map((slot) => slotLabel(slot)).join(' → ')}
        </span>
      ),
    })
  }
  if (detail.judge_calls.length > 0) {
    steps.push({
      label: `决策模型（${detail.judge_calls.length} 次调用）`,
      body: <JudgeCallsTable calls={detail.judge_calls} />,
    })
  } else if (judge) {
    steps.push({
      label: '决策模型',
      body: (
        <span className="text-meta text-muted-foreground">
          {String((judge as Record<string, unknown>).effective_source ?? '无调用')}
        </span>
      ),
    })
  }
  steps.push({
    label: '结果（Outcome）',
    body: (
      <span className="text-meta">
        {outcomeLabel(detail.outcome)} · {detail.plan_mode} · 切换 {detail.switches} 次 · TTFT{' '}
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

/**
 * 决策模型（判官 / JEV）调用表：点行展开这一调的输入与答案。
 *
 * 详情来自 `JudgeCallSummary.request/response/failure_message`（服务端落库摘要）：
 * 任务与提示词截断、选项 `{id,label}`、答案 `{kind,value,confidence,reason}`。
 */
function JudgeCallsTable({ calls }: { calls: JudgeCallSummary[] }) {
  const [expanded, setExpanded] = useState<number | null>(null)
  return (
    <table className="app-table app-table-clickable">
      <thead>
        <tr>
          <th>服务</th>
          <th>问题</th>
          <th>结果</th>
          <th>耗时</th>
          <th>Token</th>
          <th>成本</th>
        </tr>
      </thead>
      <tbody>
        {calls.map((call, index) => {
          const open = expanded === index
          return (
            <FragmentRow
              key={`${call.question_id}-${call.created_at}-${index}`}
              call={call}
              open={open}
              onToggle={() => setExpanded(open ? null : index)}
            />
          )
        })}
      </tbody>
    </table>
  )
}

function FragmentRow({
  call,
  open,
  onToggle,
}: {
  call: JudgeCallSummary
  open: boolean
  onToggle: () => void
}) {
  const source = judgeSourceLabel(call.effective_source)
  return (
    <>
      <tr
        onClick={onToggle}
        title="点开查看这次调用的输入与答案"
        className={open ? 'app-row-selected' : undefined}
      >
        <td className="font-mono">
          {call.service_id}
          {source && <span className="app-pill ml-2">{source}</span>}
        </td>
        <td className="font-mono">{call.question_id}</td>
        <td title={`${call.outcome}${call.failure_code ? ` / ${call.failure_code}` : ''}`}>
          {judgeOutcomeLabel(call.outcome)}
          {call.failure_code ? ` · ${judgeFailureLabel(call.failure_code)}` : ''}
        </td>
        <td>{fmtDuration(call.latency_ms)}</td>
        <td>
          {formatCompactNum(call.prompt_tokens ?? 0)} in /{' '}
          {formatCompactNum(call.completion_tokens ?? 0)} out
        </td>
        <td>{call.cost_micros === null ? '—' : fmtCost(call.cost_micros)}</td>
      </tr>
      {open && (
        <tr>
          <td colSpan={6}>
            <JudgeCallDetailBlock call={call} />
          </td>
        </tr>
      )}
    </>
  )
}

export function JudgeCallDetailBlock({ call }: { call: JudgeCallSummary }) {
  const request = asRecord(call.request)
  const response = asRecord(call.response)
  const choices = Array.isArray(request?.choices)
    ? (request.choices as Array<Record<string, unknown>>)
    : null
  // LLM 判官记录的是实际发出的 messages；JEV 记录任务/提示词/选项。
  const messages = Array.isArray(request?.messages)
    ? (request.messages as Array<Record<string, unknown>>)
    : null
  const answer = response
    ? `${String(response.value ?? response.kind ?? '—')}${
        typeof response.confidence === 'number'
          ? ` · 置信度 ${response.confidence.toFixed(2)}`
          : ''
      }`
    : null

  // 长文本（prompt / 选项）整行铺开；标签在上、内容在下，避免窄列里滚动。
  return (
    <div className="flex flex-col gap-3 py-1">
      {messages ? (
        <DetailField label="发送的 prompt">
          <div className="flex flex-col gap-2">
            {messages.map((message, index) => (
              <div key={index} className="flex flex-col gap-1">
                <span className="app-pill self-start" title={String(message.role ?? '')}>
                  {messageRoleLabel(String(message.role ?? ''))}
                </span>
                <pre className="app-code-block whitespace-pre-wrap">
                  {String(message.content ?? '')}
                </pre>
              </div>
            ))}
          </div>
        </DetailField>
      ) : (
        <>
          {request?.task ? (
            <DetailField label="任务">
              <div className="app-message-text max-h-40">{String(request.task)}</div>
            </DetailField>
          ) : null}
          {request?.prompt ? (
            <DetailField label="发送的 prompt">
              <pre className="app-code-block whitespace-pre-wrap">{String(request.prompt)}</pre>
            </DetailField>
          ) : null}
          {choices && choices.length > 0 ? (
            <DetailField label="选项">
              <div className="flex flex-col gap-1 font-mono text-meta break-words">
                {choices.map((choice, index) => (
                  <span key={index}>
                    {String(choice.id)}
                    {choice.label ? `（${String(choice.label)}）` : ''}
                  </span>
                ))}
              </div>
            </DetailField>
          ) : null}
        </>
      )}
      {answer ? (
        <DetailField label="答案">
          <div className="flex flex-col gap-1">
            <span className="font-mono text-ui-label break-words">{answer}</span>
            {response?.reason ? (
              <span className="text-meta text-muted-foreground">{String(response.reason)}</span>
            ) : null}
          </div>
        </DetailField>
      ) : null}
      {call.failure_code ? (
        <DetailField label="失败">
          <span className="text-meta text-status-error break-words">
            {call.failure_code}
            {call.failure_message ? `: ${call.failure_message}` : ''}
          </span>
        </DetailField>
      ) : null}
    </div>
  )
}

function DetailField({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1 min-w-0">
      <span className="text-meta text-muted-foreground">{label}</span>
      {children}
    </div>
  )
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null
}
