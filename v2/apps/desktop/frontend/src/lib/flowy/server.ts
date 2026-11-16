import { FLOWY_HOSTS } from './config'

function normalizeEdition(value: unknown): 'domestic' | 'international' {
  return value === 'international' ? 'international' : 'domestic'
}

function isTestFlagEnabled(value: unknown): boolean {
  if (typeof value !== 'string') return false
  const n = value.trim().toLowerCase()
  if (!n) return false
  return !['0', 'false', 'off', 'no'].includes(n)
}

function edition(): 'domestic' | 'international' {
  return normalizeEdition(import.meta.env.VITE_EDITION)
}

export function getEdition(): 'domestic' | 'international' {
  return edition()
}

export function isDevToolsEnabled(): boolean {
  return isTestFlagEnabled(import.meta.env.VITE_FLOWY_TEST_SERVER)
}

export function shouldUseFlowyTestServer(): boolean {
  return import.meta.env.DEV && isTestFlagEnabled(import.meta.env.VITE_FLOWY_TEST_SERVER)
}

export function getCurrentFlowyServerBase(pathPrefix = '/claw'): string {
  if (shouldUseFlowyTestServer()) {
    return `https://${FLOWY_HOSTS.testHost}${pathPrefix}`
  }
  const host =
    edition() === 'international'
      ? FLOWY_HOSTS.productionInternationalHost
      : FLOWY_HOSTS.productionDomesticHost
  return `https://${host}${pathPrefix}`
}

export function getWeChatFlowyServerBase(pathPrefix = '/claw'): string {
  const host = shouldUseFlowyTestServer()
    ? FLOWY_HOSTS.testHost
    : FLOWY_HOSTS.productionDomesticHost
  return `https://${host}${pathPrefix}`
}

export function isWeChatLoginEnabled(): boolean {
  return edition() === 'domestic'
}

export function getDefaultLoginMode(): 'wechat' | 'email' {
  return isWeChatLoginEnabled() ? 'wechat' : 'email'
}
