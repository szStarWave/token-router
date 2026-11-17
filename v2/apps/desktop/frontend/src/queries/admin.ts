/**
 * admin 查询层（技术方案 §7.2）：服务端状态唯一入口。
 *
 * 规则：
 * - 类型一律来自 `src/api/types.ts`（契约生成物）；
 * - 查询键 `['admin', resource, params]`；写后精确 invalidate；
 * - 轮询：status 10s、stats 15s；页面隐藏由 TanStack Query 自动暂停；
 * - 错误映射 ApiError；组件用 `<QueryState>` 统一加载/空/错。
 */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api, adminRequest, queryKeys } from '../api/client'
import type { KeyLimits } from '../api/types'
export type { ApiError } from '../api/client'
// 具体端点类型由 api/client.ts 封装；此处不再重复导入。



/** 把时间窗参数拼成查询串。 */
function windowQuery(params: { from?: number; to?: number }): string {
  const search = new URLSearchParams()
  if (params.from !== undefined) search.set('from', String(params.from))
  if (params.to !== undefined) search.set('to', String(params.to))
  const text = search.toString()
  return text ? `?${text}` : ''
}

// ---------------------------------------------------------------------------
// status / health
// ---------------------------------------------------------------------------

export function useStatusQuery(enabled = true) {
  return useQuery({
    queryKey: queryKeys.status(),
    queryFn: () => api.status(),
    enabled,
    refetchInterval: enabled ? 10_000 : false,
    retry: 1,
  })
}

export function useHealthQuery(enabled = true) {
  return useQuery({
    queryKey: queryKeys.health(),
    queryFn: () => api.health(),
    enabled,
    refetchInterval: enabled ? 10_000 : false,
    retry: 1,
  })
}

// ---------------------------------------------------------------------------
// config
// ---------------------------------------------------------------------------

export function useConfigQuery(enabled = true) {
  return useQuery({
    queryKey: queryKeys.config(),
    queryFn: () => api.config(),
    enabled,
    retry: 1,
  })
}

export function useConfigValidateMutation() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (documentToml: string) => api.configValidate(documentToml),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['admin', 'config', 'audit'] })
    },
  })
}

/** apply 需要自定义 If-Match 头，走独立命令（与代理约定 body 携带 revision）。 */
export function useConfigApplyMutation() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (input: { revision: string; documentToml: string }) =>
      api.configApply(input.revision, input.documentToml),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['admin', 'config'] })
      void queryClient.invalidateQueries({ queryKey: queryKeys.status() })
      void queryClient.invalidateQueries({ queryKey: ['admin', 'config', 'audit'] })
    },
  })
}

export function useConfigRollbackMutation() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => api.configRollback(),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['admin', 'config'] })
      void queryClient.invalidateQueries({ queryKey: queryKeys.status() })
      void queryClient.invalidateQueries({ queryKey: ['admin', 'config', 'audit'] })
    },
  })
}

export function useConfigAuditQuery(enabled = true) {
  return useQuery({
    queryKey: ['admin', 'config', 'audit'],
    queryFn: () => api.configAudit(),
    enabled,
    retry: 1,
  })
}

// ---------------------------------------------------------------------------
// requests
// ---------------------------------------------------------------------------

export interface RequestListParams {
  session?: string
  model?: string
  outcome?: string
  protocol?: string
  reason?: string
  limit?: number
  cursor?: string
}

function requestsQuery(params: RequestListParams): string {
  const search = new URLSearchParams()
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined && value !== '') search.set(key === 'session' ? 'session' : key, String(value))
  }
  const text = search.toString()
  return text ? `?${text}` : ''
}

export function useRequestsQuery(params: RequestListParams, enabled = true) {
  return useQuery({
    queryKey: queryKeys.requests(requestsQuery(params)),
    queryFn: () => api.requests(requestsQuery(params)),
    enabled,
    retry: 1,
  })
}

export function useRequestDetailQuery(id: string | null, enabled = true) {
  return useQuery({
    queryKey: ['admin', 'requests', 'detail', id],
    queryFn: () => api.requestDetail(id as string),
    enabled: enabled && id !== null,
    retry: 1,
  })
}

// ---------------------------------------------------------------------------
// stats
// ---------------------------------------------------------------------------

export function useStatsSummaryQuery(window: { from?: number; to?: number } = {}, enabled = true) {
  return useQuery({
    queryKey: queryKeys.stats(`summary${windowQuery(window)}`),
    queryFn: () => api.statsSummary(windowQuery(window)),
    enabled,
    refetchInterval: enabled ? 15_000 : false,
    retry: 1,
  })
}

