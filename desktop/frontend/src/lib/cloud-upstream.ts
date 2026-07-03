import { getCurrentFlowyServerBase } from './flowy/server'
import { getAuthToken } from '../stores/authStore'
import { getAvailableModelList, type CloudModel } from './flowy/api'
import { CLOUD_BUDGET_MIN } from '../constants/defaults'
import type { ApiFetch } from '../stores/edgeStore'

export const AUTO_MODEL_ID = 'auto'

let cachedModels: CloudModel[] = []

export function getCloudBaseUrl() {
  return getCurrentFlowyServerBase('/claw/v1')
}

export function getCachedCloudModels() {
  return cachedModels
}

export function withAutoModelOption(models: CloudModel[], autoLabel: string) {
  const list = models.filter((m) => m.id !== 'Auto' && m.id !== AUTO_MODEL_ID)
  list.unshift({ id: AUTO_MODEL_ID, name: autoLabel, icon: '' })
  return list
}

function normalizeModelId(id: string | null | undefined) {
  if (!id) return ''
  if (id === 'Auto') return AUTO_MODEL_ID
  return id
}

export function getCloudModelDisplayName(modelId: string, autoLabel: string) {
  if (!modelId) return ''
  const id = normalizeModelId(modelId)
  if (id === AUTO_MODEL_ID) return autoLabel
  const found = cachedModels.find((m) => m.id === id)
  return found?.name || id
}

export async function fetchCloudModels(autoLabel: string) {
  const token = getAuthToken()
  if (!token) throw new Error('未登录')
  cachedModels = withAutoModelOption(await getAvailableModelList(token), autoLabel)
  return cachedModels
}

export function normalizeCloudTokenBudget(tokenBudget: unknown) {
  if (tokenBudget === undefined || tokenBudget === null || tokenBudget === '') return 0
  const n = Number(tokenBudget)
  if (!Number.isFinite(n) || n <= 0) return 0
  return Math.floor(n)
}

export function buildCloudSavePayload(modelId: string, tokenBudget?: number | null) {
  const token = getAuthToken()
  if (!token) throw new Error('未登录')
  const payload: {
    base_url: string
    model: string
    api_key: string
    token_budget?: number
  } = {
    base_url: getCloudBaseUrl(),
    model: modelId || AUTO_MODEL_ID,
    api_key: token,
  }
  if (tokenBudget != null) payload.token_budget = tokenBudget
  return payload
}

export function sliderFromCloudBudget(budget: number) {
  const min = CLOUD_BUDGET_MIN
  const max = 1_000_000_000_000
  if (budget <= 0) return 0
  const clamped = Math.max(min, Math.min(max, budget))
  const logMin = Math.log10(min)
  const logMax = Math.log10(max)
  const logVal = Math.log10(clamped)
  return Math.round(((logVal - logMin) / (logMax - logMin)) * 1000)
}

export function budgetFromSlider(sliderVal: number) {
  const min = CLOUD_BUDGET_MIN
  const max = 1_000_000_000_000
  if (sliderVal <= 0) return 0
  const logMin = Math.log10(min)
  const logMax = Math.log10(max)
  const logVal = logMin + (sliderVal / 1000) * (logMax - logMin)
  return Math.round(Math.pow(10, logVal))
}

export async function ensureCloudUpstreamConfigured(
  apiFetch: ApiFetch | null,
  options: { currentModel?: string | null; tokenBudget?: number; silent?: boolean } = {},
) {
  const models = await fetchCloudModels('Auto')
  if (!apiFetch) return { models, posted: false }

  const modelId = normalizeModelId(options.currentModel) || AUTO_MODEL_ID
  const tokenBudget =
    options.tokenBudget === undefined
      ? undefined
      : normalizeCloudTokenBudget(options.tokenBudget)
  const cloud = buildCloudSavePayload(modelId, tokenBudget)

  try {
    const res = (await apiFetch('/v1/admin/setup', {
      method: 'POST',
      body: JSON.stringify({ cloud }),
    })) as { upstream?: unknown }
    return { models, posted: true, response: res }
  } catch (e) {
    if (!options.silent) throw e
    console.warn('[cloud-upstream]', e)
    return { models, posted: false, error: e }
  }
}
