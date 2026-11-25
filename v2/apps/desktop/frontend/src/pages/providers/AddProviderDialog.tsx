/**
 * 添加 / 导入供应商对话框。
 *
 * - 模板模式：搜索 models.dev + 内置模板（含自定义端点），选中后进入表单；
 * - 表单模式：显示名 / Base URL / 可选 API Key；创建成功后直接打开编辑器；
 * - 导入模式：扫描本机 agent 配置，只落 `env:` / `file:` 引用，不导入明文 key。
 *
 * 明文 key 只存在于本组件的 React state 里，提交时先清空再发请求；
 * 界面与查询缓存里只会出现服务端返回的指纹。
 */
import { useState } from 'react'
import { api } from '../../api/client'
import type {
  ImportedProviderDraft,
  ProviderInfo,
  ProviderTemplate,
} from '../../api/types'
import {
  useCreateAccountMutation,
  useCreateProviderMutation,
  useProviderTemplatesQuery,
} from '../../queries/admin'
import { QueryState, errorMessage } from '../../components/common/QueryState'
import { Modal } from '../../components/common/Modal'
import { useAppStore } from '../../stores/appStore'
import { providerKindLabel } from './shared'

type AddMode = 'templates' | 'form' | 'import'
type Busy = 'scan' | 'import' | null

function templateMatches(template: ProviderTemplate, needle: string): boolean {
  if (needle === '') return true
  return [
    template.display_name,
    template.id,
    template.base_url,
    providerKindLabel(template.kind),
  ].some((value) => value.toLowerCase().includes(needle))
}

