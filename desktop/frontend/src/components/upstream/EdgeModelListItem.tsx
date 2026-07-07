import type { EdgeDisplayItem } from '../../stores/edgeStore'
import type { CloudDisplayItem } from '../../stores/cloudStore'
import { formatContextWindow } from '../../lib/edge-upstream'

export type UpstreamModelListItemData = EdgeDisplayItem | CloudDisplayItem

function EditIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M12 20h9" />
      <path d="M16.5 3.5a2.12 2.12 0 013 3L7 19l-4 1 1-4 12.5-12.5z" />
    </svg>
  )
}

function DeleteIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M3 6h18" />
      <path d="M8 6V4h8v2" />
      <path d="M19 6l-1 14H6L5 6" />
      <path d="M10 11v6" />
      <path d="M14 11v6" />
    </svg>
  )
}

export interface EdgeModelListItemProps {
  item: UpstreamModelListItemData
  selected: boolean
  typeLabel: string
  selectLabel: string
  editLabel: string
  deleteLabel: string
  onSelect: () => void
  onEdit?: () => void
  onDelete?: () => void
}

export function EdgeModelListItem({
  item,
  selected,
  typeLabel,
  selectLabel,
  editLabel,
  deleteLabel,
  onSelect,
  onEdit,
  onDelete,
}: EdgeModelListItemProps) {
  const showModel = item.type === 'manual' && item.model && item.model !== item.name

  return (
    <div className={`edge-model-item${selected ? ' selected' : ''}`}>
      <button type="button" className="edge-model-item-hit" aria-label={selectLabel} onClick={onSelect} />
      <div className="edge-model-item-head">
        <span className="edge-model-item-name">{item.name}</span>
        {(onEdit || onDelete) && (
          <div className="edge-model-item-actions">
            {onEdit && (
              <button
                type="button"
                className="btn btn-ghost btn-sm btn-icon"
                aria-label={editLabel}
                onClick={onEdit}
              >
                <EditIcon />
              </button>
            )}
            {onDelete && (
              <button
                type="button"
                className="btn btn-ghost btn-sm btn-icon"
                aria-label={deleteLabel}
                onClick={onDelete}
              >
                <DeleteIcon />
              </button>
            )}
          </div>
        )}
      </div>
      {item.base_url ? <div className="edge-model-item-url">{item.base_url}</div> : null}
      {showModel ? <div className="edge-model-item-model">{item.model}</div> : null}
      <div className="edge-model-item-footer">
        <span className="tag bordered info edge-model-item-tag">{typeLabel}</span>
        {'context_window' in item && item.context_window ? (
          <span className="edge-model-item-ctx">{formatContextWindow(item.context_window)}</span>
        ) : null}
      </div>
    </div>
  )
}
