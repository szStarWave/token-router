import { useI18n } from '../../hooks/useI18n'

interface OnboardingModalProps {
  open: boolean
  onSkip: () => void
  onStart: () => void
}

function RoutingIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
      <circle cx="6" cy="6" r="2.5" />
      <circle cx="18" cy="18" r="2.5" />
      <path d="M8.5 7.5l7 7" />
    </svg>
  )
}

function EdgeCloudIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
      <rect x="2" y="3" width="10" height="8" rx="1.5" />
      <path d="M5 11v2" />
      <path d="M9 11v2" />
      <path d="M14 16h6a2.5 2.5 0 000-5 3 3 0 00-5.5-1.5" />
    </svg>
  )
}

function QuickSetupIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
      <path d="M16 21v-2a4 4 0 00-4-4H6a4 4 0 00-4 4v2" />
      <circle cx="9" cy="7" r="3.5" />
      <path d="M22 21v-2a4 4 0 00-3-3.87" />
      <path d="M16 3.13a4 4 0 010 7.75" />
    </svg>
  )
}

function StatsIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
      <path d="M3 3v18h18" />
      <path d="M7 16l4-8 4 4 5-9" />
    </svg>
  )
}

const PREVIEW_STEPS = [
  { icon: RoutingIcon, labelKey: 'onboarding.previewRouting' },
  { icon: EdgeCloudIcon, labelKey: 'onboarding.previewEdgecloud' },
  { icon: QuickSetupIcon, labelKey: 'onboarding.previewAgent' },
  { icon: StatsIcon, labelKey: 'onboarding.previewStats' },
] as const

export function OnboardingModal({ open, onSkip, onStart }: OnboardingModalProps) {
  const { t } = useI18n()

  if (!open) return null

  return (
    <div className="security-dialog open onboarding-dialog" id="onboarding-dialog">
      <div className="security-panel onboarding-panel">
        <div className="onboarding-hero">
          <div className="onboarding-hero__icon" aria-hidden="true">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" width="28" height="28">
              <circle cx="12" cy="12" r="3" />
              <path d="M12 1v4" />
              <path d="M12 19v4" />
              <path d="M1 12h4" />
              <path d="M19 12h4" />
              <path d="M4.22 4.22l2.83 2.83" />
              <path d="M16.95 16.95l2.83 2.83" />
              <path d="M4.22 19.78l2.83-2.83" />
              <path d="M16.95 7.05l2.83-2.83" />
            </svg>
          </div>
          <div className="onboarding-hero__text">
            <h3>{t('onboarding.title')}</h3>
            <p className="onboarding-hero__lead">{t('onboarding.introLead')}</p>
          </div>
        </div>

        <div className="onboarding-steps-preview" aria-label={t('onboarding.title')}>
          {PREVIEW_STEPS.map((step) => {
            const Icon = step.icon
            return (
              <div key={step.labelKey} className="onboarding-step-chip">
                <span className="onboarding-step-chip__icon" aria-hidden="true">
                  <Icon />
                </span>
                <span className="onboarding-step-chip__label">{t(step.labelKey)}</span>
              </div>
            )
          })}
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
