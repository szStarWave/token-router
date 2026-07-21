import { useCallback, useEffect, useRef, useState } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { driver, type Driver } from 'driver.js'
import { useAppStore } from '../stores/appStore'
import { useI18n } from './useI18n'
import {
  buildOnboardingSteps,
  hasSeenOnboarding,
  isPostOtaDialogOpen,
  markOnboardingSeen,
  ONBOARDING_ENABLED,
  ONBOARDING_STEP_TARGETS,
  waitForTourTarget,
} from '../lib/onboarding'
import { useOnboardingDemo } from '../lib/onboarding-demo'

async function syncTourStep(index: number, navigate: ReturnType<typeof useNavigate>) {
  switch (index) {
    case 0:
    case 1:
    case 2:
      await navigate({ to: '/overview' })
      break
    case 3:
      await navigate({ to: '/upstream/$navId', params: { navId: 'edge' } })
      break
    case 4:
    case 5:
      await navigate({ to: '/upstream/$navId', params: { navId: 'cloud' } })
      break
    case 6:
      await navigate({ to: '/overview' })
      break
    case 7:
      await navigate({ to: '/routing' })
      break
    case 8:
      await navigate({ to: '/logs' })
      break
    case 9:
    case 10:
      await navigate({ to: '/stats' })
      break
    default:
      break
  }

  const targets = ONBOARDING_STEP_TARGETS[index]
  if (targets.length > 0) {
    await waitForTourTarget(targets)
  }
}

export function useOnboardingTour() {
  const { t } = useI18n()
  const navigate = useNavigate()
  const connected = useAppStore((s) => s.connected)
  const [showIntro, setShowIntro] = useState(false)
  const [tourActive, setTourActive] = useState(false)
  const driverRef = useRef<Driver | null>(null)
  const pendingIntroRef = useRef(false)

  const destroyDriver = useCallback(() => {
    driverRef.current?.destroy()
    driverRef.current = null
    setTourActive(false)
  }, [])

  const finishTour = useCallback(() => {
    destroyDriver()
    markOnboardingSeen()
    setShowIntro(false)
    useOnboardingDemo.getState().clear()
  }, [destroyDriver])

  const createDriverInstance = useCallback(() => {
    destroyDriver()
    const driverObj = driver({
      showProgress: true,
      allowClose: true,
      overlayOpacity: 0.62,
      stagePadding: 10,
      stageRadius: 10,
      popoverClass: 'flowy-onboarding-popover',
      popoverOffset: 12,
      nextBtnText: t('onboarding.next'),
      prevBtnText: t('onboarding.prev'),
      doneBtnText: t('onboarding.done'),
      progressText: t('onboarding.progress'),
      steps: buildOnboardingSteps(t),
      onCloseClick: () => finishTour(),
      onDoneClick: () => finishTour(),
      onDestroyed: () => {
        driverRef.current = null
        setTourActive(false)
        useOnboardingDemo.getState().clear()
      },
      onNextClick: (_element, _step, { driver: d, state }) => {
        const nextIndex = (state.activeIndex ?? 0) + 1
        void syncTourStep(nextIndex, navigate).then(() => {
          d.moveNext()
        })
      },
      onPrevClick: (_element, _step, { driver: d, state }) => {
        const prevIndex = Math.max(0, (state.activeIndex ?? 0) - 1)
        void syncTourStep(prevIndex, navigate).then(() => {
          d.movePrevious()
        })
      },
      onHighlightStarted: (_element, _step, { state }) => {
        void syncTourStep(state.activeIndex ?? 0, navigate)
      },
    })
    driverRef.current = driverObj
    return driverObj
  }, [destroyDriver, finishTour, navigate, t])

  const startTour = useCallback(async () => {
    setShowIntro(false)
    setTourActive(true)
    useOnboardingDemo.getState().enable()
    await syncTourStep(0, navigate)
    const driverObj = createDriverInstance()
    driverObj.drive(0)
  }, [createDriverInstance, navigate])

  const skipTour = useCallback(() => {
    destroyDriver()
    markOnboardingSeen()
    setShowIntro(false)
    useOnboardingDemo.getState().clear()
  }, [destroyDriver])

  const restartTour = useCallback(() => {
    destroyDriver()
    setShowIntro(true)
  }, [destroyDriver])

  const scheduleIntroIfNeeded = useCallback(() => {
    if (!ONBOARDING_ENABLED || hasSeenOnboarding() || !connected) return
    if (isPostOtaDialogOpen()) {
      pendingIntroRef.current = true
      return
    }
    setShowIntro(true)
    pendingIntroRef.current = false
  }, [connected])

  useEffect(() => {
    if (!ONBOARDING_ENABLED || !connected || hasSeenOnboarding()) return

    const timer = window.setTimeout(() => {
      scheduleIntroIfNeeded()
    }, 500)

    return () => window.clearTimeout(timer)
  }, [connected, scheduleIntroIfNeeded])

  useEffect(() => {
    if (!ONBOARDING_ENABLED || !pendingIntroRef.current || !connected || hasSeenOnboarding()) return

    const poll = window.setInterval(() => {
      if (hasSeenOnboarding()) {
        pendingIntroRef.current = false
        window.clearInterval(poll)
        return
      }
      if (!isPostOtaDialogOpen()) {
        pendingIntroRef.current = false
        window.clearInterval(poll)
        setShowIntro(true)
      }
    }, 500)

    return () => window.clearInterval(poll)
  }, [connected, showIntro, tourActive])

  useEffect(() => () => destroyDriver(), [destroyDriver])

  return {
    showIntro,
    tourActive,
    startTour,
    skipTour,
    restartTour,
    finishTour,
  }
}
