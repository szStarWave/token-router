/**
 * Approvals（§7.3）：审批记录列表 + 记录决定（admin:approve）。
 * pending 列表来自账本中的 APPROVAL_REQUIRED 记录；此处展示历史与手动记录。
 */
import { useState } from 'react'
import { useApprovalsQuery, useRecordApprovalMutation } from '../queries/admin'
import { QueryState, errorMessage } from '../components/common/QueryState'
import { useAppStore } from '../stores/appStore'

export function ApprovalsPage() {
  const approvalsQuery = useApprovalsQuery()
  const recordMutation = useRecordApprovalMutation()
  const showToast = useAppStore((s) => s.showToast)

  const [requestId, setRequestId] = useState('')
  const [toolName, setToolName] = useState('')
  const [riskClass, setRiskClass] = useState('high')
  const [decision, setDecision] = useState('approved')

  const record = async () => {
    try {
      await recordMutation.mutateAsync({
        request_id: requestId.trim(),
        tool_name: toolName.trim(),
        risk_class: riskClass,
        decision,
      })
      setRequestId('')
      setToolName('')
      showToast('approval.recorded')
    } catch (error) {
      showToast(errorMessage(error), false)
    }
  }

  const rows = approvalsQuery.data?.items ?? []
  const pending = rows.filter((row) => row.decision === 'required')

  return (
    <div className="page">
      {pending.length > 0 && (
        <section className="card" aria-label="待审批">
          <h2>待审批（{pending.length}）</h2>
          <table className="table">
            <thead>
              <tr><th>请求</th><th>工具</th><th>风险</th><th>决定</th></tr>
            </thead>
            <tbody>
              {pending.map((row) => (
                <tr key={row.id}>
                  <td className="mono">{row.request_id}</td>
                  <td className="mono">{row.tool_name}</td>
                  <td><span className="pill pill-open">{row.risk_class}</span></td>
                  <td>
                    <button
                      type="button"
                      className="btn btn-sm"
                      onClick={() =>
                        void recordMutation
                          .mutateAsync({
                            request_id: row.request_id,
                            tool_name: row.tool_name,
                            risk_class: row.risk_class,
                            decision: 'approved',
                          })
                          .then(() => showToast('approval.recorded'))
                          .catch((error) => showToast(errorMessage(error), false))
                      }
                    >
                      批准
                    </button>{' '}
                    <button
                      type="button"
                      className="btn btn-sm btn-danger"
                      onClick={() =>
                        void recordMutation
                          .mutateAsync({
                            request_id: row.request_id,
                            tool_name: row.tool_name,
                            risk_class: row.risk_class,
                            decision: 'rejected',
                          })
                          .then(() => showToast('approval.recorded'))
                          .catch((error) => showToast(errorMessage(error), false))
                      }
                    >
                      拒绝
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      )}

      <section className="card" aria-label="审批记录">
        <h2>审批记录</h2>
        <QueryState
          loading={approvalsQuery.isLoading}
          error={approvalsQuery.error}
          empty={rows.length === 0}
          onRetry={() => void approvalsQuery.refetch()}
        >
          <table className="table">
            <thead>
              <tr><th>时间</th><th>请求</th><th>工具</th><th>风险</th><th>决定</th><th>操作者</th></tr>
            </thead>
            <tbody>
              {rows.map((row) => (
                <tr key={row.id}>
                  <td>{new Date(row.created_at * 1000).toLocaleString()}</td>
                  <td className="mono">{row.request_id}</td>
                  <td className="mono">{row.tool_name}</td>
                  <td>{row.risk_class}</td>
                  <td>{row.decision}</td>
                  <td>{row.operator ?? '—'}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </QueryState>
      </section>

      <section className="card" aria-label="手动记录">
        <h2>手动记录决定</h2>
        <p className="text-dim">
          审批拦截（enforce）在请求侧返回 <code>approval_required</code> 与 request_id；
          把它填到下面记录人工决定。需要 token 具备 <code>admin:approve</code> 作用域。
        </p>
        <div className="form-row">
          <label>
            request_id{' '}
            <input type="text" value={requestId} onChange={(e) => setRequestId(e.target.value)} />
          </label>
          <label>
            工具{' '}
            <input type="text" value={toolName} onChange={(e) => setToolName(e.target.value)} />
          </label>
          <label>
            风险{' '}
            <select value={riskClass} onChange={(e) => setRiskClass(e.target.value)}>
              <option value="low">low</option>
              <option value="medium">medium</option>
              <option value="high">high</option>
            </select>
          </label>
          <label>
            决定{' '}
            <select value={decision} onChange={(e) => setDecision(e.target.value)}>
              <option value="approved">approved</option>
              <option value="rejected">rejected</option>
            </select>
          </label>
          <button
            type="button"
            className="btn"
            disabled={!requestId.trim() || !toolName.trim() || recordMutation.isPending}
            onClick={() => void record()}
          >
            记录
          </button>
        </div>
      </section>
    </div>
  )
}
