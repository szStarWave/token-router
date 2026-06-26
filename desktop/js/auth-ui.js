import { BRAND } from './flowy/config.js';
import {
  getWeChatFlowyServerBase,
  isWeChatLoginEnabled,
  getDefaultLoginMode,
} from './flowy/server.js';
import {
  sendEmailCode,
  loginByEmail,
  loginByWeChatCallback,
  loginByToken,
  deviceActivateAfterLogin,
} from './flowy/api.js';
import {
  getAuthState,
  login,
  setHasAgreedToUserDeclaration,
  applyAuthShellLoggedIn,
} from './flowy/auth-store.js';

const WECHAT_STYLE_DARK =
  'data:text/css;base64,LmltcG93ZXJCb3ggLnRpdGxlIHsKICBkaXNwbGF5OiBub25lICFpbXBvcnRhbnQ7Cn0KLmltcG93ZXJCb3ggLmluZm8gewogIGRpc3BsYXk6IG5vbmUgIWltcG9ydGFudDsKfQoKLmltcG93ZXJCb3ggLnFyY29kZXsKICAgd2lkdGg6IC13ZWJraXQtZmlsbC1hdmFpbGFibGU7CiAgIG1heC13aWR0aDogMjgwcHg7CiAgIG1hcmdpbi10b3A6MHB4Owp9Ci5pbXBvd2VyQm94IHsKICBoZWlnaHQ6IDMwMHB4ICFpbXBvcnRhbnQ7CiAgb3ZlcmZsb3c6IGhpZGRlbiAhaW1wb3J0YW50Owp9Cmh0bWwgewogICAgYmFja2dyb3VuZDogIzFkMWQxZDAwICFpbXBvcnRhbnQ7Cn0KYm9keXsKICAgIGJhY2tncm91bmQ6ICMxZDFkMWQwMCAhaW1wb3J0YW50Owp9';
const WECHAT_STYLE_LIGHT =
  'data:text/css;base64,LmltcG93ZXJCb3ggLnRpdGxlIHsKICBkaXNwbGF5OiBub25lICFpbXBvcnRhbnQ7Cn0KLmltcG93ZXJCb3ggLmluZm8gewogIGRpc3BsYXk6IG5vbmUgIWltcG9ydGFudDsKfQoKLmltcG93ZXJCb3ggLnFyY29kZXsKICAgd2lkdGg6IC13ZWJraXQtZmlsbC1hdmFpbGFibGU7CiAgIG1heC13aWR0aDogMjgwcHg7CiAgIG1hcmdpbi10b3A6MHB4Owp9Ci5pbXBvd2VyQm94IHsKICBoZWlnaHQ6IDMwMHB4ICFpbXBvcnRhbnQ7CiAgb3ZlcmZsb3c6IGhpZGRlbiAhaW1wb3J0YW50Owp9Cmh0bWwgewogICAgYmFja2dyb3VuZDogI2ZjZmNmYzAwICFpbXBvcnRhbnQ7Cn0KYm9keXsKICAgIGJhY2tncm91bmQ6ICNmY2ZjZmMwMCAhaW1wb3J0YW50Owp9';

