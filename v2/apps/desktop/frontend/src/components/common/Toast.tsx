import { useAppStore } from '../../stores/appStore'
import { useI18n } from '../../hooks/useI18n'

export function Toast() {
  const { t } = useI18n()
  const toasts = useAppStore((s) => s.toasts)
  const latest = toasts[toasts.length - 1]
  if (!latest) return <div className="toast" id="toast" />

  return (
    <div className={`toast show ${latest.ok ? 'ok' : 'err'}`} id="toast">
      {t(latest.key, latest.vars)}
    </div>
  )
}

export function LoginToast({ message, ok }: { message: string; ok?: boolean }) {
  if (!message) return <div id="login-toast" className="login-toast" />
  return (
    <div id="login-toast" className={`login-toast show ${ok ? 'ok' : 'err'}`}>
      {message}
    </div>
  )
}