export interface SeriesParams {
  from?: number
  to?: number
  bucket: 'hour' | 'day'
  group_by: 'virtual_model' | 'dispatch_model' | 'outcome' | 'protocol'
  metric: 'requests' | 'successes' | 'cost' | 'input_tokens' | 'output_tokens'
}

export function useStatsSeriesQuery(params: SeriesParams, enabled = true) {
  const search = new URLSearchParams()
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined) search.set(key, String(value))
  }
  const query = `?${search.toString()}`
  return useQuery({
    queryKey: queryKeys.stats(`series${query}`),
    queryFn: () => api.statsSeries(query),
    enabled,
    retry: 1,
  })
}

export function useStatsModelsQuery(window: { from?: number; to?: number } = {}, enabled = true) {
  return useQuery({
    queryKey: queryKeys.stats(`models${windowQuery(window)}`),
    queryFn: () => api.statsModels(windowQuery(window)),
    enabled,
    retry: 1,
  })
}

// ---------------------------------------------------------------------------
// tokens
// ---------------------------------------------------------------------------

export function useTokensQuery(enabled = true) {
  return useQuery({
    queryKey: queryKeys.tokens(),
    queryFn: () => api.tokens(),
    enabled,
    retry: 1,
  })
}

export function useCreateTokenMutation() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (request: {
      name: string
      scopes: Array<'inference' | 'admin_read' | 'admin_write' | 'admin_approve'>
      expires_at: number | null
    }) => api.createToken(request),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.tokens() })
    },
  })
}

export function useRevokeTokenMutation() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => api.revokeToken(id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.tokens() })
    },
  })
}

// ---------------------------------------------------------------------------
// logs
// ---------------------------------------------------------------------------

export function useLogsQuery(params: { limit?: number; level?: string; target?: string }, enabled = true) {
  const search = new URLSearchParams()
  if (params.limit !== undefined) search.set('limit', String(params.limit))
  if (params.level) search.set('level', params.level)
  if (params.target) search.set('target', params.target)
  const query = `?${search.toString()}`
  return useQuery({
    queryKey: queryKeys.logs(query),
    queryFn: () => api.logs(query),
    enabled,
    retry: 1,
  })
}

// ---------------------------------------------------------------------------
// approvals（服务端 M1 已有 GET/POST）
// ---------------------------------------------------------------------------

export interface ApprovalRow {
  id: number
  request_id: string
  session_key: string | null
  tool_name: string
  risk_class: string
  decision: string
  operator: string | null
  created_at: number
}

export function useApprovalsQuery(limit = 50, enabled = true) {
  return useQuery({
    queryKey: ['admin', 'approvals', limit],
    queryFn: () => adminRequest<{ items: ApprovalRow[] }>("GET", `/v2/admin/approvals?limit=${limit}`),
    enabled,
    retry: 1,
  })
}

export function useRecordApprovalMutation() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (body: {
      request_id: string
      tool_name: string
      risk_class: string
      decision: string
    }) => adminRequest<{ ok: boolean }>("POST", "/v2/admin/approvals", body),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['admin', 'approvals'] })
    },
  })
}

// ---------------------------------------------------------------------------
// providers / accounts / catalog（v3 P5-A）
// ---------------------------------------------------------------------------

export function useProviderTemplatesQuery(enabled = true) {
  return useQuery({
    queryKey: queryKeys.providerTemplates(),
    queryFn: () => api.providerTemplates(),
    enabled,
    staleTime: Infinity,
    retry: 1,
  })
}

export function useProvidersQuery(enabled = true) {
  return useQuery({
    queryKey: queryKeys.providers(),
    queryFn: () => api.providers(),
    enabled,
    refetchInterval: 15_000,
    retry: 1,
  })
}

export function useAccountsQuery(providerId: string | null, enabled = true) {
  return useQuery({
    queryKey: queryKeys.accounts(providerId ?? ''),
    queryFn: () => api.accounts(providerId!),
    enabled: enabled && providerId !== null,
    retry: 1,
  })
}

export function useCatalogQuery(providerId: string | null, enabled = true) {
  return useQuery({
    queryKey: queryKeys.catalog(providerId ?? ''),
    queryFn: () => api.catalog(providerId!),
    enabled: enabled && providerId !== null,
    retry: 1,
  })
}

