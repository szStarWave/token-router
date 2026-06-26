export function getAppLocale() {
  return localStorage.getItem('tr-locale') === 'en' ? 'en' : 'zh';
}

export function appT(key, vars = {}, locale = getAppLocale()) {
  const dict = window.__appI18n?.dict;
  if (!dict) return key;
  let s = dict[locale]?.[key] ?? dict.zh?.[key] ?? key;
  for (const [k, v] of Object.entries(vars)) s = s.replaceAll(`{${k}}`, String(v));
  return s;
}
