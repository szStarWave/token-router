const UNITS = [
  { scale: 1e12, suffix: 'T' },
  { scale: 1e9, suffix: 'B' },
  { scale: 1e6, suffix: 'M' },
  { scale: 1e3, suffix: 'K' },
]

const COMPACT_THRESHOLD = 1e3

function normalizeNum(n: unknown): number | null {
  if (n == null || n === '') return null
  const num = Number(n)
  if (!Number.isFinite(num)) return null
  return num
}

function trimDecimal(s: string): string {
  if (!s.includes('.')) return s
  return s.replace(/\.0+$/, '').replace(/(\.\d*?)0+$/, '$1')
}

function decimalPlaces(scaled: number): number {
  const abs = Math.abs(scaled)
  if (abs >= 100) return 0
  if (abs >= 10) return 1
  if (abs >= 1) return 1
  return 2
}

export function formatCompactNum(n: unknown, _locale?: string): string {
  const num = normalizeNum(n)
  if (num == null) return '—'

  const abs = Math.abs(num)
  if (abs < COMPACT_THRESHOLD) {
    return String(Math.round(num))
  }

  for (const { scale, suffix } of UNITS) {
    if (abs >= scale) {
      const v = num / scale
      const d = decimalPlaces(v)
      return trimDecimal(v.toFixed(d)) + suffix
    }
  }

  return String(Math.round(num))
}

export function formatNum(n: unknown, locale?: string): string {
  return formatCompactNum(n, locale)
}

export function formatAxisNum(n: unknown, locale?: string): string {
  return formatCompactNum(n, locale)
}

/** Y-axis ticks: preserve fractional labels when the scale is small. */
export function formatAxisTick(n: unknown, locale?: string): string {
  const num = normalizeNum(n)
  if (num == null) return ''

  const abs = Math.abs(num)
  if (abs >= COMPACT_THRESHOLD) {
    return formatCompactNum(num, locale)
  }
  if (Number.isInteger(num)) {
    return num.toLocaleString(locale)
  }
  if (abs >= 1) {
    return trimDecimal(num.toLocaleString(locale, { maximumFractionDigits: 1 }))
  }
  if (abs > 0) {
    return trimDecimal(num.toLocaleString(locale, { maximumFractionDigits: 2 }))
  }
  return '0'
}
