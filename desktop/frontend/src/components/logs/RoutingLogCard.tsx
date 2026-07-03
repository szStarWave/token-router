import { useState } from 'react'
import type { RoutingLogEntry } from '../../lib/routing-log'
import { pickDisplayReasonCodes, truncatePreview } from '../../lib/routing-log'
import { explainReasonCode, stepKindLabel } from '../../lib/routing-reasons'
import { useI18n } from '../../hooks/useI18n'

interface Props {
  entry: RoutingLogEntry
}

function routeTagClass(route: RoutingLogEntry['route']): string {
  switch (route) {
    case 'edge':
      return 'ok'
    case 'cloud':
      return 'info'
    case 'cascade':
      return 'warn'
  }
}

export function RoutingLogCard({ entry }: Props) {
  const { t } = useI18n()
  const [expanded, setExpanded] = useState(false)
  const previewText = entry.hasUserPreview
    ? truncatePreview(entry.userPreview)
    : t('logs.noMessagePreview')
  const previewTitle = entry.hasUserPreview ? entry.userPreview : undefined
  const { shown: reasonTags, overflow } = pickDisplayReasonCodes(entry.reasonCodes)

  return (
    <article className="routing-log-card">
      <div className="routing-log-card__meta">
        {entry.timeLabel && <time dateTime={entry.timestamp}>{entry.timeLabel}</time>}
        {entry.model && (
          <>
            <span className="routing-log-card__sep">·</span>
            <span className="routing-log-card__model">{entry.model}</span>
          </>
        )}
      </div>
      <p className="routing-log-card__preview" title={previewTitle}>
        <span className="routing-log-card__preview-label">{t('logs.messagePreview')}:</span>{' '}
        {previewText}
      </p>
      <div className="routing-log-card__tags">
        <span className={`tag bordered ${routeTagClass(entry.route)}`}>{t(`route.${entry.route}`)}</span>
        <span className="tag bordered neutral">{stepKindLabel(entry.stepKind, t)}</span>
        {reasonTags.map((code) => (
          <span key={code} className="tag bordered neutral routing-log-card__reason-tag">
            {code}
          </span>
        ))}
        {overflow > 0 && (
          <span className="tag bordered neutral routing-log-card__reason-tag">+{overflow}</span>
        )}
      </div>
      {entry.reasonCodes.length > 0 && (
        <div className="routing-log-card__expand">
          <button
            type="button"
            className={`routing-log-card__toggle${expanded ? ' expanded' : ''}`}
            aria-expanded={expanded}
            onClick={() => setExpanded((v) => !v)}
          >
            {expanded ? t('logs.hideReasons') : t('logs.showReasons')}
            <svg className="routing-log-card__toggle-icon" viewBox="0 0 24 24" aria-hidden="true">
              <path d="M6 9l6 6 6-6" fill="none" stroke="currentColor" strokeWidth="2" />
            </svg>
          </button>
          {expanded && (
            <ul className="routing-log-card__details">
              {entry.reasonCodes.map((code) => (
                <li key={code}>
                  <code className="routing-log-card__code">{code}</code>
                  <span className="routing-log-card__explain">{explainReasonCode(code, t)}</span>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </article>
  )
}
