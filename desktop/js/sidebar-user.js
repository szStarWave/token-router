import {
  getCreditsBalance,
  getCreditsUsageByType,
  loginByToken,
} from './flowy/api.js';
import { getEdition } from './flowy/server.js';
import {
  getAuthState,
  getAuthToken,
  subscribe,
  updateUserInfo,
} from './flowy/auth-store.js';

import { appT, getAppLocale } from './locale-utils.js';

const CREDIT_TYPE_ORDER = [
  'DAILY_CHECKIN',
  'PLAN',
  'PACK',
  'SIGNUP',
  'TEAM_SEAT',
  'OTHER',
];

function ui(key) {
  const map = {
    refresh: 'action.refresh',
    viewBilling: 'sidebar.viewBilling',
    getMoreCredits: 'sidebar.getMoreCredits',
    loading: 'sidebar.loading',
    empty: 'sidebar.empty',
    failed: 'sidebar.failed',
  };
  const i18nKey = map[key] ?? key;
  if (window.__appI18n?.t) return window.__appI18n.t(i18nKey);
  return appT(i18nKey);
}

function creditTypeLabel(type) {
  const key = 'creditType.' + type;
  if (window.__appI18n?.t) return window.__appI18n.t(key);
  return appT(key);
}

function locale() {
  return window.__appI18n?.locale?.() ?? getAppLocale();
}

function refreshSidebarI18n() {
  const moreLabel = document.getElementById('sider-credits-more-label');
  const viewBilling = document.getElementById('sider-credits-view-billing');
  if (moreLabel) moreLabel.textContent = ui('getMoreCredits');
  if (viewBilling) viewBilling.textContent = ui('viewBilling');
  updateCooldownUi();
  renderCreditsPopover();
}

let refreshTimer = null;
let creditsUsageRows = [];
let usageLoading = false;
let usageFailed = false;
let cooldownUntil = 0;
let cooldownTimer = null;
let popoverVisible = false;
let hidePopoverTimer = null;

function pickNickname(user) {
  if (!user) return '';
  return user.nickname || user.name || user.username || user.email || '';
}

function pickAvatar(user) {
  if (!user) return '';
  return user.avatar || user.headImg || user.head_img || user.avatarUrl || '';
}

function formatCredits(n) {
  if (typeof n !== 'number' || Number.isNaN(n)) return '0';
  return n.toLocaleString();
}

function cooldownSeconds() {
  return Math.max(0, Math.ceil((cooldownUntil - Date.now()) / 1000));
}

function updateCooldownUi() {
  const sec = cooldownSeconds();
  const popRefresh = document.getElementById('sider-credits-popover-refresh');
  if (popRefresh) {
    popRefresh.textContent = sec > 0 ? `${sec}s` : ui('refresh');
    popRefresh.disabled = sec > 0 || usageLoading;
  }
}

function startCooldownTimer() {
  if (cooldownTimer) clearInterval(cooldownTimer);
  cooldownTimer = setInterval(() => {
    updateCooldownUi();
    if (cooldownSeconds() <= 0) {
      clearInterval(cooldownTimer);
      cooldownTimer = null;
    }
  }, 250);
}

function updateDom(user, credits) {
  const nameEl = document.getElementById('sider-user-name');
  const creditsEl = document.getElementById('sider-credits-value');
  const avatarImg = document.getElementById('sider-user-avatar-img');
  const avatarFallback = document.getElementById('sider-user-avatar-fallback');

  const nickname = pickNickname(user);
  const avatar = pickAvatar(user);

  if (nameEl) nameEl.textContent = nickname;
  if (creditsEl) creditsEl.textContent = formatCredits(credits);

  if (avatarImg && avatarFallback) {
    if (avatar) {
      avatarImg.alt = nickname || 'User';
      avatarFallback.hidden = true;
      avatarImg.hidden = false;
      avatarImg.src = avatar;
    } else {
      avatarImg.hidden = true;
      avatarImg.removeAttribute('src');
      avatarFallback.hidden = false;
      const letter = (nickname || '?').trim().slice(0, 1).toUpperCase();
      avatarFallback.textContent = letter || '?';
    }
  }
}

