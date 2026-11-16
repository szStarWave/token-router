import type { DriveStep } from 'driver.js'
import { isTauri, getUiState, isUiStateLoaded, setUiHasSeenOnboarding } from './ui-state'

/** Flip to true when onboarding is ready to ship. */
export const ONBOARDING_ENABLED = true

export const ONBOARDING_STORAGE_KEY = 'flowyRouterHasSeenOnboarding'

export const ONBOARDING_STEP_TARGETS: string[][] = [
  ['#route-tabs'],
  ['#card-grid'],
  ['#gw-stat-listen', '#gw-stat-state', '#nav-card-overview'],
  ['#edge-herdsman-model-list', '#upstream-edge-selected-model', '#nav-card-edge'],
  ['#cloud-flowy-model-list', '#upstream-cloud-selected-model', '#nav-card-cloud'],
  ['#cloud_token_budget_slider', '#upstream-cloud-budget', '#cloud-quota-fields'],
  ['#agent-quick-setup-card'],
  ['#route', '#routing_mode', '#nav-card-routing'],
  ['#log-view', '#nav-card-logs'],
  ['#stat-saved', '#stat-edge-pct', '#chart-edge', '#nav-card-stats'],
  [],
]

type TranslateFn = (key: string, vars?: Record<string, string | number>) => string

export function hasSeenOnboarding(): boolean {
  try {
    if (isTauri() && isUiStateLoaded()) {
      return getUiState().hasSeenOnboarding
    }
    return localStorage.getItem(ONBOARDING_STORAGE_KEY) === 'true'
  } catch {
    return false
  }
}

export function markOnboardingSeen(): void {
  try {
    if (isTauri() && isUiStateLoaded()) {
      setUiHasSeenOnboarding(true)
    } else {
      localStorage.setItem(ONBOARDING_STORAGE_KEY, 'true')
    }
  } catch {
    /* noop */
  }
}

export function clearOnboardingSeen(): void {
  try {
    if (isTauri() && isUiStateLoaded()) {
      setUiHasSeenOnboarding(false)
    } else {
      localStorage.removeItem(ONBOARDING_STORAGE_KEY)
    }
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
        title: t('onboarding.stepRouteTabsTitle'),
        description: t('onboarding.stepRouteTabsDesc'),
        side: 'bottom',
        align: 'start',
      },
    },
    {
      element: () => resolveTourElement(ONBOARDING_STEP_TARGETS[1]),
      popover: {
        title: t('onboarding.stepNavTitle'),
        description: t('onboarding.stepNavDesc'),
        side: 'bottom',
        align: 'start',
      },
    },
    {
      element: () => resolveTourElement(ONBOARDING_STEP_TARGETS[2]),
      popover: {
        title: t('onboarding.stepGatewayTitle'),
        description: t('onboarding.stepGatewayDesc'),
        side: 'bottom',
        align: 'start',
      },
    },
    {
      element: () => resolveTourElement(ONBOARDING_STEP_TARGETS[3]),
      popover: {
        title: t('onboarding.stepEdgeTitle'),
        description: t('onboarding.stepEdgeDesc'),
        side: 'bottom',
        align: 'start',
      },
    },
    {
      element: () => resolveTourElement(ONBOARDING_STEP_TARGETS[4]),
      popover: {
        title: t('onboarding.stepCloudTitle'),
        description: t('onboarding.stepCloudDesc'),
        side: 'bottom',
        align: 'start',
      },
    },
    {
      element: () => resolveTourElement(ONBOARDING_STEP_TARGETS[5]),
      popover: {
        title: t('onboarding.stepBudgetTitle'),
        description: t('onboarding.stepBudgetDesc'),
        side: 'bottom',
        align: 'start',
      },
    },
    {
      element: () => resolveTourElement(ONBOARDING_STEP_TARGETS[6]),
      popover: {
        title: t('onboarding.stepQuickSetupTitle'),
        description: quickSetupDesc,
        side: 'bottom',
        align: 'start',
      },
    },
    {
      element: () => resolveTourElement(ONBOARDING_STEP_TARGETS[7]),
      popover: {
        title: t('onboarding.stepRoutingTitle'),
        description: t('onboarding.stepRoutingDesc'),
        side: 'bottom',
        align: 'start',
      },
    },
    {
      element: () => resolveTourElement(ONBOARDING_STEP_TARGETS[8]),
      popover: {
        title: t('onboarding.stepLogsTitle'),
        description: t('onboarding.stepLogsDesc'),
        side: 'bottom',
        align: 'start',
      },
    },
    {
      element: () => resolveTourElement(ONBOARDING_STEP_TARGETS[9]),
      popover: {
        title: t('onboarding.stepStatsSavedTitle'),
        description: t('onboarding.stepStatsSavedDesc'),
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
