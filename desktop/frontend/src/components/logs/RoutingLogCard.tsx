import { useState } from 'react'
import type { RoutingLogEntry } from '../../lib/routing-log'
import {
  extractDifficultyScore,
  parseDifficultyBreakdown,
  pickDisplayReasonCodes,
  pickFinalRouteFactorCode,
} from '../../lib/routing-log'
import { explainDifficultyPartKey, explainFinalRouteFactor, stepKindLabel } from '../../lib/routing-reasons'
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

function ToggleButton({
  expanded,
  onClick,
  label,
}: {
  expanded: boolean
  onClick: () => void
  label: string
}) {
  return (
    <button
      type="button"
      className={`routing-log-card__toggle${expanded ? ' expanded' : ''}`}
      aria-expanded={expanded}
      onClick={onClick}
    >
      {label}
      <svg className="routing-log-card__toggle-icon" viewBox="0 0 24 24" aria-hidden="true">
        <path d="M6 9l6 6 6-6" fill="none" stroke="currentColor" strokeWidth="2" />
      </svg>
    </button>
  )
}

export function RoutingLogCard({ entry }: Props) {
  const { t } = useI18n()
  const [difficultyExpanded, setDifficultyExpanded] = useState(false)
  const [errorExpanded, setErrorExpanded] = useState(false)
  const previewText = entry.hasUserPreview
    ? entry.userPreview
    : t('logs.noMessagePreview')
  const previewTitle = entry.hasUserPreview ? entry.userPreview : undefined
  const { shown: reasonTags, overflow } = pickDisplayReasonCodes(entry.reasonCodes)
  const finalDifficulty = extractDifficultyScore(entry.reasonCodes, entry.difficulty)
  const finalFactorCode = pickFinalRouteFactorCode(entry.reasonCodes, entry.route)
  const finalFactorText = finalFactorCode
    ? explainFinalRouteFactor(finalFactorCode, entry.route, t)
    : null
  const difficultyBreakdown = parseDifficultyBreakdown(entry.reasonCodes)
  const hasErrorReason = Boolean(entry.errorReason?.trim())
  const showDifficultyToggle = entry.reasonCodes.length > 0
  const showExpandSection = showDifficultyToggle || hasErrorReason

  function formatSigned(n: number | null, digits = 3): string {
    if (n == null || !Number.isFinite(n)) return '—'
    const sign = n >= 0 ? '+' : ''
    return `${sign}${n.toFixed(digits)}`
  }

  return (
    <article className="routing-log-card">
      <div className="routing-log-card__meta">
        {entry.timeLabel && <time dateTime={entry.timestamp}>{entry.timeLabel}</time>}
        {entry.servedModel || entry.model ? (
          <>
            <span className="routing-log-card__sep">·</span>
            <span className="routing-log-card__model">{entry.servedModel ?? entry.model}</span>
          </>
        ) : null}
        {entry.tokensIn != null && entry.tokensOut != null && (
          <>
            <span className="routing-log-card__sep">·</span>
            <div className="routing-log-card__tokens">
              <span className="routing-log-card__token">{t('logs.tokensTotal')}: {entry.tokensIn + entry.tokensOut}</span>
              <span className="routing-log-card__token-sep">·</span>
              <span className="routing-log-card__token">{t('logs.tokensIn')}: {entry.tokensIn}</span>
              <span className="routing-log-card__token-sep">·</span>
              <span className="routing-log-card__token">{t('logs.tokensOut')}: {entry.tokensOut}</span>
              {entry.cachedTokens != null && entry.cachedTokens > 0 && (
                <>
                  <span className="routing-log-card__token-sep">·</span>
                  <span className="routing-log-card__token">{t('logs.tokensCached')}: {entry.cachedTokens}</span>
                </>
              )}
            </div>
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
        {hasErrorReason && (
          <span className="tag bordered danger routing-log-card__reason-tag">{t('logs.requestErrorReason')}</span>
        )}
      </div>
      {showExpandSection && (
        <div className="routing-log-card__expand">
          <div className="routing-log-card__toggle-row">
            {showDifficultyToggle && (
              <ToggleButton
                expanded={difficultyExpanded}
                onClick={() => setDifficultyExpanded((v) => !v)}
                label={difficultyExpanded ? t('logs.hideReasons') : t('logs.showReasons')}
              />
            )}
            {hasErrorReason && (
              <ToggleButton
                expanded={errorExpanded}
                onClick={() => setErrorExpanded((v) => !v)}
                label={errorExpanded ? t('logs.hideErrorReason') : t('logs.showErrorReason')}
              />
            )}
          </div>
          {difficultyExpanded && (
            <>
              {difficultyBreakdown && (
                <div className="routing-log-card__breakdown">
                  <p className="routing-log-card__breakdown-title">{t('logs.difficultyBreakdown')}</p>
                  {difficultyBreakdown.parts.length > 0 && (
                    <table className="routing-log-card__breakdown-table">
                      <thead>
                        <tr>
                          <th>{t('logs.difficultyFactor')}</th>
                          <th>{t('logs.difficultyLinear')}</th>
                          <th>{t('logs.difficultyScoreDelta')}</th>
                        </tr>
                      </thead>
                      <tbody>
                        {difficultyBreakdown.parts.map((part) => (
                          <tr key={part.key}>
                            <td>{explainDifficultyPartKey(part.key, t)}</td>
                            <td>{formatSigned(part.linear)}</td>
                            <td>{formatSigned(part.scoreDelta, 4)}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  )}
                  {difficultyBreakdown.linearSum != null && (
                    <p className="routing-log-card__breakdown-meta">
                      {t('routing.reason.DIFF_LINEAR_SUM', {
                        sum: difficultyBreakdown.linearSum.toFixed(3),
                      })}
                    </p>
                  )}
                  {difficultyBreakdown.heuristic != null && (
                    <p className="routing-log-card__breakdown-meta">
                      {t('logs.difficultyHeuristic')}: {difficultyBreakdown.heuristic.toFixed(4)}
                    </p>
                  )}
                  {difficultyBreakdown.fuse && (
                    <p className="routing-log-card__breakdown-meta">
                      {t('logs.difficultyFuseDetail', {
                        heur: difficultyBreakdown.fuse.heur.toFixed(4),
                        bayes: difficultyBreakdown.fuse.bayes.toFixed(4),
                        w: difficultyBreakdown.fuse.w.toFixed(2),
                        final: difficultyBreakdown.fuse.final.toFixed(4),
                      })}
                    </p>
                  )}
                  {difficultyBreakdown.adjustments.length > 0 && (
                    <div className="routing-log-card__breakdown-adjustments">
                      <p className="routing-log-card__breakdown-subtitle">{t('logs.difficultyAdjustments')}</p>
                      <ul>
                        {difficultyBreakdown.adjustments.map((adj) => (
                          <li key={adj.key}>
                            {explainDifficultyPartKey(adj.key, t)}: {formatSigned(adj.scoreDelta, 4)}
                          </li>
                        ))}
                      </ul>
                    </div>
                  )}
                </div>
              )}
              {(finalDifficulty != null || finalFactorText) && (
                <div className="routing-log-card__conclusion">
                  {finalDifficulty != null && (
                    <p className="routing-log-card__conclusion-row">
                      <span className="routing-log-card__conclusion-label">{t('logs.finalDifficulty')}</span>
                      <span className="routing-log-card__conclusion-value">
                        {finalDifficulty.toFixed(2)}
                      </span>
                    </p>
                  )}
                  {finalFactorText && (
                    <p className="routing-log-card__conclusion-row">
                      <span className="routing-log-card__conclusion-label">{t('logs.finalRouteDecision')}</span>
                      <span className="routing-log-card__conclusion-value">
                        {t('logs.finalRouteDecisionValue', {
                          reason: finalFactorText,
                          route: t(`route.${entry.route}`),
                        })}
                      </span>
                    </p>
                  )}
                </div>
              )}
            </>
          )}
          {errorExpanded && hasErrorReason && (
            <div className="routing-log-card__error-reason">
              <p className="routing-log-card__breakdown-title">{t('logs.requestErrorReason')}</p>
              <pre className="routing-log-card__error-text">{entry.errorReason}</pre>
            </div>
          )}
        </div>
      )}
    </article>
  )
}
