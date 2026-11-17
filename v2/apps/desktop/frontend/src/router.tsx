import {
  createRootRoute,
  createRoute,
  createRouter,
  redirect,
} from '@tanstack/react-router'
import { createHashHistory } from '@tanstack/react-router'
import { AppLayout } from './components/layout/AppLayout'
import { OverviewPage } from './pages/OverviewPage'
import { ModelsPage } from './pages/ModelsPage'
import { ProvidersPage } from './pages/ProvidersPage'
import { QuotasPage } from './pages/QuotasPage'
import { PlaygroundPage } from './pages/PlaygroundPage'
import { VirtualModelsPage } from './pages/VirtualModelsPage'
import { RequestsPage } from './pages/RequestsPage'
import { StatsPage } from './pages/StatsPage'
import { LogsPage } from './pages/LogsPage'
import { TokensPage } from './pages/TokensPage'
import { ApprovalsPage } from './pages/ApprovalsPage'
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
    throw redirect({ to: '/overview' })
  },
})

const routeTree = rootRoute.addChildren([
  indexRoute,
  createRoute({ getParentRoute: () => rootRoute, path: '/overview', component: OverviewPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/providers', component: ProvidersPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/quotas', component: QuotasPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/playground', component: PlaygroundPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/models', component: ModelsPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/virtual-models', component: VirtualModelsPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/requests', component: RequestsPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/stats', component: StatsPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/logs', component: LogsPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/tokens', component: TokensPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/approvals', component: ApprovalsPage }),
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
