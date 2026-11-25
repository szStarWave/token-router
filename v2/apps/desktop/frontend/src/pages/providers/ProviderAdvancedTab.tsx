/**
 * 高级标签页（provider 级，product-design §7.2）：显示名 / base_url /
 * 目录端点编辑（PATCH provider），折叠在"高级"里，一级视图不堆砌。
 */
import { useState } from 'react'
import type { ProviderInfo } from '../../api/types'
import { useUpdateProviderMutation } from '../../queries/admin'
import { errorMessage } from '../../components/common/QueryState'

export function ProviderAdvancedTab({ provider }: { provider: ProviderInfo }) {
  const update = useUpdateProviderMutation()
  const [displayName, setDisplayName] = useState(provider.display_name)
  const [baseUrl, setBaseUrl] = useState(provider.base_url)
  const [catalogUrl, setCatalogUrl] = useState(provider.catalog_url ?? '')
  const [saved, setSaved] = useState(false)

  function submit() {
    setSaved(false)
    update.mutate(
      {
        id: provider.id,
        request: {
          display_name: displayName.trim() === '' ? undefined : displayName.trim(),
          base_url: baseUrl.trim() === '' ? undefined : baseUrl.trim(),
          catalog_url: catalogUrl.trim() === '' ? undefined : catalogUrl.trim(),
        },
      },
      { onSuccess: () => setSaved(true) },
    )
  }

  return (
    <div className="flex flex-col gap-2">
      <h3 className="text-ui-label font-medium">厂商信息</h3>
      <div className="app-form-row">
        <label className="app-field">
          <span>显示名</span>
          <input
            className="oc-field app-field-light app-field-ctl"
            value={displayName}
            onChange={(event) => setDisplayName(event.target.value)}
          />
        </label>
        <label className="app-field">
          <span>Base URL</span>
          <input
            className="oc-field app-field-light font-mono min-w-60"
            value={baseUrl}
            onChange={(event) => setBaseUrl(event.target.value)}
          />
        </label>
        <label className="app-field">
          <span>目录端点（可选）</span>
          <input
            className="oc-field app-field-light font-mono min-w-60"
            value={catalogUrl}
            placeholder="https://…/models 或留空"
            onChange={(event) => setCatalogUrl(event.target.value)}
          />
        </label>
      </div>
      <div className="app-card-actions">
        <button
          type="button"
          className="oc-btn-primary oc-btn-sm"
          disabled={update.isPending}
          onClick={submit}
        >
          {update.isPending ? '保存中…' : '保存'}
        </button>
        {saved && <span className="text-meta text-status-success">已保存。</span>}
        {update.isError && (
          <span className="text-meta text-status-error">
            {errorMessage(update.error)}
          </span>
        )}
      </div>
    </div>
  )
}
