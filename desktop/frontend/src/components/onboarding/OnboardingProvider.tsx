import 'driver.js/dist/driver.css'
import { OnboardingContextProvider } from './OnboardingContext'
import { OnboardingModal } from './OnboardingModal'
import { useOnboardingTour } from '../../hooks/useOnboardingTour'
import { ONBOARDING_ENABLED } from '../../lib/onboarding'

export function OnboardingProvider({ children }: { children: React.ReactNode }) {
  const { showIntro, startTour, skipTour, restartTour } = useOnboardingTour()

  return (
    <OnboardingContextProvider value={{ restartTour }}>
      {children}
      <OnboardingModal
        open={ONBOARDING_ENABLED && showIntro}
        onSkip={skipTour}
        onStart={() => void startTour()}
      />
    </OnboardingContextProvider>
  )
}
