/**
 * 智能体（/apps）：把本机的 agent 接到网关上，并为每个 agent 选一个入口。
 *
 * 形态参照 Magpie 的「使用 / Agent」页：
 *   - 页头：使用 / 智能体 + 更新（重新检测全部）；
 *   - 一行一个 agent：图标 + 名称 + 状态（已接入 / 未接入 / 未安装）+
 *     右侧「详情 ›」与入口下拉；展开后是接入设置、模型列表、
 *     模型名带供应商、新会话默认与配置文件清单；
 *   - 其余 agent 折叠在「显示其余 N 个」里；
 *   - 末尾是「安装其他 Agent」清单（手动接入的环境变量写法）。
 *
 * 数据来源：
 * - 安装 / 接入状态 / 当前入口：`onboardDetect` 读回，不猜；
 * - 可选入口：只读 admin 配置里的 virtual_models；
 * - 切换流程：预览（plan）→ 写入（apply）→ 校验（verify）→ 重新 detect。
 *
 * 凭据只传引用（env: / file:），明文 token 不经过 UI。
 */
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react'
import { Link } from '@tanstack/react-router'
import { Icon, type IconName } from '../components/common/Icon'
import { Modal } from '../components/common/Modal'
import { EmptyState, QueryState } from '../components/common/QueryState'
import { AGENTS, MANUAL_AGENTS } from '../constants/agents'
import { useGatewayUrl } from '../hooks/useGatewayUrl'
import { isTauri, invokeErrorMessage as errorMessage } from '../lib/tauri'
import { planActionPillClass } from '../lib/status'
import {
  defaultVirtualModelOf,
  virtualModelSummaries,
  type VirtualModelSummary,
} from '../lib/config-edit'
import { useConfigQuery, useStatusQuery } from '../queries/admin'
import { useAppStore } from '../stores/appStore'
import {
  onboardApply,
  onboardDetect,
  onboardPlan,
  onboardVerify,
  onboardWslDetect,
  type DetectReport,
  type OnboardSetupArgs,
  type SetupPlan,
  type WslReport,
} from '../lib/tauri'

interface AgentDetectState {
  report: DetectReport | null
  error: string | null
  loading: boolean
}

const IDLE_DETECT: AgentDetectState = { report: null, error: null, loading: false }

function initialDetectState(): Record<string, AgentDetectState> {
  return Object.fromEntries(
    AGENTS.map((agent) => [agent.id, { report: null, error: null, loading: true }]),
  )
}

interface AgentRowModel {
  key: string
  agentId: string
  agentLabel: string
  icon: IconName
  /** WSL 行才有；宿主行为 null。 */
  distro: string | null
  state: AgentDetectState
}

