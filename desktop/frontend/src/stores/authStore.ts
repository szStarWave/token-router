import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import { BRAND } from '../lib/flowy/config'

export interface UserInfo {
  nickName?: string
  avatar?: string
  email?: string
  [key: string]: unknown
}

interface AuthState {
  isLoggedIn: boolean
  authToken: string | null
  userInfo: UserInfo | null
  hasAgreedToUserDeclaration: boolean
  isSessionExpired: boolean
  login: (token: string, userInfo?: UserInfo | null) => void
  logout: () => void
  setSessionExpired: (expired: boolean) => void
  setHasAgreedToUserDeclaration: (agreed: boolean) => void
  updateUserInfo: (patch: Partial<UserInfo>) => void
}

const defaultState = {
  isLoggedIn: false,
  authToken: null as string | null,
  userInfo: null as UserInfo | null,
  hasAgreedToUserDeclaration: false,
  isSessionExpired: false,
}

export const useAuthStore = create<AuthState>()(
  persist(
    (set, get) => ({
      ...defaultState,
      login: (token, userInfo) => {
        set({
          isLoggedIn: true,
          authToken: token,
          userInfo: userInfo ?? null,
          isSessionExpired: false,
        })
        document.documentElement.dataset.auth = 'logged-in'
      },
      logout: () => {
        set({ ...defaultState })
        delete document.documentElement.dataset.auth
      },
      setSessionExpired: (expired) => set({ isSessionExpired: expired }),
      setHasAgreedToUserDeclaration: (agreed) => set({ hasAgreedToUserDeclaration: agreed }),
      updateUserInfo: (patch) => {
        const current = get().userInfo ?? {}
        set({ userInfo: { ...current, ...patch } })
      },
    }),
    {
      name: `${BRAND.storePrefix}-auth`,
      partialize: (s) => ({
        isLoggedIn: s.isLoggedIn,
        authToken: s.authToken,
        userInfo: s.userInfo,
        hasAgreedToUserDeclaration: s.hasAgreedToUserDeclaration,
      }),
      onRehydrateStorage: () => (state) => {
        if (state?.isLoggedIn && state.authToken) {
          document.documentElement.dataset.auth = 'logged-in'
        }
      },
    },
  ),
)

export function getAuthToken(): string | null {
  return useAuthStore.getState().authToken
}

export function hasPersistedSession(): boolean {
  const s = useAuthStore.getState()
  return Boolean(s.isLoggedIn && s.authToken)
}
