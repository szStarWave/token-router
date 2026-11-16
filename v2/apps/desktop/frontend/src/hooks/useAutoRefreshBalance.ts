import { useCallback, useEffect, useRef } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { dailyCheckIn } from '../lib/flowy/api'
import { getAuthToken, useAuthStore } from '../stores/authStore'
import { queryKeys } from '../queries/keys'

const SCENE_INTERVAL_MS = {
  mount: 0,
  focus: 15 * 1000,
  polling: 10 * 60 * 1000,
} as const
type RefreshScene = keyof typeof SCENE_INTERVAL_MS

const SCENES = Object.keys(SCENE_INTERVAL_MS) as RefreshScene[]

const createSceneTriggerState = () =>
  Object.fromEntries(SCENES.map((scene) => [scene, 0])) as Record<RefreshScene, number>

let globalLastTriggerByScene = createSceneTriggerState()

function getTodayKey() {
  const today = new Date()
  return parseInt(
    `${today.getFullYear()}${String(today.getMonth() + 1).padStart(2, '0')}${String(today.getDate()).padStart(2, '0')}`,
    10,
  )
}

/** Auto refresh credits balance and trigger daily check-in (aligned with FlowyClaw). */
export function useAutoRefreshBalance() {
  const queryClient = useQueryClient()
  const isLoggedIn = useAuthStore((s) => s.isLoggedIn)
  const lastCheckInDayKeyRef = useRef(0)
  const isCheckingInRef = useRef(false)

  const refreshCredits = useCallback(() => {
    const token = getAuthToken()
    if (!token) return
    void queryClient.invalidateQueries({ queryKey: queryKeys.flowyCredits })
    void queryClient.invalidateQueries({ queryKey: queryKeys.flowyUsage })
  }, [queryClient])

  const checkIn = useCallback(async () => {
    const token = getAuthToken()
    if (!token || isCheckingInRef.current) return

    const todayKey = getTodayKey()
    if (lastCheckInDayKeyRef.current >= todayKey) return

    isCheckingInRef.current = true
    try {
      const data = await dailyCheckIn(token)
      if (typeof data.dayKey === 'number' && data.dayKey > 0) {
        lastCheckInDayKeyRef.current = data.dayKey
      } else {
        lastCheckInDayKeyRef.current = todayKey
      }
      refreshCredits()
    } catch (error) {
      console.warn('[credits/checkin]', error)
    } finally {
      isCheckingInRef.current = false
    }
  }, [refreshCredits])

  const triggerBalance = useCallback(
    (scene: RefreshScene) => {
      if (!getAuthToken()) return

      const now = Date.now()
      const lastTrigger = globalLastTriggerByScene[scene]
      if (now - lastTrigger < SCENE_INTERVAL_MS[scene]) return

      globalLastTriggerByScene[scene] = now
      refreshCredits()
    },
    [refreshCredits],
  )

  useEffect(() => {
    if (!isLoggedIn) {
      globalLastTriggerByScene = createSceneTriggerState()
      lastCheckInDayKeyRef.current = 0
    }
  }, [isLoggedIn])

  useEffect(() => {
    if (!isLoggedIn) return

    const onMount = () => {
      triggerBalance('mount')
      void checkIn()
    }
    onMount()

    const onFocus = () => {
      triggerBalance('focus')
      void checkIn()
    }
    window.addEventListener('focus', onFocus)

    const interval = setInterval(() => {
      triggerBalance('polling')
      void checkIn()
    }, SCENE_INTERVAL_MS.polling)

    return () => {
      window.removeEventListener('focus', onFocus)
      clearInterval(interval)
    }
  }, [checkIn, isLoggedIn, triggerBalance])
}
