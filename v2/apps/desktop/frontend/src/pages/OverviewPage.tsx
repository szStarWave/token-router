/**
 * 首页（product-design.zh.md §5 J1 / §6 关键状态）：七态渲染。
 * A 零配置 / B 部分配置 / C 有配置无请求 / D 正常 / E 降级 /
 * F 不可达 / G 更新中。判定来自真实查询；未知 ≠ 0，缺数据不冒充健康。
 *
 * 诚实纪律：
 * - 成本卡显示账本已计价合计（stats）+「覆盖未知」；计价覆盖率 DTO 未落地。
 * - 健康卡只显示熔断 / 候选健康信号，不冒充入口可用数。
 * - 配额耗尽只说「路由将避开它们」。
 * - 断连旧数据标「数据截至 HH:mm」，不显绿。
 * - G（supervisor starting/restarting）优先于 status 查询错误——
 *   重启窗口内 status 请求失败是正常现象，不算不可达。
 * - 停止服务需二次确认。
 */
import { useEffect, useState } from 'react'
import { Link, useNavigate } from '@tanstack/react-router'
import { useQueryClient } from '@tanstack/react-query'
import {
  useConfigQuery,
  useHealthQuery,
  useProvidersQuery,
  useQuotasQuery,
  useRequestsQuery,
  useStatsSummaryQuery,
  useStatusQuery,
} from '../queries/admin'
import { QueryState } from '../components/common/QueryState'
import { Icon, type IconName } from '../components/common/Icon'
import { outcomePillClass } from '../lib/status'
import { draftFromDocument, type ConfigDraft } from '../lib/config-edit'
import type { ProviderInfo } from '../api/types'
import {
  supervisorStart,
  supervisorStop,
  supervisorStatus,
  isTauri,
  invokeErrorMessage,
  type SupervisorStatus,
} from '../lib/tauri'

/** micros → $ 可读值（micros 原值不进一级视图）。 */
function fmtUsd(micros: number): string {
  const dollars = micros / 1e6
  if (dollars >= 1) return `$${dollars.toFixed(2)}`
  if (dollars >= 0.01) return `$${dollars.toFixed(3)}`
  return `$${dollars.toFixed(4)}`
}

function fmtUptime(ms: number): string {
  const totalSec = Math.floor(ms / 1000)
  const h = Math.floor(totalSec / 3600)
  const m = Math.floor((totalSec % 3600) / 60)
  if (h > 0) return `${h}h ${m}m`
  if (m > 0) return `${m}m ${totalSec % 60}s`
  return `${totalSec}s`
}

// ---------------------------------------------------------------------------
// 起步清单（§5 J1）
// ---------------------------------------------------------------------------

interface StepView {
  state: 'done' | 'pending' | 'skipped'
  title: string
  hint: string
  actionLabel?: string
  actionTo?: '/providers' | '/virtual-models' | '/onboarding'
}

const STEP_MARK: Record<
  StepView['state'],
  { icon: IconName; className: string; label: string }
> = {
  done: {
    icon: 'success',
    className: 'app-checklist-mark text-status-success',
    label: '已完成',
  },
  pending: {
    icon: 'state-pending',
    className: 'app-checklist-mark text-muted-foreground',
    label: '未完成',
  },
  skipped: {
    icon: 'state-skipped',
    className: 'app-checklist-mark text-muted-foreground',
    label: '已跳过',
  },
}

