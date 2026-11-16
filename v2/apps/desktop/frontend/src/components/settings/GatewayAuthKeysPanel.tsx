import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  createGatewayAuthKey,
  deleteGatewayAuthKey,
  fetchGatewayAuthKeys,
  formatAuthKeyCreatedAt,
  updateGatewayAuthKeyName,
} from '../../lib/gateway-auth-keys'
import { useAppStore } from '../../stores/appStore'
import { useI18n } from '../../hooks/useI18n'
import { queryKeys } from '../../queries/keys'

export function GatewayAuthKeysPanel() {
  const { t, locale } = useI18n()
  const connected = useAppStore((s) => s.connected)
  const showToast = useAppStore((s) => s.showToast)
  const qc = useQueryClient()

  const [dialogOpen, setDialogOpen] = useState(false)
  const [editingId, setEditingId] = useState<string | null>(null)
  const [nameInput, setNameInput] = useState('')
  const [createdKey, setCreatedKey] = useState('')

  const keysQuery = useQuery({
    queryKey: queryKeys.gatewayAuthKeys,
    queryFn: fetchGatewayAuthKeys,
    enabled: connected,
  })

  const invalidate = () => void qc.invalidateQueries({ queryKey: queryKeys.gatewayAuthKeys })

  const createMutation = useMutation({
    mutationFn: (name: string) => createGatewayAuthKey(name),
    onSuccess: (res) => {
      setCreatedKey(res.full_key)
      invalidate()
      showToast('toast.authKeyCreated')
    },
    onError: (e: Error) => showToast('toast.saveFail', false, { msg: e.message }),
  })

  const updateMutation = useMutation({
    mutationFn: ({ id, name }: { id: string; name: string }) => updateGatewayAuthKeyName(id, name),
    onSuccess: () => {
      closeDialog()
      invalidate()
      showToast('toast.authKeyUpdated')
    },
    onError: (e: Error) => showToast('toast.saveFail', false, { msg: e.message }),
  })

  const deleteMutation = useMutation({
    mutationFn: (id: string) => deleteGatewayAuthKey(id),
    onSuccess: () => {
      invalidate()
      showToast('toast.authKeyDeleted')
    },
    onError: (e: Error) => showToast('toast.saveFail', false, { msg: e.message }),
  })

  const openCreate = () => {
    setEditingId(null)
    setNameInput('')
    setCreatedKey('')
    setDialogOpen(true)
  }

  const openEdit = (id: string, name: string) => {
    setEditingId(id)
    setNameInput(name)
    setCreatedKey('')
    setDialogOpen(true)
  }

  const closeDialog = () => {
    setDialogOpen(false)
    setEditingId(null)
    setNameInput('')
    setCreatedKey('')
  }

  const submitDialog = () => {
    const name = nameInput.trim()
    if (!name) {
      showToast('toast.authKeyNameRequired', false)
      return
    }
    if (editingId) {
      updateMutation.mutate({ id: editingId, name })
      return
    }
    createMutation.mutate(name)
  }

  const copyCreatedKey = async () => {
    if (!createdKey) return
    await navigator.clipboard.writeText(createdKey)
    showToast('toast.copied')
  }

  const rows = keysQuery.data ?? []
  const busy = createMutation.isPending || updateMutation.isPending || deleteMutation.isPending

  return (
    <div className="panel" id="gateway-auth-keys-panel">
      <div className="panel-title-row">
        <div className="panel-title">{t('settings.gatewayAuthKeys')}</div>
        <button
          type="button"
          className="btn btn-primary btn-sm"
          id="btn-gateway-auth-key-add"
          disabled={!connected || busy}
          onClick={openCreate}
        >
          {t('action.add')}
        </button>
      </div>

      <div className="table-wrap">
        <table>
          <thead>
            <tr>
              <th>{t('col.authKeyName')}</th>
              <th>{t('col.authKey')}</th>
              <th>{t('col.createdAt')}</th>
              <th className="table-actions-col">{t('col.actions')}</th>
            </tr>
          </thead>
          <tbody id="gateway-auth-key-table">
            {!connected ? (
              <tr><td colSpan={4} style={{ color: 'var(--default-400)' }}>{t('conn.offline')}</td></tr>
            ) : keysQuery.isLoading ? (
              <tr><td colSpan={4} style={{ color: 'var(--default-400)' }}>{t('sidebar.loading')}</td></tr>
            ) : keysQuery.isError ? (
              <tr><td colSpan={4} style={{ color: 'var(--default-400)' }}>{t('sidebar.failed')}</td></tr>
            ) : !rows.length ? (
              <tr><td colSpan={4} style={{ color: 'var(--default-400)' }}>{t('authKeys.empty')}</td></tr>
            ) : (
              rows.map((row) => (
                <tr key={row.id}>
                  <td>
                    <span className="auth-key-name-cell">
                      {row.is_default && (
                        <span className="tag bordered neutral" title={t('authKeys.builtinHint')}>
                          {t('authKeys.builtin')}
                        </span>
                      )}
                      {row.name}
                    </span>
                  </td>
                  <td><code>{row.key_preview}</code></td>
                  <td>{formatAuthKeyCreatedAt(row.created_at, locale)}</td>
                  <td className="table-actions-col">
                    {!row.is_default && (
                      <div className="table-actions">
                        <button
                          type="button"
                          className="btn btn-ghost btn-sm"
                          disabled={busy}
                          onClick={() => openEdit(row.id, row.name)}
                        >
                          {t('action.edit')}
                        </button>
                        <button
                          type="button"
                          className="btn btn-ghost btn-sm"
                          disabled={busy}
                          onClick={() => deleteMutation.mutate(row.id)}
                        >
                          {t('action.delete')}
                        </button>
                      </div>
                    )}
                  </td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>

      <div className={`security-dialog gateway-auth-key-dialog${dialogOpen ? ' open' : ''}`}>
        <div className="security-panel">
          <h3>{editingId ? t('authKeys.editTitle') : t('authKeys.addTitle')}</h3>
          <div className="form-row">
            <div>
              <label htmlFor="gateway_auth_key_name">{t('col.authKeyName')}</label>
              <input
                id="gateway_auth_key_name"
                value={nameInput}
                placeholder={t('ph.authKeyName')}
                onChange={(e) => setNameInput(e.target.value)}
              />
            </div>
          </div>
          {createdKey && !editingId && (
            <div className="auth-key-created-box">
              <div className="setting-label">{t('authKeys.createdOnce')}</div>
              <div className="endpoint-box">
                <code id="gateway-auth-key-created-value">{createdKey}</code>
              </div>
              <div className="upstream-actions">
                <button type="button" className="btn btn-ghost btn-sm" onClick={() => void copyCreatedKey()}>
                  {t('action.copy')}
                </button>
                <button type="button" className="btn btn-primary btn-sm" onClick={closeDialog}>
                  {t('action.confirm')}
                </button>
              </div>
            </div>
          )}
          {(!createdKey || editingId) && (
            <div className="security-actions">
              <button type="button" className="btn btn-ghost" onClick={closeDialog}>{t('action.cancel')}</button>
              <button
                type="button"
                className="btn btn-primary"
                disabled={busy}
                onClick={submitDialog}
              >
                {t('action.confirm')}
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
