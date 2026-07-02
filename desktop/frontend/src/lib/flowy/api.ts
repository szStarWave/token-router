import { BRAND, FLOWY_CLIENT_APP } from './config'
import { getCurrentFlowyServerBase } from './server'
import { getAuthToken, useAuthStore } from '../../stores/authStore'

async function parseJsonResponse(response: Response) {
  if (response.status === 401) {
    useAuthStore.getState().setSessionExpired(true)
    throw new Error('Session expired')
  }
  const data = await response.json().catch(() => ({}))
  if (data?.code === 401) {
    useAuthStore.getState().setSessionExpired(true)
    throw new Error('Session expired')
  }
  if (!response.ok) {
    throw new Error(data?.msg || data?.message || '请求失败')
  }
  return data
}

async function post(path: string, body: unknown, token?: string | null) {
  const headers: Record<string, string> = { 'Content-Type': 'application/json' }
  if (token) {
    headers.token = token
    headers.Authorization = `Bearer ${token}`
  }
  const response = await fetch(`${getCurrentFlowyServerBase()}${path}`, {
    method: 'POST',
    headers,
    body: body ? JSON.stringify(body) : undefined,
  })
  return parseJsonResponse(response)
}

async function get(url: string, token?: string | null, absolute = false) {
  const headers: Record<string, string> = {}
  if (token) {
    headers.token = token
    headers.Authorization = `Bearer ${token}`
  }
  const target = absolute ? url : `${getCurrentFlowyServerBase()}${url}`
  const response = await fetch(target, { method: 'GET', headers })
  return parseJsonResponse(response)
}

export async function sendEmailCode(email: string) {
  const res = await post('/user/getEmailRegisterValidCode', { email, channel: BRAND.id })
  const reqNo = res?.data
  if (!reqNo) throw new Error('验证码发送失败')
  return reqNo as string
}

export async function loginByEmail(
  email: string,
  validCode: string,
  validCodeReqNo: string,
  inviteCode?: string,
) {
  const res = await post('/user/doLoginByEmail', {
    email,
    validCode,
    validCodeReqNo,
    inviteCode: inviteCode?.trim() || undefined,
    channel: BRAND.id,
    device: '',
    app: FLOWY_CLIENT_APP,
  })
  if (res?.code !== 200) throw new Error(res?.msg || '登录失败')
  const token = res?.data
  if (!token) throw new Error('登录失败')
  return token as string
}

export async function loginByWeChatCallback(callbackUrl: string) {
  const url = new URL(callbackUrl)
  url.searchParams.set('app', FLOWY_CLIENT_APP)
  const res = await get(url.toString(), null, true)
  if (res?.code !== 200) throw new Error(res?.msg || '微信登录失败')
  const token = res?.data
  if (!token) throw new Error('微信登录失败')
  return token as string
}

export async function getCreditsBalance(token?: string | null) {
  const authToken = token ?? getAuthToken()
  if (!authToken) return 0
  const res = await get('/credits/balance', authToken)
  if (res?.code !== 200) throw new Error(res?.msg || '获取余额失败')
  const balance = res?.data?.balance
  return typeof balance === 'number' ? balance : 0
}

export async function getCreditsUsageByType(token?: string | null) {
  const authToken = token ?? getAuthToken()
  if (!authToken) throw new Error('未登录')
  const res = await get('/credits/usageByType', authToken)
  if (res?.code !== 200) throw new Error(res?.msg || '获取积分使用情况失败')
  return res?.data
}

export interface CloudModel {
  id: string
  name: string
  icon?: string
}

export async function getAvailableModelList(token?: string | null): Promise<CloudModel[]> {
  const authToken = token ?? getAuthToken()
  if (!authToken) throw new Error('未登录')
  const res = await get('/model/availableListClaw', authToken)
  if (res?.code !== 200) throw new Error(res?.msg || '获取模型列表失败')
  const models = res?.data?.cloud
  if (!Array.isArray(models)) throw new Error('模型列表格式错误')
  return models
}

export async function loginByToken(token?: string | null) {
  const authToken = token ?? getAuthToken()
  if (!authToken) throw new Error('未登录')
  const res = await get('/user/me', authToken)
  if (!res?.data) throw new Error('获取用户信息失败')
  return res.data
}

export async function deviceActivateAfterLogin(token: string) {
  try {
    await post('/device/activate', { app: FLOWY_CLIENT_APP, channel: BRAND.id }, token)
  } catch (e) {
    console.warn('[device/activate]', e)
  }
}

export interface DailyCheckInResult {
  alreadyCheckedIn: boolean
  grantedPoints: number
  balance: number
  dayKey: number
}

export async function dailyCheckIn(token?: string | null): Promise<DailyCheckInResult> {
  const authToken = token ?? getAuthToken()
  if (!authToken) throw new Error('未登录')
  const timeZone = Intl.DateTimeFormat().resolvedOptions().timeZone
  const res = await post('/credits/checkin', { timeZone }, authToken)
  if (res?.code !== 200) throw new Error(res?.msg || '签到失败')
  const data = res?.data
  if (!data || typeof data !== 'object') throw new Error('签到失败')
  return data as DailyCheckInResult
}

export async function reportLocalModelUsage(
  params: Record<string, unknown>,
  token?: string | null,
) {
  const authToken = token ?? getAuthToken()
  if (!authToken) throw new Error('未登录')
  const res = await post('/model/localUsage/report', params, authToken)
  if (res?.code !== 200) throw new Error(res?.msg || '本地模型用量上报失败')
  if (!res?.data) throw new Error('本地模型用量上报失败')
  return res.data as { savedPoints?: number }
}
