import { useCallback, useEffect, useRef, useState } from 'react'
import { useI18n } from '../../hooks/useI18n'
import { HERDSMAN_INSTALL_URL, openHerdsmanOrInstall, startHerdsman } from '../../lib/edge-upstream'
import { useEdgeStore } from '../../stores/edgeStore'

const START_TIMEOUT_MS = 60_000

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

function DownloadIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M12 3v12" />
      <path d="M7 10l5 5 5-5" />
      <path d="M5 21h14" />
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
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const hint = installed ? t('edgeModel.herdsmanInstalledHint') : t('edgeModel.herdsmanNotInstalledHint')

  const clearStartTimeout = useCallback(() => {
    if (timeoutRef.current) {
      clearTimeout(timeoutRef.current)
      timeoutRef.current = null
    }
  }, [])

  useEffect(() => {
    if (starting && herdsmanConnected) {
      setStarting(false)
      clearStartTimeout()
    }
  }, [starting, herdsmanConnected, clearStartTimeout])

  useEffect(() => () => clearStartTimeout(), [clearStartTimeout])

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
            <>
              <button type="button" className="btn btn-primary btn-sm" onClick={() => void openHerdsmanOrInstall()}>
                <DownloadIcon />
                {t('herdsman.download')}
              </button>
              <a
                className="btn btn-ghost btn-sm edge-herdsman-link"
                href={HERDSMAN_INSTALL_URL}
                target="_blank"
                rel="noreferrer"
              >
                {t('edgeModel.herdsmanDownloadLink')}
                <ExternalIcon />
              </a>
            </>
          )}
        </div>
      </div>
    </div>
  )
}