export function AddProviderDialog({
  initialMode,
  onClose,
  onCreated,
}: {
  initialMode: 'templates' | 'import'
  onClose: () => void
  onCreated: (provider: ProviderInfo) => void
}) {
  const templatesQuery = useProviderTemplatesQuery()
  const createProvider = useCreateProviderMutation()
  const createAccount = useCreateAccountMutation()
  const showToast = useAppStore((state) => state.showToast)

  const [mode, setMode] = useState<AddMode>(initialMode)
  const [query, setQuery] = useState('')
  const [templateId, setTemplateId] = useState<string | null>(null)
  const [displayName, setDisplayName] = useState('')
  const [baseUrl, setBaseUrl] = useState('')
  const [key, setKey] = useState('')
  const [formError, setFormError] = useState<string | null>(null)
  const [submittingLocal, setSubmittingLocal] = useState(false)
  const [busy, setBusy] = useState<Busy>(null)
  const [importDrafts, setImportDrafts] = useState<ImportedProviderDraft[] | null>(null)
  const [importError, setImportError] = useState<string | null>(null)

  const templates = templatesQuery.data ?? []
  const needle = query.trim().toLowerCase()
  const filteredTemplates = templates.filter((template) => templateMatches(template, needle))
  const template = templates.find((item) => item.id === templateId)
  const submitting =
    submittingLocal || createProvider.isPending || createAccount.isPending
  const canSubmit = template !== undefined && baseUrl.trim() !== '' && !submitting

  function chooseTemplate(next: ProviderTemplate) {
    setTemplateId(next.id)
    setDisplayName(next.display_name)
    setBaseUrl(next.base_url)
    setKey('')
    setFormError(null)
    setMode('form')
  }

  async function submit() {
    if (!template || baseUrl.trim() === '') return
    // 先清空组件内的明文，再发请求：成功/失败都不会把 key 留在状态里。
    const submittedKey = key.trim()
    setKey('')
    setFormError(null)
    setSubmittingLocal(true)
    try {
      const provider = await createProvider.mutateAsync({
        template_id: template.id,
        display_name: displayName.trim() || template.display_name,
        kind: template.kind,
        base_url: baseUrl.trim(),
        catalog_url: template.catalog_url ?? undefined,
      })
      if (submittedKey !== '') {
        try {
          await createAccount.mutateAsync({
            providerId: provider.id,
            request: { label: '主账号', key: submittedKey },
          })
        } catch (error) {
          // provider 已创建成功：不再重试整条流程，避免重复建厂商。
          showToast(`厂商已创建，但账号创建失败：${errorMessage(error)}`, false)
        }
      }
      setSubmittingLocal(false)
      onCreated(provider)
    } catch (error) {
      setFormError(errorMessage(error))
      setSubmittingLocal(false)
    }
  }

  async function scanAgents() {
    setBusy('scan')
    setImportError(null)
    try {
      const response = await api.providerImport()
      setImportDrafts(response.drafts)
    } catch (error) {
      // 扫描失败 ≠ 没有可导入项：给错误与重试，不误报"没发现"。
      setImportError(errorMessage(error))
    } finally {
      setBusy(null)
    }
  }

  async function importDraft(draft: ImportedProviderDraft) {
    setBusy('import')
    setImportError(null)
    try {
      const provider = await createProvider.mutateAsync({
        display_name: draft.display_name,
        base_url: draft.base_url,
        kind: 'open_ai_compatible',
      })
      if (draft.credential_ref) {
        try {
          await createAccount.mutateAsync({
            providerId: provider.id,
            request: { label: '导入（引用）', credential_ref: draft.credential_ref },
          })
        } catch (error) {
          showToast(`厂商已创建，但账号导入失败：${errorMessage(error)}`, false)
        }
      }
      setImportDrafts((current) =>
        (current ?? []).filter((item) => item.base_url !== draft.base_url),
      )
      onCreated(provider)
    } catch (error) {
      setImportError(errorMessage(error))
    } finally {
      setBusy(null)
    }
  }

  const title =
    mode === 'import' ? '导入供应商' : mode === 'form' ? '配置供应商' : '添加供应商'
  const subtitle =
    mode === 'import'
      ? '只导入 env: / file: 引用，不导入明文 key。'
      : mode === 'form'
        ? template
          ? `${providerKindLabel(template.kind)} · ${template.base_url || '自定义端点'}`
          : '请选择模板'
        : '模板来自 models.dev（内置常用厂商优先），或导入本机 agent 配置。'

  const footer =
    mode === 'form' ? (
      <>
        <button
          type="button"
          className="oc-btn-default"
          onClick={() => {
            setMode('templates')
            setFormError(null)
            setKey('')
          }}
        >
          返回模板
        </button>
        <span className="flex-1" />
        <button
          type="button"
          className="oc-btn-primary"
          disabled={!canSubmit}
          onClick={() => void submit()}
        >
          {submitting ? '创建中…' : '创建并编辑'}
        </button>
      </>
    ) : (
      <>
        {mode === 'import' && (
          <button type="button" className="oc-btn-default" onClick={() => setMode('templates')}>
            返回模板
          </button>
        )}
        <span className="flex-1" />
        <button type="button" className="oc-btn-default" onClick={onClose}>
          取消
        </button>
      </>
    )

  return (
    <Modal title={title} subtitle={subtitle} footer={footer} onClose={onClose}>
      {mode === 'templates' && (
        <div className="flex flex-col gap-3">
          <input
            className="app-field-sm"
            type="search"
            value={query}
            placeholder="搜索模板 / 端点 / 类型…"
            aria-label="搜索供应商模板"
            onChange={(event) => setQuery(event.target.value)}
          />
          <QueryState
            loading={templatesQuery.isPending}
            error={templatesQuery.error}
            empty={filteredTemplates.length === 0}
            onRetry={() => templatesQuery.refetch()}
            emptyTitle="没有匹配的模板"
            emptyHint="换个关键词，或返回使用「自定义 OpenAI 兼容」模板。"
          >
            <div className="app-provider-tile-grid">
              {filteredTemplates.map((item) => (
                <button
                  key={item.id}
                  type="button"
                  className="app-provider-tile"
                  onClick={() => chooseTemplate(item)}
                >
                  <span className="app-provider-tile-name">{item.display_name}</span>
                  <span className="app-provider-tile-sub">
                    {item.base_url || '自定义端点'} · {providerKindLabel(item.kind)}
                  </span>
                  <span className="app-provider-tile-sub">
                    {item.catalog_url ? '支持在线目录' : '无在线目录，可手工添加模型'}
                  </span>
                </button>
              ))}
            </div>
          </QueryState>
          <div className="app-card-actions">
            <button
              type="button"
              className="oc-btn-default oc-btn-sm"
              onClick={() => setMode('import')}
            >
              导入…
            </button>
            <span className="text-meta text-muted-foreground">
              模板只预填默认端点；创建后继续添加账号、刷新目录。
            </span>
          </div>
        </div>
      )}

      {mode === 'form' && (
        <div className="flex flex-col gap-3">
          <div className="app-form-row">
            <label className="app-field">
              <span>显示名</span>
              <input
                className="oc-field app-field-light app-field-ctl"
                value={displayName}
                placeholder={template?.display_name ?? '供应商名称'}
                onChange={(event) => setDisplayName(event.target.value)}
              />
            </label>
            <label className="app-field">
              <span>Base URL（必填）</span>
              <input
                className="oc-field app-field-light font-mono min-w-60"
                value={baseUrl}
                placeholder="https://…/v1"
                onChange={(event) => setBaseUrl(event.target.value)}
              />
            </label>
          </div>
          <div className="app-kv">
            <div className="app-kv-item">
              <span className="app-kv-key">类型</span>
              <span className="app-kv-value">
                {template ? providerKindLabel(template.kind) : '—'}
              </span>
            </div>
            <div className="app-kv-item">
              <span className="app-kv-key">目录端点</span>
              <span className="app-kv-value font-mono">
                {template?.catalog_url ?? '无（可手工添加模型）'}
              </span>
            </div>
          </div>
          <label className="app-field">
            <span>API Key（可选，用于创建主账号）</span>
            <input
              className="oc-field app-field-light font-mono min-w-60"
              type="password"
              autoComplete="new-password"
              spellCheck={false}
              value={key}
              placeholder="留空则创建后手动添加账号"
              onChange={(event) => setKey(event.target.value)}
            />
          </label>
          {formError !== null && <div className="app-banner app-banner-err">{formError}</div>}
        </div>
      )}

      {mode === 'import' && (
        <div className="flex flex-col gap-3">
          <div className="app-card-actions">
            <button
              type="button"
              className="oc-btn-default"
              disabled={busy !== null}
              onClick={() => void scanAgents()}
            >
              {busy === 'scan' ? '扫描中…' : importDrafts ? '重新扫描' : '扫描本机 agent 配置'}
            </button>
          </div>
          {importError !== null && (
            <div className="app-banner app-banner-err">{importError}</div>
          )}
          {importDrafts !== null &&
            (importDrafts.length === 0 ? (
              <p className="text-meta text-muted-foreground">没有发现可导入的上游端点。</p>
            ) : (
              <ul className="flex flex-col gap-2">
                {importDrafts.map((draft) => (
                  <li
                    key={`${draft.base_url}-${draft.source}`}
                    className="app-plan-entry"
                  >
                    <div className="app-plan-head">
                      <span className="truncate text-ui-label font-medium">
                        {draft.display_name}
                      </span>
                      <span className="app-pill">{draft.credential_ref ?? '无 key 引用'}</span>
                    </div>
                    <span className="truncate font-mono text-meta text-muted-foreground">
                      {draft.base_url}
                    </span>
                    <div className="flex items-center gap-2">
                      <span className="min-w-0 flex-1 truncate text-micro text-muted-foreground">
                        {draft.source}
                      </span>
                      <button
                        type="button"
                        className="oc-btn-primary oc-btn-sm"
                        disabled={busy !== null}
                        onClick={() => void importDraft(draft)}
                      >
                        {busy === 'import' ? '创建中…' : '创建'}
                      </button>
                    </div>
                  </li>
                ))}
              </ul>
            ))}
        </div>
      )}
    </Modal>
  )
}
