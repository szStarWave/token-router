export function formatRelativeTime(
  unixSec: number | null | undefined,
  t: (key: string, vars?: Record<string, string | number>) => string,
  locale?: string,
): string {
  if (unixSec == null || !Number.isFinite(unixSec) || unixSec <= 0) return '—'
  const diffMs = Math.max(0, Date.now() - unixSec * 1000)
  const mins = Math.floor(diffMs / 60_000)
  if (mins < 1) return t('time.justNow')
  if (mins < 60) return t('time.minutesAgo', { n: mins })
  const hours = Math.floor(mins / 60)
  if (hours < 24) return t('time.hoursAgo', { n: hours })
  const days = Math.floor(hours / 24)
  if (days < 7) return t('time.daysAgo', { n: days })
  return new Date(unixSec * 1000).toLocaleString(locale)
}
