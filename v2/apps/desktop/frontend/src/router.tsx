import {
  createRootRoute,
  createRoute,
  createRouter,
  redirect,
} from '@tanstack/react-router'
import { createHashHistory } from '@tanstack/react-router'
import { AppLayout } from './components/layout/AppLayout'
import { OverviewPage } from './pages/OverviewPage'
import { AppsPage } from './pages/AppsPage'
import { ProvidersPage } from './pages/ProvidersPage'
import { QuotasPage } from './pages/QuotasPage'
import { PlaygroundPage } from './pages/PlaygroundPage'
import { VirtualModelsPage } from './pages/VirtualModelsPage'
import { RequestsPage } from './pages/RequestsPage'
import { SessionsPage } from './pages/SessionsPage'
import { StatsPage } from './pages/StatsPage'
import { LogsPage } from './pages/LogsPage'
import { GatewayPage } from './pages/GatewayPage'
import { TokensPage } from './pages/TokensPage'
import { DiagnosticsPage } from './pages/DiagnosticsPage'
import { OnboardingPage } from './pages/OnboardingPage'
import { SettingsPage } from './pages/SettingsPage'

const rootRoute = createRootRoute({
  component: AppLayout,
})

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/',
  beforeLoad: () => {
    throw redirect({ to: '/apps' })
  },
})

const routeTree = rootRoute.addChildren([
  indexRoute,
  createRoute({ getParentRoute: () => rootRoute, path: '/overview', component: OverviewPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/apps', component: AppsPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/providers', component: ProvidersPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/quotas', component: QuotasPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/playground', component: PlaygroundPage }),
  // 旧「高级：候选模型」入口已删除；候选只由供应商页编译，旧链接回落到路由组页。
  createRoute({
    getParentRoute: () => rootRoute,
    path: '/models',
    beforeLoad: () => {
      throw redirect({ to: '/virtual-models' })
    },
  }),
  createRoute({ getParentRoute: () => rootRoute, path: '/virtual-models', component: VirtualModelsPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/requests', component: RequestsPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/sessions', component: SessionsPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/stats', component: StatsPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/logs', component: LogsPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/gateway', component: GatewayPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/tokens', component: TokensPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/diagnostics', component: DiagnosticsPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/onboarding', component: OnboardingPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/settings', component: SettingsPage }),
])

export const router = createRouter({
  routeTree,
  history: createHashHistory(),
})

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}
