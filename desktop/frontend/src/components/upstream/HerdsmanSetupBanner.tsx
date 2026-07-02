import { useCallback, useEffect, useRef, useState } from 'react'
import { useI18n } from '../../hooks/useI18n'
import { HERDSMAN_INSTALL_URL, refreshHerdsmanStatus, startHerdsman } from '../../lib/edge-upstream'
import { openExternalUrl } from '../../lib/open-external'
import { useEdgeStore } from '../../stores/edgeStore'

const START_TIMEOUT_MS = 60_000
const INSTALL_POLL_INTERVAL_MS = 15_000
const INSTALL_POLL_TIMEOUT_MS = 10 * 60_000

function HerdsmanIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <rect x="3" y="4" width="18" height="12" rx="2" />
      <path d="M8 20h8" />
      <path d="M12 16v4" />
    </svg>
  )
}

function PlayIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <polygon points="8,5 19,12 8,19" />
    </svg>
  )
}

function ExternalIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6" />
      <path d="M15 3h6v6" />
      <path d="M10 14L21 3" />
    </svg>
  )
}

function BtnSpinner() {
  return <span className="btn-spinner" aria-hidden="true" />
}

export function HerdsmanSetupBanner({ installed }: { installed: boolean }) {
  const { t } = useI18n()
  const herdsmanConnected = useEdgeStore((s) => s.herdsmanConnected)
  const [starting, setStarting] = useState(false)
  const [detectingInstall, setDetectingInstall] = useState(false)
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const pollIntervalRef = useRef<ReturnType<typeof setInterval> | null>(null)
  const pollTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const hint = installed ? t('edgeModel.herdsmanInstalledHint') : t('edgeModel.herdsmanNotInstalledHint')

  const clearStartTimeout = useCallback(() => {
    if (timeoutRef.current) {
      clearTimeout(timeoutRef.current)
      timeoutRef.current = null
    }
  }, [])

  const clearInstallPoll = useCallback(() => {
    if (pollIntervalRef.current) {
      clearInterval(pollIntervalRef.current)
      pollIntervalRef.current = null
    }
    if (pollTimeoutRef.current) {
      clearTimeout(pollTimeoutRef.current)
      pollTimeoutRef.current = null
    }
  }, [])

  useEffect(() => {
    if (starting && herdsmanConnected) {
      setStarting(false)
      clearStartTimeout()
    }
  }, [starting, herdsmanConnected, clearStartTimeout])

  useEffect(() => {
    if (detectingInstall && installed) {
      setDetectingInstall(false)
      clearInstallPoll()
    }
  }, [detectingInstall, installed, clearInstallPoll])

  useEffect(() => () => {
    clearStartTimeout()
    clearInstallPoll()
  }, [clearStartTimeout, clearInstallPoll])

  const handleStart = async () => {
    if (starting) return
    setStarting(true)
    clearStartTimeout()
    timeoutRef.current = setTimeout(() => setStarting(false), START_TIMEOUT_MS)
    try {
      await startHerdsman()
    } catch {
      setStarting(false)
      clearStartTimeout()
    }
  }

  const handleDownload = () => {
    if (detectingInstall) return
    void openExternalUrl(HERDSMAN_INSTALL_URL)
    setDetectingInstall(true)
    clearInstallPoll()
    void refreshHerdsmanStatus()
    pollIntervalRef.current = setInterval(() => {
      void refreshHerdsmanStatus()
    }, INSTALL_POLL_INTERVAL_MS)
    pollTimeoutRef.current = setTimeout(() => {
      setDetectingInstall(false)
      clearInstallPoll()
    }, INSTALL_POLL_TIMEOUT_MS)
  }

  return (
    <div className="edge-herdsman-banner" id="edge-herdsman-setup">
      <div className="edge-herdsman-banner-icon">
        <HerdsmanIcon />
      </div>
      <div className="edge-herdsman-banner-main">
        <p className="edge-herdsman-banner-hint">{hint}</p>
        <div className="edge-herdsman-actions">
          {installed ? (
            <button
              type="button"
              className="btn btn-primary btn-sm"
              disabled={starting}
              aria-busy={starting}
              onClick={() => void handleStart()}
            >
              {starting ? <BtnSpinner /> : <PlayIcon />}
              {starting ? t('herdsman.starting') : t('herdsman.start')}
            </button>
          ) : (
            <button
              type="button"
              className="btn btn-primary btn-sm edge-herdsman-link"
              disabled={detectingInstall}
              aria-busy={detectingInstall}
              onClick={handleDownload}
            >
              {detectingInstall ? <BtnSpinner /> : <ExternalIcon />}
              <span>{detectingInstall ? t('herdsman.detectingInstall') : t('edgeModel.herdsmanDownloadLink')}</span>
            </button>
          )}
        </div>
      </div>
    </div>
  )
}
