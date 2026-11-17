/** 页面级 toast（服务端错误提示）。 */
import { useAppStore } from '../../stores/appStore'
import { useI18n } from '../../hooks/useI18n'

export function Toast() {
  const { t } = useI18n()
  const toasts = useAppStore((s) => s.toasts)
  const removeToast = useAppStore((s) => s.removeToast)
  const latest = toasts[toasts.length - 1]

  if (!latest) return <div className="app-toast" id="toast" />

  // 渲染后自动消失。
  setTimeout(() => removeToast(latest.id), 4000)

  const text = latest.key.includes(':')
    ? latest.key // 原始消息（携带数据，如 revision）直接显示
    : t(latest.key, latest.vars)
  return (
    <div
      className={
        latest.ok
          ? 'app-toast app-toast-show app-toast-ok'
          : 'app-toast app-toast-show app-toast-err'
      }
      id="toast"
      role="status"
    >
      {text}
    </div>
  )
}