function renderCreditsPopover() {
  const breakdown = document.getElementById('sider-credits-breakdown');
  const popover = document.getElementById('sider-credits-popover');
  if (!breakdown || !popover) return;

  if (!popoverVisible) {
    popover.hidden = true;
    return;
  }

  popover.hidden = false;

  if (usageLoading) {
    breakdown.innerHTML = `<p class="sider-credits-popover-hint">${ui('loading')}</p>`;
    return;
  }

  if (usageFailed) {
    breakdown.innerHTML = `<p class="sider-credits-popover-hint">${ui('failed')}</p>`;
    return;
  }

  if (!creditsUsageRows.length) {
    breakdown.innerHTML = `<p class="sider-credits-popover-hint">${ui('empty')}</p>`;
    return;
  }

  breakdown.innerHTML = creditsUsageRows
    .map((row) => `
      <div class="sider-credits-breakdown-row">
        <span class="sider-credits-breakdown-label">${row.label}</span>
        <span class="sider-credits-breakdown-value">${formatCredits(row.remaining)}</span>
      </div>
    `)
    .join('');

  const viewBilling = document.getElementById('sider-credits-view-billing');
  if (viewBilling) viewBilling.textContent = ui('viewBilling');
  updateCooldownUi();
}

async function fetchCreditsUsage() {
  const token = getAuthToken();
  if (!token) {
    creditsUsageRows = [];
    return;
  }

  usageLoading = true;
  usageFailed = false;
  renderCreditsPopover();

  try {
    const data = await getCreditsUsageByType(token);
    const list = Array.isArray(data?.list) ? data.list : [];
    const typeSet = new Set(list.map((item) => item.type));

    creditsUsageRows = CREDIT_TYPE_ORDER
      .filter((type) => typeSet.has(type))
      .map((type) => {
        const item = list.find((i) => i.type === type);
        const remaining =
          typeof item?.remaining === 'number' && Number.isFinite(item.remaining) && item.remaining >= 0
            ? item.remaining
            : 0;
        const label = item?.title?.trim() || creditTypeLabel(type);
        return { type, label, remaining };
      });
  } catch (e) {
    console.warn('[sidebar-user] credits/usageByType', e);
    usageFailed = true;
    creditsUsageRows = [];
  } finally {
    usageLoading = false;
    renderCreditsPopover();
  }
}

async function openExternalUrl(url) {
  if (window.__TAURI__?.opener?.openUrl) {
    try {
      await window.__TAURI__.opener.openUrl(url);
      return;
    } catch (e) {
      console.warn('[sidebar-user] openUrl', e);
    }
  }
  window.open(url, '_blank', 'noopener,noreferrer');
}

function billingProfileUrl() {
  const edition = getEdition();
  const host = edition === 'international' ? 'flowyaipc.com' : 'flowyaipc.cn';
  const lang = locale() === 'en' ? 'en' : 'zh';
  const token = getAuthToken();
  const base = `https://${host}/`;
  const q = token
    ? `?token=${encodeURIComponent(token)}&language=${lang}`
    : `?language=${lang}`;
  return `${base}${q}#profile?tab=records`;
}

async function openBillingProfile() {
  await openExternalUrl(billingProfileUrl());
}

function paymentPageUrl() {
  const edition = getEdition();
  const host = edition === 'international' ? 'flowyaipc.com' : 'flowyaipc.cn';
  const lang = locale() === 'en' ? 'en' : 'zh';
  const token = getAuthToken();
  const base = `https://${host}/#pricing`;
  return token
    ? `${base}?token=${encodeURIComponent(token)}&language=${lang}`
    : `${base}?language=${lang}`;
}

async function openPaymentPage() {
  await openExternalUrl(paymentPageUrl());
}

async function refreshCreditsData(manual = false) {
  if (manual) {
    if (cooldownSeconds() > 0 || usageLoading) return;
    cooldownUntil = Date.now() + 5000;
    startCooldownTimer();
    updateCooldownUi();
  }
  await Promise.allSettled([fetchCreditsUsage(), refreshSidebarUser()]);
}

