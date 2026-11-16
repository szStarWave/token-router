import { createContext, useContext, type ReactNode } from 'react'

export interface OnboardingContextValue {
  restartTour: () => void
}

const OnboardingContext = createContext<OnboardingContextValue | null>(null)

export function OnboardingContextProvider({
  value,
  children,
}: {
  value: OnboardingContextValue
  children: ReactNode
}) {
  return <OnboardingContext.Provider value={value}>{children}</OnboardingContext.Provider>
}

export function useOnboardingContext(): OnboardingContextValue {
  const ctx = useContext(OnboardingContext)
  if (!ctx) {
    throw new Error('useOnboardingContext must be used within OnboardingProvider')
  }
  return ctx
}
