import { useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { BRAND } from '../../lib/flowy/config'
import {
  getWeChatFlowyServerBase,
  isWeChatLoginEnabled,
  getDefaultLoginMode,
} from '../../lib/flowy/server'
import {
  sendEmailCode,
  loginByEmail,
  loginByWeChatCallback,
  loginByToken,
  deviceActivateAfterLogin,
} from '../../lib/flowy/api'
import { useAuthStore } from '../../stores/authStore'
import { useI18n, useTheme } from '../../hooks/useI18n'
import { TitleBar } from '../layout/TitleBar'
import { LoginToast } from '../common/Toast'
import { isTauri } from '../../lib/tauri'

declare global {
  interface Window {
    WxLogin?: new (opts: Record<string, unknown>) => void
  }
}

const WECHAT_STYLE_DARK =
  'data:text/css;base64,LmltcG93ZXJCb3ggLnRpdGxlIHsKICBkaXNwbGF5OiBub25lICFpbXBvcnRhbnQ7Cn0KLmltcG93ZXJCb3ggLmluZm8gewogIGRpc3BsYXk6IG5vbmUgIWltcG9ydGFudDsKfQoKLmltcG93ZXJCb3ggLnFyY29kZXsKICAgd2lkdGg6IC13ZWJraXQtZmlsbC1hdmFpbGFibGU7CiAgIG1heC13aWR0aDogMjgwcHg7CiAgIG1hcmdpbi10b3A6MHB4Owp9Ci5pbXBvd2VyQm94IHsKICBoZWlnaHQ6IDMwMHB4ICFpbXBvcnRhbnQ7CiAgb3ZlcmZsb3c6IGhpZGRlbiAhaW1wb3J0YW50Owp9Cmh0bWwgewogICAgYmFja2dyb3VuZDogIzFkMWQxZDAwICFpbXBvcnRhbnQ7Cn0KYm9keXsKICAgIGJhY2tncm91bmQ6ICMxZDFkMWQwMCAhaW1wb3J0YW50Owp9'
const WECHAT_STYLE_LIGHT =
  'data:text/css;base64,LmltcG93ZXJCb3ggLnRpdGxlIHsKICBkaXNwbGF5OiBub25lICFpbXBvcnRhbnQ7Cn0KLmltcG93ZXJCb3ggLmluZm8gewogIGRpc3BsYXk6IG5vbmUgIWltcG9ydGFudDsKfQoKLmltcG93ZXJCb3ggLnFyY29kZXsKICAgd2lkdGg6IC13ZWJraXQtZmlsbC1hdmFpbGFibGU7CiAgIG1heC13aWR0aDogMjgwcHg7CiAgIG1hcmdpbi10b3A6MHB4Owp9Ci5pbXBvd2VyQm94IHsKICBoZWlnaHQ6IDMwMHB4ICFpbXBvcnRhbnQ7CiAgb3ZlcmZsb3c6IGhpZGRlbiAhaW1wb3J0YW50Owp9Cmh0bWwgewogICAgYmFja2dyb3VuZDogI2ZjZmNmYzAwICFpbXBvcnRhbnQ7Cn0KYm9keXsKICAgIGJhY2tncm91bmQ6ICNmY2ZjZmMwMCAhaW1wb3J0YW50Owp9'

const USER_DECLARATION = {
  zh: '您使用本产品的前提是，您承认和接受用户声明的全部内容。AI 生成内容仅供参考，请独立核查。继续使用即表示您已阅读并理解相关风险。',
  en: 'By using this product you acknowledge and accept the user agreement. AI-generated content is for reference only; verify independently. Continuing means you have read and understood the risks.',
}

interface LoginScreenProps {
  onComplete: () => void
}

export function LoginScreen({ onComplete }: LoginScreenProps) {
  const { locale, t } = useI18n()
  const { effectiveTheme, applyTheme } = useTheme()
  const login = useAuthStore((s) => s.login)
  const hasAgreed = useAuthStore((s) => s.hasAgreedToUserDeclaration)
  const setAgreed = useAuthStore((s) => s.setHasAgreedToUserDeclaration)

  const wechatEnabled = isWeChatLoginEnabled()
  const [mode, setMode] = useState<'wechat' | 'email'>(getDefaultLoginMode())
  const [email, setEmail] = useState('')
  const [code, setCode] = useState('')
  const [invite, setInvite] = useState(() => new URLSearchParams(location.search).get('inviteCode')?.trim() || '')
  const [emailReqNo, setEmailReqNo] = useState<string | null>(null)
  const [codeSec, setCodeSec] = useState(0)
  const [submitting, setSubmitting] = useState(false)
  const [toast, setToast] = useState({ message: '', ok: false })
  const [securityOpen, setSecurityOpen] = useState(false)
  const pendingLogin = useRef<{ token: string; userInfo: unknown } | null>(null)
  const wechatProcessing = useRef(false)
  const qrRef = useRef<HTMLDivElement>(null)

  const showToast = (message: string, ok = false) => {
    setToast({ message, ok })
    setTimeout(() => setToast({ message: '', ok: false }), 3200)
  }

  useEffect(() => {
    applyTheme()
  }, [applyTheme])

  const finishLogin = async (token: string, userInfo: unknown) => {
    login(token, userInfo as import('../../stores/authStore').UserInfo)
    void deviceActivateAfterLogin(token)
    onComplete()
  }

  const handleToken = async (token: string, userInfo: unknown) => {
    if (!hasAgreed) {
      pendingLogin.current = { token, userInfo }
      setSecurityOpen(true)
      showToast(t('action.confirm'), true)
      return
    }
    await finishLogin(token, userInfo)
  }

  const waitForWxLogin = (maxMs = 8000) =>
    new Promise<boolean>((resolve) => {
      if (window.WxLogin) {
        resolve(true)
        return
      }
      const start = Date.now()
      const tick = () => {
        if (window.WxLogin) {
          resolve(true)
          return
        }
        if (Date.now() - start >= maxMs) {
          resolve(false)
          return
        }
        requestAnimationFrame(tick)
      }
      requestAnimationFrame(tick)
    })

  const mountWeChatQr = async () => {
    const container = qrRef.current
    if (!container) return
    container.innerHTML = ''
    const ready = await waitForWxLogin()
    if (!ready) {
      container.innerHTML = `<p class="login-hint">${t('login.wechatLoadFail')}</p>`
      return
    }
    const theme = effectiveTheme()
    const redirectUri = encodeURIComponent(`${getWeChatFlowyServerBase()}/auth/third/callback?platform=WECHAT`)
    try {
      new window.WxLogin!({
        self_redirect: true,
        id: 'wechat-qr-container',
        appid: BRAND.wechatAppId,
        scope: 'snsapi_login',
        redirect_uri: redirectUri,
        state: `wechat_login_${Date.now()}`,
        style: 'black',
        href: theme === 'dark' ? WECHAT_STYLE_DARK : WECHAT_STYLE_LIGHT,
      })
    } catch (e) {
      console.error('WxLogin init failed', e)
      container.innerHTML = `<p class="login-hint">${t('login.wechatLoadFail')}</p>`
    }
  }

  useEffect(() => {
    if (mode === 'wechat' && wechatEnabled) void mountWeChatQr()
  }, [mode, wechatEnabled, locale])

  useEffect(() => {
    if (!isTauri()) return
    const unlisten = listen<string>('wechat-login-callback', (event) => {
      const url = event.payload
      if (typeof url === 'string' && url) void processWeChatCallback(url)
    })
    return () => {
      void unlisten.then((fn) => fn())
    }
  }, [])

  const processWeChatCallback = async (callbackUrl: string) => {
    if (wechatProcessing.current) return
    wechatProcessing.current = true
    try {
      const params = new URLSearchParams()
      params.set('channel', BRAND.id)
      if (invite.trim()) params.set('inviteCode', invite.trim())
      const sep = callbackUrl.includes('?') ? '&' : '?'
      const url = `${callbackUrl}${sep}${params.toString()}`
      const token = await loginByWeChatCallback(url)
      let userInfo = null
      try {
        userInfo = await loginByToken(token)
      } catch {
        /* optional */
      }
      await handleToken(token, userInfo)
    } catch (e) {
      showToast(e instanceof Error ? e.message : t('login.fail'))
      if (mode === 'wechat') void mountWeChatQr()
    } finally {
      wechatProcessing.current = false
    }
  }

  const validateEmail = (v: string) => /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(v)

  const sendCode = async () => {
    if (!email.trim()) {
      showToast(t('login.emailTips'))
      return
    }
    if (!validateEmail(email)) {
      showToast(t('login.emailInvalid'))
      return
    }
    if (codeSec > 0) return
    try {
      const reqNo = await sendEmailCode(email.trim())
      setEmailReqNo(reqNo)
      setCodeSec(60)
    } catch (e) {
      showToast(e instanceof Error ? e.message : t('login.sendCodeFail'))
    }
  }

  useEffect(() => {
    if (codeSec <= 0) return
    const tmr = setInterval(() => setCodeSec((s) => (s <= 1 ? 0 : s - 1)), 1000)
    return () => clearInterval(tmr)
  }, [codeSec])

  const emailLogin = async () => {
    if (!email.trim()) {
      showToast(t('login.emailTips'))
      return
    }
    if (!code.trim()) {
      showToast(t('login.codeTips'))
      return
    }
    if (!emailReqNo) {
      showToast(t('login.sendCodeTips'))
      return
    }
    setSubmitting(true)
    try {
      const token = await loginByEmail(email.trim(), code.trim(), emailReqNo, invite.trim())
      let userInfo = null
      try {
        userInfo = await loginByToken(token)
      } catch {
        /* optional */
      }
      await handleToken(token, userInfo)
    } catch (e) {
      showToast(e instanceof Error ? e.message : t('login.fail'))
    } finally {
      setSubmitting(false)
    }
  }

  const loginT = (key: string) => {
    const map: Record<string, string> = {
      emailTitle: locale === 'zh' ? '邮箱登录' : 'Email login',
      wechatTitle: locale === 'zh' ? '微信登录' : 'WeChat login',
      wechatHint: locale === 'zh' ? '请使用微信扫描二维码登录' : 'Scan the QR code with WeChat',
      wechatLoadFail: locale === 'zh' ? '微信登录加载失败，请刷新重试' : 'Failed to load WeChat login. Please refresh.',
      emailPlaceholder: locale === 'zh' ? '请输入邮箱' : 'Email',
      codePlaceholder: locale === 'zh' ? '请输入验证码' : 'Verification code',
      inviteCodePlaceholder: locale === 'zh' ? '请输入邀请码（选填）' : 'Invite code (optional)',
      getCode: locale === 'zh' ? '获取验证码' : 'Get code',
      login: locale === 'zh' ? '登录' : 'Sign in',
      loggingIn: locale === 'zh' ? '登录中...' : 'Signing in...',
      emailTips: locale === 'zh' ? '请输入邮箱' : 'Enter your email',
      emailInvalid: locale === 'zh' ? '请输入有效的邮箱地址' : 'Enter a valid email',
      codeTips: locale === 'zh' ? '请输入验证码' : 'Enter verification code',
      sendCodeTips: locale === 'zh' ? '请先发送验证码' : 'Send verification code first',
      sendCodeFail: locale === 'zh' ? '验证码发送失败' : 'Failed to send code',
      tabWeChat: locale === 'zh' ? '微信扫码登录' : 'WeChat QR',
      tabEmail: locale === 'zh' ? '邮箱登录' : 'Email',
      fail: locale === 'zh' ? '登录失败' : 'Login failed',
    }
    return map[key] ?? key
  }

  return (
    <>
      <div id="login-screen" className="login-screen">
        <header className="login-screen-header window-drag">
          <div className="brand">
            <div className="brand-icon">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M12 2L2 7l10 5 10-5-10-5z" />
                <path d="M2 17l10 5 10-5" />
                <path d="M2 12l10 5 10-5" />
              </svg>
            </div>
            <h1>Token Router</h1>
          </div>
          <TitleBar prefix="login-" className="window-controls login-window-controls" />
        </header>
        <div className="login-screen-body">
          <div className="login-card">
            <div className="login-brand">
              <div className="login-brand-icon">
                <svg viewBox="0 0 24 24">
                  <path d="M12 2L2 7l10 5 10-5-10-5z" />
                  <path d="M2 17l10 5 10-5" />
                </svg>
              </div>
              <h2>Token Router</h2>
            </div>
            <div className="login-viewport">
              <div id="login-wechat-view" className={`login-view${mode === 'wechat' ? ' active' : ''}${wechatEnabled ? '' : ' hidden'}`}>
                <div className="login-title" id="login-wechat-title">{loginT('wechatTitle')}</div>
                <div className="login-wechat-wrap">
                  <div id="wechat-qr-container" ref={qrRef} />
                  <p className="login-hint" id="login-wechat-hint">{loginT('wechatHint')}</p>
                </div>
              </div>
              <div id="login-email-view" className={`login-view${mode === 'email' ? ' active' : ''}`}>
                <div className="login-title" id="login-email-title">{loginT('emailTitle')}</div>
                <div className="login-form">
                  <input
                    type="email"
                    id="login-email"
                    placeholder={loginT('emailPlaceholder')}
                    autoComplete="email"
                    value={email}
                    onChange={(e) => setEmail(e.target.value)}
                  />
                  <div className="login-code-row">
                    <input
                      type="text"
                      id="login-code"
                      placeholder={loginT('codePlaceholder')}
                      autoComplete="one-time-code"
                      value={code}
                      onChange={(e) => setCode(e.target.value)}
                    />
                    <button type="button" className="btn btn-ghost btn-sm" id="login-send-code" disabled={codeSec > 0} onClick={() => void sendCode()}>
                      {codeSec > 0 ? `${codeSec}s` : loginT('getCode')}
                    </button>
                  </div>
                  <button type="button" className="btn btn-primary login-submit" id="login-submit" disabled={submitting} onClick={() => void emailLogin()}>
                    {submitting ? loginT('loggingIn') : loginT('login')}
                  </button>
                </div>
              </div>
            </div>
            <input
              type="text"
              id="login-invite"
              className="login-invite"
              placeholder={loginT('inviteCodePlaceholder')}
              maxLength={16}
              autoComplete="off"
              value={invite}
              onChange={(e) => setInvite(e.target.value)}
            />
            <div className={`login-tabs${wechatEnabled ? '' : ' hidden'}`} id="login-tabs">
              <button type="button" className={`login-tab${mode === 'wechat' ? ' active' : ''}`} id="login-tab-wechat" onClick={() => setMode('wechat')}>
                {loginT('tabWeChat')}
              </button>
              <button type="button" className={`login-tab${mode === 'email' ? ' active' : ''}`} id="login-tab-email" onClick={() => setMode('email')}>
                {loginT('tabEmail')}
              </button>
            </div>
          </div>
        </div>
      </div>

      <div id="security-dialog" className={`security-dialog${securityOpen ? ' open' : ''}`}>
        <div className="security-panel">
          <h3 id="security-title">{locale === 'zh' ? '用户声明' : 'User agreement'}</h3>
          <div className="security-body" id="security-body">{USER_DECLARATION[locale] ?? USER_DECLARATION.zh}</div>
          <div className="security-actions">
            <button type="button" className="btn btn-ghost" id="security-cancel" onClick={() => { pendingLogin.current = null; setSecurityOpen(false) }}>
              {t('action.cancel')}
            </button>
            <button
              type="button"
              className="btn btn-primary"
              id="security-agree"
              onClick={() => {
                setAgreed(true)
                setSecurityOpen(false)
                if (pendingLogin.current) {
                  const { token, userInfo } = pendingLogin.current
                  pendingLogin.current = null
                  void finishLogin(token, userInfo)
                }
              }}
            >
              {locale === 'zh' ? '同意并继续' : 'Agree and continue'}
            </button>
          </div>
        </div>
      </div>

      <LoginToast message={toast.message} ok={toast.ok} />
    </>
  )
}
