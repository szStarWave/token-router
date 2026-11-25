/**
 * 配置草稿本机暂存（product-design §8.4）：**默认关闭的实验能力**。
 *
 * D1 决策（按任务约束"若不能保证安全，先内存跨页 + 关闭提醒，不声称持久化"）：
 * - 服务稳定标识（data-dir / listen）在 WebView 内拿不到可靠来源（status.listen 是运行时
 *   监听地址、可被配置改掉；hostAppInfo 需要 Tauri 且非服务身份），**本期不启用持久化**；
 * - 本模块只提供"如果将来启用"的严格框架：
 *   - 标识 = 服务标识 + baseRevision + schemaVersion，三者全等才允许恢复；
 *   - 序列化走**字段 allowlist**：只落配置结构（凭据字段本身就是引用 `env:`/`file:`，
 *     不含明文；`admin_token`/`inbound_key`/`credential` 若是占位符或引用原样保留），
 *     绝不新增任何 prompt/自由文本字段；
 *   - 标识不匹配的草稿**隔离保存**（`tr2.configDraft.quarantine`），不静默丢弃；
 *   - 恢复前必须可比较（调用方用 diffDrafts 出清单让用户决定）。
 * - 当前唯一入口 `persistDraftIfEnabled` 恒返回 `{ persisted: false }`，
 *   页面据此显示"草稿仅保留在当前窗口内存中，关闭应用即丢失"。
 */

export interface DraftServiceIdentity {
  /** 服务稳定标识（启用持久化时由宿主提供 data-dir 或用户确认的 base-url）。 */
  serviceId: string
  /** 草稿基准 revision。 */
  baseRevision: string
  /** 配置 schema 版本。 */
  schemaVersion: number
}

export interface PersistableDraftPayload {
  identity: DraftServiceIdentity
  /** 序列化前的草稿（只允许 allowlist 字段进入存储）。 */
  draft: Record<string, unknown>
  savedAt: number
}

export interface PersistResult {
  persisted: boolean
  /** 未持久化的原因（如实提示用）。 */
  reason: 'disabled' | 'identity-unavailable' | 'storage-unavailable'
}

const DRAFT_KEY = 'tr2.configDraft.active'
const QUARANTINE_KEY = 'tr2.configDraft.quarantine'

/**
 * 草稿允许进入本机存储的字段 allowlist（顶层）。
 * 未列出的字段一律不落盘——宁可少存不可多存。
 */
export const PERSISTABLE_TOP_LEVEL = [
  'schemaVersion',
  'defaultVirtualModel',
  'server',
  'profiles',
  'candidates',
  'virtualModels',
  'passthrough',
] as const

/**
 * server 段允许落盘的字段。admin_token / inbound_key 只允许引用或空串；
 * 含明文（非空且不是引用形态）时剔除该字段，不让明文进任何存储。
 */
const PERSISTABLE_SERVER = ['listen', 'auth_enabled', 'auth_mode', 'admin_token', 'inbound_key'] as const

function isCredentialRef(value: string): boolean {
  return (
    value === '' ||
    value.startsWith('env:') ||
    value.startsWith('file:') ||
    value.startsWith('keyring:')
  )
}

/** node 测试环境无 window：localStorage 访问统一走此 getter。 */
function storage(): Storage | null {
  try {
    if (typeof window === 'undefined') return null
    return window.localStorage
  } catch {
    return null
  }
}

function pickRecord(value: unknown, keys: readonly string[]): Record<string, unknown> {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return {}
  const source = value as Record<string, unknown>
  const out: Record<string, unknown> = {}
  for (const key of keys) {
    if (key in source) out[key] = source[key]
  }
  return out
}

