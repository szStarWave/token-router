import { useEffect, useMemo, useState } from 'react'
import { useAppStore } from '../stores/appStore'

/** Tick every second from the last known uptime anchor (or status snapshot). */
export function useLiveUptime(): number {
  const uptimeAnchor = useAppStore((s) => s.uptimeAnchor)
  const statusUptime = useAppStore((s) => s.status?.uptime_secs ?? 0)
  const [now, setNow] = useState(() => Date.now())

  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(id)
  }, [])

  return useMemo(() => {
    if (!uptimeAnchor) return statusUptime
    return uptimeAnchor.secs + Math.floor((now - uptimeAnchor.at) / 1000)
  }, [uptimeAnchor, statusUptime, now])
}
