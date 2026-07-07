import { RouterProvider } from '@tanstack/react-router'
import { router } from './router'
import { OtaModal } from './components/ota/OtaModal'
import { PostOtaNoticeModal } from './components/ota/PostOtaNoticeModal'
import { isOtaDesktop } from './lib/tauri'

export default function App() {
  return (
    <>
      <RouterProvider router={router} />
      {isOtaDesktop() && (
        <>
          <OtaModal />
          <PostOtaNoticeModal />
        </>
      )}
    </>
  )
}