function ChecklistItem({ step }: { step: StepView }) {
  const mark = STEP_MARK[step.state]
  return (
    <div className="app-checklist-item" title={step.hint}>
      <span className={mark.className} role="img" aria-label={mark.label}>
        <Icon name={mark.icon} className="size-4" />
      </span>
      <div className="app-checklist-body">
        <span className="app-checklist-title">{step.title}</span>
      </div>
      {step.actionLabel && step.actionTo && (
        <span className="app-checklist-action">
          <Link to={step.actionTo} className="oc-btn-default oc-btn-sm">
            {step.actionLabel}
            <Icon name="next" className="size-3.5" />
          </Link>
        </span>
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// 结论卡
// ---------------------------------------------------------------------------

function MetricCard({
  label,
  value,
  note,
  warn = false,
  children,
}: {
  label: string
  value?: string
  note?: string
  warn?: boolean
  children?: React.ReactNode
}) {
  return (
    <div className="app-metric-card">
      <span className="app-metric-key">{label}</span>
      <span className={warn ? 'app-metric-value text-status-warning' : 'app-metric-value'}>
        {value}
      </span>
      {note && <span className="app-metric-note">{note}</span>}
      {children}
    </div>
  )
}

/** 数据未知态的三种诚实呈现：加载中 / 查询失败 / 有数据。 */
function MetricValue({
  loading,
  error,
  value,
  retry,
}: {
  loading: boolean
  error: boolean
  value: string
  retry: () => void
}) {
  if (loading) return <span className="app-metric-value text-muted-foreground">加载中…</span>
  if (error) {
    return (
      <button type="button" className="oc-btn-ghost oc-btn-sm self-start" onClick={retry}>
        <Icon name="refresh" />
        数据获取失败，重试
      </button>
    )
  }
  return <span className="app-metric-value">{value}</span>
}

// ---------------------------------------------------------------------------
// 首页
// ---------------------------------------------------------------------------

export function OverviewPage() {
  const queryClient = useQueryClient()

  const [supervisor, setSupervisor] = useState<SupervisorStatus | null>(null)
  const [busy, setBusy] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)
  const [confirmStop, setConfirmStop] = useState(false)

  // supervisor 轮询（仅 Tauri）。
  useEffect(() => {
    if (!isTauri()) return
    let cancelled = false
    const poll = () => {
      void supervisorStatus()
        .then((s) => {
          if (!cancelled) setSupervisor(s)
        })
        .catch(() => {
          if (!cancelled) setSupervisor(null)
        })
    }
    poll()
    const timer = window.setInterval(poll, 3000)
    return () => {
      cancelled = true
      window.clearInterval(timer)
    }
  }, [])

  const statusQuery = useStatusQuery()
  const supervisorReady = supervisor?.state === 'ready'

  // G → D/E 收尾：supervisor 回 ready 后刷新数据。
  useEffect(() => {
    if (supervisorReady) {
      void queryClient.invalidateQueries({ queryKey: ['admin'] })
    }
  }, [supervisorReady, queryClient])

  // ── 状态判定（优先级：G 更新中 > F 不可达 > 其余）─────────────────
  // - starting/restarting = 更新中，此时 admin 查询失败属正常，不算不可达。
  // - status.isError 一律阻断结论子树：supervisor ready 只是进程探测事实，
  //   不代表 admin 端点可达；缓存 status 不能当新鲜数据渲染。
  // - 本地启停动作只在 supervisor 已知且明确 external === false 时可用；
  //   external 未知（旧宿主 / 连接外部服务）不显示启停按钮。
  const supervisorBusy = isTauri() && supervisor !== null &&
    (supervisor.state === 'starting' || supervisor.state === 'restarting')
  const supervisorLocalKnown = isTauri() && supervisor !== null && supervisor.external === false
  const supervisorDown = supervisorLocalKnown &&
    (supervisor.state === 'stopped' || supervisor.state === 'failed')
  const updating = supervisorBusy
  const unreachable = !updating && (supervisorDown || statusQuery.isError)
  const statusDataFresh = !statusQuery.isError && statusQuery.data !== undefined

  const enabled = !unreachable && !updating && statusDataFresh
  const status = statusQuery.data

  const configQuery = useConfigQuery(enabled)
  const requestsQuery = useRequestsQuery({ limit: 5 }, enabled)
  const statsQuery = useStatsSummaryQuery({}, enabled)
  const healthQuery = useHealthQuery(enabled)
  const quotasQuery = useQuotasQuery(enabled)
  const providersQuery = useProvidersQuery(enabled)

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
      setConfirmStop(false)
    }
  }

  return (
    <div className="app-page">
      {/* G 更新中（蓝） */}
      {updating && (
        <div className="app-banner app-banner-info" role="status">
          <Icon name="info" className="size-4" />
          <span>
            网关{supervisor?.state === 'restarting' ? '重启中' : '启动中'}…完成后数据会自动刷新。
          </span>
        </div>
      )}

      {/* F 不可达（黄） */}
      {unreachable && (
        <div className="app-banner app-banner-warn" role="alert">
          <Icon name="warning" className="size-4" />
          <span>网关未连接。{!isTauri() && '在浏览器模式下请先启动 token-router-v2。'}</span>
          {/* 本地托管（external === false）且已停止/失败/探测不到：可启动。 */}
          {supervisorLocalKnown && (supervisor.state === 'stopped' || supervisor.state === 'failed') && (
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
          {statusQuery.dataUpdatedAt > 0 && (
            <span className="text-meta">
              数据截至 {new Date(statusQuery.dataUpdatedAt).toLocaleTimeString()}
            </span>
          )}
        </div>
      )}

      {actionError && <div className="app-banner app-banner-err">{actionError}</div>}

      {/* F/G 期间不渲染结论子树；status 有错也阻断（ready 只是进程探测事实，
          不代表 admin 可达，缓存数据不冒充新鲜）。 */}
      {!enabled ? null : (
        <QueryState
          loading={statusQuery.isLoading}
          error={null}
          onRetry={() => void statusQuery.refetch()}
        >
          {status && (
            <OverviewReady
              status={status}
              configQuery={configQuery}
              requestsQuery={requestsQuery}
              statsQuery={statsQuery}
              healthQuery={healthQuery}
              quotasQuery={quotasQuery}
              providersQuery={providersQuery}
              confirmStop={confirmStop}
              setConfirmStop={setConfirmStop}
              busy={busy}
              onStop={() => void stop()}
              onRefresh={refreshAll}
              canStopService={supervisorLocalKnown}
            />
          )}
        </QueryState>
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// 就绪态子树（A–E）
// ---------------------------------------------------------------------------

function OverviewReady(
  props: Omit<Parameters<typeof OverviewReadyBody>[0], 'navigate'>,
) {
  // 路由跳转（子树内）；其余全部透传。
  const navigate = useNavigate()
  return <OverviewReadyBody {...props} navigate={navigate} />
}

function OverviewReadyBody({
  status,
  configQuery,
  requestsQuery,
  statsQuery,
  healthQuery,
  quotasQuery,
  providersQuery,
  confirmStop,
  setConfirmStop,
  busy,
  onStop,
  onRefresh,
  navigate,
  canStopService,
}: {
  status: NonNullable<ReturnType<typeof useStatusQuery>['data']>
  configQuery: ReturnType<typeof useConfigQuery>
  requestsQuery: ReturnType<typeof useRequestsQuery>
  statsQuery: ReturnType<typeof useStatsSummaryQuery>
  healthQuery: ReturnType<typeof useHealthQuery>
  quotasQuery: ReturnType<typeof useQuotasQuery>
  providersQuery: ReturnType<typeof useProvidersQuery>
  confirmStop: boolean
  setConfirmStop: (v: boolean) => void
  busy: boolean
  onStop: () => void
  onRefresh: () => void
  navigate: ReturnType<typeof useNavigate>
  /** 仅本地托管（supervisor.external === false）时可用停止动作。 */
  canStopService: boolean
}) {
  // 配置事实推导（config 未知 ≠ 零配置：loading/error 各自呈现）。
  const draft: ConfigDraft | null = configQuery.data ? draftFromDocument(configQuery.data) : null
  const candidateIds = new Set(draft?.candidates.map((c) => c.id) ?? [])
  const usableVMs = (draft?.virtualModels ?? []).filter(
    (vm) => vm.candidate_pool.length > 0 && vm.candidate_pool.every((id) => candidateIds.has(id)),
  )
  const hasManualCandidates = (draft?.candidates.length ?? 0) > 0
  const configUnknown = configQuery.isLoading || configQuery.isError

  // 磁盘配置 VM 非空 ≠ 运行已生效：digest 与 revision 比对，不一致提示待重启。
  const configPendingRestart =
    draft !== null &&
    configQuery.data !== undefined &&
    configQuery.data.revision !== status.config_digest

  const providers: ProviderInfo[] = providersQuery.data ?? []
  const providersUnknown = providersQuery.isLoading || providersQuery.isError
  const activeProviders = providers.filter((p) => p.account_count > 0)
  const hasProviders = activeProviders.length > 0
  const catalogFailed = activeProviders.filter((p) => p.status === 'catalog_error')
  const providersOk = hasProviders && catalogFailed.length === 0

  const hasCompiled = providers.some((p) => p.candidate_count > 0)
  const hasVM = usableVMs.length > 0
  // 请求事实：requests 未知 ≠ 无请求。
  const requestsUnknown = requestsQuery.isLoading || requestsQuery.isError
  const hasRequests = (requestsQuery.data?.items.length ?? 0) > 0

  // ── 状态分型（§6）。配置未知时七态悬置，只显示未知提示；
  //    待重启（磁盘配置 ≠ 运行 digest）不算「配置就绪/入口可用」，落 B 态提示。──
  const vmEffective = hasVM && !configPendingRestart
  const stateA = !configUnknown && !providersUnknown && !hasProviders && !hasManualCandidates && !hasVM
  const stateB = !configUnknown && !providersUnknown && !stateA && !vmEffective
  const stateC = vmEffective && !configUnknown && !requestsUnknown && !hasRequests
  const stateDE = vmEffective && !configUnknown && !requestsUnknown && hasRequests
  const suspended = !stateA && !stateB && !stateC && !stateDE

  // E 行动项。
  const exhaustedQuotas = (quotasQuery.data?.items ?? []).filter(
    (item) => item.remaining === 0 || item.consecutive_429 > 0,
  )
  const openBreakers = status.breakers.open
  const stats = statsQuery.data
  const lowSuccess = stats !== undefined && stats.requests >= 10 && stats.success_rate < 0.9

  // ── 起步清单（A/B）───────────────────────────────────────────────
  const steps: StepView[] = []
  if (!hasProviders) {
    steps.push({
      state: 'pending',
      title: '① 添加模型来源（厂商 + Key）',
      hint: '不需要手写配置文件；验证连通性可跳过，跳过后账号标「未验证」，本步在验证前不算完成。',
      actionLabel: '去模型来源页',
      actionTo: '/providers',
    })
  } else if (providersOk) {
    steps.push({
      state: 'done',
      title: `① 已有 ${activeProviders.length} 个模型来源`,
      hint: '账号已配置；目录就绪的厂商可正常编译候选。账号验证状态以模型来源页为准。',
    })
  } else {
    steps.push({
      state: 'pending',
      title: `① 已有 ${activeProviders.length} 个模型来源，目录有问题`,
      hint: '部分厂商目录刷新失败，建议去模型来源页验证账号 / 刷新目录。',
      actionLabel: '去验证',
      actionTo: '/providers',
    })
  }

  if (hasCompiled) {
    const compiled = providers.reduce((sum, p) => sum + p.candidate_count, 0)
    steps.push({
      state: 'done',
      title: `② 已应用模型选择（编译出 ${compiled} 个候选）`,
      hint: '勾选的目录模型已写回配置。',
    })
  } else if (hasManualCandidates) {
    steps.push({
      state: 'skipped',
      title: '② 目录编译已跳过',
      hint: '配置里已有手写候选（如本地模型），不强制走目录流程；已跳过不算完成。',
    })
  } else {
    steps.push({
      state: 'pending',
      title: '② 从目录勾选模型并应用模型选择',
      hint: '将勾选的模型编译为候选写回配置；本地模型无 key 可跳过本步。',
      actionLabel: '去模型来源页',
      actionTo: '/providers',
    })
  }

  if (hasVM) {
    steps.push({
      state: 'done',
      title: `③ 模型入口已配置（${usableVMs.length} 个）${configPendingRestart ? ' · 待重启生效' : ''}`,
      hint: configPendingRestart
        ? '磁盘配置与运行中的网关不一致；重启网关后入口才真正生效。'
        : `候选池非空且候选可解析${status.default_virtual_model ? `；默认入口 ${status.default_virtual_model}` : ''}。`,
    })
  } else {
    steps.push({
      state: 'pending',
      title: '③ 创建可用的模型入口',
      hint: '绑定非空候选池的入口是清单完成的判定线；没有它，前面的步骤不算完成。',
      actionLabel: '去模型与路由',
      actionTo: '/virtual-models',
    })
  }

  // ④ 接入：无持久验证数据，只有真实请求记录是证据。
  if (hasRequests) {
    steps.push({
      state: 'done',
      title: '④ 网关有真实请求记录',
      hint: '请求可能来自已接入的 agent，也可能来自模型测试；详情见请求活动页。',
    })
  } else {
    steps.push({
      state: 'pending',
      title: '④ 把 agent 指到网关',
      hint: '接入验证通过只代表配置读回校验，不代表 agent 真实跑过请求。',
      actionLabel: '去接入向导',
      actionTo: '/onboarding',
    })
  }

  const doneCount = steps.filter((s) => s.state === 'done').length
  const nextStep = steps.find((s) => s.state === 'pending')

  const healthItems = healthQuery.data?.items ?? []
  const unhealthy = healthItems.filter((item) => item.status !== 'closed')

  return (
    <>
      {/* E 行动横幅（≤3 条） */}
      {stateDE && openBreakers > 0 && (
        <div className="app-banner app-banner-warn">
          <span>{openBreakers} 个候选熔断打开，路由将避开它们，直到探测恢复。</span>
          <Link to="/diagnostics" className="underline">
            去诊断
          </Link>
        </div>
      )}
      {stateDE && exhaustedQuotas.length > 0 && (
        <div className="app-banner app-banner-warn">
          <span>
            {exhaustedQuotas.length} 个候选配额耗尽或被上游限流（
            {exhaustedQuotas
              .slice(0, 3)
              .map((item) => item.candidate_id)
              .join(', ')}
            {exhaustedQuotas.length > 3 ? ' …' : ''}
            ），路由将避开它们。
          </span>
          <Link to="/quotas" className="underline">
            查看配额
          </Link>
        </div>
      )}
      {stateDE && catalogFailed.length > 0 && (
        <div className="app-banner app-banner-warn">
          <span>
            {catalogFailed.length} 个厂商目录刷新失败（
            {catalogFailed
              .slice(0, 3)
              .map((p) => p.display_name)
              .join(', ')}
            ），目录里的模型可能不是最新。
          </span>
          <Link to="/providers" className="underline">
            去模型来源
          </Link>
        </div>
      )}
      {stateDE && lowSuccess && stats && (
        <div className="app-banner app-banner-warn">
          <span>
            最近 24 小时成功率 {(stats.success_rate * 100).toFixed(1)}%（{stats.upstream_errors}{' '}
            次上游错误），建议查一下失败请求的原因。
          </span>
          <Link to="/requests" className="underline">
            去请求活动
          </Link>
        </div>
      )}
      {configPendingRestart && !configUnknown && (
        <div className="app-banner app-banner-warn">
          <span>磁盘配置与运行中的网关不一致；重启网关后，已保存的配置才真正生效。</span>
        </div>
      )}

      {/* A / B：起步清单 */}
      {(stateA || stateB) && (
        <section className="app-card" aria-label="起步清单">
          <h2 className="app-card-title">{stateA ? '开始使用' : '继续设置'}</h2>
          <div className="app-checklist">
            {steps.map((step) => (
              <ChecklistItem key={step.title} step={step} />
            ))}
          </div>
          <div className="app-checklist-footer">
            <span>
              已完成 {doneCount}/{steps.length}
              {nextStep ? ' · 下一步：' + nextStep.title.replace(/^[①②③④]\s*/, '') : ' · 全部完成'}
            </span>
          </div>
        </section>
      )}

      {/* C：清单收起为「接入 agent」提醒（待重启时不算配置就绪，不渲染） */}
      {stateC && (
        <div className="app-banner app-banner-info" role="status">
          <Icon name="info" className="size-4" />
          <span>配置已就绪，还没有请求记录。把 agent 接到网关后，这里会出现每一次模型选择。</span>
          <Link to="/onboarding" className="oc-btn-default oc-btn-sm">
            去应用接入
          </Link>
        </div>
      )}

      {/* 配置未知（loading/error）：不冒充零配置 */}
      {configUnknown && (
        <div className="app-banner app-banner-info" role="status">
          <Icon name="info" className="size-4" />
          <span>
            {configQuery.isLoading
              ? '正在读取配置…'
              : '配置读取失败，暂时无法判断配置状态。'}
          </span>
          {configQuery.isError && (
            <button
              type="button"
              className="oc-btn-default oc-btn-sm"
              onClick={() => void configQuery.refetch()}
            >
              <Icon name="refresh" />
              重试
            </button>
          )}
        </div>
      )}

      {/* D / E：结论卡（口径：最近 24 小时） */}
      {stateDE && (
        <>
          <div className="grid grid-cols-2 gap-3 max-[900px]:grid-cols-1">
            <MetricCard
              label="最近 24 小时请求"
              note={stats && stats.switches > 0 ? `其中 ${stats.switches} 次模型切换` : undefined}
            >
              <MetricValue
                loading={statsQuery.isLoading}
                error={statsQuery.isError}
                value={stats ? String(stats.requests) : '暂无数据'}
                retry={() => void statsQuery.refetch()}
              />
            </MetricCard>
            <MetricCard
              label="成功率"
              note={stats && stats.upstream_errors > 0 ? `${stats.upstream_errors} 次上游错误` : undefined}
              warn={Boolean(stats && stats.upstream_errors > 0)}
            >
              <MetricValue
                loading={statsQuery.isLoading}
                error={statsQuery.isError}
                value={stats ? `${(stats.success_rate * 100).toFixed(1)}%` : '暂无数据'}
                retry={() => void statsQuery.refetch()}
              />
            </MetricCard>
            {/* 成本：账本已计价合计；覆盖未知（覆盖率 DTO 未落地）。
                $0 有数据显示 $0；查询失败如实呈现。 */}
            <MetricCard label="已计价成本" note="账本中已有分账的请求合计 · 覆盖未知">
              <MetricValue
                loading={statsQuery.isLoading}
                error={statsQuery.isError}
                value={
                  stats
                    ? stats.cost_micros.answer + stats.cost_micros.routing > 0
                      ? fmtUsd(stats.cost_micros.answer + stats.cost_micros.routing)
                      : '$0'
                    : '暂无数据'
                }
                retry={() => void statsQuery.refetch()}
              />
            </MetricCard>
            <MetricCard
              label="模型入口健康"
              warn={openBreakers > 0 || unhealthy.length > 0}
              note={
                healthQuery.isLoading
                  ? undefined
                  : healthQuery.isError
                    ? undefined
                    : unhealthy.length > 0
                      ? `候选 ${healthItems.length} 个，${unhealthy.length} 个非闭合`
                      : `候选 ${healthItems.length} 个，全部闭合`
              }
            >
              <MetricValue
                loading={healthQuery.isLoading}
                error={healthQuery.isError}
                value={`熔断 ${openBreakers} 打开`}
                retry={() => void healthQuery.refetch()}
              />
            </MetricCard>
          </div>
        </>
      )}

      {/* D / E：配置入口摘要 */}
      {stateDE && (
        <section className="app-card" aria-label="配置入口">
          <h2 className="app-card-title">配置入口</h2>
          <div>
            <Link to="/virtual-models" className="app-entry-row hover:bg-interactive-hover">
              <Icon name="nav-virtual-models" className="size-4 text-muted-foreground" />
              <span className="app-entry-main">
                <span className="app-entry-title">模型入口</span>
                <span className="app-entry-meta">
                  {status.default_virtual_model
                    ? `默认 ${status.default_virtual_model} · 共 ${usableVMs.length} 个${configPendingRestart ? ' · 待重启生效' : ''}`
                    : `共 ${usableVMs.length} 个${configPendingRestart ? ' · 待重启生效' : ''}`}
                </span>
              </span>
              <Icon name="expand" className="size-4 text-muted-foreground" />
            </Link>
            <Link to="/providers" className="app-entry-row hover:bg-interactive-hover">
              <Icon name="nav-providers" className="size-4 text-muted-foreground" />
              <span className="app-entry-main">
                <span className="app-entry-title">模型来源</span>
                <span className="app-entry-meta">
                  {providers.length > 0
                    ? `${providers.length} 家厂商 · ${providers.reduce((s, p) => s + p.account_count, 0)} 个账号`
                    : '尚未配置'}
                </span>
              </span>
              <Icon name="expand" className="size-4 text-muted-foreground" />
            </Link>
          </div>
        </section>
      )}

      {/* D / E：最近请求（行可点进请求活动页） */}
      {stateDE && (
        <QueryState
          loading={requestsQuery.isLoading}
          error={requestsQuery.error}
          empty={(requestsQuery.data?.items.length ?? 0) === 0}
          onRetry={() => void requestsQuery.refetch()}
        >
          <section className="app-card" aria-label="最近请求">
            <h2 className="app-card-title">最近请求</h2>
            <table className="app-table app-table-clickable">
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
                  <tr
                    key={row.request_id}
                    tabIndex={0}
                    aria-label={`查看请求 ${row.request_id}，去请求活动页`}
                    onClick={() => void navigate({ to: '/requests' })}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') void navigate({ to: '/requests' })
                    }}
                  >
                    <td className="font-mono">{new Date(row.created_at).toLocaleTimeString()}</td>
                    <td className="font-mono">{row.dispatch_model ?? row.selection_model}</td>
                    <td>
                      <span className={outcomePillClass(row.outcome)}>{row.outcome}</span>
                    </td>
                    <td>{row.duration_ms}ms</td>
                  </tr>
                ))}
              </tbody>
            </table>
            <Link to="/requests" className="text-meta underline self-start">
              去请求活动查看全部
            </Link>
          </section>
        </QueryState>
      )}

      {/* 服务行：版本/监听/运行时长 + 启停（Tauri）；停止需确认 */}
      <section className="app-card" aria-label="服务">
        <div className="flex items-center gap-3 flex-wrap text-meta text-muted-foreground">
          <span>
            版本 {status.version}（api v{status.api_version}）· 监听{' '}
            <span className="font-mono">{status.listen}</span> · 运行 {fmtUptime(status.uptime_ms)}
          </span>
          <span className="ml-auto flex items-center gap-2">
            {confirmStop ? (
              <>
                <span className="app-confirm">确认停止网关？正在进行的请求会中断。</span>
                <button
                  type="button"
                  className="oc-btn-destructive oc-btn-sm"
                  disabled={busy}
                  onClick={onStop}
                >
                  <Icon name="stop" />
                  {busy ? '停止中…' : '确认停止'}
                </button>
                <button
                  type="button"
                  className="oc-btn-ghost oc-btn-sm"
                  onClick={() => setConfirmStop(false)}
                >
                  取消
                </button>
              </>
            ) : (
              <>
                {canStopService && (
                  <button
                    type="button"
                    className="oc-btn-destructive oc-btn-sm"
                    disabled={busy}
                    onClick={() => setConfirmStop(true)}
                  >
                    <Icon name="stop" />
                    停止服务
                  </button>
                )}
                <button type="button" className="oc-btn-default oc-btn-sm" onClick={onRefresh}>
                  <Icon name="refresh" />
                  刷新
                </button>
              </>
            )}
          </span>
        </div>
      </section>

      {/* 七态悬置（providers 未知等）：不做任何状态断言 */}
      {suspended && !configUnknown && (
        <QueryState loading={providersQuery.isLoading} error={providersQuery.error}>
          {null}
        </QueryState>
      )}
    </>
  )
}
