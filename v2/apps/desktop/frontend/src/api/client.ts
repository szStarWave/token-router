/**
 * 统一 admin 客户端（技术方案 §7.2）：底层走 Tauri `admin_request` 代理。
 *
 * 硬规则：
 * - admin token 不在 JS 侧：Rust 代理注入 Authorization；
 * - 错误统一映射为 ApiError {status, type, message, issues}；
 * - 查询键约定 `['admin', resource, params]`（TanStack Query 在 M4 接入）。
 */

export interface ApiError {
  status: number;
  type: string;
  message: string;
  issues?: Array<{ path: string; code: string; message: string }>;
}

export class AdminApiError extends Error implements ApiError {
  status: number;
  type: string;
  issues?: Array<{ path: string; code: string; message: string }>;

  constructor(error: ApiError) {
    super(error.message);
    this.name = "AdminApiError";
    this.status = error.status;
    this.type = error.type;
    this.issues = error.issues;
  }
}

/** admin_request 的 Tauri 命令名。 */
const ADMIN_COMMAND = "admin_request";

/** Tauri invoke 的最小类型（避免强依赖 @tauri-apps/api 形状）。 */
declare global {
  interface Window {
    __TAURI_INTERNALS__?: { invoke: (cmd: string, args?: unknown) => Promise<unknown> };
  }
}

async function invoke<T>(command: string, args?: unknown): Promise<T> {
  if (typeof window !== "undefined" && window.__TAURI_INTERNALS__?.invoke) {
    return (await window.__TAURI_INTERNALS__.invoke(command, args)) as T;
  }
  // 非 Tauri 环境（纯浏览器开发）：生产恒走代理。
  throw new AdminApiError({
    status: 0,
    type: "no_bridge",
    message: "not running inside Tauri; admin bridge unavailable",
  });
}

export interface AdminResponse {
  status: number;
  body?: unknown;
  error?: { error: { type: string; message: string; issues?: ApiError["issues"] } };
}

async function request<T>(
  method: "GET" | "POST" | "PATCH" | "DELETE",
  path: string,
  body?: unknown,
  ifMatch?: string,
): Promise<T> {
  const response = await invoke<AdminResponse>(ADMIN_COMMAND, {
    request: { method, path, body: body ?? null, if_match: ifMatch ?? null },
  });
  if (response.status >= 200 && response.status < 300 && response.body !== undefined) {
    return response.body as T;
  }
  const error = response.error?.error;
  throw new AdminApiError({
    status: response.status,
    type: error?.type ?? "unknown",
    message: error?.message ?? "unknown error",
    issues: error?.issues,
  });
}

export const admin = {
  get: <T>(path: string) => request<T>("GET", path),
  post: <T>(path: string, body?: unknown, ifMatch?: string) =>
    request<T>("POST", path, body, ifMatch),
  patch: <T>(path: string, body?: unknown) => request<T>("PATCH", path, body),
  del: <T>(path: string) => request<T>("DELETE", path),
};

// 直接导出底层方法（approvals 等未类型化端点用）。
export { request as adminRequest };

// ---------------------------------------------------------------------------
// 类型化端点封装（契约冻结 v1；类型来自 src/api/types.ts 的生成物）
// ---------------------------------------------------------------------------

import type {
  AccountCreateRequest,
  AccountInfo,
  AccountUpdateRequest,
  AdminScope,
  ApplyResponse,
  CatalogRefreshRequest,
  CatalogResponse,
  CatalogSelectRequest,
  CompileRequest,
  CompileResponse,
  ConfigAuditEntry,
  ConfigDocument,
  CreatedToken,
  CreateTokenRequest,
  DoctorResponse,
  HealthResponse,
  LogEntry,
  Page,
  ProviderInfo,
  ProviderTemplate,
  ProviderUpsertRequest,
  RequestDetail,
  RequestSummary,
  SeriesResponse,
  StatsSummary,
  StatusResponse,
  TokenInfo,
  ValidateResponse,
} from "./types";

