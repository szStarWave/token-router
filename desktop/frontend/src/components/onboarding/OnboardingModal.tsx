import { useI18n } from '../../hooks/useI18n'

interface OnboardingModalProps {
  open: boolean
  onSkip: () => void
  onStart: () => void
}

const PREVIEW_STEPS = [
  { emoji: '💻', labelKey: 'onboarding.previewEdge' },
  { emoji: '☁️', labelKey: 'onboarding.previewCloud' },
  { emoji: '⚡', labelKey: 'onboarding.previewAgent' },
  { emoji: '📊', labelKey: 'onboarding.previewStats' },
] as const

export function OnboardingModal({ open, onSkip, onStart }: OnboardingModalProps) {
  const { t } = useI18n()

  if (!open) return null

  return (
    <div className="security-dialog open onboarding-dialog" id="onboarding-dialog">
      <div className="security-panel onboarding-panel">
        <div className="onboarding-hero">
          <div className="onboarding-hero__icon" aria-hidden="true">
            🚀
          </div>
          <div className="onboarding-hero__text">
            <h3>{t('onboarding.title')}</h3>
            <p className="onboarding-hero__lead">{t('onboarding.introLead')}</p>
          </div>
        </div>

        <div className="onboarding-steps-preview" aria-label={t('onboarding.title')}>
          {PREVIEW_STEPS.map((step) => (
            <div key={step.labelKey} className="onboarding-step-chip">
              <span className="onboarding-step-chip__emoji" aria-hidden="true">
                {step.emoji}
              </span>
              <span className="onboarding-step-chip__label">{t(step.labelKey)}</span>
            </div>
          ))}
        </div>

        <ul className="onboarding-modal__list">
          <li>{t('onboarding.introItem1')}</li>
          <li>{t('onboarding.introItem2')}</li>
          <li>{t('onboarding.introItem3')}</li>
        </ul>

        <div className="security-actions onboarding-actions">
          <button type="button" className="btn btn-ghost" onClick={onSkip}>
            {t('onboarding.skip')}
          </button>
          <button type="button" className="btn btn-primary onboarding-start-btn" onClick={onStart}>
            {t('onboarding.start')}
          </button>
        </div>
      </div>
    </div>
  )
}
