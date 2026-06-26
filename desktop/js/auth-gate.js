import {
  hasPersistedSession,
  subscribe,
  logout,
  applyAuthShellLoggedIn,
  applyAuthShellLoggedOut,
} from './flowy/auth-store.js';
import { initLoginUI } from './auth-ui.js';

let sessionHooked = false;

function hookSessionExpired() {
  if (sessionHooked) return;
  sessionHooked = true;
  subscribe((state) => {
    if (state.isSessionExpired) {
      logout();
      location.reload();
    }
  });
}

function whenStartMainAppReady(fn) {
  if (typeof window.startMainApp === 'function') {
    fn();
    return;
  }
  const wait = () => {
    if (typeof window.startMainApp === 'function') {
      fn();
    } else {
      requestAnimationFrame(wait);
    }
  };
  requestAnimationFrame(wait);
}

export function bootAuth() {
  if (hasPersistedSession()) {
    applyAuthShellLoggedIn();
    hookSessionExpired();
    whenStartMainAppReady(() => window.startMainApp());
    return;
  }

  applyAuthShellLoggedOut();
  initLoginUI(() => {
    hookSessionExpired();
    whenStartMainAppReady(() => window.startMainApp());
  });
}