function scheduleHideCreditsPopover() {
  if (hidePopoverTimer) clearTimeout(hidePopoverTimer);
  hidePopoverTimer = setTimeout(() => {
    hidePopoverTimer = null;
    hideCreditsPopover();
  }, 160);
}

function showCreditsPopover() {
  if (hidePopoverTimer) {
    clearTimeout(hidePopoverTimer);
    hidePopoverTimer = null;
  }
  popoverVisible = true;
  renderCreditsPopover();
  if (!creditsUsageRows.length && !usageLoading && !usageFailed) {
    void fetchCreditsUsage();
  }
}

function hideCreditsPopover() {
  popoverVisible = false;
  const popover = document.getElementById('sider-credits-popover');
  if (popover) popover.hidden = true;
}

function bindCreditsHover() {
  const creditsMain = document.getElementById('sider-credits-main');
  const popover = document.getElementById('sider-credits-popover');
  const onEnter = () => showCreditsPopover();
  const onLeave = () => scheduleHideCreditsPopover();

  creditsMain?.addEventListener('mouseenter', onEnter);
  creditsMain?.addEventListener('mouseleave', onLeave);
  popover?.addEventListener('mouseenter', onEnter);
  popover?.addEventListener('mouseleave', onLeave);
}

export async function refreshSidebarUser() {
  const token = getAuthToken();
  if (!token) {
    updateDom(null, 0);
    return;
  }

  let user = getAuthState().userInfo || {};
  try {
    const fresh = await loginByToken(token);
    if (fresh) {
      user = { ...user, ...fresh };
    }
  } catch (e) {
    console.warn('[sidebar-user] user/me', e);
  }

  let credits = typeof user.creditsBalance === 'number' ? user.creditsBalance : null;
  try {
    credits = await getCreditsBalance(token);
  } catch (e) {
    console.warn('[sidebar-user] credits/balance', e);
    if (credits === null) credits = 0;
  }

  const merged = { ...user, creditsBalance: credits };
  updateUserInfo(merged);
  updateDom(merged, credits);
}

export function initSidebarUser() {
  const moreBtn = document.getElementById('sider-credits-more');
  const moreLabel = document.getElementById('sider-credits-more-label');
  const popRefresh = document.getElementById('sider-credits-popover-refresh');
  const viewBilling = document.getElementById('sider-credits-view-billing');

  bindCreditsHover();

  const avatarImg = document.getElementById('sider-user-avatar-img');
  const avatarFallback = document.getElementById('sider-user-avatar-fallback');
  if (avatarImg && avatarFallback) {
    avatarImg.addEventListener('error', () => {
      avatarImg.hidden = true;
      avatarImg.removeAttribute('src');
      avatarFallback.hidden = false;
      const nickname = pickNickname(getAuthState().userInfo);
      const letter = (nickname || '?').trim().slice(0, 1).toUpperCase();
      avatarFallback.textContent = letter || '?';
    });
  }

  if (moreLabel) moreLabel.textContent = ui('getMoreCredits');

  moreBtn?.addEventListener('click', (e) => {
    e.stopPropagation();
    void openPaymentPage();
  });

  popRefresh?.addEventListener('click', (e) => {
    e.stopPropagation();
    void refreshCreditsData(true);
  });

  viewBilling?.addEventListener('click', (e) => {
    e.stopPropagation();
    void openBillingProfile();
  });

  subscribe((auth) => {
    if (!auth.isLoggedIn) {
      creditsUsageRows = [];
      usageFailed = false;
      hideCreditsPopover();
      updateDom(null, 0);
    }
  });

  if (refreshTimer) clearInterval(refreshTimer);
  refreshTimer = setInterval(() => {
    if (getAuthToken()) {
      void fetchCreditsUsage();
      void refreshSidebarUser();
    }
  }, 120000);

  void refreshSidebarUser();
  void fetchCreditsUsage();

  window.__sidebarUser = {
    refresh: async () => {
      await refreshCreditsData(false);
    },
    refreshI18n: refreshSidebarI18n,
  };

  document.addEventListener('app-locale-change', () => refreshSidebarI18n());
}