function useInvalidateProviderData() {
  const queryClient = useQueryClient()
  return () => {
    void queryClient.invalidateQueries({ queryKey: ['admin', 'providers'] })
    void queryClient.invalidateQueries({ queryKey: ['admin', 'accounts'] })
    void queryClient.invalidateQueries({ queryKey: ['admin', 'catalog'] })
  }
}

export function useCreateProviderMutation() {
  const invalidate = useInvalidateProviderData()
  return useMutation({ mutationFn: api.createProvider, onSuccess: invalidate })
}

export function useUpdateProviderMutation() {
  const invalidate = useInvalidateProviderData()
  return useMutation({
    mutationFn: ({ id, request }: { id: string; request: Parameters<typeof api.updateProvider>[1] }) =>
      api.updateProvider(id, request),
    onSuccess: invalidate,
  })
}

export function useDeleteProviderMutation() {
  const invalidate = useInvalidateProviderData()
  return useMutation({ mutationFn: api.deleteProvider, onSuccess: invalidate })
}

export function useCreateAccountMutation() {
  const invalidate = useInvalidateProviderData()
  return useMutation({
    mutationFn: ({ providerId, request }: { providerId: string; request: Parameters<typeof api.createAccount>[1] }) =>
      api.createAccount(providerId, request),
    onSuccess: invalidate,
  })
}

export function useUpdateAccountMutation() {
  const invalidate = useInvalidateProviderData()
  return useMutation({
    mutationFn: ({ accountId, request }: { accountId: string; request: Parameters<typeof api.updateAccount>[1] }) =>
      api.updateAccount(accountId, request),
    onSuccess: invalidate,
  })
}

export function useDeleteAccountMutation() {
  const invalidate = useInvalidateProviderData()
  return useMutation({ mutationFn: api.deleteAccount, onSuccess: invalidate })
}

export function useVerifyAccountMutation() {
  const invalidate = useInvalidateProviderData()
  return useMutation({
    mutationFn: ({ providerId, accountId }: { providerId: string; accountId: string }) =>
      api.verifyAccount(providerId, accountId),
    onSuccess: invalidate,
  })
}

export function useRefreshCatalogMutation() {
  const invalidate = useInvalidateProviderData()
  return useMutation({
    mutationFn: (providerId: string) => api.refreshCatalog(providerId, {}),
    onSuccess: invalidate,
  })
}

export function useSelectCatalogModelMutation() {
  const invalidate = useInvalidateProviderData()
  return useMutation({
    mutationFn: ({ providerId, request }: { providerId: string; request: Parameters<typeof api.selectCatalogModel>[1] }) =>
      api.selectCatalogModel(providerId, request),
    onSuccess: invalidate,
  })
}

export function useCompileCandidatesMutation() {
  const queryClient = useQueryClient()
  const invalidate = useInvalidateProviderData()
  return useMutation({
    mutationFn: (request: Parameters<typeof api.compileCandidates>[0]) =>
      api.compileCandidates(request),
    onSuccess: () => {
      invalidate()
      // 编译产物写进了 config（candidates/virtual_models）。
      void queryClient.invalidateQueries({ queryKey: ['admin', 'config'] })
    },
  })
}


// ---------------------------------------------------------------------------
// quotas / key limits（v3 P5-B）
// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

export function useQuotasQuery(enabled = true) {
  return useQuery({
    queryKey: queryKeys.quotas(),
    queryFn: () => api.quotaList(),
    enabled,
    refetchInterval: 15_000,
    retry: 1,
  })
}

export function useResetQuotaMutation() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (candidateId: string) => api.quotaReset(candidateId),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.quotas() })
    },
  })
}

export function useUpdateTokenLimitsMutation() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ id, limits }: { id: string; limits: KeyLimits | null }) =>
      api.updateTokenLimits(id, limits),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['admin', 'tokens'] })
    },
  })
}


// ---------------------------------------------------------------------------
// usage / heatmap（v3 P5-C）
// ---------------------------------------------------------------------------

export function useUsageQuery(by: string, from: number, to: number, enabled = true) {
  return useQuery({
    queryKey: queryKeys.usage(by, from, to),
    queryFn: () => api.usageBy(by, from, to),
    enabled,
    refetchInterval: 30_000,
    retry: 1,
  })
}

export function useHeatmapQuery(from: number, to: number, enabled = true) {
  return useQuery({
    queryKey: queryKeys.heatmap(from, to),
    queryFn: () => api.usageHeatmap(from, to),
    enabled,
    refetchInterval: 60_000,
    retry: 1,
  })
}
