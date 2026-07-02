export function toastErrorVars(err: unknown): Record<string, string> {
  const msg = err instanceof Error ? err.message : String(err)
  return { msg }
}

export function toastErrorKey(
  err: unknown,
  fallbackKey: string,
): { key: string; vars?: Record<string, string> } {
  const msg = err instanceof Error ? err.message : String(err)
  if (msg === 'offline') return { key: 'conn.offline' }
  return { key: fallbackKey, vars: { msg } }
}
