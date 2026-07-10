import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { useLiveUptime } from '../../hooks/useLiveUptime'
import { Link, useRouterState } from '@tanstack/react-router'
import { useCreditsQuery, useCreditsUsageQuery } from '../../queries/flowy'
import { useSaveSetupMutation } from '../../queries/gateway'
import { useAppStore } from '../../stores/appStore'
import { useAuthStore } from '../../stores/authStore'
import { useSetupStore } from '../../stores/setupStore'
import { useEdgeStore } from '../../stores/edgeStore'
import { useCloudStore } from '../../stores/cloudStore'
import { useI18n } from '../../hooks/useI18n'
import { fmtNum, formatSavedCredits, formatUptime, sidebarTokenShares, tierTokenTotal } from '../../lib/stats-utils'
import { getEdition } from '../../lib/flowy/server'
import { getAuthToken } from '../../stores/authStore'
import { resolveEdgeModelLabel, isEdgeUpstreamConfigured } from '../../lib/edge-upstream'
import { resolveCloudModelLabel } from '../../lib/cloud-upstream'
import type { RouteMode } from '../../types/gateway'
import { openExternalUrl } from '../../lib/open-external'

const CREDIT_TYPE_ORDER = ['DAILY_CHECKIN', 'PLAN', 'PACK', 'SIGNUP', 'TEAM_SEAT', 'OTHER']

const NAV_CARDS = [
  { page: 'overview', navId: 'overview', wide: true, accent: '' },
  { page: 'stats', navId: 'stats', wide: true, accent: '' },
  { page: 'upstream', navId: 'edge', wide: false, accent: 'accent-edge' },
  { page: 'upstream', navId: 'cloud', wide: false, accent: 'accent-cloud' },
  { page: 'routing', navId: 'routing', wide: false, accent: '' },
  { page: 'logs', navId: 'logs', wide: false, accent: '' },
  { page: 'settings', navId: 'settings', wide: false, accent: '' },
] as const

function navTo(page: string, navId: string): { to: '/upstream/$navId'; params: { navId: string } } | { to: '/overview' | '/stats' | '/routing' | '/agents' | '/logs' | '/settings' } {
  if (page === 'upstream') return { to: '/upstream/$navId', params: { navId } }
  return { to: `/${page}` as '/overview' | '/stats' | '/routing' | '/agents' | '/logs' | '/settings' }
}

function pickNickname(user: Record<string, unknown> | null | undefined) {
  if (!user) return ''
  return String(user.nickName || user.nickname || user.name || user.username || user.email || '')
}

function pickAvatar(user: Record<string, unknown> | null | undefined) {
  if (!user) return ''
  return String(user.avatar || user.headImg || user.head_img || user.avatarUrl || '')
}

