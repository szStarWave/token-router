/**
 * 会话（活动 → 会话）：按会话查看**完整模型调用记录**。
 *
 * 列表来自服务端会话聚合（`GET /v2/admin/sessions`，89 天窗口内最多 200 个会话）；
 * 点开会话 → `SessionPanel`：该会话的每一轮客户端请求 + 每一次决策模型（判官）调用，
 * 合并按时间倒序展示，点行可继续看单请求详情。
 */
import { useEffect, useState } from 'react'
import type { JudgeCallSummary } from '../api/types'
import {
  useJudgeCallsQuery,
  useRequestDetailQuery,
  useSessionRequestsQuery,
  useSessionsQuery,
} from '../queries/admin'
import { QueryState } from '../components/common/QueryState'
import { JudgeCallPanel } from './activity/JudgeCallPanel'
import { RequestDetailPanel } from './activity/RequestDetailPanel'
import { SessionList } from './activity/SessionList'
import { SessionPanel } from './activity/SessionPanel'
import { RANGES, inRange, type RangeKey } from './activity/shared'

/** 会话汇总窗口：接口上限 90 天；取 89 天留边界余量。 */
const SESSION_WINDOW_MS = 89 * 86_400_000
/** 窗口按 5 分钟对齐，避免每次渲染换查询键。 */
const SESSION_WINDOW_STEP_MS = 5 * 60_000
/** 服务端 `limit` 上限 200。 */
const SESSION_LIMIT = 200

export function SessionsPage() {
  const [range, setRange] = useState<RangeKey>('all')
  const [selectedSession, setSelectedSession] = useState<string | null>(null)
  const [selectedRequest, setSelectedRequest] = useState<string | null>(null)
  const [selectedJudge, setSelectedJudge] = useState<JudgeCallSummary | null>(null)
  // 时间范围过滤要一个「现在」：30 秒走一格（与请求页同节奏）。
  const [nowMs, setNowMs] = useState(() => Date.now())
  useEffect(() => {
    const timer = window.setInterval(() => setNowMs(Date.now()), 30_000)
    return () => window.clearInterval(timer)
  }, [])

  const to = Math.ceil(nowMs / SESSION_WINDOW_STEP_MS) * SESSION_WINDOW_STEP_MS
  const sessionsQuery = useSessionsQuery({
    from: to - SESSION_WINDOW_MS,
    to,
    limit: SESSION_LIMIT,
  })
  const sessionRequestsQuery = useSessionRequestsQuery(selectedSession)
  const sessionJudgeQuery = useJudgeCallsQuery(
    { session: selectedSession ?? undefined, limit: 200 },
    selectedSession !== null,
  )
  const detailQuery = useRequestDetailQuery(selectedRequest)

  const sessions = sessionsQuery.data?.items ?? []
  const visible = sessions.filter((session) => inRange(session.last_at, range, nowMs))
  const summary = sessions.find((session) => session.session_key === selectedSession) ?? null

  const pickSession = (key: string) => {
    setSelectedSession(key === selectedSession ? null : key)
    setSelectedRequest(null)
    setSelectedJudge(null)
  }

  const pickRequest = (id: string) => {
    setSelectedRequest(id === selectedRequest ? null : id)
    setSelectedJudge(null)
  }

  const pickJudge = (call: JudgeCallSummary) => {
    const same =
      selectedJudge !== null &&
      selectedJudge.request_id === call.request_id &&
      selectedJudge.created_at === call.created_at &&
      selectedJudge.question_id === call.question_id
    setSelectedJudge(same ? null : call)
    setSelectedRequest(null)
  }

  return (
    <div className="app-page-wide">
      <div className="app-split">
        <section className="app-card" aria-label="会话">
          <div className="app-filter-bar">
            {RANGES.map((item) => (
              <button
                key={item.key}
                type="button"
                className={range === item.key ? 'app-chip app-chip-active' : 'app-chip'}
                onClick={() => setRange(item.key)}
              >
                {item.label}
              </button>
            ))}
          </div>
          <div className="app-split-scroll">
            <QueryState
              loading={sessionsQuery.isLoading}
              error={sessionsQuery.error}
              onRetry={() => void sessionsQuery.refetch()}
            >
              <SessionList
                sessions={visible}
                selectedSession={selectedSession}
                onSelect={pickSession}
              />
            </QueryState>
          </div>
        </section>

        <section className="app-card" aria-label="会话调用记录">
          {selectedSession === null ? (
            <p className="text-meta text-muted-foreground">选择左侧会话查看每一轮调用与消息。</p>
          ) : (
            <>
              <div className="app-split-scroll">
                <QueryState
                  loading={sessionRequestsQuery.isLoading || sessionJudgeQuery.isLoading}
                  error={sessionRequestsQuery.error ?? sessionJudgeQuery.error}
                  onRetry={() => {
                    void sessionRequestsQuery.refetch()
                    void sessionJudgeQuery.refetch()
                  }}
                >
                  <SessionPanel
                    sessionKey={selectedSession}
                    session={summary}
                    requests={sessionRequestsQuery.data ?? []}
                    judgeCalls={sessionJudgeQuery.data?.items ?? []}
                    onSelectRequest={pickRequest}
                    onSelectJudge={pickJudge}
                  />
                </QueryState>
                {selectedJudge !== null && (
                  <div className="app-detail-panel mt-3">
                    <JudgeCallPanel call={selectedJudge} />
                  </div>
                )}
                {selectedRequest !== null && (
                  <div className="app-detail-panel mt-3">
                    <QueryState
                      loading={detailQuery.isLoading}
                      error={detailQuery.error}
                      onRetry={() => void detailQuery.refetch()}
                    >
                      {detailQuery.data && <RequestDetailPanel detail={detailQuery.data} />}
                    </QueryState>
                  </div>
                )}
              </div>
            </>
          )}
        </section>
      </div>
    </div>
  )
}
