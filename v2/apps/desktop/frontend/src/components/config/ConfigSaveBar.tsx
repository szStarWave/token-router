/**
 * ConfigSaveBar：配置页底部的保存状态栏（替代旧的「校验 / 应用 / 放弃」操作栏）。
 *
 * 自动保存下这里只做三件事：
 * - 如实显示状态（已保存 / 保存中 / 待修复 / 保存失败 / 待重启 / 冲突）；
 * - 提供必要的补救动作：立即保存、重试、还原到已保存、重启网关（仅待重启时）；
 * - 冲突时把"重叠字段"和两个出路（保留我的 / 用最新的）摆出来，不做静默覆盖。
 */
import { useState } from 'react'
import { Icon } from '../common/Icon'
import { useConfigDraft } from '../../hooks/useConfigDraft'
import { useAppStore } from '../../stores/appStore'
import { queryKeys } from '../../api/client'
import { queryClient } from '../../lib/query-client'
import { supervisorRestart } from '../../lib/tauri'

export function ConfigSaveBar() {
  const {
    draft,
    status,
    issueCount,
    serverIssues,
    saveError,
    restartPending,
    conflict,
    saveNow,
    retrySave,
    keepMine,
    takeTheirs,
    discard,
    clearRestartPending,
  } = useConfigDraft()
  const showToast = useAppStore((s) => s.showToast)
  const [confirmDiscard, setConfirmDiscard] = useState(false)
  const [restarting, setRestarting] = useState(false)

  if (!draft) return null

  if (conflict) {
    return (
      <section className="app-card" aria-label="配置冲突">
        <h2 className="app-card-title">
          <Icon name="warning" className="size-4 text-status-error" />
          配置保存冲突
        </h2>
        <p className="app-card-sub">{conflict.message}</p>
        {conflict.paths.length > 0 && (
          <ul className="oc-tile-inset mt-2 p-3 flex flex-col gap-1">
            {conflict.paths.slice(0, 20).map((path) => (
              <li key={path} className="text-meta font-mono">
                {path}
              </li>
            ))}
          </ul>
        )}
        <div className="app-card-actions">
          <button
            type="button"
            className="oc-btn-primary"
            onClick={() => void keepMine()}
          >
            保留我的（重叠字段取我的值）
          </button>
          <button
            type="button"
            className="oc-btn-default"
            onClick={() => void takeTheirs()}
          >
            用最新的（丢弃我的草稿）
          </button>
        </div>
      </section>
    )
  }

  return (
    <section
      className="app-card"
      aria-label="配置保存状态"
      title="修改会自动保存并热加载（不重启网关）。文本输入在失焦后提交；结构操作立即提交。"
    >
      <div className="app-card-actions">
        <span className="text-ui-label">
          {status === 'saved' && '✓ 已保存'}
          {status === 'pending' && '● 修改中…'}
          {status === 'saving' && '保存中…'}
          {status === 'invalid' &&
            `待修复：${issueCount + serverIssues.length} 个问题（修复后自动保存）`}
          {status === 'error' && `保存失败：${saveError ?? '未知错误'}`}
          {status === 'restart-required' && '✓ 已保存；监听/鉴权变化需重启网关生效'}
        </span>

        {status === 'pending' && (
          <button type="button" className="oc-btn-default oc-btn-sm" onClick={() => void saveNow()}>
            <Icon name="apply" />
            立即保存
          </button>
        )}

        {status === 'error' && (
          <button type="button" className="oc-btn-primary oc-btn-sm" onClick={() => void retrySave()}>
            <Icon name="refresh" />
            重试
          </button>
        )}

        {status === 'restart-required' && (
          <button
            type="button"
            className="oc-btn-primary oc-btn-sm"
            disabled={restarting}
            onClick={() => {
              setRestarting(true)
              void supervisorRestart()
                .then(async () => {
                  clearRestartPending()
                  await queryClient.invalidateQueries({ queryKey: queryKeys.status() })
                  showToast('网关已重启，新配置已生效')
                })
                .catch((error: unknown) => {
                  showToast(`重启网关失败：${String(error)}`, false)
                })
                .finally(() => setRestarting(false))
            }}
          >
            <Icon name="refresh" />
            {restarting ? '重启中…' : '重启网关'}
          </button>
        )}

        {status !== 'saved' && status !== 'saving' && !restartPending && (
          confirmDiscard ? (
            <span className="app-confirm">
              放弃未保存的修改，回到已保存的配置？
              <button
                type="button"
                className="oc-btn-destructive oc-btn-sm"
                onClick={() => {
                  setConfirmDiscard(false)
                  void discard()
                }}
              >
                确认放弃
              </button>
              <button
                type="button"
                className="oc-btn-default oc-btn-sm"
                onClick={() => setConfirmDiscard(false)}
              >
                取消
              </button>
            </span>
          ) : (
            <button
              type="button"
              className="oc-btn-default oc-btn-sm"
              disabled={status === 'pending'}
              onClick={() => setConfirmDiscard(true)}
            >
              还原到已保存
            </button>
          )
        )}
      </div>
    </section>
  )
}