export function AppsPage() {
  const gateway = useGatewayUrl()
  const [credentialRef, setCredentialRef] = useState('')
  const [detect, setDetect] = useState<Record<string, AgentDetectState>>(initialDetectState)
  const [picker, setPicker] = useState<{
    agentId: string
    distro: string | null
    model: string | null
  } | null>(null)
  const [expanded, setExpanded] = useState<string | null>(null)
  const [wsl, setWsl] = useState<WslReport | null>(null)
  const [wslDistro, setWslDistro] = useState('')
  const [wslDetect, setWslDetect] = useState<AgentDetectState>(IDLE_DETECT)
  const [showOthers, setShowOthers] = useState(false)

  const runDetect = useCallback(async (ids: readonly string[]) => {
    if (!isTauri()) return
    setDetect((prev) => {
      const next = { ...prev }
      for (const id of ids) {
        next[id] = { ...(next[id] ?? IDLE_DETECT), loading: true, error: null }
      }
      return next
    })

    type Outcome = { id: string; report: DetectReport } | { id: string; error: string }
    const outcomes = await Promise.all(
      ids.map(async (id): Promise<Outcome> => {
        try {
          return { id, report: await onboardDetect({ agent: id, wsl_distro: null }) }
        } catch (err) {
          return { id, error: errorMessage(err) }
        }
      }),
    )

    setDetect((prev) => {
      const next = { ...prev }
      for (const outcome of outcomes) {
        const current = next[outcome.id] ?? IDLE_DETECT
        next[outcome.id] =
          'report' in outcome
            ? { report: outcome.report, error: null, loading: false }
            : { ...current, loading: false, error: outcome.error }
      }
      return next
    })
  }, [])

  useEffect(() => {
    if (!isTauri()) return
    void runDetect(AGENTS.map((agent) => agent.id))
  }, [runDetect])

  const runWslDetect = useCallback(async (distro: string) => {
    if (!isTauri() || distro === '') return
    setWslDetect({ report: null, error: null, loading: true })
    try {
      const report = await onboardDetect({ agent: 'claude-code', wsl_distro: distro })
      setWslDetect({ report, error: null, loading: false })
    } catch (err) {
      setWslDetect({ report: null, error: errorMessage(err), loading: false })
    }
  }, [])

  useEffect(() => {
    if (!isTauri()) return
    void onboardWslDetect()
      .then((report) => {
        setWsl(report)
        const distro = report.default_distro ?? report.distros[0] ?? ''
        setWslDistro(distro)
        if (distro !== '') void runWslDetect(distro)
      })
      .catch(() => setWsl(null))
  }, [runWslDetect])

  const models = virtualModelSummaries(gateway.document)
  const defaultModel = defaultVirtualModelOf(gateway.document)

  // 磁盘配置与应用中的网关不一致时，新入口在运行时还不存在：
  // 允许浏览，但不能把 agent 切过去（否则立刻 404）。
  const configQuery = useConfigQuery()
  const statusQuery = useStatusQuery()
  const pendingRestart =
    configQuery.data !== undefined &&
    statusQuery.data !== undefined &&
    configQuery.data.revision !== statusQuery.data.config_digest
  const showToast = useAppStore((state) => state.showToast)

  const rows: AgentRowModel[] = useMemo(
    () => [
      ...AGENTS.map((agent) => ({
        key: agent.id,
        agentId: agent.id,
        agentLabel: agent.label,
        icon: agent.icon as IconName,
        distro: null,
        state: detect[agent.id] ?? IDLE_DETECT,
      })),
      ...(wsl?.wsl_available === true && wslDistro !== ''
        ? [
            {
              key: `wsl:${wslDistro}`,
              agentId: 'claude-code',
              agentLabel: 'Claude Code',
              icon: 'agent-terminal' as IconName,
              distro: wslDistro,
              state: wslDetect,
            },
          ]
        : []),
    ],
    [detect, wsl, wslDistro, wslDetect],
  )

  if (!isTauri()) {
    return (
      <div className="app-page">
        <section className="app-card" aria-label="智能体">
          <h2 className="app-card-title">智能体</h2>
          <EmptyState
            title="需要桌面客户端"
            action={<code className="app-inline-code">tr-cli admin</code>}
          />
        </section>
      </div>
    )
  }

  const detecting =
    AGENTS.some((agent) => detect[agent.id]?.loading) || wslDetect.loading
  const hasAnyReport = AGENTS.some((agent) => detect[agent.id]?.report)

  const installedRows = rows.filter((row) => {
    const { report, loading, error } = row.state
    if (loading || error !== null || report === null) return true
    return report.installed || report.already_configured
  })
  const otherRows = rows.filter((row) => !installedRows.includes(row))

  const pickerAgentLabel = (() => {
    if (!picker) return ''
    if (picker.distro) return `Claude Code（WSL ${picker.distro}）`
    return AGENTS.find((agent) => agent.id === picker.agentId)?.label ?? picker.agentId
  })()
  const pickerReport = (() => {
    if (!picker) return null
    return picker.distro ? wslDetect.report : (detect[picker.agentId]?.report ?? null)
  })()

  const refreshAll = () => {
    gateway.refetch()
    void runDetect(AGENTS.map((agent) => agent.id))
    if (wslDistro !== '') void runWslDetect(wslDistro)
  }

  /** 待重启时禁止切换入口：目标在运行中的网关里还不存在，写进 agent 必然 404。 */
  const pickModel = (agentId: string, distro: string | null, model: string) => {
    if (pendingRestart) {
      showToast('配置已修改但网关未重启：新入口暂时不可用，请先重启网关', false)
      return
    }
    setPicker({ agentId, distro, model })
  }

  return (
    <div className="app-page">
      {/* 智能体列表 */}
      <section className="app-card" aria-label="智能体列表">
        <div className="oc-tile flex flex-wrap items-end gap-3 rounded-lg p-2.5">
          <div className="app-kv-item min-w-0">
            <span className="app-kv-key">网关地址</span>
            <span className="app-kv-value font-mono">{gateway.url}</span>
          </div>
          <label className="app-field min-w-45 flex-1" htmlFor="agents-credential-ref">
            凭据引用
            <input
              id="agents-credential-ref"
              type="text"
              className="oc-field app-field-light font-mono"
              value={credentialRef}
              placeholder="env:TR_TOKEN 或 file:admin.token"
              aria-required="true"
              autoComplete="off"
              spellCheck={false}
              onChange={(event) => setCredentialRef(event.target.value)}
            />
          </label>
          <button
            type="button"
            className="oc-btn-default oc-btn-sm"
            disabled={detecting}
            onClick={refreshAll}
          >
            <Icon name="refresh" />
            {detecting ? '检测中…' : '更新'}
          </button>
        </div>

        {pendingRestart && (
          <div className="app-banner app-banner-warn text-meta">
            配置已修改但网关未重启，重启前不要切换入口。
          </div>
        )}

        <QueryState
          loading={gateway.isLoading || (detecting && !hasAnyReport)}
          error={gateway.error}
          onRetry={refreshAll}
        >
          <div className="flex flex-col">
            {installedRows.map((row) => (
              <AgentRow
                key={row.key}
                row={row}
                models={models}
                defaultModel={defaultModel}
                gatewayUrl={gateway.url}
                expanded={expanded === row.key}
                onToggle={() => setExpanded(expanded === row.key ? null : row.key)}
                onPickModel={(model) => pickModel(row.agentId, row.distro, model)}
                switchingDisabled={pendingRestart}
              />
            ))}
          </div>

          {otherRows.length > 0 && (
            <div className="flex flex-col gap-1.5">
              <button
                type="button"
                className="oc-btn-default oc-btn-sm self-start"
                aria-expanded={showOthers}
                aria-controls="agents-others"
                onClick={() => setShowOthers((value) => !value)}
              >
                显示其余 {otherRows.length} 个
                <span aria-hidden>{showOthers ? '▾' : '▸'}</span>
              </button>
              {showOthers && (
                <div id="agents-others" className="flex flex-col">
                  {otherRows.map((row) => (
                    <AgentRow
                      key={row.key}
                      row={row}
                      models={models}
                      defaultModel={defaultModel}
                      gatewayUrl={gateway.url}
                      expanded={expanded === row.key}
                      onToggle={() => setExpanded(expanded === row.key ? null : row.key)}
                      onPickModel={(model) => pickModel(row.agentId, row.distro, model)}
                      switchingDisabled={pendingRestart}
                    />
                  ))}
                </div>
              )}
            </div>
          )}
        </QueryState>

        <div className="app-card-actions">
          <Link to="/onboarding" className="oc-btn-default oc-btn-sm">
            <Icon name="nav-onboarding" />
            手动接入
          </Link>
          <Link to="/providers" className="oc-btn-default oc-btn-sm">
            <Icon name="nav-providers" className="size-3.5" />
            管理模型来源
          </Link>
        </div>
      </section>

      {/* 其他智能体 */}
      <section className="app-card" aria-label="其他智能体">
        <span className="app-card-title">其他智能体 ({MANUAL_AGENTS.length})</span>
        <div className="flex flex-wrap gap-2">
          {MANUAL_AGENTS.map((agent) => (
            <span key={agent.id} className="app-pill" title={`${agent.env}=${gateway.url}/v1`}>
              {agent.label}
            </span>
          ))}
        </div>
      </section>

      {picker !== null && (
        <AppModelModal
          key={`${picker.agentId}:${picker.distro ?? 'host'}`}
          agentId={picker.agentId}
          agentLabel={pickerAgentLabel}
          report={pickerReport}
          gatewayUrl={gateway.url}
          credentialRef={credentialRef}
          models={models}
          defaultModel={defaultModel}
          initialModel={picker.model}
          wslDistro={picker.distro}
          onApplied={() =>
            picker.distro ? runWslDetect(picker.distro) : runDetect([picker.agentId])
          }
          onClose={() => setPicker(null)}
        />
      )}
    </div>
  )
}

