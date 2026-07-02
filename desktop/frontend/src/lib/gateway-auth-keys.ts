import { apiFetch } from './gateway'

export interface GatewayAuthKeyView {
  id: string
  name: string
  key_preview: string
  created_at: number
}

export interface CreateGatewayAuthKeyResponse {
  key: GatewayAuthKeyView
  full_key: string
}

export function fetchGatewayAuthKeys() {
  return apiFetch<GatewayAuthKeyView[]>('/v1/admin/auth-keys')
}

export function createGatewayAuthKey(name: string) {
  return apiFetch<CreateGatewayAuthKeyResponse>('/v1/admin/auth-keys', {
    method: 'POST',
    body: JSON.stringify({ name }),
  })
}

export function updateGatewayAuthKeyName(id: string, name: string) {
  return apiFetch<GatewayAuthKeyView>(`/v1/admin/auth-keys/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    body: JSON.stringify({ name }),
  })
}

export function deleteGatewayAuthKey(id: string) {
  return apiFetch<{ ok: boolean }>(`/v1/admin/auth-keys/${encodeURIComponent(id)}`, {
    method: 'DELETE',
  })
}

export function formatAuthKeyCreatedAt(ts: number, locale: string) {
  if (!ts) return '—'
  return new Intl.DateTimeFormat(locale === 'en' ? 'en-US' : 'zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(ts * 1000))
}
