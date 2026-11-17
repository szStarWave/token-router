/**
 * Approvals（§7.3）：审批记录列表 + 记录决定（admin:approve）。
 * pending 列表来自账本中的 APPROVAL_REQUIRED 记录；此处展示历史与手动记录。
 */
import { useState } from 'react'
import { useApprovalsQuery, useRecordApprovalMutation } from '../queries/admin'
import { QueryState, errorMessage } from '../components/common/QueryState'
import { Icon } from '../components/common/Icon'
import { useAppStore } from '../stores/appStore'
import { riskPillClass } from '../lib/status'

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
    <div className="app-page">
      {pending.length > 0 && (
        <section className="app-card" aria-label="待审批">
          <h2 className="app-card-title">待审批（{pending.length}）</h2>
          <table className="app-table">
            <thead>
              <tr>
                <th>请求</th>
                <th>工具</th>
                <th>风险</th>
                <th>决定</th>
              </tr>
            </thead>
            <tbody>
              {pending.map((row) => (
                <tr key={row.id}>
                  <td className="font-mono">{row.request_id}</td>
                  <td className="font-mono">{row.tool_name}</td>
                  <td>
                    <span className={riskPillClass(row.risk_class)}>{row.risk_class}</span>
                  </td>
                  <td>
                    <button
                      type="button"
                      className="oc-btn-default oc-btn-sm"
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
                      <Icon name="approve" />
                      批准
                    </button>{' '}
                    <button
                      type="button"
                      className="oc-btn-destructive oc-btn-sm"
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
                      <Icon name="reject" />
                      拒绝
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      )}

      <section className="app-card" aria-label="审批记录">
        <h2 className="app-card-title">审批记录</h2>
        <QueryState
          loading={approvalsQuery.isLoading}
          error={approvalsQuery.error}
          empty={rows.length === 0}
          onRetry={() => void approvalsQuery.refetch()}
        >
          <table className="app-table">
            <thead>
              <tr>
                <th>时间</th>
                <th>请求</th>
                <th>工具</th>
                <th>风险</th>
                <th>决定</th>
                <th>操作者</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => (
                <tr key={row.id}>
                  <td>{new Date(row.created_at * 1000).toLocaleString()}</td>
                  <td className="font-mono">{row.request_id}</td>
                  <td className="font-mono">{row.tool_name}</td>
                  <td>{row.risk_class}</td>
                  <td>{row.decision}</td>
                  <td>{row.operator ?? '—'}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </QueryState>
      </section>

      <section className="app-card" aria-label="手动记录">
        <h2 className="app-card-title">手动记录决定</h2>
        <p className="app-card-sub">
          审批拦截（enforce）在请求侧返回{' '}
          <code className="app-inline-code">approval_required</code> 与 request_id；
          把它填到下面记录人工决定。需要 token 具备{' '}
          <code className="app-inline-code">admin:approve</code> 作用域。
        </p>
        <div className="app-form-row">
          <label className="app-field">
            request_id
            <input
              type="text"
              className="oc-field app-field-light font-mono"
              value={requestId}
              onChange={(e) => setRequestId(e.target.value)}
            />
          </label>
          <label className="app-field">
            工具
            <input
              type="text"
              className="oc-field app-field-light font-mono"
              value={toolName}
              onChange={(e) => setToolName(e.target.value)}
            />
          </label>
          <label className="app-field">
            风险
            <select
              className="app-select"
              value={riskClass}
              onChange={(e) => setRiskClass(e.target.value)}
            >
              <option value="low">low</option>
              <option value="medium">medium</option>
              <option value="high">high</option>
            </select>
          </label>
          <label className="app-field">
            决定
            <select
              className="app-select"
              value={decision}
              onChange={(e) => setDecision(e.target.value)}
            >
              <option value="approved">approved</option>
              <option value="rejected">rejected</option>
            </select>
          </label>
          <button
            type="button"
            className="oc-btn-default"
            disabled={!requestId.trim() || !toolName.trim() || recordMutation.isPending}
            onClick={() => void record()}
          >
            <Icon name="approve" />
            记录
          </button>
        </div>
      </section>
    </div>
  )
}
