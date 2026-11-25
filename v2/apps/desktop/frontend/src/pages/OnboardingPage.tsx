/**
 * Onboarding（§7.3 / §8）：Agent 接入 detect → plan(diff) → apply → verify。
 *
 * 流程：
 * 1. 选 Agent（+ 可选 WSL 发行版）→ detect 显示安装/配置状态；
 * 2. 填凭据引用（env:/file:）与模型 → plan 显示逐文件 diff 预览；
 * 3. 用户确认 → apply（备份 + 原子写；token 由 host 解析，不进 UI）；
 * 4. verify 读回校验。
 */
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useAppStore } from '../stores/appStore'
import { Icon } from '../components/common/Icon'
import { AGENTS, MANUAL_AGENTS } from '../constants/agents'
import { useGatewayUrl } from '../hooks/useGatewayUrl'
import { isTauri, invokeErrorMessage as errorMessage } from '../lib/tauri'
import { planActionPillClass } from '../lib/status'
import { defaultVirtualModelOf, virtualModelSummaries } from '../lib/config-edit'
import {
  onboardApply,
  onboardDetect,
  onboardPlan,
  onboardVerify,
  onboardWslDetect,
  type DetectReport,
  type PlanEntry,
  type SetupPlan,
  type VerifyReport,
  type WslReport,
} from '../lib/tauri'

type Step = 'select' | 'plan' | 'applied'