const LOGIN_I18N = {
  zh: {
    emailTitle: '邮箱登录',
    wechatTitle: '微信登录',
    wechatHint: '请使用微信扫描二维码登录',
    wechatLoadFail: '微信登录加载失败，请刷新重试',
    emailPlaceholder: '请输入邮箱',
    codePlaceholder: '请输入验证码',
    inviteCodePlaceholder: '请输入邀请码（选填）',
    getCode: '获取验证码',
    login: '登录',
    loggingIn: '登录中...',
    emailTips: '请输入邮箱',
    emailInvalid: '请输入有效的邮箱地址',
    codeTips: '请输入验证码',
    sendCodeTips: '请先发送验证码',
    sendCodeFail: '验证码发送失败',
    tabWeChat: '微信扫码登录',
    tabEmail: '邮箱登录',
    agreeAndContinue: '同意并继续',
    cancel: '取消',
    userDeclarationTitle: '用户声明',
    loginFail: '登录失败',
  },
  en: {
    emailTitle: 'Email login',
    wechatTitle: 'WeChat login',
    wechatHint: 'Scan the QR code with WeChat',
    wechatLoadFail: 'Failed to load WeChat login. Please refresh.',
    emailPlaceholder: 'Email',
    codePlaceholder: 'Verification code',
    inviteCodePlaceholder: 'Invite code (optional)',
    getCode: 'Get code',
    login: 'Sign in',
    loggingIn: 'Signing in...',
    emailTips: 'Enter your email',
    emailInvalid: 'Enter a valid email',
    codeTips: 'Enter verification code',
    sendCodeTips: 'Send verification code first',
    sendCodeFail: 'Failed to send code',
    tabWeChat: 'WeChat QR',
    tabEmail: 'Email',
    agreeAndContinue: 'Agree and continue',
    cancel: 'Cancel',
    userDeclarationTitle: 'User agreement',
    loginFail: 'Login failed',
  },
};

const USER_DECLARATION = {
  zh: '您使用本产品的前提是，您承认和接受用户声明的全部内容。AI 生成内容仅供参考，请独立核查。继续使用即表示您已阅读并理解相关风险。',
  en: 'By using this product you acknowledge and accept the user agreement. AI-generated content is for reference only; verify independently. Continuing means you have read and understood the risks.',
};

let locale = 'zh';
let mode = 'email';
let onComplete = null;
let codeTimer = null;
let emailReqNo = null;
let pendingLogin = null;
let wechatListenerBound = false;
let wechatCallbackProcessing = false;

function enterMainApp() {
  applyAuthShellLoggedIn();
  $('#login-screen')?.classList.add('hidden');
}

async function finishLogin(token, userInfo) {
  login(token, userInfo);
  void deviceActivateAfterLogin(token);
  enterMainApp();
  onComplete?.();
}

function t(key) {
  return LOGIN_I18N[locale]?.[key] ?? LOGIN_I18N.zh[key] ?? key;
}

function $(sel) {
  if (!sel) return null;
  if (sel.startsWith('#')) return document.getElementById(sel.slice(1));
  return document.getElementById(sel);
}

function showLoginToast(msg, ok = false) {
  const el = $('#login-toast');
  if (!el) return;
  el.textContent = msg;
  el.className = 'login-toast show' + (ok ? ' ok' : ' err');
  setTimeout(() => el.classList.remove('show'), 3200);
}

function effectiveTheme() {
  const pref = localStorage.getItem('tr-theme') || 'system';
  if (pref === 'system') {
    return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }
  return pref;
}

function applyLoginTheme() {
  document.documentElement.dataset.effectiveTheme = effectiveTheme();
}

function updateLoginTabs(m) {
  const wechatEnabled = isWeChatLoginEnabled();
  $('#login-tabs')?.classList.toggle('hidden', !wechatEnabled);
  $('#login-tab-wechat')?.classList.toggle('active', m === 'wechat');
  $('#login-tab-email')?.classList.toggle('active', m === 'email');
}

function showMode(m) {
  mode = m;
  const wechatEnabled = isWeChatLoginEnabled();
  $('#login-email-view')?.classList.toggle('active', m === 'email');
  $('#login-wechat-view')?.classList.toggle('active', m === 'wechat');
  $('#login-wechat-view')?.classList.toggle('hidden', !wechatEnabled);
  updateLoginTabs(m);
  if (m === 'wechat' && wechatEnabled) mountWeChatQr();
}

async function handleLoginToken(token, userInfo) {
  const auth = getAuthState();
  if (!auth.hasAgreedToUserDeclaration) {
    pendingLogin = { token, userInfo };
    $('#security-dialog')?.classList.add('open');
    showLoginToast(t('agreeAndContinue'), true);
    return;
  }
  await finishLogin(token, userInfo);
}

function validateEmail(email) {
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email);
}

