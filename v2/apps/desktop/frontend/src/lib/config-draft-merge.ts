/**
 * 配置草稿三方合并（自动保存的 409 rebase 用）。
 *
 * 语义（与 git merge 同构，但只处理配置结构）：
 * - 三方相等 → 任取；
 * - 只有我改 → 我的值；
 * - 只有对方改 → 对方的值；
 * - 双方改了同一处（值不同）→ 冲突，先保留我的值，把路径记进 `conflicts`；
 * - 数组：元素是带字符串 `id` 的对象时按 id 对齐逐个合并（候选 / 虚拟模型）；
 *   否则整个数组当原子值处理（避免对顺序敏感的数组做出错误合并）。
 *
 * 纯函数：不读写存储、不触网；冲突路径可直接展示给用户。
 */
import { deepEqual } from './config-draft-diff'

export interface MergeResult<T> {
  merged: T
  /** 双方都改过的路径（`candidates[gpt-5].model` 形式）。 */
  conflicts: string[]
}

type UnknownRecord = Record<string, unknown>

function isRecord(value: unknown): value is UnknownRecord {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function idOf(value: unknown): string | null {
  if (!isRecord(value)) return null
  const id = value.id
  return typeof id === 'string' ? id : null
}

function mergeValue(
  base: unknown,
  ours: unknown,
  theirs: unknown,
  path: string,
  conflicts: string[],
): unknown {
  if (deepEqual(ours, theirs)) return ours
  if (deepEqual(base, ours)) return theirs
  if (deepEqual(base, theirs)) return ours

  // 数组：id 对齐；否则原子处理。
  if (Array.isArray(ours) && Array.isArray(theirs) && Array.isArray(base)) {
    const oursById = new Map<string, unknown>()
    const theirsById = new Map<string, unknown>()
    const baseById = new Map<string, unknown>()
    for (const item of ours) {
      const id = idOf(item)
      if (id !== null) oursById.set(id, item)
    }
    for (const item of theirs) {
      const id = idOf(item)
      if (id !== null) theirsById.set(id, item)
    }
    for (const item of base) {
      const id = idOf(item)
      if (id !== null) baseById.set(id, item)
    }
    const idAligned =
      oursById.size === ours.length &&
      theirsById.size === theirs.length &&
      baseById.size === base.length
    if (idAligned) {
      const ids: string[] = []
      for (const item of [...base, ...ours, ...theirs]) {
        const id = idOf(item)
        if (id !== null && !ids.includes(id)) ids.push(id)
      }
      const merged: unknown[] = []
      for (const id of ids) {
        const inBase = baseById.get(id)
        const inOurs = oursById.get(id)
        const inTheirs = theirsById.get(id)
        const elementPath = `${path}[${id}]`
        if (inOurs === undefined && inTheirs === undefined) continue
        // 新增（基准里没有这个 id）：两边各加一个不同元素都要保留。
        if (inBase === undefined) {
          if (inOurs === undefined) {
            merged.push(inTheirs)
            continue
          }
          if (inTheirs === undefined) {
            merged.push(inOurs)
            continue
          }
          merged.push(mergeValue(undefined, inOurs, inTheirs, elementPath, conflicts))
          continue
        }
        if (inOurs === undefined) {
          // 我删了、对方没删：若对方改过 → 冲突（删除 vs 修改），否则删除。
          if (!deepEqual(inBase, inTheirs)) conflicts.push(elementPath)
          continue
        }
        if (inTheirs === undefined) {
          // 对方删了、我没删：若我改过 → 冲突，否则删除。
          if (!deepEqual(inBase, inOurs)) conflicts.push(elementPath)
          else continue
        }
        merged.push(mergeValue(inBase, inOurs, inTheirs, elementPath, conflicts))
      }
      return merged
    }
  }

  if (isRecord(ours) && isRecord(theirs) && isRecord(base)) {
    const merged: UnknownRecord = {}
    const keys: string[] = []
    for (const key of [...Object.keys(base), ...Object.keys(ours), ...Object.keys(theirs)]) {
      if (!keys.includes(key)) keys.push(key)
    }
    for (const key of keys) {
      const value = mergeValue(
        base[key],
        ours[key],
        theirs[key],
        path === '' ? key : `${path}.${key}`,
        conflicts,
      )
      if (value !== undefined) merged[key] = value
    }
    return merged
  }

  conflicts.push(path === '' ? '(root)' : path)
  return ours
}

/** 三方合并：`base` = 草稿基准，`ours` = 我的草稿，`theirs` = 磁盘最新草稿。 */
export function mergeDraft<T>(base: T, ours: T, theirs: T): MergeResult<T> {
  const conflicts: string[] = []
  const merged = mergeValue(base, ours, theirs, '', conflicts) as T
  return { merged, conflicts }
}
