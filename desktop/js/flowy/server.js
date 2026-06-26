import { FLOWY_HOSTS } from './config.js';

function normalizeEdition(value) {
  return value === 'international' ? 'international' : 'domestic';
}

function isTestFlagEnabled(value) {
  if (typeof value !== 'string') return false;
  const n = value.trim().toLowerCase();
  if (!n) return false;
  return !['0', 'false', 'off', 'no'].includes(n);
}

function serverOptions() {
  return {
    dev: import.meta.env.DEV,
    testServerFlag: import.meta.env.VITE_FLOWY_TEST_SERVER,
  };
}

function edition() {
  return normalizeEdition(import.meta.env.VITE_EDITION);
}

export function getEdition() {
  return edition();
}

export function isDevToolsEnabled() {
  return isTestFlagEnabled(import.meta.env.VITE_FLOWY_TEST_SERVER);
}

export function shouldUseFlowyTestServer() {
  const o = serverOptions();
  return o.dev && isTestFlagEnabled(o.testServerFlag);
}

export function getCurrentFlowyServerBase(pathPrefix = '/claw') {
  const o = serverOptions();
  if (shouldUseFlowyTestServer()) {
    return `https://${FLOWY_HOSTS.testHost}${pathPrefix}`;
  }
  const host = edition() === 'international'
    ? FLOWY_HOSTS.productionInternationalHost
    : FLOWY_HOSTS.productionDomesticHost;
  return `https://${host}${pathPrefix}`;
}

export function getWeChatFlowyServerBase(pathPrefix = '/claw') {
  const host = shouldUseFlowyTestServer()
    ? FLOWY_HOSTS.testHost
    : FLOWY_HOSTS.productionDomesticHost;
  return `https://${host}${pathPrefix}`;
}

export function isWeChatLoginEnabled() {
  return edition() === 'domestic';
}

export function getDefaultLoginMode() {
  return isWeChatLoginEnabled() ? 'wechat' : 'email';
}
