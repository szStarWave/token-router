import { useCallback, useEffect, useState } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { isTauri } from '../../lib/tauri'
import { useI18n } from '../../hooks/useI18n'
import { FeedbackModal } from '../feedback/FeedbackModal'

interface TitleBarProps {
  prefix?: string
  className?: string
}

export function TitleBar({ prefix = '', className = 'window-controls' }: TitleBarProps) {
  const { t } = useI18n()
  const [maximized, setMaximized] = useState(false)
  const [feedbackOpen, setFeedbackOpen] = useState(false)
  const minId = prefix ? `${prefix}win-min` : 'btn-win-min'
  const maxId = prefix ? `${prefix}win-max` : 'btn-win-max'
  const closeId = prefix ? `${prefix}win-close` : 'btn-win-close'
  const feedbackId = prefix ? `${prefix}win-feedback` : 'btn-win-feedback'
  const iconMaxId = prefix ? `${prefix}icon-win-max` : 'icon-win-max'
  const iconRestoreId = prefix ? `${prefix}icon-win-restore` : 'icon-win-restore'

  const refreshMax = useCallback(async () => {
    if (!isTauri()) return
    try {
      const win = getCurrentWindow()
      setMaximized(await win.isMaximized())
    } catch {
      /* ignore */
    }
  }, [])

  useEffect(() => {
    if (!isTauri()) return
    void refreshMax()
    const win = getCurrentWindow()
    const unlisten = win.onResized(() => void refreshMax())
    return () => {
      void unlisten.then((fn) => fn())
    }
  }, [refreshMax])

  if (!isTauri()) return null

  const win = getCurrentWindow()

  return (
    <>
      <button
        className="titlebar-feedback-btn"
        id={feedbackId}
        type="button"
        aria-label={t('titlebar.feedback')}
        onClick={() => setFeedbackOpen(true)}
      >
        {t('titlebar.feedback')}
      </button>
      <div className={className} id={prefix ? undefined : 'window-controls'}>
        <button
          className="window-ctrl"
          id={minId}
          type="button"
          aria-label={t('window.minimize')}
          title={t('window.minimize')}
          onClick={() => void win.minimize()}
        >
          <span className="window-ctrl-icon">
            <svg viewBox="0 0 12 12" fill="none" aria-hidden="true">
              <path d="M2 6h8" stroke="currentColor" strokeWidth="1.25" strokeLinecap="round" />
            </svg>
          </span>
        </button>
        <button
          className="window-ctrl"
          id={maxId}
          type="button"
          aria-label={maximized ? t('window.restore') : t('window.maximize')}
          title={maximized ? t('window.restore') : t('window.maximize')}
          onClick={() => void win.toggleMaximize().then(refreshMax)}
        >
          <span className="window-ctrl-icon">
            {!maximized && (
              <svg id={iconMaxId} viewBox="0 0 12 12" fill="none" aria-hidden="true">
                <rect x="1.75" y="1.75" width="8.5" height="8.5" stroke="currentColor" strokeWidth="1.25" />
              </svg>
            )}
            {maximized && (
              <svg id={iconRestoreId} viewBox="0 0 12 12" fill="none" aria-hidden="true">
                <rect x="3.25" y="0.75" width="7.5" height="7.5" stroke="currentColor" strokeWidth="1.25" />
                <rect x="0.75" y="3.25" width="7.5" height="7.5" stroke="currentColor" strokeWidth="1.25" />
              </svg>
            )}
          </span>
        </button>
        <button
          className="window-ctrl close"
          id={closeId}
          type="button"
          aria-label={t('window.close')}
          title={t('window.close')}
          onClick={() => void win.close()}
        >
          <span className="window-ctrl-icon">
            <svg viewBox="0 0 12 12" fill="none" aria-hidden="true">
              <path d="M2.5 2.5l7 7M9.5 2.5l-7 7" stroke="currentColor" strokeWidth="1.25" strokeLinecap="round" />
            </svg>
          </span>
        </button>
      </div>
      <FeedbackModal open={feedbackOpen} onClose={() => setFeedbackOpen(false)} />
    </>
  )
}
