/**
 * Diagnostics（§7.3）：doctor 自检、health、配置审计、回滚。
 */
import { useState } from 'react'
import {
  useHealthQuery,
  useConfigAuditQuery,
  useConfigRollbackMutation,
} from '../queries/admin'
import { useQuery } from '@tanstack/react-query'
import { api } from '../api/client'
import { QueryState, errorMessage } from '../components/common/QueryState'
import { useAppStore } from '../stores/appStore'
import { hostOpenPath, isTauri } from '../lib/tauri'

export function DiagnosticsPage() {
  const doctorQuery = useQuery({
    queryKey: ['admin', 'doctor'],
    queryFn: () => api.doctor(),
    retry: 1,
  })
  const healthQuery = useHealthQuery()
  const auditQuery = useConfigAuditQuery()
  const rollbackMutation = useConfigRollbackMutation()
  const showToast = useAppStore((s) => s.showToast)

  const [confirming, setConfirming] = useState(false)

  const doctor = doctorQuery.data
  const failed = doctor?.checks.filter((check) => check.status === 'fail') ?? []

  const rollback = async () => {
    try {
      const response = await rollbackMutation.mutateAsync()
      showToast(`config.rolled_back:${response.revision.slice(0, 12)}`)
      setConfirming(false)
    } catch (error) {
      showToast(errorMessage(error), false)
    }
  }

  return (
    <div className="page">
      <section className="card" aria-label="doctor 自检">
        <h2>运行自检（doctor）</h2>
        <QueryState
          loading={doctorQuery.isLoading}
          error={doctorQuery.error}
          onRetry={() => void doctorQuery.refetch()}
        >
          {doctor && (
            <>
              {failed.length > 0 && (
                <div className="banner banner-err">
                  {failed.length} 项失败：{failed.map((check) => check.name).join(', ')}
                </div>
              )}
              <table className="table">
                <thead>
                  <tr><th>检查项</th><th>状态</th><th>说明</th></tr>
                </thead>
                <tbody>
                  {doctor.checks.map((check) => (
                    <tr key={check.name}>
                      <td>{check.name}</td>
                      <td>
                        <span className={`pill pill-${check.status === 'ok' ? 'closed' : check.status === 'warn' ? 'half_open' : 'open'}`}>
                          {check.status}
                        </span>
                      </td>
                      <td>{check.detail ?? '—'}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </>
          )}
        </QueryState>
      </section>

      <section className="card" aria-label="熔断状态">
        <h2>候选熔断</h2>
        <QueryState
          loading={healthQuery.isLoading}
          error={healthQuery.error}
          empty={(healthQuery.data?.items.length ?? 0) === 0}
          onRetry={() => void healthQuery.refetch()}
        >
          <table className="table">
            <thead>
              <tr><th>候选</th><th>状态</th><th>打开次数</th><th>冷却截止</th></tr>
            </thead>
            <tbody>
              {healthQuery.data?.items.map((item) => (
                <tr key={item.candidate_id}>
                  <td className="mono">{item.candidate_id}</td>
                  <td><span className={`pill pill-${item.status}`}>{item.status}</span></td>
                  <td>{item.opened_count}</td>
                  <td>{item.open_until ? new Date(item.open_until).toLocaleTimeString() : '—'}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </QueryState>
      </section>

      <section className="card" aria-label="配置审计">
        <h2>配置审计</h2>
        <QueryState
          loading={auditQuery.isLoading}
          error={auditQuery.error}
          empty={(auditQuery.data?.items.length ?? 0) === 0}
          onRetry={() => void auditQuery.refetch()}
        >
          <table className="table">
            <thead>
              <tr><th>时间</th><th>动作</th><th>操作者</th><th>revision</th><th>diff</th></tr>
            </thead>
            <tbody>
              {auditQuery.data?.items.map((entry) => (
                <tr key={entry.id}>
                  <td>{new Date(entry.at).toLocaleString()}</td>
                  <td>{entry.action}</td>
                  <td>{entry.actor}</td>
                  <td className="mono">
                    {entry.from_revision?.slice(0, 10) ?? '—'} → {entry.to_revision?.slice(0, 10) ?? '—'}
                  </td>
                  <td>{entry.diff.length}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </QueryState>
        {isTauri() && (
          <button type="button" className="btn" onClick={() => void hostOpenPath()}>
            打开数据目录
          </button>
        )}
      </section>

      <section className="card" aria-label="配置回滚">
        <h2>回滚配置</h2>
        <p className="text-dim">
          把最近一份 <code>config.toml.bak-&lt;ts&gt;</code> 还原为当前配置；
          回滚后需要重启服务生效。当前 token 需要 <code>admin:write</code> 作用域。
        </p>
        {confirming ? (
          <div className="confirm-inline">
            确认回滚到最近一份备份？
            <button type="button" className="btn btn-danger" disabled={rollbackMutation.isPending} onClick={() => void rollback()}>
              {rollbackMutation.isPending ? '回滚中…' : '确认回滚'}
            </button>
            <button type="button" className="btn" onClick={() => setConfirming(false)}>
              取消
            </button>
          </div>
        ) : (
          <button type="button" className="btn" onClick={() => setConfirming(true)}>
            回滚到上一份备份
          </button>
        )}
      </section>
    </div>
  )
}
