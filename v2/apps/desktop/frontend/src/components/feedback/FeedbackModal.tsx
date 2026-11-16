import { useCallback, useEffect, useState } from 'react'
import { feedbackAppVersion, feedbackSubmit, invokeErrorMessage } from '../../lib/tauri'
import { pickNickname, pickUserId } from '../../lib/user-info'
import { useAppStore } from '../../stores/appStore'
import { useAuthStore } from '../../stores/authStore'
import { useI18n } from '../../hooks/useI18n'

interface FeedbackModalProps {
  open: boolean
  onClose: () => void
}

export function FeedbackModal({ open, onClose }: FeedbackModalProps) {
  const { t } = useI18n()
  const showToast = useAppStore((s) => s.showToast)
  const userInfo = useAuthStore((s) => s.userInfo)
  const [text, setText] = useState('')
  const [version, setVersion] = useState('—')
  const [submitting, setSubmitting] = useState(false)

  useEffect(() => {
    if (!open) return
    setText('')
    void feedbackAppVersion()
      .then((v) => setVersion(v || 'unknown'))
      .catch(() => setVersion('unknown'))
  }, [open])

  const submit = useCallback(async () => {
    const body = text.trim()
    if (!body) {
      showToast('titlebar.feedbackEmpty', false)
      return
    }
    setSubmitting(true)
    try {
      const userId = pickUserId(userInfo) || null
      const userNickname = pickNickname(userInfo) || null
      await feedbackSubmit(body, undefined, userId, userNickname)
      showToast('titlebar.feedbackSuccess', true)
      onClose()
    } catch (err) {
      showToast('toast.feedbackFailed', false, { msg: invokeErrorMessage(err) })
    } finally {
      setSubmitting(false)
    }
  }, [text, showToast, onClose, userInfo])

  if (!open) return null

  return (
    <div className="security-dialog open" id="feedback-dialog">
      <div className="security-panel feedback-panel">
        <h3>{t('titlebar.feedbackModalTitle')}</h3>
        <p className="feedback-version">
          {t('titlebar.feedbackVersion')}: <span>{version}</span>
        </p>
        <label className="feedback-label" htmlFor="feedback-input">
          {t('titlebar.feedbackContentLabel')}
        </label>
        <textarea
          id="feedback-input"
          className="feedback-textarea"
          placeholder={t('titlebar.feedbackPlaceholder')}
          maxLength={3000}
          rows={8}
          value={text}
          onChange={(e) => setText(e.target.value)}
        />
        <p className="feedback-contact">
          {t('titlebar.feedbackContactHint')}{' '}
          <a href="mailto:friend@starwaveai.com">friend@starwaveai.com</a>
        </p>
        <div className="security-actions">
          <button type="button" className="btn btn-ghost" disabled={submitting} onClick={onClose}>
            {t('action.cancel')}
          </button>
          <button type="button" className="btn btn-primary" disabled={submitting} onClick={() => void submit()}>
            {t('titlebar.feedbackSubmit')}
          </button>
        </div>
      </div>
    </div>
  )
}