export function OnboardingPage() {
  const showToast = useAppStore((s) => s.showToast)
  const gateway = useGatewayUrl()
  const gatewayUrl = gateway.url

  const [wsl, setWsl] = useState<WslReport | null>(null)
  const [distro, setDistro] = useState<string>('')
  const [agentId, setAgentId] = useState<string>('claude-code')
  const [credentialRef, setCredentialRef] = useState('')
  const [model, setModel] = useState('')
  const [detect, setDetect] = useState<DetectReport | null>(null)
  const [plan, setPlan] = useState<SetupPlan | null>(null)
  const [verify, setVerify] = useState<VerifyReport | null>(null)
  const [step, setStep] = useState<Step>('select')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // 入口来自当前配置：默认值优先用 default_virtual_model，否则第一个启用入口。
  // 不再接受自由文本 "auto"，避免把不存在的模型写进 agent 配置。
  const models = useMemo(() => virtualModelSummaries(gateway.document), [gateway.document])
  const configuredDefault = defaultVirtualModelOf(gateway.document)
  // 用户没选过时取配置默认（其次第一个启用入口）；不让效果回写 state，避免级联渲染。
  const preferredModel =
    models.find((option) => option.id === configuredDefault && option.enabled)?.id ??
    models.find((option) => option.enabled)?.id ??
    ''
  // 配置刷新后旧选择可能已不存在：有下拉时回退到默认/第一个启用入口。
  const modelStillExists = models.length === 0 || models.some((option) => option.id === model)
  const effectiveModel = model !== '' && modelStillExists ? model : preferredModel

  useEffect(() => {
    if (isTauri()) {
      void onboardWslDetect().then(setWsl).catch(() => setWsl(null))
    }
  }, [])

  const runDetect = useCallback(async () => {
    if (!isTauri()) return
    setBusy(true)
    setError(null)
    try {
      setDetect(await onboardDetect({ agent: agentId, wsl_distro: distro || null }))
    } catch (err) {
      setError(errorMessage(err))
      setDetect(null)
    } finally {
      setBusy(false)
    }
  }, [agentId, distro])

  // Agent / 发行版变化时重新 detect。
  useEffect(() => {
    void runDetect()
  }, [runDetect])

  const runPlan = async () => {
    setBusy(true)
    setError(null)
    try {
      const setupPlan = await onboardPlan({
        agent: agentId,
        wsl_distro: distro || null,
        gateway_url: gatewayUrl,
        credential_ref: credentialRef,
        model: effectiveModel,
      })
      setPlan(setupPlan)
      setStep('plan')
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const runApply = async () => {
    setBusy(true)
    setError(null)
    try {
      await onboardApply({
        agent: agentId,
        wsl_distro: distro || null,
        gateway_url: gatewayUrl,
        credential_ref: credentialRef,
        model: effectiveModel,
      })
      const report = await onboardVerify({ agent: agentId, wsl_distro: distro || null })
      setVerify(report)
      setStep('applied')
      showToast('onboarding.applied')
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const reset = () => {
    setStep('select')
    setPlan(null)
    setVerify(null)
    setDetect(null)
    void runDetect()
  }

  if (!isTauri()) {
    return (
      <div className="app-page">
        <section className="app-card">
          <h2 className="app-card-title">Agent 接入</h2>
          <code className="app-inline-code">tr-cli admin</code>
        </section>
      </div>
    )
  }

  return (
    <div className="app-page">
      <section className="app-card" aria-label="接入目标">
        <h2 className="app-card-title">接入目标</h2>
        <div className="app-form-row">
          <label className="app-field">
            Agent
            <select
              className="app-select"
              value={agentId}
              onChange={(e) => {
                setAgentId(e.target.value)
                setStep('select')
                setVerify(null)
              }}
            >
              {AGENTS.map((agent) => (
                <option key={agent.id} value={agent.id}>
                  {agent.label}
                </option>
              ))}
            </select>
          </label>
          {wsl?.wsl_available && (
            <label className="app-field">
              目标
              <select
                className="app-select"
                value={distro}
                onChange={(e) => {
                  setDistro(e.target.value)
                  setStep('select')
                  setVerify(null)
                }}
              >
                <option value="">宿主 (Windows)</option>
                {wsl.distros.map((name) => (
                  <option key={name} value={name}>
                    WSL: {name}
                  </option>
                ))}
              </select>
            </label>
          )}
          {wsl && !wsl.wsl_available && (
            <span className="text-meta text-muted-foreground">
              WSL 不可用（{wsl.error ?? '未安装'}）；仅宿主接入。
            </span>
          )}
        </div>
        {detect && (
          <dl className="app-kv">
            <div className="app-kv-item">
              <dt className="app-kv-key">已安装</dt>
              <dd className="app-kv-value">{detect.installed ? '是' : '否（将创建新配置）'}</dd>
            </div>
            <div className="app-kv-item">
              <dt className="app-kv-key">已指向 token-router</dt>
              <dd className="app-kv-value">
                {detect.already_configured ? '是（幂等重跑）' : '否'}
              </dd>
            </div>
            <div className="app-kv-item">
              <dt className="app-kv-key">配置文件</dt>
              <dd className="app-kv-value font-mono">
                {detect.config_files.map((file) => file.path).join(', ')}
              </dd>
            </div>
          </dl>
        )}
      </section>

      {step === 'select' && (
        <section className="app-card" aria-label="接入参数">
          <h2 className="app-card-title">接入参数</h2>
          <div className="app-form-row">
            <label className="app-field">
              凭据引用
              <input
                type="text"
                className="oc-field app-field-light font-mono"
                value={credentialRef}
                placeholder="env:TR_TOKEN 或 file:admin.token"
                onChange={(e) => setCredentialRef(e.target.value)}
              />
            </label>
            <label className="app-field">
              模型入口
              {models.length > 0 ? (
                <select
                  className="app-select"
                  value={effectiveModel}
                  aria-label="模型入口"
                  onChange={(e) => setModel(e.target.value)}
                >
                  <option value="">选择入口…</option>
                  {models.map((option) => (
                    <option key={option.id} value={option.id} disabled={!option.enabled}>
                      {option.id}
                      {option.name ? `（${option.name}）` : ''}
                      {option.enabled ? '' : '（已停用）'}
                    </option>
                  ))}
                </select>
              ) : (
                <input
                  type="text"
                  className="oc-field app-field-light font-mono"
                  value={effectiveModel}
                  placeholder="配置里还没有入口，请先去路由组创建"
                  onChange={(e) => setModel(e.target.value)}
                />
              )}
            </label>
          </div>
          <button
            type="button"
            className="oc-btn-primary self-start"
            disabled={!credentialRef.trim() || effectiveModel.trim() === '' || busy}
            onClick={() => void runPlan()}
          >
            <Icon name="apply" />
            {busy ? '生成计划…' : '生成计划（预览 diff）'}
          </button>
        </section>
      )}

      {step === 'plan' && plan && (
        <section className="app-card" aria-label="计划预览">
          <h2 className="app-card-title">计划预览（{plan.entries.length} 个文件）</h2>
          {plan.entries.map((entry) => (
            <PlanEntryView key={entry.path} entry={entry} />
          ))}
          <div className="app-card-actions">
            <button
              type="button"
              className="oc-btn-primary"
              disabled={busy}
              onClick={() => void runApply()}
            >
              <Icon name="apply" />
              {busy ? '应用中…' : '确认应用（备份 + 原子写）'}
            </button>
            <button type="button" className="oc-btn-default" onClick={() => setStep('select')}>
              返回修改
            </button>
          </div>
        </section>
      )}

      {step === 'applied' && verify && (
        <section className="app-card" aria-label="应用结果">
          <h2 className="app-card-title">应用完成</h2>
          <p className="flex items-center gap-2 text-ui-label">
            校验结果：
            <span className={verify.ok ? 'app-pill app-pill-ok' : 'app-pill app-pill-error'}>
              {verify.ok ? '通过' : '失败'}
            </span>
          </p>
          <ul className="list-disc pl-5 text-meta text-muted-foreground">
            {verify.messages.map((message) => (
              <li key={message} className="font-mono">
                {message}
              </li>
            ))}
          </ul>
          <button type="button" className="oc-btn-default self-start" onClick={reset}>
            <Icon name="refresh" />
            接入下一个
          </button>
        </section>
      )}

      {error && <div className="app-banner app-banner-err">{error}</div>}

      <ManualAgents />
    </div>
  )
}

/**
 * 更多 agent（手动接入）：精确合并器覆盖 8 个；其余 agent 用统一环境变量接入。
 * base URL 从当前配置的 server.listen 推导，避免写死端口。
 */
function ManualAgents() {
  const gateway = useGatewayUrl()
  const baseUrl = `${gateway.url}/v1`

  return (
    <section className="app-card" aria-label="更多 agent（手动接入）">
      <h2 className="app-card-title">更多 agent（手动接入）</h2>
      <table className="app-table">
        <thead>
          <tr>
            <th>Agent</th>
            <th>接入方式</th>
          </tr>
        </thead>
        <tbody>
          {MANUAL_AGENTS.map((agent) => (
            <tr key={agent.id}>
              <td>{agent.label}</td>
              <td className="font-mono text-meta">
                {agent.env}={baseUrl}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  )
}

function PlanEntryView({ entry }: { entry: PlanEntry }) {
  const [expanded, setExpanded] = useState(false)
  return (
    <div className="app-plan-entry">
      <div className="app-plan-head">
        <span className={planActionPillClass(entry.action)}>
          {entry.action === 'create' ? '新建' : '合并'}
        </span>
        <span className="font-mono">{entry.path}</span>
        <span className="text-muted-foreground">({entry.format})</span>
        <button
          type="button"
          className="oc-btn-default oc-btn-sm app-plan-toggle"
          onClick={() => setExpanded(!expanded)}
        >
          <Icon name={expanded ? 'collapse' : 'expand'} />
          {expanded ? '收起' : '展开 diff'}
        </button>
      </div>
      {expanded && <pre className="app-code-block">{entry.after_preview}</pre>}
    </div>
  )
}
