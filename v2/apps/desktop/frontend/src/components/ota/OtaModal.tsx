import { useCallback, useEffect, useState } from 'react'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import {
  otaAppVersion,
  otaDoUpdate,
  otaDownloadUpdate,
  invokeErrorMessage,
  type OtaEventPayload,
} from '../../lib/tauri'
import { formatBytes, formatSpeed } from '../../lib/format-bytes'
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

function resetDownloadProgress() {
  return {
    downloadProgress: 0,
    totalBytes: 0,
  }
}

export function OtaModal() {
  const { t } = useI18n()
  const [visible, setVisible] = useState(false)
  const [isChecking, setIsChecking] = useState(false)
  const [isDownloading, setIsDownloading] = useState(false)
  const [isApplying, setIsApplying] = useState(false)
  const [downloadProgress, setDownloadProgress] = useState(0)
  const [totalBytes, setTotalBytes] = useState(0)
  const [progressDetail, setProgressDetail] = useState('')
  const [progressSpeed, setProgressSpeed] = useState('')
  const [updateStatus, setUpdateStatus] = useState<OtaStatus | null>(null)
  const [errorMessage, setErrorMessage] = useState('')

  const updateDownloadProgressUi = useCallback(
    (downloaded: number, total: number, pct: number, speed: number) => {
      setDownloadProgress(pct >= 0 ? pct : 0)
      setTotalBytes(total)

      const downloadedLabel = formatBytes(downloaded)
      const speedLabel = formatSpeed(speed)
      setProgressSpeed(speedLabel)

      if (total > 0) {
        setProgressDetail(
          t('ota.downloadProgressDetail', {
            downloaded: downloadedLabel,
            total: formatBytes(total),
            percent: pct >= 0 ? pct : 0,
          }),
        )
      } else {
        setProgressDetail(
          t('ota.downloadProgressUnknownTotal', {
            downloaded: downloadedLabel,
            speed: speedLabel,
          }),
        )
      }
    },
    [t],
  )

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
        case 'ota.downloadStarted': {
          const total = numField(data, 'total') ?? 0
          setIsDownloading(true)
          setIsApplying(false)
          setDownloadProgress(0)
          setTotalBytes(total)
          setProgressDetail(t('ota.downloading'))
          setProgressSpeed('')
          break
        }
        case 'ota.downloadProgress': {
          const pct = numField(data, 'progress') ?? 0
          const downloaded = numField(data, 'downloaded') ?? 0
          const total = numField(data, 'total') ?? 0
          const speed = numField(data, 'speed_bps') ?? 0
          updateDownloadProgressUi(downloaded, total, pct, speed)
          break
        }
        case 'ota.downloadComplete':
          setIsDownloading(false)
          setDownloadProgress(100)
          setProgressDetail(t('ota.downloadComplete'))
          break
        case 'ota.downloadFailed':
          setIsDownloading(false)
          setIsApplying(false)
          setErrorMessage(strField(data, 'error') ?? t('ota.downloadFailed'))
          break
        case 'ota.updateApplyStarted':
          setIsDownloading(false)
          setIsApplying(true)
          setProgressDetail(t('ota.installing'))
          setProgressSpeed('')
          break
        case 'ota.updateApplyFailed':
          setIsApplying(false)
          setErrorMessage(strField(data, 'error') ?? t('ota.installFailed'))
          break
        default:
          break
      }
    },
    [t, updateDownloadProgressUi],
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
    setIsApplying(false)
    const reset = resetDownloadProgress()
    setDownloadProgress(reset.downloadProgress)
    setTotalBytes(reset.totalBytes)
    setProgressDetail(t('ota.downloading'))
    setProgressSpeed('')
    try {
      await otaDownloadUpdate()
      setIsApplying(true)
      setProgressDetail(t('ota.installing'))
      setProgressSpeed('')
      await otaDoUpdate()
    } catch (err) {
      setIsDownloading(false)
      setIsApplying(false)
      setErrorMessage(invokeErrorMessage(err))
    }
  }

  const showIndeterminateProgress = isApplying || (isDownloading && totalBytes === 0)

  if (!visible) return null

  const showUpdateActions =
    !isChecking &&
    !errorMessage &&
    updateStatus &&
    !updateStatus.isUpToDate &&
    !isDownloading &&
    !isApplying

  return (
    <div className="security-dialog open ota-dialog" id="ota-dialog">
      <div className="security-panel ota-panel">
        <h3>{t('ota.modalTitle')}</h3>

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
          <>
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

            {(isDownloading || isApplying) && (
              <div className="ota-progress-container">
                <div className="ota-progress-bar">
                  <div
                    className={`ota-progress-fill${showIndeterminateProgress ? ' indeterminate' : ''}`}
                    style={showIndeterminateProgress ? undefined : { width: `${downloadProgress}%` }}
                  />
                </div>
                <span className="ota-progress-detail">{progressDetail}</span>
                {!isApplying && totalBytes > 0 && progressSpeed !== '—' && (
                  <span className="ota-progress-speed">{progressSpeed}</span>
                )}
              </div>
            )}

            {updateStatus.isUpToDate && <p className="ota-up-to-date">{t('ota.upToDate')}</p>}
          </>
        )}

        {showUpdateActions && (
          <div className="security-actions">
            <button type="button" className="btn btn-primary" onClick={() => void downloadUpdate()}>
              {t('ota.install')}
            </button>
          </div>
        )}
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
