import { BRAND } from './config.js';

const STORAGE_KEY = `${BRAND.storePrefix}-auth`;

const defaultState = () => ({
  isLoggedIn: false,
  authToken: null,
  userInfo: null,
  hasAgreedToUserDeclaration: false,
  isSessionExpired: false,
});

let state = defaultState();
let listeners = [];

function load() {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return;
    const parsed = JSON.parse(raw);
    state = { ...defaultState(), ...parsed.state };
    if (state.authToken && !state.isLoggedIn) {
      state.isLoggedIn = true;
    }
  } catch {
    state = defaultState();
  }
}

function persist() {
  localStorage.setItem(STORAGE_KEY, JSON.stringify({ state: {
    isLoggedIn: state.isLoggedIn,
    authToken: state.authToken,
    userInfo: state.userInfo,
    hasAgreedToUserDeclaration: state.hasAgreedToUserDeclaration,
  } }));
}

function emit() {
  listeners.forEach((fn) => fn(state));
}

export function subscribe(fn) {
  listeners.push(fn);
  return () => { listeners = listeners.filter((f) => f !== fn); };
}

export function getAuthState() {
  return { ...state };
}

export function login(token, userInfo) {
  state.isLoggedIn = true;
  state.authToken = token;
  state.userInfo = userInfo ?? null;
  state.isSessionExpired = false;
  persist();
  emit();
}

export function logout() {
  state = defaultState();
  persist();
  emit();
}

export function setSessionExpired(expired) {
  state.isSessionExpired = expired;
  emit();
}

export function setHasAgreedToUserDeclaration(agreed) {
  state.hasAgreedToUserDeclaration = agreed;
  persist();
  emit();
}

export function getAuthToken() {
  return state.authToken;
}

export function updateUserInfo(patch) {
  if (!patch || typeof patch !== 'object') return;
  state.userInfo = { ...(state.userInfo || {}), ...patch };
  persist();
  emit();
}

export function applyAuthShellLoggedIn() {
  document.documentElement.dataset.auth = 'logged-in';
}

export function applyAuthShellLoggedOut() {
  delete document.documentElement.dataset.auth;
}

export function hasPersistedSession() {
  return state.isLoggedIn && state.authToken;
}

load();
