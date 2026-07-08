import type { DriveStep } from 'driver.js'
import { isTauri } from './tauri'

/** Flip to true when onboarding is ready to ship. */
export const ONBOARDING_ENABLED = false

export const ONBOARDING_STORAGE_KEY = 'flowyRouterHasSeenOnboarding'

export const ONBOARDING_STEP_TARGETS: string[][] = [
  ['#edge-herdsman-model-list', '#upstream-edge-selected-model', '#nav-card-edge'],
  ['#cloud-flowy-model-list', '#nav-card-cloud'],
  ['#agent-quick-setup-card'],
  ['#stat-edge-pct', '#chart-edge'],
  [],
]

type TranslateFn = (key: string, vars?: Record<string, string | number>) => string

export function hasSeenOnboarding(): boolean {
  try {
    return localStorage.getItem(ONBOARDING_STORAGE_KEY) === 'true'
  } catch {
    return false
  }
}

export function markOnboardingSeen(): void {
  try {
    localStorage.setItem(ONBOARDING_STORAGE_KEY, 'true')
  } catch {
    /* noop */
  }
}

export function clearOnboardingSeen(): void {
  try {
    localStorage.removeItem(ONBOARDING_STORAGE_KEY)
  } catch {
    /* noop */
  }
}

export function queryTourTarget(selectors: string[]): HTMLElement | null {
  if (typeof document === 'undefined') return null
  for (const selector of selectors) {
    const target = document.querySelector<HTMLElement>(selector)
    if (!target) continue
    if (target.offsetParent !== null) {
      const rect = target.getBoundingClientRect()
      if (rect.width > 0 && rect.height > 0) return target
    }
  }
  return null
}

export function waitForTourTarget(selectors: string[], timeoutMs = 4000): Promise<HTMLElement | null> {
  return new Promise((resolve) => {
    const started = Date.now()
    const tick = () => {
      const target = queryTourTarget(selectors)
      if (target || Date.now() - started >= timeoutMs) {
        resolve(target)
        return
      }
      requestAnimationFrame(tick)
    }
    tick()
  })
}

export function isPostOtaDialogOpen(): boolean {
  return !!document.querySelector('#post-ota-dialog.open')
}

export function resolveTourElement(selectors: string[]): Element {
  return queryTourTarget(selectors) ?? document.getElementById('app') ?? document.body
}

export function buildOnboardingSteps(t: TranslateFn): DriveStep[] {
  const desktopApp = isTauri()
  const quickSetupDesc = desktopApp
    ? t('onboarding.stepQuickSetupDesc')
    : t('onboarding.stepQuickSetupDescWeb')

  return [
    {
      element: () => resolveTourElement(ONBOARDING_STEP_TARGETS[0]),
      popover: {
        title: t('onboarding.stepEdgeTitle'),
        description: t('onboarding.stepEdgeDesc'),
        side: 'bottom',
        align: 'start',
      },
    },
    {
      element: () => resolveTourElement(ONBOARDING_STEP_TARGETS[1]),
      popover: {
        title: t('onboarding.stepCloudTitle'),
        description: t('onboarding.stepCloudDesc'),
        side: 'bottom',
        align: 'start',
      },
    },
    {
      element: () => resolveTourElement(ONBOARDING_STEP_TARGETS[2]),
      popover: {
        title: t('onboarding.stepQuickSetupTitle'),
        description: quickSetupDesc,
        side: 'bottom',
        align: 'start',
      },
    },
    {
      element: () => resolveTourElement(ONBOARDING_STEP_TARGETS[3]),
      popover: {
        title: t('onboarding.stepStatsTitle'),
        description: t('onboarding.stepStatsDesc'),
        side: 'bottom',
        align: 'start',
      },
    },
    {
      popover: {
        title: t('onboarding.stepFinishTitle'),
        description: t('onboarding.stepFinishDesc'),
        align: 'center',
      },
    },
  ]
}