export const api = {
  status: () => admin.get<StatusResponse>("/v2/admin/status"),
  health: () => admin.get<HealthResponse>("/v2/admin/health"),
  doctor: () => admin.get<DoctorResponse>("/v2/admin/doctor"),
  config: () => admin.get<ConfigDocument>("/v2/admin/config"),
  configValidate: (documentToml: string) =>
    admin.post<ValidateResponse>("/v2/admin/config/validate", { document_toml: documentToml }),
  configApply: (revision: string, documentToml: string) =>
    admin.post<ApplyResponse>("/v2/admin/config/apply", {
      document_toml: documentToml,
    }, revision),
  configRollback: () => admin.post<ApplyResponse>("/v2/admin/config/rollback"),
  configAudit: () => admin.get<{ items: ConfigAuditEntry[] }>("/v2/admin/config/audit"),
  requests: (query: string) => admin.get<Page<RequestSummary>>(`/v2/admin/requests${query}`),
  requestDetail: (id: string) => admin.get<RequestDetail>(`/v2/admin/requests/${id}`),
  statsSummary: (query: string) => admin.get<StatsSummary>(`/v2/admin/stats/summary${query}`),
  statsSeries: (query: string) => admin.get<SeriesResponse>(`/v2/admin/stats/series${query}`),
  statsModels: (query: string) =>
    admin.get<{ items: Array<Record<string, unknown>> }>(`/v2/admin/stats/models${query}`),
  tokens: () => admin.get<Page<TokenInfo>>("/v2/admin/tokens"),
  createToken: (request: CreateTokenRequest) =>
    admin.post<CreatedToken>("/v2/admin/tokens", request),
  revokeToken: (id: string) => admin.del<{ ok: boolean }>(`/v2/admin/tokens/${id}`),
  logs: (query: string) => admin.get<{ items: LogEntry[]; next_cursor: number | null }>(`/v2/admin/logs${query}`),
  shutdown: () => admin.post<{ ok: boolean }>("/v2/admin/shutdown"),
  // ── Providers（v3 P5-A）───────────────────────────────────────────────
  providerTemplates: () =>
    admin.get<ProviderTemplate[]>("/v2/admin/provider-templates"),
  providers: () => admin.get<ProviderInfo[]>("/v2/admin/providers"),
  createProvider: (request: ProviderUpsertRequest) =>
    admin.post<ProviderInfo>("/v2/admin/providers", request),
  updateProvider: (id: string, request: ProviderUpsertRequest) =>
    admin.patch<ProviderInfo>(`/v2/admin/providers/${id}`, request),
  deleteProvider: (id: string) =>
    admin.del<{ ok: boolean }>(`/v2/admin/providers/${id}`),
  accounts: (providerId: string) =>
    admin.get<AccountInfo[]>(`/v2/admin/providers/${providerId}/accounts`),
  createAccount: (providerId: string, request: AccountCreateRequest) =>
    admin.post<AccountInfo>(`/v2/admin/providers/${providerId}/accounts`, request),
  updateAccount: (accountId: string, request: AccountUpdateRequest) =>
    admin.patch<AccountInfo>(`/v2/admin/accounts/${accountId}`, request),
  deleteAccount: (accountId: string) =>
    admin.del<{ ok: boolean }>(`/v2/admin/accounts/${accountId}`),
  verifyAccount: (providerId: string, accountId: string) =>
    admin.post<AccountInfo>(`/v2/admin/providers/${providerId}/accounts/verify`, {
      account_id: accountId,
    }),
  catalog: (providerId: string) =>
    admin.get<CatalogResponse>(`/v2/admin/providers/${providerId}/catalog`),
  refreshCatalog: (providerId: string, request: CatalogRefreshRequest = {}) =>
    admin.post<CatalogResponse>(`/v2/admin/providers/${providerId}/catalog/refresh`, request),
  selectCatalogModel: (providerId: string, request: CatalogSelectRequest) =>
    admin.post<CatalogResponse>(`/v2/admin/providers/${providerId}/catalog/select`, request),
  compileCandidates: (request: CompileRequest) =>
    admin.post<CompileResponse>("/v2/admin/providers/compile", request),
};

export type { AdminScope };

/** 查询键工厂（TanStack Query 用）。 */
export const queryKeys = {
  status: () => ["admin", "status"] as const,
  health: () => ["admin", "health"] as const,
  config: () => ["admin", "config"] as const,
  requests: (params: string) => ["admin", "requests", params] as const,
  stats: (params: string) => ["admin", "stats", params] as const,
  tokens: () => ["admin", "tokens"] as const,
  logs: (params: string) => ["admin", "logs", params] as const,
  providerTemplates: () => ["admin", "provider-templates"] as const,
  providers: () => ["admin", "providers"] as const,
  provider: (id: string) => ["admin", "providers", id] as const,
  accounts: (providerId: string) => ["admin", "accounts", providerId] as const,
  catalog: (providerId: string) => ["admin", "catalog", providerId] as const,
};
