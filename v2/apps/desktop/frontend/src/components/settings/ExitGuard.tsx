/**
 * 退出前询问（设置页开关）。
 *
 * 壳在窗口关闭请求里拦截并 emit `app://close-requested`；这里弹确认框：
 * 确认 → `app_force_close`（置确认标记后关窗），取消 → `app_cancel_exit`。
 * 放在 App 根部，任何页面都能用。
 */
import { useEffect, useState } from 'react'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { Modal } from '../common/Modal'
import {
  appCancelExit,
  appForceClose,
  appSetCloseToTray,
  appSetExitGuard,
  isTauri,
} from '../../lib/tauri'
import { draftAllowsClose } from '../../stores/configDraftStore'
import { flushConfigSave } from '../../stores/autosaveController'
import { usePrefsStore } from '../../stores/prefsStore'

export function ExitGuard() {
  const [asking, setAsking] = useState(false)
  const [dirtyDraft, setDirtyDraft] = useState(false)
  const confirmOnExit = usePrefsStore((s) => s.confirmOnExit)
  const closeToTray = usePrefsStore((s) => s.closeToTray)

  // 偏好变化时同步给壳（壳在 Rust 侧拦截关闭请求）。
  useEffect(() => {
    if (!isTauri()) return
    void appSetExitGuard(confirmOnExit).catch(() => undefined)
  }, [confirmOnExit])

  useEffect(() => {
    if (!isTauri()) return
    void appSetCloseToTray(closeToTray).catch(() => undefined)
  }, [closeToTray])

  useEffect(() => {
    if (!isTauri()) return
    let unlisten: UnlistenFn | null = null
    let cancelled = false
    void (async () => {
      unlisten = await listen('app://close-requested', () => {
        if (cancelled) return
        // 草稿守卫在 confirmOnExit 开启时不注册：由这里一并处理。
        // 先尝试自动保存（连同输入框内容），仍有未保存修改才在对话框里提示。
        void flushConfigSave({ force: true })
          .catch(() => undefined)
          .then(() => {
            if (cancelled) return
            setDirtyDraft(!draftAllowsClose())
            setAsking(true)
          })
      })
    })()
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [])

  if (!asking) return null

  return (
    <Modal
      title="退出 Token Router？"
      subtitle="退出会停掉本机托管的网关进程"
      onClose={() => {
        setAsking(false)
        void appCancelExit()
      }}
      footer={
        <>
          <button
            type="button"
            className="oc-btn-default"
            onClick={() => {
              setAsking(false)
              void appCancelExit()
            }}
          >
            取消
          </button>
          <button
            type="button"
            className="oc-btn-destructive"
            onClick={() => {
              setAsking(false)
              void appForceClose()
            }}
          >
            退出
          </button>
        </>
      }
    >
      <p className="text-ui-label">确定要退出吗？退出后网关会停止，在途请求会被打断。</p>
      {dirtyDraft && (
        <p className="text-ui-label text-status-warning">
          还有未保存的配置修改：退出会丢失这些修改（保存失败的修改不会写入本机配置）。
        </p>
      )}
    </Modal>
  )
}