export function Sidebar() {
  const { t, locale } = useI18n()
  const routeTab = useAppStore((s) => s.routeTab)
  const setRouteTab = useAppStore((s) => s.setRouteTab)
  const status = useAppStore((s) => s.status)
  const stats = useAppStore((s) => s.stats)
  const globalStats = useAppStore((s) => s.globalStats)
  const globalSavedPoints = useAppStore((s) => s.globalSavedPoints)
  const liveUptime = useLiveUptime()
  const userInfo = useAuthStore((s) => s.userInfo)
  const logout = useAuthStore((s) => s.logout)
  const setup = useSetupStore((s) => s.setup)
  const herdsmanConnected = useEdgeStore((s) => s.herdsmanConnected)
  const edgeSelectedKey = useEdgeStore((s) => s.selectedKey)
  const cachedModels = useEdgeStore((s) => s.cachedModels)
  const cloudSelectedKey = useCloudStore((s) => s.selectedKey)
  const cloudFlowyModels = useCloudStore((s) => s.flowyModels)
  const saveSetup = useSaveSetupMutation()

  const pathname = useRouterState({ select: (s) => s.location.pathname })
  const activeNavId = useMemo(() => {
    const m = pathname.match(/^\/upstream\/(\w+)/)
    if (m) return m[1]
    const seg = pathname.replace(/^\//, '').split('/')[0]
    return seg || 'overview'
  }, [pathname])

  const { data: credits = 0 } = useCreditsQuery()
  const [popoverVisible, setPopoverVisible] = useState(false)
  const [userMenuOpen, setUserMenuOpen] = useState(false)
  const [cooldownUntil, setCooldownUntil] = useState(0)
  const hideTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const userMenuRef = useRef<HTMLDivElement>(null)

  const usageQuery = useCreditsUsageQuery()
  const usageRows = useMemo(() => {
    const list = Array.isArray((usageQuery.data as { list?: { type: string; remaining?: number; title?: string }[] })?.list)
      ? (usageQuery.data as { list: { type: string; remaining?: number; title?: string }[] }).list
      : []
    const typeSet = new Set(list.map((i) => i.type))
    return CREDIT_TYPE_ORDER.filter((type) => typeSet.has(type)).map((type) => {
      const item = list.find((i) => i.type === type)
      const remaining = typeof item?.remaining === 'number' && item.remaining >= 0 ? item.remaining : 0
      const i18nKey = `creditType.${type}`
      const translated = t(i18nKey)
      const label = translated !== i18nKey ? translated : (item?.title?.trim() || type)
      return { type, label, remaining }
    })
  }, [usageQuery.data, t])

  const cooldownSec = Math.max(0, Math.ceil((cooldownUntil - Date.now()) / 1000))

  const tb = (globalStats?.token_breakdown ?? stats?.token_breakdown) as Record<string, unknown> | undefined
  const sidebarStats = globalStats ?? stats
  const shares = sidebarTokenShares(tb)
  const edgeConfigured = useMemo(
    () => isEdgeUpstreamConfigured(setup?.edge),
    [setup?.edge, herdsmanConnected, edgeSelectedKey, cachedModels],
  )
  const edgeModelLabel = useMemo(
    () => resolveEdgeModelLabel(setup?.edge),
    [setup?.edge, herdsmanConnected, edgeSelectedKey, cachedModels],
  )
  const cloudModelLabel = useMemo(
    () => resolveCloudModelLabel(setup?.cloud),
    [setup?.cloud, cloudSelectedKey, cloudFlowyModels],
  )
  const cloudConfigured = setup?.cloud?.configured || status?.cloud_configured

  const onRouteTab = (route: RouteMode) => {
    setRouteTab(route)
    if (useAppStore.getState().connected) {
      saveSetup.mutate({ gateway: { route } })
    }
  }

  const openExternal = useCallback(async (url: string) => {
    try {
      await openExternalUrl(url)
    } catch (e) {
      console.warn('[sidebar] openUrl', e)
    }
  }, [])

  const openBilling = useCallback(() => {
    const edition = getEdition()
    const host = edition === 'international' ? 'flowyaipc.com' : 'flowyaipc.cn'
    const lang = locale === 'en' ? 'en' : 'zh'
    const token = getAuthToken()
    const q = token ? `?token=${encodeURIComponent(token)}&language=${lang}` : `?language=${lang}`
    const url = `https://${host}/${q}#profile?tab=records`
    void openExternal(url)
  }, [locale, openExternal])

  const openPayment = useCallback(() => {
    const edition = getEdition()
    const host = edition === 'international' ? 'flowyaipc.com' : 'flowyaipc.cn'
    const lang = locale === 'en' ? 'en' : 'zh'
    const token = getAuthToken()
    const base = `https://${host}/#pricing`
    const url = token ? `${base}?token=${encodeURIComponent(token)}&language=${lang}` : `${base}?language=${lang}`
    void openExternal(url)
  }, [locale, openExternal])

  const nickname = pickNickname(userInfo as Record<string, unknown>)
  const avatar = pickAvatar(userInfo as Record<string, unknown>)
  const [avatarFailed, setAvatarFailed] = useState(false)

  useEffect(() => {
    setAvatarFailed(false)
  }, [avatar])

  useEffect(() => {
    const onMouseDown = (e: MouseEvent) => {
      if (!userMenuRef.current?.contains(e.target as Node)) setUserMenuOpen(false)
    }
    document.addEventListener('mousedown', onMouseDown)
    return () => document.removeEventListener('mousedown', onMouseDown)
  }, [])

  const handleLogout = () => {
    setUserMenuOpen(false)
    logout()
    location.reload()
  }

  const showPopover = () => {
    if (hideTimer.current) clearTimeout(hideTimer.current)
    setPopoverVisible(true)
    if (!usageRows.length && !usageQuery.isFetching) void usageQuery.refetch()
  }

  const scheduleHide = () => {
    hideTimer.current = setTimeout(() => setPopoverVisible(false), 160)
  }

  const refreshUsage = () => {
    if (cooldownSec > 0 || usageQuery.isFetching) return
    setCooldownUntil(Date.now() + 5000)
    void usageQuery.refetch()
  }

  const profileLabel = setup?.gateway?.default_profile
    ? t(`profile.${setup.gateway.default_profile}`)
    : t('profile.balanced')

  return (
    <aside className="sider" id="sider">
      <div className="sider-header window-drag">
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
      </div>

      <div className="sider-scroll">
        <div className="route-tabs-card">
          <div className="route-tabs-title">{t('route.switch')}</div>
          <div className="route-tabs" id="route-tabs">
            {(['auto', 'edge', 'cloud', 'cascade'] as RouteMode[]).map((route) => (
              <button
                key={route}
                type="button"
                className={`route-tab${routeTab === route ? ' active' : ''}`}
                data-route={route}
                onClick={() => onRouteTab(route)}
              >
                {t(`route.${route}`)}
              </button>
            ))}
          </div>
        </div>

        <div className="card-grid" id="card-grid">
        {NAV_CARDS.map((card) => {
          const active = activeNavId === card.navId
          const link = navTo(card.page, card.navId)
          const title = t(`nav.${card.navId === 'overview' ? 'gateway' : card.navId === 'stats' ? 'routeStats' : card.navId}`)
          return (
            <Link
              key={card.navId}
              {...link}
              id={`nav-card-${card.navId}`}
              className={`nav-card${card.wide ? ' nav-card-wide' : ''}${card.accent ? ` ${card.accent}` : ''}${active ? ' active' : ''}`}
              data-page={card.page}
              data-nav-id={card.navId}
              title={title}
            >
              <span className="nav-card-only" aria-hidden="true">
                <NavCardIcon navId={card.navId} />
              </span>
              <div className="nav-card-expand">
                <NavCardBody navId={card.navId} status={status} sidebarStats={sidebarStats} savedPoints={globalSavedPoints} shares={shares} tb={tb} edgeConfigured={!!edgeConfigured} cloudConfigured={!!cloudConfigured} setup={setup} profileLabel={profileLabel} liveUptime={liveUptime} t={t} locale={locale} />
                <NavCardFoot navId={card.navId} status={status} sidebarStats={sidebarStats} edgeConfigured={!!edgeConfigured} cloudConfigured={!!cloudConfigured} setup={setup} profileLabel={profileLabel} routeTab={routeTab} edgeModelLabel={edgeModelLabel} cloudModelLabel={cloudModelLabel} t={t} locale={locale} />
              </div>
            </Link>
          )
        })}
        </div>
      </div>

      <footer className="sider-footer no-drag" id="sider-footer">
        <div className="sider-credits-row">
          <div
            className="sider-credits-main"
            id="sider-credits-main"
            onMouseEnter={showPopover}
            onMouseLeave={scheduleHide}
          >
            <div className="sider-credits-hover" id="sider-credits-hover">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
                <circle cx="12" cy="12" r="8" />
                <path d="M12 6v2" />
                <path d="M12 16v2" />
                <path d="M9 9.5h4.5a2 2 0 110 4H9" />
              </svg>
              <span id="sider-credits-value">{credits.toLocaleString()}</span>
            </div>
            <div className="sider-credits-popover" id="sider-credits-popover" hidden={!popoverVisible} onMouseEnter={showPopover} onMouseLeave={scheduleHide}>
              <div id="sider-credits-breakdown">
                {usageQuery.isFetching ? (
                  <p className="sider-credits-popover-hint">{t('sidebar.loading')}</p>
                ) : usageQuery.isError ? (
                  <p className="sider-credits-popover-hint">{t('sidebar.failed')}</p>
                ) : !usageRows.length ? (
                  <p className="sider-credits-popover-hint">{t('sidebar.empty')}</p>
                ) : (
                  usageRows.map((row) => (
                    <div key={row.type} className="sider-credits-breakdown-row">
                      <span className="sider-credits-breakdown-label">{row.label}</span>
                      <span className="sider-credits-breakdown-value">{row.remaining.toLocaleString()}</span>
                    </div>
                  ))
                )}
              </div>
              <div className="sider-credits-popover-foot">
                <button type="button" className="sider-credits-popover-link" id="sider-credits-view-billing" onClick={() => void openBilling()}>
                  {t('sidebar.viewBilling')}
                </button>
                <button
                  type="button"
                  className="sider-credits-popover-refresh"
                  id="sider-credits-popover-refresh"
                  disabled={cooldownSec > 0 || usageQuery.isFetching}
                  onClick={(e) => {
                    e.stopPropagation()
                    refreshUsage()
                  }}
                >
                  {cooldownSec > 0 ? `${cooldownSec}s` : t('action.refresh')}
                </button>
              </div>
            </div>
          </div>
          <button type="button" className="sider-credits-more" id="sider-credits-more" onClick={() => void openPayment()}>
            <span id="sider-credits-more-label">{t('sidebar.getMoreCredits')}</span>
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
              <path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6" />
              <path d="M15 3h6v6" />
              <path d="M10 14L21 3" />
            </svg>
          </button>
        </div>
        <div className="sider-user-main" id="sider-user-main" ref={userMenuRef}>
          <button
            type="button"
            className="sider-user-row"
            id="sider-user-row"
            aria-haspopup="menu"
            aria-expanded={userMenuOpen}
            onClick={(e) => {
              e.stopPropagation()
              setUserMenuOpen(!userMenuOpen)
            }}
          >
            <div className="sider-user-avatar" id="sider-user-avatar">
              {avatar && !avatarFailed ? (
                <img id="sider-user-avatar-img" alt={nickname || 'User'} src={avatar} onError={() => setAvatarFailed(true)} />
              ) : (
                <span id="sider-user-avatar-fallback">{(nickname || '?').trim().slice(0, 1).toUpperCase() || '?'}</span>
              )}
            </div>
            <div className="sider-user-meta">
              <span className="sider-user-name" id="sider-user-name">{nickname}</span>
            </div>
          </button>
          <div className="sider-user-menu" id="sider-user-menu" role="menu" hidden={!userMenuOpen}>
            <button type="button" className="sider-user-menu-item" id="sider-user-logout" role="menuitem" onClick={handleLogout}>
              {t('action.logout')}
            </button>
          </div>
        </div>
      </footer>
    </aside>
  )
}

function NavCardIcon({ navId }: { navId: string }) {
  const icons: Record<string, ReactNode> = {
    overview: <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 2L2 7l10 5 10-5-10-5z" /><path d="M2 17l10 5 10-5" /><path d="M2 12l10 5 10-5" /></svg>,
    stats: <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 3v18h18" /><path d="M7 16l4-8 4 4 5-9" /></svg>,
    edge: <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="4" width="18" height="12" rx="2" /><path d="M8 20h8" /><path d="M12 16v4" /></svg>,
    cloud: <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 18h11a4 4 0 010-8 5 5 0 00-9.9-1.1A4 4 0 007 18z" /></svg>,
    routing: <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="6" cy="6" r="2.5" /><circle cx="18" cy="18" r="2.5" /><path d="M8.5 7.5l7 7" /></svg>,
    agents: <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M16 21v-2a4 4 0 00-4-4H6a4 4 0 00-4 4v2" /><circle cx="9" cy="7" r="3.5" /><path d="M22 21v-2a4 4 0 00-3-3.87" /><path d="M16 3.13a4 4 0 010 7.75" /></svg>,
    logs: <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" /><path d="M14 2v6h6" /><path d="M8 13h8" /><path d="M8 17h5" /><path d="M8 9h2" /></svg>,
    settings: <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" /><circle cx="12" cy="12" r="3" /></svg>,
  }
  return icons[navId] ?? icons.overview
}

function NavCardBody({
  navId, status, sidebarStats, savedPoints, shares, tb, edgeConfigured, cloudConfigured, setup, profileLabel: _profileLabel, liveUptime, t, locale,
}: {
  navId: string
  status: ReturnType<typeof useAppStore.getState>['status']
  sidebarStats: ReturnType<typeof useAppStore.getState>['stats']
  savedPoints: number | null
  shares: { edgePct: number; cloudPct: number }
  tb: Record<string, unknown> | undefined
  edgeConfigured: boolean
  cloudConfigured: boolean
  setup: ReturnType<typeof useSetupStore.getState>['setup']
  profileLabel: string
  liveUptime: number
  t: (k: string, v?: Record<string, string | number>) => string
  locale: string
}) {
  if (navId === 'overview') {
    return (
      <div className="nav-card-body nav-card-body-wide">
        <div className="nav-card-head">
          <span className="nav-card-stat" id="card-gateway-state">
            {status?.status === 'running' ? formatUptime(liveUptime, t) : '—'}
          </span>
        </div>
        <div className="nav-card-sub" id="card-gateway-sub">{status?.listen ?? ''}</div>
      </div>
    )
  }
  if (navId === 'stats') {
    return (
      <div className="nav-card-body nav-card-body-wide">
        <div className="nav-card-head">
          <span className="nav-card-stat" id="card-saved">
            {formatSavedCredits(savedPoints, t)}
          </span>
        </div>
        <div className="progress-track sider-token-track">
          <div className="sider-token-edge" id="bar-edge" style={{ width: `${shares.edgePct}%` }} />
          <div className="sider-token-cloud" id="bar-cloud" style={{ width: `${shares.cloudPct}%` }} />
        </div>
        <div className="nav-card-sub" id="card-route-sub">
          {t('routeStats.siderTokens', { edge: fmtNum(tierTokenTotal(tb?.edge as Record<string, unknown>), locale), cloud: fmtNum(tierTokenTotal(tb?.cloud as Record<string, unknown>), locale) })}
        </div>
      </div>
    )
  }
  if (navId === 'edge' || navId === 'cloud' || navId === 'routing' || navId === 'agents') {
    return (
      <div className="nav-card-body">
        <div className="nav-card-top">
          <div className="nav-card-icon"><NavCardIcon navId={navId} /></div>
          {navId === 'edge' && (
            <span className={`tag bordered ${edgeConfigured ? 'ok' : 'off'} nav-card-corner`} id="chip-edge">
              <span className="dot" />
              <span>{edgeConfigured ? t('status.configured') : t('status.notConfigured')}</span>
            </span>
          )}
          {navId === 'cloud' && (
            <span className={`tag bordered ${cloudConfigured ? 'ok' : 'off'} nav-card-corner`} id="chip-cloud">
              <span className="dot" />
              <span>{cloudConfigured ? t('status.configured') : t('status.notConfigured')}</span>
            </span>
          )}
          {navId === 'routing' && (
            <span className="tag bordered nav-card-corner" id="card-route-tag">{t(`route.${setup?.gateway?.route ?? 'auto'}`)}</span>
          )}
          {navId === 'agents' && (
            <span className="tag bordered nav-card-corner" id="card-agents">{sidebarStats?.agent_budgets?.length ?? 0}</span>
          )}
        </div>
      </div>
    )
  }
  return (
    <div className="nav-card-body">
      <div className="nav-card-top">
        <div className="nav-card-icon"><NavCardIcon navId={navId} /></div>
      </div>
    </div>
  )
}

function NavCardFoot({
  navId, status, sidebarStats, edgeConfigured: _edgeConfigured, cloudConfigured: _cloudConfigured, setup: _setup, profileLabel, routeTab: _routeTab, edgeModelLabel, cloudModelLabel, t, locale: _locale,
}: {
  navId: string
  status: ReturnType<typeof useAppStore.getState>['status']
  sidebarStats: ReturnType<typeof useAppStore.getState>['stats']
  edgeConfigured: boolean
  cloudConfigured: boolean
  setup: ReturnType<typeof useSetupStore.getState>['setup']
  profileLabel: string
  routeTab: string
  edgeModelLabel: string
  cloudModelLabel: string
  t: (k: string, v?: Record<string, string | number>) => string
  locale: string
}) {
  return (
    <div className="nav-card-foot">
      <span className="nav-card-title">{t(`nav.${navId === 'overview' ? 'gateway' : navId === 'stats' ? 'routeStats' : navId}`)}</span>
      {navId === 'overview' && (
        <span className={`tag bordered ${status?.status === 'running' ? 'ok' : 'off'}`} id="card-gateway-status">
          <span className={`dot${status?.status === 'running' ? ' pulse' : ''}`} />
          <span>{status?.status === 'running' ? t('status.running') : t('status.stopped')}</span>
        </span>
      )}
      {navId === 'stats' && (
        <span className="tag bordered" id="card-req-total">{t('requests', { n: sidebarStats?.requests_total ?? 0 })}</span>
      )}
      {navId === 'edge' && (
        <span className="nav-card-meta" id="card-edge-model">{edgeModelLabel}</span>
      )}
      {navId === 'cloud' && (
        <span className="nav-card-meta" id="card-cloud-model">
          {cloudModelLabel || '—'}
        </span>
      )}
      {navId === 'routing' && (
        <span className="nav-card-meta" id="card-profile">{profileLabel}</span>
      )}
    </div>
  )
}
