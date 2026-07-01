import {
  createRootRoute,
  createRoute,
  createRouter,
  redirect,
} from '@tanstack/react-router'
import { createHashHistory } from '@tanstack/react-router'
import { AuthGate } from './components/auth/AuthGate'
import { AppLayout } from './components/layout/AppLayout'
import { OverviewPage } from './pages/OverviewPage'
import { UpstreamPage } from './pages/UpstreamPage'
import { RoutingPage } from './pages/RoutingPage'
import { StatsPage } from './pages/StatsPage'
import { AgentsPage } from './pages/AgentsPage'
import { LogsPage } from './pages/LogsPage'
import { SettingsPage } from './pages/SettingsPage'

const rootRoute = createRootRoute({
  component: AuthGate,
})

const layoutRoute = createRoute({
  getParentRoute: () => rootRoute,
  id: 'layout',
  component: AppLayout,
})

const indexRoute = createRoute({
  getParentRoute: () => layoutRoute,
  path: '/',
  beforeLoad: () => {
    throw redirect({ to: '/overview' })
  },
})

const overviewRoute = createRoute({
  getParentRoute: () => layoutRoute,
  path: '/overview',
  component: OverviewPage,
})

const upstreamRoute = createRoute({
  getParentRoute: () => layoutRoute,
  path: '/upstream/$navId',
  component: UpstreamPage,
})

const routingRoute = createRoute({
  getParentRoute: () => layoutRoute,
  path: '/routing',
  component: RoutingPage,
})

const statsRoute = createRoute({
  getParentRoute: () => layoutRoute,
  path: '/stats',
  component: StatsPage,
})

const agentsRoute = createRoute({
  getParentRoute: () => layoutRoute,
  path: '/agents',
  component: AgentsPage,
})

const logsRoute = createRoute({
  getParentRoute: () => layoutRoute,
  path: '/logs',
  component: LogsPage,
})

const settingsRoute = createRoute({
  getParentRoute: () => layoutRoute,
  path: '/settings',
  component: SettingsPage,
})

const routeTree = rootRoute.addChildren([
  layoutRoute.addChildren([
    indexRoute,
    overviewRoute,
    upstreamRoute,
    routingRoute,
    statsRoute,
    agentsRoute,
    logsRoute,
    settingsRoute,
  ]),
])

export const router = createRouter({
  routeTree,
  history: createHashHistory(),
  defaultPreload: 'intent',
})

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}
