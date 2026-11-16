import { useEffect, useState } from 'react'
import { otaGetPostRestartNotice, type PostOtaRestartNotice } from '../../lib/tauri'
import { useI18n } from '../../hooks/useI18n'
import { useAppStore } from '../../stores/appStore'

export function PostOtaNoticeModal() {
  const { t } = useI18n()
  const locale = useAppStore((s) => s.locale)
  const [notice, setNotice] = useState<PostOtaRestartNotice | null>(null)

  useEffect(() => {
    void otaGetPostRestartNotice()
      .then((n) => {
        if (n?.show) setNotice(n)
      })
      .catch(() => {
        /* ignore */
      })
  }, [])

  if (!notice?.show) return null

  const langKey = locale === 'zh' ? 'zh-CN' : 'en-US'
  const notes = notice.release_notes?.[langKey] ?? notice.release_notes?.['en-US'] ?? []

  return (
    <div className="security-dialog open post-ota-dialog" id="post-ota-dialog">
      <div className="security-panel post-ota-panel">
        <h3>{t('ota.postOtaUpdateTitle')}</h3>
        <p>{t('ota.restartedNewVersion', { version: notice.version })}</p>
        {notes.length > 0 && (
          <div className="post-ota-notes">
            <div className="post-ota-notes-title">{t('ota.releaseNotesTitle')}</div>
            <ul>
              {notes.map((item) => (
                <li key={item}>{item}</li>
              ))}
            </ul>
          </div>
        )}
        <div className="security-actions">
          <button type="button" className="btn btn-primary" onClick={() => setNotice(null)}>
            {t('action.confirm')}
          </button>
        </div>
      </div>
    </div>
  )
}
