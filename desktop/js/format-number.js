/** Compact number labels: raw below 1K; K / M / B / T above. */

const UNITS = [
  { scale: 1e12, suffix: 'T' },
  { scale: 1e9, suffix: 'B' },
  { scale: 1e6, suffix: 'M' },
  { scale: 1e3, suffix: 'K' },
];

const COMPACT_THRESHOLD = 1e3;

function normalizeNum(n) {
  if (n == null || n === '') return null;
  const num = Number(n);
  if (!Number.isFinite(num)) return null;
  return num;
}

function trimDecimal(s) {
  if (!s.includes('.')) return s;
  return s.replace(/\.0+$/, '').replace(/(\.\d*?)0+$/, '$1');
}

function decimalPlaces(scaled) {
  const abs = Math.abs(scaled);
  if (abs >= 100) return 0;
  if (abs >= 10) return 1;
  if (abs >= 1) return 1;
  return 2;
}

/**
 * @param {unknown} n
 * @param {string} [_locale] ignored; always K/M/B/T
 */
export function formatCompactNum(n, _locale) {
  const num = normalizeNum(n);
  if (num == null) return '—';

  const abs = Math.abs(num);
  if (abs < COMPACT_THRESHOLD) {
    return String(Math.round(num));
  }

  for (const { scale, suffix } of UNITS) {
    if (abs >= scale) {
      const v = num / scale;
      const d = decimalPlaces(v);
      return trimDecimal(v.toFixed(d)) + suffix;
    }
  }

  return String(Math.round(num));
}

export function formatNum(n, locale) {
  return formatCompactNum(n, locale);
}

export function formatAxisNum(n, locale) {
  return formatCompactNum(n, locale);
}

export function installNumberFormatGlobals() {
  window.__numberFormat = {
    formatCompactNum,
    formatNum,
    formatAxisNum,
  };
}
