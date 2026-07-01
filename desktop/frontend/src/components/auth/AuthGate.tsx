import { useEffect, useState } from 'react'
import { hasPersistedSession, useAuthStore } from '../../stores/authStore'
import { LoginScreen } from './LoginScreen'
import { Outlet } from '@tanstack/react-router'
import { useBootstrap } from '../../hooks/useBootstrap'

export function AuthGate() {
  const isLoggedIn = useAuthStore((s) => s.isLoggedIn)
  const isSessionExpired = useAuthStore((s) => s.isSessionExpired)
  const logout = useAuthStore((s) => s.logout)
  const [hydrated, setHydrated] = useState(() => useAuthStore.persist.hasHydrated())
  const [showApp, setShowApp] = useState(() => hasPersistedSession())

  useEffect(() => {
    const unsub = useAuthStore.persist.onFinishHydration(() => setHydrated(true))
    return unsub
  }, [])

  useEffect(() => {
    if (isSessionExpired) {
      logout()
      location.reload()
    }
  }, [isSessionExpired, logout])

  useEffect(() => {
    if (isLoggedIn) setShowApp(true)
  }, [isLoggedIn])

  useBootstrap(hydrated && isLoggedIn && showApp)

  if (!hydrated) return null

  if (!isLoggedIn) {
    return <LoginScreen onComplete={() => setShowApp(true)} />
  }

  return <Outlet />
}