/** 一行智能体：左侧名称与状态，右侧「详情」与入口下拉；展开后是接入详情。 */
function AgentRow({
  row,
  models,
  defaultModel,
  gatewayUrl,
  expanded,
  onToggle,
  onPickModel,
  switchingDisabled = false,
}: {
  row: AgentRowModel
  models: VirtualModelSummary[]
  defaultModel: string | null
  gatewayUrl: string
  expanded: boolean
  onToggle: () => void
  onPickModel: (model: string) => void
  /** 配置待重启：新入口在运行时不存在，禁止切换。 */
  switchingDisabled?: boolean
}) {
  const { report, loading, error } = row.state
  const configuredModel = report?.configured_model ?? null
  const configuredOption = models.find((option) => option.id === configuredModel)

  const statusText = (() => {
    if (loading) return '检测中…'
    if (error) return '检测失败'
    if (!report) return '尚未检测'
    if (!report.installed) return '未安装'
    if (configuredModel || report.already_configured) return '已接入'
    return '未接入'
  })()

  return (
    <div className="app-agent-row">
      <div className="flex items-center gap-2 flex-wrap min-w-0">
        <Icon name={row.icon} className="size-4 shrink-0 text-muted-foreground" />
        <span className="text-ui-label font-semibold truncate">{row.agentLabel}</span>
        {row.distro && <span className="app-pill">WSL {row.distro}</span>}
        <span
          className={
            report?.already_configured === true ? 'app-pill app-pill-ok' : 'app-pill'
          }
        >
          {statusText}
        </span>
      </div>

      <div className="flex items-center gap-2 flex-wrap justify-end">
        <button
          type="button"
          className="oc-btn-default oc-btn-sm"
          aria-expanded={expanded}
          onClick={onToggle}
        >
          {expanded ? '收起' : '详情'}
          <span aria-hidden>{expanded ? '▴' : '›'}</span>
        </button>
        <select
          className="app-select-sm"
          value={configuredModel ?? ''}
          aria-label={`为 ${row.agentLabel} 选择入口`}
          disabled={models.length === 0 || loading || error !== null || switchingDisabled}
          title={switchingDisabled ? '配置待重启，重启后再切换入口' : undefined}
          onChange={(event) => onPickModel(event.target.value)}
        >
          <option value="">选择入口…</option>
          {configuredModel && !configuredOption && (
            <option value={configuredModel} disabled>
              {configuredModel}（不可用）
            </option>
          )}
          {models.map((option) => (
            <option key={option.id} value={option.id} disabled={!option.enabled}>
              {option.id}
              {option.enabled ? '' : '（已停用）'}
            </option>
          ))}
        </select>
      </div>

      {expanded && (
        <div className="app-agent-body">
          <div className="app-gateway-row">
            <span className="app-gateway-key">网关地址</span>
            <span className="font-mono text-meta">{gatewayUrl}</span>
          </div>

          <div className="app-gateway-row">
            <span className="app-gateway-key">模型</span>
            <div className="flex items-center gap-2 flex-wrap min-w-0">
              {models.length === 0 ? (
                <span className="text-meta text-muted-foreground">—</span>
              ) : (
                models.map((option) => (
                  <button
                    key={option.id}
                    type="button"
                    className="app-pill app-pill-clickable"
                    disabled={!option.enabled || switchingDisabled}
                    title={`${option.id} · ${option.candidateCount ?? 0} 个模型`}
                    onClick={() => onPickModel(option.id)}
                  >
                    {option.id} {option.candidateCount ?? 0}
                  </button>
                ))
              )}
            </div>
          </div>

          <div className="app-gateway-row">
            <span className="app-gateway-key">默认入口</span>
            <div className="flex items-center gap-2 flex-wrap">
              <span className="font-mono text-meta">{defaultModel ?? '—'}</span>
              <Link to="/virtual-models" className="oc-btn-default oc-btn-sm">
                <Icon name="nav-virtual-models" className="size-3.5" />
                路由组
              </Link>
            </div>
          </div>

          <div className="app-gateway-row">
            <span className="app-gateway-key">配置文件</span>
            <div className="flex flex-col gap-0.5 min-w-0">
              {report && report.config_files.length > 0 ? (
                report.config_files.map((file) => (
                  <span key={file.path} className="font-mono text-micro text-muted-foreground">
                    {file.path}
                    {file.exists ? '' : '（不存在）'}
                  </span>
                ))
              ) : (
                <span className="text-micro text-muted-foreground">
                  {loading ? '检测中…' : '—'}
                </span>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

function AppModelModal({
  agentId,
  agentLabel,
  report,
  gatewayUrl,
  credentialRef,
  models,
  defaultModel,
  initialModel = null,
  wslDistro = null,
  onApplied,
  onClose,
}: {
  agentId: string
  agentLabel: string
  report: DetectReport | null
  gatewayUrl: string
  credentialRef: string
  models: VirtualModelSummary[]
  defaultModel: string | null
  initialModel?: string | null
  wslDistro?: string | null
  onApplied: () => Promise<void>
  onClose: () => void
}) {
  const showToast = useAppStore((store) => store.showToast)
  const searchRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)

  const configuredModel = report?.configured_model ?? null
  const configuredOption = models.find((option) => option.id === configuredModel)
  const preferredModel = (() => {
    // 显式选中的目标优先；停用入口不可作为新目标。
    if (
      initialModel !== null &&
      models.some((option) => option.id === initialModel && option.enabled)
    ) {
      return initialModel
    }
    if (
      configuredModel &&
      models.some((option) => option.id === configuredModel && option.enabled)
    ) {
      return configuredModel
    }
    if (defaultModel && models.some((option) => option.id === defaultModel && option.enabled)) {
      return defaultModel
    }
    return models.find((option) => option.enabled)?.id ?? ''
  })()

  const [model, setModel] = useState(preferredModel)
  const [query, setQuery] = useState('')
  const [plan, setPlan] = useState<SetupPlan | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const requestFor = (target: string): OnboardSetupArgs => ({
    agent: agentId,
    wsl_distro: wslDistro,
    gateway_url: gatewayUrl,
    credential_ref: credentialRef.trim(),
    model: target,
    force_model: true,
  })

  // 改选模型后旧预览不再可信（凭据输入在弹窗背后，打开期间改不了）。
  const chooseModel = (id: string) => {
    setModel(id)
    setPlan(null)
    setError(null)
  }

  const canSubmit = credentialRef.trim() !== '' && model !== '' && !busy

  const runPreview = async (target: string = model) => {
    if (busy) return
    if (credentialRef.trim() === '') {
      setError('请先填写凭据引用（env:NAME / file:path）。')
      return
    }
    if (target === '') {
      setError('请先选择一个入口。')
      return
    }
    setBusy(true)
    setError(null)
    try {
      setPlan(await onboardPlan(requestFor(target)))
    } catch (err) {
      setPlan(null)
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const runApply = async () => {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      await onboardApply(requestFor(model))
      const verify = await onboardVerify({ agent: agentId, wsl_distro: wslDistro })
      // 无论校验结果如何，重新 detect 让列表反映最新读回值。
      await onApplied()
      if (!verify.ok) {
        setError(`配置已写入，但校验未通过：${verify.messages.join('；')}`)
        return
      }
      showToast(`已将 ${agentLabel} 切换到 ${model}`)
      onClose()
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const needle = query.trim().toLowerCase()
  const visibleModels =
    needle === ''
      ? models
      : models.filter(
          (option) =>
            option.id.toLowerCase().includes(needle) ||
            option.name.toLowerCase().includes(needle),
        )
  const firstEnabled = visibleModels.find((option) => option.enabled)

  const focusFirstEnabled = () => {
    listRef.current
      ?.querySelector<HTMLButtonElement>('button[role="option"]:not(:disabled)')
      ?.focus()
  }

  const onSearchKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'ArrowDown') {
      event.preventDefault()
      focusFirstEnabled()
      return
    }
    if (event.key === 'Enter') {
      event.preventDefault()
      if (!firstEnabled) return
      chooseModel(firstEnabled.id)
      void runPreview(firstEnabled.id)
    }
  }

  const onListKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
    const items = Array.from(
      listRef.current?.querySelectorAll<HTMLButtonElement>(
        'button[role="option"]:not(:disabled)',
      ) ?? [],
    )
    const index = items.indexOf(document.activeElement as HTMLButtonElement)
    if (index < 0) return
    event.preventDefault()
    if (event.key === 'ArrowDown') {
      items[index + 1]?.focus()
    } else if (index === 0) {
      searchRef.current?.focus()
    } else {
      items[index - 1]?.focus()
    }
  }

  return (
    <Modal
      size="wide"
      title={`为 ${agentLabel} 选择入口`}
      initialFocusRef={searchRef}
      onClose={() => {
        if (!busy) onClose()
      }}
      footer={
        <>
          <span className="flex-1" />
          <button type="button" className="oc-btn-default" disabled={busy} onClick={onClose}>
            关闭
          </button>
          <button
            type="button"
            className="oc-btn-default"
            disabled={!canSubmit}
            onClick={() => void runPreview()}
          >
            <Icon name="search" />
            {busy ? '处理中…' : '预览变更'}
          </button>
          <button
            type="button"
            className="oc-btn-primary"
            disabled={!canSubmit}
            onClick={() => void runApply()}
          >
            <Icon name="apply" />
            {busy ? '处理中…' : '确认切换'}
          </button>
        </>
      }
    >
      <div className="flex flex-col gap-3">
        <input
          ref={searchRef}
          type="search"
          className="app-field-sm"
          value={query}
          placeholder="搜索入口 ID / 名称"
          aria-label="搜索入口"
          autoComplete="off"
          spellCheck={false}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={onSearchKeyDown}
        />

        {models.length === 0 ? (
          <EmptyState
            title="还没有路由组"
            action={
              <Link to="/virtual-models" className="oc-btn-default oc-btn-sm">
                去路由组
              </Link>
            }
          />
        ) : visibleModels.length === 0 ? (
          <p className="text-meta text-muted-foreground">没有匹配的入口。</p>
        ) : (
          <div
            ref={listRef}
            className="flex max-h-72 flex-col overflow-y-auto rounded-lg border border-border"
            role="listbox"
            aria-label="客户端入口"
            onKeyDown={onListKeyDown}
          >
            {visibleModels.map((option) => {
              const isSelected = option.id === model
              const isCurrent = option.id === configuredModel
              const isDefault = option.id === defaultModel
              return (
                <button
                  key={option.id}
                  type="button"
                  role="option"
                  aria-selected={isSelected}
                  aria-disabled={!option.enabled || undefined}
                  disabled={!option.enabled}
                  className={
                    !option.enabled
                      ? 'flex w-full items-center gap-2.5 border-b border-border px-3 py-2 text-left text-ui-label last:border-b-0 opacity-60 cursor-not-allowed'
                      : isSelected
                        ? 'flex w-full items-center gap-2.5 border-b border-border px-3 py-2 text-left text-ui-label last:border-b-0 cursor-pointer transition-colors bg-interactive-selection'
                        : 'flex w-full items-center gap-2.5 border-b border-border px-3 py-2 text-left text-ui-label last:border-b-0 cursor-pointer transition-colors hover:bg-interactive-hover'
                  }
                  onClick={() => chooseModel(option.id)}
                >
                  <span className="flex min-w-0 flex-1 flex-col items-start gap-0.5">
                    <span className="flex items-center gap-1.5 flex-wrap">
                      <span className="font-mono">{option.id}</span>
                      {option.name !== '' && (
                        <span className="text-muted-foreground">{option.name}</span>
                      )}
                    </span>
                    <span className="text-micro text-muted-foreground">
                      {option.candidateCount ?? 0} 个模型
                    </span>
                  </span>
                  <span className="flex items-center gap-1 flex-wrap justify-end">
                    {isCurrent && <span className="app-pill app-pill-ok">当前</span>}
                    {isDefault && <span className="app-pill">默认</span>}
                    {!option.enabled && <span className="app-pill app-pill-warn">已停用</span>}
                  </span>
                </button>
              )
            })}
          </div>
        )}

        {configuredModel && !configuredOption && (
          <div className="app-banner app-banner-warn">
            当前入口 <span className="font-mono">{configuredModel}</span> 不在运行配置中。
          </div>
        )}
        {configuredOption && !configuredOption.enabled && (
          <div className="app-banner app-banner-warn">
            当前入口 <span className="font-mono">{configuredModel}</span> 已停用。
          </div>
        )}
        {report?.already_configured && configuredModel === null && (
          <div className="app-banner app-banner-warn">已接入，但读不回模型 id。</div>
        )}

        {credentialRef.trim() === '' && (
          <div className="app-banner app-banner-warn">填写凭据引用后才能切换。</div>
        )}

        {plan && (
          <div className="flex flex-col gap-2">
            <h3 className="text-ui-label font-semibold">
              变更预览（{plan.entries.length} 个文件）
            </h3>

            <div className="flex items-center gap-3">
              <div className="flex min-w-0 flex-1 flex-col gap-0.5 rounded-lg border border-border px-3 py-2">
                <span className="text-micro text-muted-foreground">当前入口</span>
                <span className="font-mono text-ui-label font-semibold break-all">
                  {configuredModel ?? '—'}
                </span>
              </div>
              <span className="text-muted-foreground" aria-hidden>
                →
              </span>
              <div className="flex min-w-0 flex-1 flex-col gap-0.5 rounded-lg border border-status-success px-3 py-2">
                <span className="text-micro text-muted-foreground">新入口</span>
                <span className="font-mono text-ui-label font-semibold break-all">{model}</span>
              </div>
            </div>

            {plan.entries.map((entry) => (
              <div key={entry.path} className="app-plan-entry">
                <div className="app-plan-head">
                  <span className={planActionPillClass(entry.action)}>
                    {entry.action === 'create' ? '新建' : '合并'}
                  </span>
                  <span className="font-mono">{entry.path}</span>
                  <span className="text-muted-foreground">({entry.format})</span>
                </div>
                <pre className="app-code-block">{entry.after_preview}</pre>
              </div>
            ))}
          </div>
        )}

        {error && <div className="app-banner app-banner-err">{error}</div>}
      </div>
    </Modal>
  )
}
