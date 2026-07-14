import type { UserInfo } from '../stores/authStore'

export function pickUserId(user: UserInfo | Record<string, unknown> | null | undefined): string {
  if (!user) return ''
  const record = user as Record<string, unknown>
  const id = record.id ?? record.userId ?? record.user_id
  if (id == null || id === '') return ''
  return String(id)
}

export function pickNickname(user: UserInfo | Record<string, unknown> | null | undefined): string {
  if (!user) return ''
  const record = user as Record<string, unknown>
  return String(record.nickName || record.nickname || record.name || record.username || record.email || '')
}

export function pickAvatar(user: UserInfo | Record<string, unknown> | null | undefined): string {
  if (!user) return ''
  const record = user as Record<string, unknown>
  return String(record.avatar || record.headImg || record.head_img || record.avatarUrl || '')
}

export function formatFeedbackUserLabel(
  user: UserInfo | Record<string, unknown> | null | undefined,
  fallback = '未登录',
): string {
  const id = pickUserId(user)
  const nickname = pickNickname(user)
  if (id && nickname) return `${id} ${nickname}`
  if (id) return id
  if (nickname) return nickname
  return fallback
}