/** 按 allowlist 过滤草稿；凭据字段非引用形态时整段剔除（守卫明文）。 */
export function toPersistablePayload(draft: Record<string, unknown>): Record<string, unknown> {
  const payload = pickRecord(draft, PERSISTABLE_TOP_LEVEL)
  const server = pickRecord(payload.server, PERSISTABLE_SERVER)
  if (!isCredentialRef(String(server.admin_token ?? ''))) delete server.admin_token
  if (!isCredentialRef(String(server.inbound_key ?? ''))) delete server.inbound_key
  payload.server = server

  // 候选凭据：非引用形态的剔除该候选（整条不落盘，避免半截配置误导恢复）。
  if (Array.isArray(payload.candidates)) {
    payload.candidates = (payload.candidates as Array<Record<string, unknown>>).filter(
      (candidate) => isCredentialRef(String(candidate.credential ?? '')),
    )
  }
  // 组级判断服务凭据：非引用形态 → 该虚拟模型的 judge 段不落盘（规则保留，恢复后需重填判断配置）。
  if (Array.isArray(payload.virtualModels)) {
    payload.virtualModels = (payload.virtualModels as Array<Record<string, unknown>>).map((model) => {
      const judge = model.judge
      if (!judge || typeof judge !== 'object' || Array.isArray(judge)) return model
      const service = (judge as Record<string, unknown>).service
      if (!service || typeof service !== 'object' || Array.isArray(service)) return model
      if (isCredentialRef(String((service as Record<string, unknown>).credential ?? ''))) return model
      const cleaned = { ...model }
      delete cleaned.judge
      return cleaned
    })
  }
  // passthrough 里可能藏 judge.service.credential：非引用则整体剔除该段。
  const passthrough = payload.passthrough
  if (passthrough && typeof passthrough === 'object' && !Array.isArray(passthrough)) {
    const judge = (passthrough as Record<string, unknown>).judge
    const service =
      judge && typeof judge === 'object' && !Array.isArray(judge)
        ? (judge as Record<string, unknown>).service
        : undefined
    if (service && typeof service === 'object' && !Array.isArray(service)) {
      if (!isCredentialRef(String((service as Record<string, unknown>).credential ?? ''))) {
        const cleaned = { ...(passthrough as Record<string, unknown>) }
        delete cleaned.judge
        payload.passthrough = cleaned
      }
    }
  }
  return payload
}

/**
 * 身份是否匹配（三者全等才算同一服务的同一基准）。
 * 不匹配 → 隔离，绝不静默丢弃，也绝不静默恢复。
 */
export function identityMatches(
  stored: Pick<PersistableDraftPayload, 'identity'>,
  current: DraftServiceIdentity,
): boolean {
  return (
    stored.identity.serviceId === current.serviceId &&
    stored.identity.baseRevision === current.baseRevision &&
    stored.identity.schemaVersion === current.schemaVersion
  )
}

/**
 * 写草稿到本机存储。**本期恒不启用**（identity 不可靠），返回 disabled；
 * 启用开关收敛在这一个函数，未来接宿主 data-dir 后只需放开这里。
 */
export function persistDraftIfEnabled(
  _identity: DraftServiceIdentity,
  _draft: Record<string, unknown>,
): PersistResult {
  return { persisted: false, reason: 'identity-unavailable' }
}

/** 读暂存草稿（未启用时恒 null）。隔离键只由测试与未来启用路径读取。 */
export function readStoredDraft(): { active: PersistableDraftPayload | null; quarantined: PersistableDraftPayload | null } {
  const store = storage()
  if (!store) return { active: null, quarantined: null }
  try {
    const active = store.getItem(DRAFT_KEY)
    const quarantined = store.getItem(QUARANTINE_KEY)
    return {
      active: active ? (JSON.parse(active) as PersistableDraftPayload) : null,
      quarantined: quarantined ? (JSON.parse(quarantined) as PersistableDraftPayload) : null,
    }
  } catch {
    return { active: null, quarantined: null }
  }
}

/** 隔离保存（身份不匹配时调用；不覆盖已有隔离内容——保留更旧的以供对比）。 */
export function quarantineDraft(payload: PersistableDraftPayload): boolean {
  const store = storage()
  if (!store) return false
  try {
    if (store.getItem(QUARANTINE_KEY)) return false
    store.setItem(QUARANTINE_KEY, JSON.stringify(payload))
    return true
  } catch {
    return false
  }
}

/** 清除暂存（用户明确丢弃 / 应用成功后调用）。 */
export function clearStoredDraft(): void {
  const store = storage()
  if (!store) return
  try {
    store.removeItem(DRAFT_KEY)
    store.removeItem(QUARANTINE_KEY)
  } catch {
    /* ignore */
  }
}

export const STORAGE_KEYS = { DRAFT_KEY, QUARANTINE_KEY } as const
