import { getAvailableModelList } from './flowy/api.js';
import { getAuthToken } from './flowy/auth-store.js';
import { getCurrentFlowyServerBase } from './flowy/server.js';
import { mountModelSelect } from './model-select.js';

import { appT } from './locale-utils.js';

let cachedModels = [];
let cloudModelSelect = null;

export function getCloudBaseUrl() {
  return getCurrentFlowyServerBase('/claw/v1');
}

export function getCachedCloudModels() {
  return cachedModels;
}

function resolveDefaultModelId(models, preferred) {
  if (preferred && models.some((m) => m.id === preferred)) return preferred;
  const auto = models.find((m) => m.id === 'Auto' || m.id === 'auto');
  if (auto) return auto.id;
  return models[0]?.id || 'Auto';
}

export function getCloudModelDisplayName(modelId) {
  if (!modelId) return '';
  const found = cachedModels.find((m) => m.id === modelId);
  if (found?.name) return found.name;
  if (modelId === 'auto') return 'Auto';
  return modelId;
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
  cachedModels = await getAvailableModelList(token);
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
  if (cloudModelSelect) {
    cloudModelSelect.setModels(models, selectedId);
  }
}

export function buildCloudSavePayload(modelId, tokenBudget) {
  const token = getAuthToken();
  if (!token) throw new Error(notLoggedInMessage());

  const model = modelId || resolveDefaultModelId(cachedModels);
  const payload = {
    base_url: getCloudBaseUrl(),
    model,
    api_key: token,
  };

  if (tokenBudget !== undefined && tokenBudget !== null && tokenBudget !== '') {
    const n = Number(tokenBudget);
    if (!Number.isNaN(n)) payload.token_budget = n;
  }

  return payload;
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

  const budgetEl = document.getElementById('cloud_token_budget');
  const budgetRaw = tokenBudget ?? budgetEl?.value?.trim() ?? '';
  const modelId = getCloudModelValue() || resolveDefaultModelId(models, currentModel);
  const cloud = buildCloudSavePayload(modelId, budgetRaw);

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
    refreshI18n,
  };
}
