/**
 * Settings（§7.3）：服务启停/重启、监听/鉴权编辑（validate+apply）、应用信息、外观。
 */
import { useEffect, useState } from 'react'
import {
  useStatusQuery,
  useConfigQuery,
  useConfigValidateMutation,
  useConfigApplyMutation,
} from '../queries/admin'
import { QueryState, errorMessage } from '../components/common/QueryState'
import { useAppStore } from '../stores/appStore'
import { statusPillClass } from '../lib/status'
import {
  supervisorStart,
  supervisorRestart,
  supervisorStop,
  supervisorStatus,
  hostAppInfo,
  isTauri,
  invokeErrorMessage,
  type SupervisorStatus,
  type AppInfo,
} from '../lib/tauri'
import { draftFromDocument, draftToToml } from '../lib/config-edit'

const THEME_OPTIONS = [
  { id: 'light', label: '浅色' },
  { id: 'dark', label: '深色' },
  { id: 'system', label: '跟随系统' },
] as const

export function SettingsPage() {
  const statusQuery = useStatusQuery()
  const configQuery = useConfigQuery()
  const validateMutation = useConfigValidateMutation()
  const applyMutation = useConfigApplyMutation()
  const showToast = useAppStore((s) => s.showToast)
  const themePref = useAppStore((s) => s.themePref)
  const setThemePref = useAppStore((s) => s.setThemePref)

  const [supervisor, setSupervisor] = useState<SupervisorStatus | null>(null)
  const [info, setInfo] = useState<AppInfo | null>(null)
  const [busy, setBusy] = useState(false)
  const [listen, setListen] = useState<string | null>(null)
  const [validRevision, setValidRevision] = useState<string | null>(null)
  const [confirmingRestart, setConfirmingRestart] = useState(false)

  useEffect(() => {
    if (!isTauri()) return
    void supervisorStatus().then(setSupervisor).catch(() => setSupervisor(null))
    void hostAppInfo().then(setInfo).catch(() => setInfo(null))
  }, [])

  const draft = configQuery.data && listen === null ? draftFromDocument(configQuery.data) : null
  const effectiveListen = listen ?? draft?.server.listen ?? ''

  const applyServer = async () => {
    if (!configQuery.data || !draft) return
    const toml = draftToToml({
      ...draft,
      server: { ...draft.server, listen: effectiveListen },
    })
    try {
      const response = await validateMutation.mutateAsync(toml)
      if (!response.valid) {
        showToast('config.invalid', false)
        return
      }
      const applied = await applyMutation.mutateAsync({
        revision: response.revision ?? configQuery.data.revision,
        documentToml: toml,
      })
      showToast(`config.applied:${applied.revision.slice(0, 12)}|需重启生效`)
      setValidRevision(applied.revision)
    } catch (error) {
      showToast(errorMessage(error), false)
    }
  }

  const restart = async () => {
    setBusy(true)
    try {
      await supervisorRestart()
      setSupervisor(await supervisorStatus())
      showToast('service.restarted')
      setConfirmingRestart(false)
    } catch (error) {
      showToast(invokeErrorMessage(error), false)
    } finally {
      setBusy(false)
    }
  }

  const stop = async () => {
    setBusy(true)
    try {
      await supervisorStop()
      setSupervisor(await supervisorStatus())
    } catch (error) {
      showToast(invokeErrorMessage(error), false)
    } finally {
      setBusy(false)
    }
  }

  const start = async () => {
    setBusy(true)
    try {
      await supervisorStart()
      setSupervisor(await supervisorStatus())
    } catch (error) {
      showToast(invokeErrorMessage(error), false)
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="app-page">
      <section className="app-card" aria-label="服务">
        <h2 className="app-card-title">服务</h2>
        {isTauri() && supervisor ? (
          <>
            <p className="flex items-center gap-2 text-ui-label">
              状态：
              <span className={statusPillClass(supervisor.state)}>{supervisor.state}</span>
              {supervisor.listen && (
                <>
                  · <span className="font-mono">{supervisor.listen}</span>
                </>
              )}
            </p>
            <div className="app-card-actions">
              {supervisor.state === 'ready' ? (
                <>
                  {confirmingRestart ? (
                    <span className="app-confirm">
                      重启会打断在途请求，确认？
                      <button
                        type="button"
                        className="oc-btn-primary"
                        disabled={busy}
                        onClick={() => void restart()}
                      >
                        确认重启
                      </button>
                      <button
                        type="button"
                        className="oc-btn-default"
                        onClick={() => setConfirmingRestart(false)}
                      >
                        取消
                      </button>
                    </span>
                  ) : (
                    <button
                      type="button"
                      className="oc-btn-default"
                      onClick={() => setConfirmingRestart(true)}
                    >
                      重启
                    </button>
                  )}
                  <button
                    type="button"
                    className="oc-btn-destructive"
                    disabled={busy}
                    onClick={() => void stop()}
                  >
                    停止
                  </button>
                </>
              ) : (
                <button
                  type="button"
                  className="oc-btn-primary"
                  disabled={busy}
                  onClick={() => void start()}
                >
                  {busy ? '启动中…' : '启动'}
                </button>
              )}
            </div>
          </>
        ) : (
          <p className="app-card-sub">
            浏览器模式：请用命令行管理服务（<code className="app-inline-code">tr-cli admin</code>）。
          </p>
        )}
        {validRevision && (
          <div className="app-banner app-banner-warn">
            配置已应用但尚未重启；重启后生效（digest 以 /status 为准）。
          </div>
        )}
      </section>

      <QueryState
        loading={statusQuery.isLoading}
        error={statusQuery.error}
        onRetry={() => void statusQuery.refetch()}
      >
        {statusQuery.data && (
          <section className="app-card" aria-label="监听与鉴权">
            <h2 className="app-card-title">监听与鉴权</h2>
            <div className="app-form-row">
              <label className="app-field">
                监听地址
                <input
                  className="oc-field app-field-light font-mono"
                  value={effectiveListen}
                  onChange={(e) => setListen(e.target.value)}
                  aria-label="监听地址"
                />
              </label>
              <label className="app-field">
                鉴权模式
                <select
                  className="app-select"
                  value={draft?.server.auth_mode ?? 'token_store'}
                  disabled
                  aria-label="鉴权模式（P4 固定 token_store）"
                >
                  <option value="token_store">token_store</option>
                </select>
              </label>
            </div>
            <button
              type="button"
              className="oc-btn-default self-start"
              disabled={effectiveListen === draft?.server.listen || validateMutation.isPending}
              onClick={() => void applyServer()}
            >
              {validateMutation.isPending ? '校验中…' : '校验并应用'}
            </button>
            <p className="app-card-sub">
              管理 API 默认只监听 loopback；非 loopback 需要显式开启
              <code className="app-inline-code"> allow_remote_admin</code>（P4 暂不提供开关，保持安全默认）。
            </p>
          </section>
        )}
      </QueryState>

      {info && (
        <section className="app-card" aria-label="应用信息">
          <h2 className="app-card-title">应用信息</h2>
          <dl className="app-kv">
            <div className="app-kv-item">
              <dt className="app-kv-key">壳版本</dt>
              <dd className="app-kv-value">{info.version}</dd>
            </div>
            <div className="app-kv-item">
              <dt className="app-kv-key">平台</dt>
              <dd className="app-kv-value">{info.platform}</dd>
            </div>
            <div className="app-kv-item">
              <dt className="app-kv-key">数据目录</dt>
              <dd className="app-kv-value font-mono">{info.data_dir}</dd>
            </div>
            <div className="app-kv-item">
              <dt className="app-kv-key">服务版本</dt>
              <dd className="app-kv-value">{statusQuery.data?.version ?? '—'}</dd>
            </div>
          </dl>
        </section>
      )}

      <section className="app-card" aria-label="外观">
        <h2 className="app-card-title">外观</h2>
        <div className="oc-label mb-2">主题</div>
        <div className="oc-seg" role="group" aria-label="主题">
          {THEME_OPTIONS.map((option) => (
            <button
              key={option.id}
              type="button"
              className={themePref === option.id ? 'oc-seg-item-active' : 'oc-seg-item'}
              aria-pressed={themePref === option.id}
              onClick={() => setThemePref(option.id)}
            >
              {option.label}
            </button>
          ))}
        </div>
        <p className="app-card-sub">
          深浅两套配色都按 OpenChamber 规范定义；跟随系统时随系统外观切换。
        </p>
      </section>
    </div>
  )
}