async function sendCodeAction() {
  const email = $('#login-email')?.value?.trim() || '';
  if (!email) {
    showLoginToast(t('emailTips'));
    return;
  }
  if (!validateEmail(email)) {
    showLoginToast(t('emailInvalid'));
    return;
  }
  const btn = $('#login-send-code');
  if (btn?.disabled) return;
  btn.disabled = true;
  try {
    emailReqNo = await sendEmailCode(email);
    let sec = 60;
    btn.textContent = `${sec}s`;
    codeTimer = setInterval(() => {
      sec -= 1;
      if (sec <= 0) {
        clearInterval(codeTimer);
        codeTimer = null;
        btn.disabled = false;
        btn.textContent = t('getCode');
        return;
      }
      btn.textContent = `${sec}s`;
    }, 1000);
  } catch (e) {
    btn.disabled = false;
    showLoginToast(e.message || t('sendCodeFail'));
  }
}

function getInviteCode() {
  return $('#login-invite')?.value?.trim() || '';
}

async function emailLoginAction() {
  const email = $('#login-email')?.value?.trim() || '';
  const code = $('#login-code')?.value?.trim() || '';
  const invite = getInviteCode();
  if (!email) { showLoginToast(t('emailTips')); return; }
  if (!code) { showLoginToast(t('codeTips')); return; }
  if (!emailReqNo) { showLoginToast(t('sendCodeTips')); return; }
  const btn = $('#login-submit');
  if (btn?.disabled) return;
  btn.disabled = true;
  btn.textContent = t('loggingIn');
  try {
    const token = await loginByEmail(email, code, emailReqNo, invite);
    let userInfo = null;
    try { userInfo = await loginByToken(token); } catch { /* optional */ }
    await handleLoginToken(token, userInfo);
  } catch (e) {
    showLoginToast(e.message || t('loginFail'));
  } finally {
    btn.disabled = false;
    btn.textContent = t('login');
  }
}

