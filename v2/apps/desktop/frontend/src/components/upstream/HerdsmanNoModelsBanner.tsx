import { useI18n } from '../../hooks/useI18n'
import { openHerdsmanOrInstall } from '../../lib/edge-upstream'

function HerdsmanIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <rect x="3" y="4" width="18" height="12" rx="2" />
      <path d="M8 20h8" />
      <path d="M12 16v4" />
    </svg>
  )
}

export function HerdsmanNoModelsBanner() {
  const { t } = useI18n()

  return (
    <div className="edge-herdsman-banner" id="edge-herdsman-no-models">
      <div className="edge-herdsman-banner-icon">
        <HerdsmanIcon />
      </div>
      <div className="edge-herdsman-banner-main">
        <p className="edge-herdsman-banner-hint">{t('edgeModel.herdsmanNoRunningHint')}</p>
        <div className="edge-herdsman-actions">
          <button
            type="button"
            className="btn btn-primary btn-sm"
            onClick={() => void openHerdsmanOrInstall()}
          >
            {t('edgeModel.herdsmanOpenApp')}
          </button>
        </div>
      </div>
    </div>
  )
}
