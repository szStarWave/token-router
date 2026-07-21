import { useAuthStore } from '../stores/authStore'
import { useAppStore } from '../stores/appStore'
import type { GatewayStatus, UpstreamSetupUpdate } from '../types/gateway'

function adminHeaders(): Record<string, string> {
  return { 'Content-Type': 'application/json' }
}

export function getGatewayBase(): string {
  return useAppStore.getState().gatewayBase.replace(/\/$/, '')
}

export async function apiFetch<T = unknown>(
  path: string,
  opts: RequestInit = {},
): Promise<T> {
  const url = getGatewayBase() + path
  const res = await fetch(url, {
    ...opts,
    headers: { ...adminHeaders(), ...(opts.headers as Record<string, string>) },
  })
  const json = (await res.json().catch(() => ({}))) as T & { error?: string }
  if (!res.ok) throw new Error(json.error || res.statusText)
  return json
}

export function normalizeClientGatewayBase(url: string): string {
  const trimmed = url.trim().replace(/\/$/, '')
  if (!trimmed) return trimmed
  try {
    const u = new URL(trimmed)
    return `${u.protocol}//${u.host}`
  } catch {
    return trimmed
  }
}

export function generateGatewayAuthKey(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(24))
  const hex = [...bytes].map((b) => b.toString(16).padStart(2, '0')).join('')
  return `token-${hex}`
}

export function maskGatewayAuthKey(key: string): string {
  if (key.length <= 12) return key
  return `${key.slice(0, 8)}…${key.slice(-4)}`
}

export async function postSetup(body: UpstreamSetupUpdate) {
  return apiFetch<{ message?: string; upstream: import('../types/gateway').UpstreamSetupView }>(
    '/v1/admin/setup',
    { method: 'POST', body: JSON.stringify(body) },
  )
}

/** Fetch /v1/admin/status and sync sidebar gateway card + uptime anchor. */
export async function refreshGatewayStatus(): Promise<GatewayStatus> {
  const status = await apiFetch<GatewayStatus>('/v1/admin/status')
  const { setStatus, setUptimeAnchor } = useAppStore.getState()
  setStatus(status)
  setUptimeAnchor({ secs: status.uptime_secs, at: Date.now() })
  return status
}

/** Retry status fetch after gateway restart — process may not accept HTTP immediately. */
export async function refreshGatewayStatusAfterRestart(
  maxAttempts = 10,
  delayMs = 300,
): Promise<GatewayStatus> {
  let lastError: unknown
  for (let attempt = 0; attempt < maxAttempts; attempt++) {
    try {
      return await refreshGatewayStatus()
    } catch (e) {
      lastError = e
      if (attempt < maxAttempts - 1) {
        await new Promise((resolve) => setTimeout(resolve, delayMs))
      }
    }
  }
  throw lastError instanceof Error ? lastError : new Error(String(lastError))
}

export { useAuthStore, useAppStore }
