/**
 * admin 查询层（技术方案 §7.2）：服务端状态唯一入口。
 *
 * 规则：
 * - 类型一律来自 `src/api/types.ts`（契约生成物）；
 * - 查询键 `['admin', resource, params]`；写后精确 invalidate；
 * - 轮询：status 10s、stats 15s；页面隐藏由 TanStack Query 自动暂停；
 * - 错误映射 ApiError；组件用 `<QueryState>` 统一加载/空/错。
 */
import { useMutation, useQuery, useQueries, useQueryClient } from '@tanstack/react-query'
import { api, queryKeys } from '../api/client'
import type { KeyLimits, RequestSummary } from '../api/types'
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

export function useRequestsQuery(
  params: RequestListParams,
  enabled = true,
  refetchInterval: number | false = false,
) {
  return useQuery({
    queryKey: queryKeys.requests(requestsQuery(params)),
    queryFn: () => api.requests(requestsQuery(params)),
    enabled,
    // 轻量模式（无 SSE）下由调用方给轮询间隔；其他情况靠事件失效。
    refetchInterval: enabled ? refetchInterval : false,
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

/** 一轮请求的消息与回复（`[recording] transcript` 控制是否记录）。 */
export function useRequestTranscriptQuery(id: string | null, enabled = true) {
  return useQuery({
    queryKey: ['admin', 'requests', 'transcript', id],
    queryFn: () => api.requestTranscript(id as string),
    enabled: enabled && id !== null,
    retry: 1,
  })
}

/** 会话级完整汇总（服务端聚合，不受请求列表分页影响）。 */
export function useSessionsQuery(params: { from: number; to: number; limit?: number }) {
  const search = new URLSearchParams({
    from: String(params.from),
    to: String(params.to),
    limit: String(params.limit ?? 100),
  })
  const query = `?${search.toString()}`
  return useQuery({
    queryKey: queryKeys.sessions(query),
    queryFn: () => api.sessions(query),
    retry: 1,
  })
}

/** 判官（决策模型）调用审计；按会话或按请求过滤。 */
export function useJudgeCallsQuery(
  params: { session?: string; requestId?: string; limit?: number },
  enabled = true,
) {
  const search = new URLSearchParams()
  if (params.session) search.set('session', params.session)
  if (params.requestId) search.set('request_id', params.requestId)
  search.set('limit', String(params.limit ?? 200))
  const query = `?${search.toString()}`
  return useQuery({
    queryKey: queryKeys.judgeCalls(query),
    queryFn: () => api.judgeCalls(query),
    enabled,
    retry: 1,
  })
}

/** 完整会话调用：把该会话的请求按游标翻到底（上限 10 页 × 200）。 */
async function fetchAllSessionRequests(sessionKey: string): Promise<RequestSummary[]> {
  const items: RequestSummary[] = []
  let cursor: string | null = null
  for (let page = 0; page < 10; page += 1) {
    const search = new URLSearchParams({ session: sessionKey, limit: '200' })
    if (cursor) search.set('cursor', cursor)
    const result = await api.requests(`?${search.toString()}`)
    items.push(...result.items)
    cursor = result.next_cursor
    if (!cursor) break
  }
  return items
}

export function useSessionRequestsQuery(sessionKey: string | null) {
  return useQuery({
    queryKey: ['admin', 'session-requests', sessionKey],
    queryFn: () => fetchAllSessionRequests(sessionKey as string),
    enabled: sessionKey !== null,
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
      scopes: Array<'inference' | 'admin_read' | 'admin_write'>
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

/** 重新签发 token 明文：旧明文立即失效，新明文只在响应里出现一次。 */
export function useRotateTokenMutation() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => api.rotateToken(id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.tokens() })
    },
  })
}

/** 硬删除已撤销的 token 记录。 */
export function usePurgeTokenMutation() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => api.purgeToken(id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.tokens() })
    },
  })
}

// ---------------------------------------------------------------------------
// logs
// ---------------------------------------------------------------------------

export function useLogsQuery(
  params: { limit?: number; level?: string; target?: string },
  enabled = true,
  refetchInterval: number | false = false,
) {
  const search = new URLSearchParams()
  if (params.limit !== undefined) search.set('limit', String(params.limit))
  if (params.level) search.set('level', params.level)
  if (params.target) search.set('target', params.target)
  const query = `?${search.toString()}`
  return useQuery({
    queryKey: queryKeys.logs(query),
    queryFn: () => api.logs(query),
    enabled,
    // 轻量模式（无 SSE）下由调用方给轮询间隔。
    refetchInterval: enabled ? refetchInterval : false,
    retry: 1,
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

/**
 * 全部 provider 的账号（聚合）。
 *
 * admin 侧只有 `/providers/{id}/accounts`，没有跨 provider 的账号列表；
 * 这里对已知 provider 并发取回并合并，供「账号与配额」这类跨 provider 视图使用。
 * provider 列表未知时返回空数组（不报错）。
 */
export function useAllAccountsQuery(enabled = true) {
  const providersQuery = useProvidersQuery(enabled)
  const providerIds = (providersQuery.data ?? []).map((provider) => provider.id)
  const results = useQueries({
    queries: providerIds.map((providerId) => ({
      queryKey: queryKeys.accounts(providerId),
      queryFn: () => api.accounts(providerId),
      enabled,
      retry: 1,
    })),
  })
  const accounts = results.flatMap((result) => result.data ?? [])
  // 任一账号查询还在加载都算加载中；错误优先取 provider 列表的，再取第一个账号错误。
  const accountLoading = results.some((result) => result.isLoading)
  const accountError = results.find((result) => result.error !== null)?.error ?? null
  return {
    providers: providersQuery.data ?? [],
    accounts,
    loading: providersQuery.isLoading || accountLoading,
    error: providersQuery.error ?? accountError,
  }
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
    // 目录/账号/供应商变更后服务端会立即重编译候选并写回 config：
    // 组页的「添加模型」列表来自编译产物，必须让 config 查询跟上。
    void queryClient.invalidateQueries({ queryKey: ['admin', 'config'] })
    void queryClient.invalidateQueries({ queryKey: queryKeys.status() })
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

export function useDeleteCatalogModelMutation() {
  const invalidate = useInvalidateProviderData()
  return useMutation({
    mutationFn: ({ providerId, modelId }: { providerId: string; modelId: string }) =>
      api.deleteCatalogModel(providerId, modelId),
    onSuccess: invalidate,
  })
}

/** 用 models.dev 补全目录元数据（窗口/输出/价格/推理档位，只填空）。 */
export function useEnrichCatalogMutation() {
  const invalidate = useInvalidateProviderData()
  return useMutation({
    mutationFn: (providerId: string) => api.enrichCatalog(providerId),
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
// 配额窗口（路由侧限流观测）
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

// ---------------------------------------------------------------------------
// 账户余额 / 订阅（厂商 usage 接口）
// ---------------------------------------------------------------------------

export function useProviderUsageQuery(enabled = true) {
  return useQuery({
    queryKey: queryKeys.providerUsage(),
    queryFn: () => api.providerUsage(),
    enabled,
    // 厂商接口有速率限制：页面内不轮询，后端 60s 缓存 + 手动刷新。
    staleTime: 60_000,
    retry: 1,
  })
}

export function useRefreshProviderUsageMutation() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => api.providerUsage(true),
    onSuccess: (data) => {
      queryClient.setQueryData(queryKeys.providerUsage(), data)
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
