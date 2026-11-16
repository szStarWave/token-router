/**
 * Onboarding（§7.3 / §8）：Agent 接入 detect → plan(diff) → apply → verify。
 *
 * 流程：
 * 1. 选 Agent（+ 可选 WSL 发行版）→ detect 显示安装/配置状态；
 * 2. 填凭据引用（env:/file:）与模型 → plan 显示逐文件 diff 预览；
 * 3. 用户确认 → apply（备份 + 原子写；token 由 host 解析，不进 UI）；
 * 4. verify 读回校验。
 */
import { useCallback, useEffect, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { api } from '../api/client'
import { useAppStore } from '../stores/appStore'
import { isTauri, invokeErrorMessage as errorMessage } from '../lib/tauri'
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

const AGENTS = [
  { id: 'openclaw', label: 'OpenClaw' },
  { id: 'hermes', label: 'Hermes' },
  { id: 'hermes-flash', label: 'Hermes Flash' },
  { id: 'claude-code', label: 'Claude Code' },
  { id: 'codex', label: 'Codex' },
  { id: 'opencode', label: 'OpenCode' },
  { id: 'codebuddy', label: 'CodeBuddy' },
  { id: 'workbuddy', label: 'WorkBuddy' },
] as const

type Step = 'select' | 'plan' | 'applied'

export function OnboardingPage() {
  const showToast = useAppStore((s) => s.showToast)
  const configQuery = useQuery({
    queryKey: ['admin', 'config'],
    queryFn: () => api.config(),
    retry: 1,
  })

  const [wsl, setWsl] = useState<WslReport | null>(null)
  const [distro, setDistro] = useState<string>('')
  const [agentId, setAgentId] = useState<string>('claude-code')
  const [credentialRef, setCredentialRef] = useState('')
  const [model, setModel] = useState('auto')
  const [detect, setDetect] = useState<DetectReport | null>(null)
  const [plan, setPlan] = useState<SetupPlan | null>(null)
  const [verify, setVerify] = useState<VerifyReport | null>(null)
  const [step, setStep] = useState<Step>('select')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // 默认模型：status 的 default_virtual_model。
  const statusQuery = useQuery({
    queryKey: ['admin', 'status'],
    queryFn: () => api.status(),
    retry: 1,
    refetchInterval: 10_000,
  })
  useEffect(() => {
    if (statusQuery.data?.default_virtual_model) {
      setModel((current) => (current === 'auto' ? statusQuery.data.default_virtual_model : current))
    }
  }, [statusQuery.data?.default_virtual_model])

  useEffect(() => {
    if (isTauri()) {
      void onboardWslDetect().then(setWsl).catch(() => setWsl(null))
    }
  }, [])

  const gatewayUrl = configQuery.data
    ? `http://${(configQuery.data.document as Record<string, Record<string, unknown>>)?.server?.listen ?? '127.0.0.1:17621'}`
    : 'http://127.0.0.1:17621'

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
        model,
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
        model,
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
      <div className="page">
        <section className="card">
          <h2>Agent 接入</h2>
          <p className="text-dim">
            一键接入（detect → plan → apply → verify）需要桌面壳的 host 能力；
            浏览器模式请用 <code>tr-cli admin</code> 手动配置。
          </p>
        </section>
      </div>
    )
  }

  return (
    <div className="page">
      <section className="card" aria-label="接入目标">
        <h2>接入目标</h2>
        <div className="form-row">
          <label>
            Agent{' '}
            <select value={agentId} onChange={(e) => { setAgentId(e.target.value); setStep('select'); setVerify(null) }}>
              {AGENTS.map((agent) => (
                <option key={agent.id} value={agent.id}>{agent.label}</option>
              ))}
            </select>
          </label>
          {wsl?.wsl_available && (
            <label>
              目标{' '}
              <select value={distro} onChange={(e) => { setDistro(e.target.value); setStep('select'); setVerify(null) }}>
                <option value="">宿主 (Windows)</option>
                {wsl.distros.map((name) => (
                  <option key={name} value={name}>WSL: {name}</option>
                ))}
              </select>
            </label>
          )}
          {wsl && !wsl.wsl_available && (
            <span className="text-dim">WSL 不可用（{wsl.error ?? '未安装'}）；仅宿主接入。</span>
          )}
        </div>
        {detect && (
          <dl className="kv-grid">
            <div><dt>已安装</dt><dd>{detect.installed ? '是' : '否（将创建新配置）'}</dd></div>
            <div><dt>已指向 token-router</dt><dd>{detect.already_configured ? '是（幂等重跑）' : '否'}</dd></div>
            <div>
              <dt>配置文件</dt>
              <dd className="mono">{detect.config_files.map((file) => file.path).join(', ')}</dd>
            </div>
          </dl>
        )}
      </section>

      {step === 'select' && (
        <section className="card" aria-label="接入参数">
          <h2>接入参数</h2>
          <div className="form-row">
            <label>
              凭据引用{' '}
              <input
                type="text"
                className="mono"
                value={credentialRef}
                placeholder="env:TR_TOKEN 或 file:admin.token"
                onChange={(e) => setCredentialRef(e.target.value)}
              />
            </label>
            <label>
              模型{' '}
              <input type="text" className="mono" value={model} onChange={(e) => setModel(e.target.value)} />
            </label>
          </div>
          <p className="text-dim">
            网关：<code className="mono">{gatewayUrl}</code>（来自当前配置）。
            明文 token 不经过 UI：host 在 apply 时从 <code>env:</code>/<code>file:</code> 解析。
          </p>
          <button
            type="button"
            className="btn btn-primary"
            disabled={!credentialRef.trim() || busy}
            onClick={() => void runPlan()}
          >
            {busy ? '生成计划…' : '生成计划（预览 diff）'}
          </button>
        </section>
      )}

      {step === 'plan' && plan && (
        <section className="card" aria-label="计划预览">
          <h2>计划预览（{plan.entries.length} 个文件）</h2>
          {plan.entries.map((entry) => (
            <PlanEntryView key={entry.path} entry={entry} />
          ))}
          <div className="card-actions">
            <button type="button" className="btn btn-primary" disabled={busy} onClick={() => void runApply()}>
              {busy ? '应用中…' : '确认应用（备份 + 原子写）'}
            </button>
            <button type="button" className="btn" onClick={() => setStep('select')}>
              返回修改
            </button>
          </div>
        </section>
      )}

      {step === 'applied' && verify && (
        <section className="card" aria-label="应用结果">
          <h2>应用完成</h2>
          <p>
            校验结果：
            <span className={`pill pill-${verify.ok ? 'closed' : 'open'}`}>
              {verify.ok ? '通过' : '失败'}
            </span>
          </p>
          <ul>
            {verify.messages.map((message) => (
              <li key={message} className="mono text-dim">{message}</li>
            ))}
          </ul>
          <button type="button" className="btn" onClick={reset}>
            接入下一个
          </button>
        </section>
      )}

      {error && <div className="banner banner-err">{error}</div>}
    </div>
  )
}

function PlanEntryView({ entry }: { entry: PlanEntry }) {
  const [expanded, setExpanded] = useState(false)
  return (
    <div className="plan-entry">
      <div className="plan-entry-head">
        <span className={`pill pill-${entry.action === 'create' ? 'half_open' : 'closed'}`}>
          {entry.action === 'create' ? '新建' : '合并'}
        </span>
        <span className="mono">{entry.path}</span>
        <span className="text-dim">({entry.format})</span>
        <button type="button" className="btn btn-sm" onClick={() => setExpanded(!expanded)}>
          {expanded ? '收起' : '展开 diff'}
        </button>
      </div>
      {expanded && (
        <pre className="pre-json">{entry.after_preview}</pre>
      )}
    </div>
  )
}
