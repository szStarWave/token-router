import { useCallback, useEffect, useState } from 'react'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import {
  otaAppVersion,
  otaDoUpdate,
  otaDownloadUpdate,
  invokeErrorMessage,
  type OtaEventPayload,
} from '../../lib/tauri'
import { useI18n } from '../../hooks/useI18n'

interface OtaStatus {
  currentVersion: string
  latestVersion: string
  isUpToDate: boolean
}

function strField(data: Record<string, unknown> | undefined, key: string): string | undefined {
  const v = data?.[key]
  return typeof v === 'string' ? v : undefined
}

function numField(data: Record<string, unknown> | undefined, key: string): number | undefined {
  const v = data?.[key]
  return typeof v === 'number' && !Number.isNaN(v) ? v : undefined
}

export function OtaModal() {
  const { t } = useI18n()
  const [visible, setVisible] = useState(false)
  const [isChecking, setIsChecking] = useState(false)
  const [isDownloading, setIsDownloading] = useState(false)
  const [isApplying, setIsApplying] = useState(false)
  const [downloadProgress, setDownloadProgress] = useState(0)
  const [progressText, setProgressText] = useState('')
  const [updateStatus, setUpdateStatus] = useState<OtaStatus | null>(null)
  const [errorMessage, setErrorMessage] = useState('')

  const applyOtaEvent = useCallback(
    (ev: OtaEventPayload) => {
      const message = ev.message
      const data = (ev.data ?? {}) as Record<string, unknown>

      switch (message) {
        case 'ota.checking':
          setIsChecking(true)
          setErrorMessage('')
          break
        case 'ota.upToDate':
          setIsChecking(false)
          setErrorMessage('')
          setUpdateStatus({
            currentVersion: strField(data, 'current_version') ?? '',
            latestVersion: strField(data, 'remote_version') ?? '',
            isUpToDate: true,
          })
          setVisible(false)
          break
        case 'ota.newVersion':
          setIsChecking(false)
          setErrorMessage('')
          setUpdateStatus({
            currentVersion: strField(data, 'current_version') ?? '',
            latestVersion: strField(data, 'new_version') ?? '',
            isUpToDate: false,
          })
          setVisible(true)
          break
        case 'ota.checkFailed':
        case 'ota.compareFailed':
          setIsChecking(false)
          setErrorMessage(
            strField(data, 'error') ??
              t(message === 'ota.checkFailed' ? 'ota.checkFailed' : 'ota.compareFailed'),
          )
          setUpdateStatus(null)
          break
        case 'ota.downloadStarted':
          setIsDownloading(true)
          setDownloadProgress(0)
          setProgressText(t('ota.downloading'))
          break
        case 'ota.downloadProgress': {
          const pct = numField(data, 'progress') ?? 0
          setDownloadProgress(pct)
          setProgressText(t('ota.downloadProgress', { percent: pct }))
          break
        }
        case 'ota.downloadComplete':
          setIsDownloading(false)
          setDownloadProgress(100)
          setProgressText(t('ota.downloadComplete'))
          break
        case 'ota.downloadFailed':
          setIsDownloading(false)
          setIsApplying(false)
          setErrorMessage(strField(data, 'error') ?? t('ota.downloadFailed'))
          break
        case 'ota.updateApplyStarted':
          setIsApplying(true)
          setProgressText(t('ota.installing'))
          break
        case 'ota.updateApplyFailed':
          setIsApplying(false)
          setErrorMessage(strField(data, 'error') ?? t('ota.installFailed'))
          break
        default:
          break
      }
    },
    [t],
  )

  useEffect(() => {
    let unlisten: UnlistenFn | undefined
    void listen<OtaEventPayload>('ota:event', (event) => {
      applyOtaEvent(event.payload)
    }).then((fn) => {
      unlisten = fn
    })
    return () => {
      void unlisten?.()
    }
  }, [applyOtaEvent])

  const downloadUpdate = async () => {
    setErrorMessage('')
    setIsDownloading(true)
    try {
      await otaDownloadUpdate()
      setIsApplying(true)
      setProgressText(t('ota.installing'))
      await otaDoUpdate()
    } catch (err) {
      setIsDownloading(false)
      setIsApplying(false)
      setErrorMessage(invokeErrorMessage(err))
    }
  }

  if (!visible) return null

  return (
    <div className="security-dialog open ota-dialog" id="ota-dialog">
      <div className="security-panel ota-panel">
        <h3>{t('ota.modalTitle')}</h3>
        <div className="ota-modal-body">
          {isChecking && (
            <div className="ota-loading">
              <span className="ota-spinner" aria-hidden="true" />
              <span>{t('ota.checking')}</span>
            </div>
          )}
          {!isChecking && errorMessage && (
            <div className="ota-error">
              <p>{errorMessage}</p>
            </div>
          )}
          {!isChecking && !errorMessage && updateStatus && (
            <div className="ota-status">
              <div className="ota-version-info">
                <div className="ota-version-item">
                  <span className="ota-version-label">{t('ota.currentVersion')}</span>
                  <span className="ota-version-value">{updateStatus.currentVersion}</span>
                </div>
                {!updateStatus.isUpToDate && (
                  <div className="ota-version-item highlight">
                    <span className="ota-version-label">{t('ota.newVersion')}</span>
                    <span className="ota-version-value new">{updateStatus.latestVersion}</span>
                  </div>
                )}
              </div>
              {!updateStatus.isUpToDate && (
                <div className="ota-actions">
                  {!isDownloading && !isApplying && (
                    <button type="button" className="btn btn-primary" onClick={() => void downloadUpdate()}>
                      {t('ota.install')}
                    </button>
                  )}
                  {(isDownloading || isApplying) && (
                    <div className="ota-progress-container">
                      <div className="ota-progress-bar">
                        <div className="ota-progress-fill" style={{ width: `${downloadProgress}%` }} />
                      </div>
                      <span className="ota-progress-text">{progressText}</span>
                    </div>
                  )}
                </div>
              )}
              {updateStatus.isUpToDate && <p className="ota-up-to-date">{t('ota.upToDate')}</p>}
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

export function useOtaAppVersion() {
  const [version, setVersion] = useState('—')
  useEffect(() => {
    void otaAppVersion()
      .then(setVersion)
      .catch(() => setVersion('—'))
  }, [])
  return version
}