function waitForWxLogin(maxMs = 8000) {
  return new Promise((resolve) => {
    if (window.WxLogin) {
      resolve(true);
      return;
    }
    const script = document.querySelector('script[src*="wxLogin.js"]');
    const done = (ok) => resolve(ok);
    if (script) {
      script.addEventListener('load', () => done(!!window.WxLogin), { once: true });
      script.addEventListener('error', () => done(false), { once: true });
    }
    const start = Date.now();
    const tick = () => {
      if (window.WxLogin) {
        done(true);
        return;
      }
      if (Date.now() - start >= maxMs) {
        done(false);
        return;
      }
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
}

async function mountWeChatQr() {
  const container = $('#wechat-qr-container');
  if (!container) return;
  container.innerHTML = '';

  const ready = await waitForWxLogin();
  if (!ready) {
    container.innerHTML = `<p class="login-hint">${t('wechatLoadFail')}</p>`;
    return;
  }

  const theme = effectiveTheme();
  const redirectUri = encodeURIComponent(
    `${getWeChatFlowyServerBase()}/auth/third/callback?platform=WECHAT`,
  );
  try {
    new window.WxLogin({
      self_redirect: true,
      id: 'wechat-qr-container',
      appid: BRAND.wechatAppId,
      scope: 'snsapi_login',
      redirect_uri: redirectUri,
      state: `wechat_login_${Date.now()}`,
      style: 'black',
      href: theme === 'dark' ? WECHAT_STYLE_DARK : WECHAT_STYLE_LIGHT,
    });
  } catch (e) {
    console.error('WxLogin init failed', e);
    container.innerHTML = `<p class="login-hint">${t('wechatLoadFail')}</p>`;
  }
}

async function processWeChatCallback(callbackUrl) {
  if (wechatCallbackProcessing) return;
  wechatCallbackProcessing = true;
  try {
    const params = new URLSearchParams();
    params.set('channel', BRAND.id);
    const invite = getInviteCode();
    if (invite) params.set('inviteCode', invite);
    const sep = callbackUrl.includes('?') ? '&' : '?';
    const url = `${callbackUrl}${sep}${params.toString()}`;
    const token = await loginByWeChatCallback(url);
    let userInfo = null;
    try { userInfo = await loginByToken(token); } catch { /* optional */ }
    await handleLoginToken(token, userInfo);
  } catch (e) {
    showLoginToast(e.message || t('loginFail'));
    if (mode === 'wechat') mountWeChatQr();
  } finally {
    wechatCallbackProcessing = false;
  }
}

function bindWeChatListener() {
  if (wechatListenerBound) return;
  wechatListenerBound = true;

  const handler = (event) => {
    const url = event?.payload;
    if (typeof url === 'string' && url) processWeChatCallback(url);
  };

  if (window.__TAURI__?.event?.listen) {
    void window.__TAURI__.event.listen('wechat-login-callback', handler);
  }
  const webview = window.__TAURI__?.webview?.getCurrentWebview?.();
  if (webview?.listen) {
    void webview.listen('wechat-login-callback', handler);
  }
}

function bindSecurityDialog() {
  $('#security-agree')?.addEventListener('click', async () => {
    setHasAgreedToUserDeclaration(true);
    $('#security-dialog')?.classList.remove('open');
    if (pendingLogin) {
      const { token, userInfo } = pendingLogin;
      pendingLogin = null;
      await finishLogin(token, userInfo);
    }
  });
  $('#security-cancel')?.addEventListener('click', () => {
    pendingLogin = null;
    $('#security-dialog')?.classList.remove('open');
  });
}

function setText(id, text) {
  const el = $(id);
  if (el) el.textContent = text;
}

function applyLoginI18n() {
  locale = localStorage.getItem('tr-locale') || 'zh';
  setText('login-email-title', t('emailTitle'));
  setText('login-wechat-title', t('wechatTitle'));
  setText('login-wechat-hint', t('wechatHint'));
  $('#login-email')?.setAttribute('placeholder', t('emailPlaceholder'));
  $('#login-code')?.setAttribute('placeholder', t('codePlaceholder'));
  $('#login-invite')?.setAttribute('placeholder', t('inviteCodePlaceholder'));
  setText('login-submit', t('login'));
  setText('login-send-code', t('getCode'));
  setText('login-tab-wechat', t('tabWeChat'));
  setText('login-tab-email', t('tabEmail'));
  setText('security-title', t('userDeclarationTitle'));
  setText('security-agree', t('agreeAndContinue'));
  setText('security-cancel', t('cancel'));
  setText('security-body', USER_DECLARATION[locale] ?? USER_DECLARATION.zh);
  refreshLoginWindowAria();
}

function refreshLoginWindowAria() {
  const map = {
    'login-win-min': 'window.minimize',
    'login-win-max': 'window.maximize',
    'login-win-close': 'window.close',
  };
  for (const [id, key] of Object.entries(map)) {
    const el = $(id);
    if (!el) continue;
    const label = window.__appI18n?.t?.(key) ?? (
      key === 'window.minimize' ? (locale === 'en' ? 'Minimize' : '最小化')
        : key === 'window.maximize' ? (locale === 'en' ? 'Maximize' : '最大化')
        : (locale === 'en' ? 'Close' : '关闭')
    );
    el.setAttribute('aria-label', label);
  }
}

export function initLoginUI(complete) {
  onComplete = complete;

  applyLoginTheme();
  $('#login-screen')?.classList.remove('hidden');

  const inviteFromUrl = new URLSearchParams(location.search).get('inviteCode')?.trim() || '';
  if (inviteFromUrl && $('#login-invite')) $('#login-invite').value = inviteFromUrl;

  mode = getDefaultLoginMode();
  showMode(mode);

  $('#login-tab-email')?.addEventListener('click', () => showMode('email'));
  $('#login-tab-wechat')?.addEventListener('click', () => showMode('wechat'));
  $('#login-send-code')?.addEventListener('click', sendCodeAction);
  $('#login-submit')?.addEventListener('click', emailLoginAction);

  bindSecurityDialog();
  bindWeChatListener();

  applyLoginI18n();
  document.addEventListener('app-locale-change', applyLoginI18n);
}
