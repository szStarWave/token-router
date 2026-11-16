/**
 * Settings（§7.3）：服务启停/重启、监听/鉴权编辑（validate+apply）、应用信息。
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

export function SettingsPage() {
  const statusQuery = useStatusQuery()
  const configQuery = useConfigQuery()
  const validateMutation = useConfigValidateMutation()
  const applyMutation = useConfigApplyMutation()
  const showToast = useAppStore((s) => s.showToast)

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
    <div className="page">
      <section className="card" aria-label="服务">
        <h2>服务</h2>
        {isTauri() && supervisor ? (
          <>
            <p>
              状态：<span className={`pill pill-${supervisor.state === 'ready' ? 'closed' : 'open'}`}>{supervisor.state}</span>
              {supervisor.listen && <> · <span className="mono">{supervisor.listen}</span></>}
            </p>
            <div className="card-actions">
              {supervisor.state === 'ready' ? (
                <>
                  {confirmingRestart ? (
                    <span className="confirm-inline">
                      重启会打断在途请求，确认？
                      <button type="button" className="btn btn-primary" disabled={busy} onClick={() => void restart()}>
                        确认重启
                      </button>
                      <button type="button" className="btn" onClick={() => setConfirmingRestart(false)}>
                        取消
                      </button>
                    </span>
                  ) : (
                    <button type="button" className="btn" onClick={() => setConfirmingRestart(true)}>
                      重启
                    </button>
                  )}
                  <button type="button" className="btn btn-danger" disabled={busy} onClick={() => void stop()}>
                    停止
                  </button>
                </>
              ) : (
                <button type="button" className="btn btn-primary" disabled={busy} onClick={() => void start()}>
                  {busy ? '启动中…' : '启动'}
                </button>
              )}
            </div>
          </>
        ) : (
          <p className="text-dim">浏览器模式：请用命令行管理服务（<code>tr-cli admin</code>）。</p>
        )}
        {validRevision && (
          <div className="banner banner-warn">
            配置已应用但尚未重启；重启后生效（digest 以 /status 为准）。
          </div>
        )}
      </section>

      <QueryState loading={statusQuery.isLoading} error={statusQuery.error} onRetry={() => void statusQuery.refetch()}>
        {statusQuery.data && (
          <section className="card" aria-label="监听与鉴权">
            <h2>监听与鉴权</h2>
            <div className="form-row">
              <label>
                监听地址{' '}
                <input
                  className="mono"
                  value={effectiveListen}
                  onChange={(e) => setListen(e.target.value)}
                  aria-label="监听地址"
                />
              </label>
              <label>
                鉴权模式{' '}
                <select
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
              className="btn"
              disabled={effectiveListen === draft?.server.listen || validateMutation.isPending}
              onClick={() => void applyServer()}
            >
              {validateMutation.isPending ? '校验中…' : '校验并应用'}
            </button>
            <p className="text-dim">
              管理 API 默认只监听 loopback；非 loopback 需要显式开启
              <code> allow_remote_admin</code>（P4 暂不提供开关，保持安全默认）。
            </p>
          </section>
        )}
      </QueryState>

      {info && (
        <section className="card" aria-label="应用信息">
          <h2>应用信息</h2>
          <dl className="kv-grid">
            <div><dt>壳版本</dt><dd>{info.version}</dd></div>
            <div><dt>平台</dt><dd>{info.platform}</dd></div>
            <div><dt>数据目录</dt><dd className="mono">{info.data_dir}</dd></div>
            <div><dt>服务版本</dt><dd>{statusQuery.data?.version ?? '—'}</dd></div>
          </dl>
        </section>
      )}
    </div>
  )
}
