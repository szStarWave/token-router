/**
 * Overview（§7.3）：status、health、最近 5 条 requests、服务启停。
 * 空/错状态：服务未启动 → 启动按钮；未配置 → 引导。
 */
import { useQueryClient } from '@tanstack/react-query'
import {
  useHealthQuery,
  useQuotasQuery,
  useRequestsQuery,
  useStatsSummaryQuery,
  useStatusQuery,
} from '../queries/admin'
import { QueryState, errorMessage } from '../components/common/QueryState'
import { Icon } from '../components/common/Icon'
import { outcomePillClass, statusPillClass } from '../lib/status'
import {
  supervisorStart,
  supervisorStop,
  supervisorStatus,
  isTauri,
  invokeErrorMessage,
  type SupervisorStatus,
} from '../lib/tauri'
import { useEffect, useState } from 'react'

export function OverviewPage() {
  const statusQuery = useStatusQuery()
  const healthQuery = useHealthQuery()
  const requestsQuery = useRequestsQuery({ limit: 5 })
  const statsQuery = useStatsSummaryQuery()
  const queryClient = useQueryClient()

  const [supervisor, setSupervisor] = useState<SupervisorStatus | null>(null)
  const [busy, setBusy] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)

  useEffect(() => {
    if (!isTauri()) return
    void supervisorStatus().then(setSupervisor).catch(() => setSupervisor(null))
  }, [])

  const serviceDown =
    (isTauri() && supervisor !== null && supervisor.state !== 'ready') ||
    (!isTauri() && statusQuery.isError)

  const refreshAll = () => {
    void queryClient.invalidateQueries({ queryKey: ['admin'] })
  }

  const start = async () => {
    setBusy(true)
    setActionError(null)
    try {
      await supervisorStart()
      setSupervisor(await supervisorStatus())
      refreshAll()
    } catch (error) {
      setActionError(invokeErrorMessage(error))
    } finally {
      setBusy(false)
    }
  }

  const stop = async () => {
    setBusy(true)
    setActionError(null)
    try {
      await supervisorStop()
      setSupervisor(await supervisorStatus())
      refreshAll()
    } catch (error) {
      setActionError(invokeErrorMessage(error))
    } finally {
      setBusy(false)
    }
  }

  const status = statusQuery.data
  const stats = statsQuery.data
  const quotasQuery = useQuotasQuery(!serviceDown)
  const exhaustedQuotas = (quotasQuery.data?.items ?? []).filter(
    (item) => item.remaining === 0 || item.consecutive_429 > 0,
  )

  return (
    <div className="app-page">
      {exhaustedQuotas.length > 0 && (
        <div className="app-banner app-banner-warn">
          <span>
            {exhaustedQuotas.length} 个候选配额耗尽或被上游限流（
            {exhaustedQuotas
              .slice(0, 3)
              .map((item) => item.candidate_id)
              .join(', ')}
            {exhaustedQuotas.length > 3 ? ' …' : ''}
            ），路由已自动改用其他候选。
          </span>
          <a href="#/quotas" className="underline">
            查看配额
          </a>
        </div>
      )}
      {serviceDown && (
        <div className="app-banner app-banner-warn">
          <span>服务未启动。</span>
          {isTauri() && (
            <button
              type="button"
              className="oc-btn-default oc-btn-sm"
              disabled={busy}
              onClick={() => void start()}
            >
              <Icon name="start" />
              {busy ? '启动中…' : '启动服务'}
            </button>
          )}
          {!isTauri() && <span>在浏览器模式下请先启动 token-router-v2。</span>}
        </div>
      )}
      {actionError && <div className="app-banner app-banner-err">{actionError}</div>}

      <QueryState
        loading={statusQuery.isLoading}
        error={serviceDown ? null : statusQuery.error}
        onRetry={() => void statusQuery.refetch()}
      >
        {status && (
          <>
            <section className="app-card" aria-label="服务状态">
              <h2 className="app-card-title">服务状态</h2>
              <dl className="app-kv">
                <div className="app-kv-item">
                  <dt className="app-kv-key">版本</dt>
                  <dd className="app-kv-value">
                    {status.version} (api v{status.api_version})
                  </dd>
                </div>
                <div className="app-kv-item">
                  <dt className="app-kv-key">监听</dt>
                  <dd className="app-kv-value font-mono">{status.listen}</dd>
                </div>
                <div className="app-kv-item">
                  <dt className="app-kv-key">运行时长</dt>
                  <dd className="app-kv-value">{Math.floor(status.uptime_ms / 1000)}s</dd>
                </div>
                <div className="app-kv-item">
                  <dt className="app-kv-key">默认模型</dt>
                  <dd className="app-kv-value font-mono">{status.default_virtual_model}</dd>
                </div>
                <div className="app-kv-item">
                  <dt className="app-kv-key">模式</dt>
                  <dd className="app-kv-value">
                    判官 {status.modes.judge} / 成本闸 {status.modes.cost_gate} / 审批{' '}
                    {status.modes.approvals}
                  </dd>
                </div>
                <div className="app-kv-item">
                  <dt className="app-kv-key">请求总数</dt>
                  <dd className="app-kv-value">{status.requests_total}</dd>
                </div>
                <div className="app-kv-item">
                  <dt className="app-kv-key">熔断</dt>
                  <dd className="app-kv-value">
                    closed {status.breakers.closed} · half-open {status.breakers.half_open} ·{' '}
                    <span className={status.breakers.open > 0 ? 'text-status-warning' : ''}>
                      open {status.breakers.open}
                    </span>
                  </dd>
                </div>
                <div className="app-kv-item">
                  <dt className="app-kv-key">配置 digest</dt>
                  <dd className="app-kv-value font-mono text-muted-foreground">
                    {status.config_digest.slice(0, 16)}…
                  </dd>
                </div>
              </dl>
              <div className="app-card-actions">
                {isTauri() && (
                  <button
                    type="button"
                    className="oc-btn-destructive"
                    disabled={busy}
                    onClick={() => void stop()}
                  >
                    <Icon name="stop" />
                    {busy ? '停止中…' : '停止服务'}
                  </button>
                )}
                <button type="button" className="oc-btn-default" onClick={refreshAll}>
                  <Icon name="refresh" />
                  刷新
                </button>
              </div>
            </section>

            <QueryState loading={statsQuery.isLoading} error={statsQuery.error}>
              {stats && (
                <section className="app-card" aria-label="24 小时统计">
                  <h2 className="app-card-title">最近 24 小时</h2>
                  <dl className="app-kv">
                    <div className="app-kv-item">
                      <dt className="app-kv-key">请求</dt>
                      <dd className="app-kv-value">{stats.requests}</dd>
                    </div>
                    <div className="app-kv-item">
                      <dt className="app-kv-key">成功率</dt>
                      <dd className="app-kv-value">{(stats.success_rate * 100).toFixed(1)}%</dd>
                    </div>
                    <div className="app-kv-item">
                      <dt className="app-kv-key">上游错误</dt>
                      <dd className="app-kv-value">{stats.upstream_errors}</dd>
                    </div>
                    <div className="app-kv-item">
                      <dt className="app-kv-key">切换</dt>
                      <dd className="app-kv-value">{stats.switches}</dd>
                    </div>
                    <div className="app-kv-item">
                      <dt className="app-kv-key">输入 token</dt>
                      <dd className="app-kv-value">{stats.tokens.input}</dd>
                    </div>
                    <div className="app-kv-item">
                      <dt className="app-kv-key">输出 token</dt>
                      <dd className="app-kv-value">{stats.tokens.output}</dd>
                    </div>
                  </dl>
                </section>
              )}
            </QueryState>

            <QueryState
              loading={healthQuery.isLoading}
              error={healthQuery.error}
              empty={(healthQuery.data?.items.length ?? 0) === 0}
            >
              <section className="app-card" aria-label="候选健康">
                <h2 className="app-card-title">候选健康</h2>
                <table className="app-table">
                  <thead>
                    <tr>
                      <th>候选</th>
                      <th>状态</th>
                      <th>连续失败</th>
                      <th>最近错误</th>
                    </tr>
                  </thead>
                  <tbody>
                    {healthQuery.data?.items.map((item) => (
                      <tr key={item.candidate_id}>
                        <td className="font-mono">{item.candidate_id}</td>
                        <td>
                          <span className={statusPillClass(item.status)}>{item.status}</span>
                        </td>
                        <td>{item.consecutive_failures}</td>
                        <td>{item.last_error_class ?? '—'}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </section>
            </QueryState>

            <QueryState
              loading={requestsQuery.isLoading}
              error={requestsQuery.error}
              empty={(requestsQuery.data?.items.length ?? 0) === 0}
            >
              <section className="app-card" aria-label="最近请求">
                <h2 className="app-card-title">最近请求</h2>
                <table className="app-table">
                  <thead>
                    <tr>
                      <th>时间</th>
                      <th>模型</th>
                      <th>结果</th>
                      <th>耗时</th>
                    </tr>
                  </thead>
                  <tbody>
                    {requestsQuery.data?.items.map((row) => (
                      <tr key={row.request_id}>
                        <td className="font-mono">
                          {new Date(row.created_at).toLocaleTimeString()}
                        </td>
                        <td className="font-mono">
                          {row.dispatch_model ?? row.selection_model}
                        </td>
                        <td>
                          <span className={outcomePillClass(row.outcome)}>{row.outcome}</span>
                        </td>
                        <td>{row.duration_ms}ms</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
                <p className="app-card-sub">完整列表见「请求」页。</p>
              </section>
            </QueryState>
          </>
        )}
      </QueryState>
    </div>
  )
}

// 保留 errorMessage 引用（banner 场景复用）。
void errorMessage
