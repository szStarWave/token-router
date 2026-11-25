import { RouterProvider } from '@tanstack/react-router'
import { router } from './router'
import { ExitGuard } from './components/settings/ExitGuard'

export default function App() {
  return (
    <>
      <RouterProvider router={router} />
      <ExitGuard />
    </>
  )
}
