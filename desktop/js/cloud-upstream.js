import { getAvailableModelList } from './flowy/api.js';
import { getAuthToken } from './flowy/auth-store.js';
import { getCurrentFlowyServerBase } from './flowy/server.js';
import { mountModelSelect } from './model-select.js';

import { appT } from './locale-utils.js';

export const AUTO_MODEL_ID = 'auto';
export const DEFAULT_CLOUD_TOKEN_BUDGET = 1_000_000;

let cachedModels = [];
let cloudModelSelect = null;

export function getCloudBaseUrl() {
  return getCurrentFlowyServerBase('/claw/v1');
}

export function getCachedCloudModels() {
  return cachedModels;
}

function autoModelLabel() {
  return window.__appI18n?.t?.('cloudModel.auto') ?? appT('cloudModel.auto');
}

function autoModelOption() {
  return { id: AUTO_MODEL_ID, name: autoModelLabel(), icon: '' };
}

function normalizeModelId(id) {
  if (!id) return '';
  if (id === 'Auto') return AUTO_MODEL_ID;
  return id;
}

export function withAutoModelOption(models) {
  const list = Array.isArray(models) ? models.filter((m) => m.id !== 'Auto' && m.id !== AUTO_MODEL_ID) : [];
  list.unshift(autoModelOption());
  return list;
}

function resolveDefaultModelId(models, preferred) {
  const pref = normalizeModelId(preferred);
  if (pref && models.some((m) => m.id === pref)) return pref;
  if (models.some((m) => m.id === AUTO_MODEL_ID)) return AUTO_MODEL_ID;
  return models[0]?.id || AUTO_MODEL_ID;
}

export function getCloudModelDisplayName(modelId) {
  if (!modelId) return '';
  const id = normalizeModelId(modelId);
  if (id === AUTO_MODEL_ID) return autoModelLabel();
  const found = cachedModels.find((m) => m.id === id);
  if (found?.name) return found.name;
  return id;
}

export function getCloudModelValue() {
  return cloudModelSelect?.getValue() || '';
}

export function initCloudModelSelect(onChange) {
  if (!cloudModelSelect) {
    cloudModelSelect = mountModelSelect('cloud_model_picker', {
      onChange,
      placeholder: modelSelectPlaceholder(),
    });
  }
  return cloudModelSelect;
}

function notLoggedInMessage() {
  return window.__appI18n?.t?.('err.notLoggedIn') ?? appT('err.notLoggedIn');
}

function modelSelectPlaceholder() {
  return window.__appI18n?.t?.('ph.pickModel') ?? appT('ph.pickModel');
}

export async function fetchCloudModels() {
  const token = getAuthToken();
  if (!token) throw new Error(notLoggedInMessage());
  cachedModels = withAutoModelOption(await getAvailableModelList(token));
  return cachedModels;
}

function syncHiddenCloudUrl() {
  const hidden = document.getElementById('cloud_url');
  if (hidden) hidden.value = getCloudBaseUrl();
}

export function populateCloudModelSelect(models, selectedId) {
  if (!cloudModelSelect) {
    cloudModelSelect = mountModelSelect('cloud_model_picker', { placeholder: modelSelectPlaceholder() });
  }
  const list = withAutoModelOption(models);
  cachedModels = list;
  if (cloudModelSelect) {
    cloudModelSelect.setModels(list, normalizeModelId(selectedId));
  }
}

export function normalizeCloudTokenBudget(tokenBudget) {
  if (tokenBudget === undefined || tokenBudget === null || tokenBudget === '') return 0;
  const n = Number(tokenBudget);
  if (!Number.isFinite(n) || n <= 0) return 0;
  return Math.floor(n);
}

export function readCloudTokenBudgetFromDom() {
  const enabledEl = document.getElementById('cloud_quota_enabled');
  const budgetEl = document.getElementById('cloud_token_budget');
  if (enabledEl && !enabledEl.checked) return 0;
  const raw = budgetEl?.value?.trim();
  if (!raw) return DEFAULT_CLOUD_TOKEN_BUDGET;
  const n = Number(raw);
  if (!Number.isFinite(n) || n <= 0) return DEFAULT_CLOUD_TOKEN_BUDGET;
  return Math.floor(n);
}

export function buildCloudSavePayload(modelId, tokenBudget) {
  const token = getAuthToken();
  if (!token) throw new Error(notLoggedInMessage());

  const model = modelId || resolveDefaultModelId(cachedModels);
  const budget = tokenBudget === undefined
    ? readCloudTokenBudgetFromDom()
    : normalizeCloudTokenBudget(tokenBudget);

  return {
    base_url: getCloudBaseUrl(),
    model,
    api_key: token,
    token_budget: budget,
  };
}

export async function refreshCloudUpstreamUi(selectedModel) {
  syncHiddenCloudUrl();
  if (!cloudModelSelect) {
    cloudModelSelect = mountModelSelect('cloud_model_picker', { placeholder: modelSelectPlaceholder() });
  }
  const models = await fetchCloudModels();
  populateCloudModelSelect(models, selectedModel);
  return models;
}

export async function ensureCloudUpstreamConfigured(apiFetch, options = {}) {
  const { silent = true, currentModel, tokenBudget } = options;
  const models = await refreshCloudUpstreamUi(currentModel);
  if (!apiFetch) return { models, posted: false };

  const budget = tokenBudget === undefined
    ? readCloudTokenBudgetFromDom()
    : normalizeCloudTokenBudget(tokenBudget);
  const modelId = getCloudModelValue() || resolveDefaultModelId(models, currentModel);
  const cloud = buildCloudSavePayload(modelId, budget);

  try {
    const res = await apiFetch('/v1/admin/setup', {
      method: 'POST',
      body: JSON.stringify({ cloud }),
    });
    return { models, posted: true, response: res };
  } catch (e) {
    if (!silent) throw e;
    console.warn('[cloud-upstream] auto setup failed', e);
    return { models, posted: false, error: e };
  }
}

export function installCloudUpstreamGlobals() {
  initCloudModelSelect((value) => {
    document.dispatchEvent(new CustomEvent('cloud-model-change', { detail: { value } }));
  });

  function refreshI18n() {
    if (cloudModelSelect?.setPlaceholder) {
      cloudModelSelect.setPlaceholder(modelSelectPlaceholder());
    }
    if (cachedModels.length) {
      const current = getCloudModelValue();
      cachedModels = withAutoModelOption(cachedModels.filter((m) => m.id !== AUTO_MODEL_ID && m.id !== 'Auto'));
      populateCloudModelSelect(cachedModels, current || AUTO_MODEL_ID);
    }
  }

  document.addEventListener('app-locale-change', refreshI18n);

  window.__cloudUpstream = {
    getCloudBaseUrl,
    fetchCloudModels,
    refreshCloudUpstreamUi,
    buildCloudSavePayload,
    ensureCloudUpstreamConfigured,
    populateCloudModelSelect,
    getCloudModelValue,
    getCloudModelDisplayName,
    initCloudModelSelect,
    getCachedCloudModels,
    withAutoModelOption,
    readCloudTokenBudgetFromDom,
    normalizeCloudTokenBudget,
    DEFAULT_CLOUD_TOKEN_BUDGET,
    refreshI18n,
  };
}
