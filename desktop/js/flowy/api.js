import { BRAND } from './config.js';
import { getCurrentFlowyServerBase } from './server.js';
import { getAuthToken, setSessionExpired } from './auth-store.js';

async function parseJsonResponse(response) {
  if (response.status === 401) {
    setSessionExpired(true);
    throw new Error('Session expired');
  }
  const data = await response.json().catch(() => ({}));
  if (data?.code === 401) {
    setSessionExpired(true);
    throw new Error('Session expired');
  }
  if (!response.ok) {
    throw new Error(data?.msg || data?.message || '请求失败');
  }
  return data;
}

async function post(path, body, token) {
  const headers = { 'Content-Type': 'application/json' };
  if (token) {
    headers.token = token;
    headers.Authorization = `Bearer ${token}`;
  }
  const response = await fetch(`${getCurrentFlowyServerBase()}${path}`, {
    method: 'POST',
    headers,
    body: body ? JSON.stringify(body) : undefined,
  });
  return parseJsonResponse(response);
}

async function get(url, token, absolute = false) {
  const headers = {};
  if (token) {
    headers.token = token;
    headers.Authorization = `Bearer ${token}`;
  }
  const target = absolute ? url : `${getCurrentFlowyServerBase()}${url}`;
  const response = await fetch(target, { method: 'GET', headers });
  return parseJsonResponse(response);
}

export async function sendEmailCode(email) {
  const res = await post('/user/getEmailRegisterValidCode', { email, channel: BRAND.id });
  const reqNo = res?.data;
  if (!reqNo) throw new Error('验证码发送失败');
  return reqNo;
}

export async function loginByEmail(email, validCode, validCodeReqNo, inviteCode) {
  const res = await post('/user/doLoginByEmail', {
    email,
    validCode,
    validCodeReqNo,
    inviteCode: inviteCode?.trim() || undefined,
    channel: BRAND.id,
    device: '',
    app: 'aipc',
  });
  if (res?.code !== 200) throw new Error(res?.msg || '登录失败');
  const token = res?.data;
  if (!token) throw new Error('登录失败');
  return token;
}

export async function loginByWeChatCallback(callbackUrl) {
  const url = new URL(callbackUrl);
  url.searchParams.set('app', 'aipc');
  const res = await get(url.toString(), null, true);
  if (res?.code !== 200) throw new Error(res?.msg || '微信登录失败');
  const token = res?.data;
  if (!token) throw new Error('微信登录失败');
  return token;
}

export async function getCreditsBalance(token) {
  const authToken = token ?? getAuthToken();
  if (!authToken) return 0;
  const res = await get('/credits/balance', authToken);
  if (res?.code !== 200) throw new Error(res?.msg || '获取余额失败');
  const balance = res?.data?.balance;
  return typeof balance === 'number' ? balance : 0;
}

export async function getCreditsUsageByType(token) {
  const authToken = token ?? getAuthToken();
  if (!authToken) throw new Error('未登录');
  const res = await get('/credits/usageByType', authToken);
  if (res?.code !== 200) throw new Error(res?.msg || '获取积分使用情况失败');
  return res?.data;
}

export async function getAvailableModelList(token) {
  const authToken = token ?? getAuthToken();
  if (!authToken) throw new Error('未登录');
  const res = await get('/model/availableListClaw', authToken);
  if (res?.code !== 200) throw new Error(res?.msg || '获取模型列表失败');
  const models = res?.data?.cloud;
  if (!Array.isArray(models)) throw new Error('模型列表格式错误');
  return models;
}

export async function loginByToken(token) {
  const authToken = token ?? getAuthToken();
  if (!authToken) throw new Error('未登录');
  const res = await get('/user/me', authToken);
  if (!res?.data) throw new Error('获取用户信息失败');
  return res.data;
}

export async function deviceActivateAfterLogin(token) {
  try {
    await post('/device/activate', { app: 'aipc', channel: BRAND.id }, token);
  } catch (e) {
    console.warn('[device/activate]', e);
  }
}
