/**
 * SourcesBlock — the sidebar's "Sources" block under Collections
 * (docs/design/shared-library-sources.md 7.5): one row per added source,
 * "Browse the catalog", and "Add a source…". A followed source shows a mark;
 * one the catalog removed, or that could not be reached the last time Hew
 * looked, shows a warning mark. Where sources are Off the block is one line
 * and nothing else.
 */

import { Trans, useLingui } from '@lingui/react/macro'
import { sourcesOffText } from '../../library/sourceMessages'
import type { SourceViewTarget } from '../../library/sourceModel'
import type { SourceStatus } from '../../library/sources'

export interface SourcesBlockProps {
  off: boolean
  catalogEnabled: boolean
  statuses: SourceStatus[]
  active: SourceViewTarget | null
  onOpenCatalog(): void
  onOpenSource(source: string): void
  onAddSource(): void
}

export function SourcesBlock({ off, catalogEnabled, statuses, active, onOpenCatalog, onOpenSource, onAddSource }: SourcesBlockProps) {
  const { t } = useLingui()
  return (
    <>
      <div className="hwlib__section-label"><Trans>Sources</Trans></div>
      <div className="hwlib__sources" data-testid="library-sources">
        {off ? (
          <div className="hwlib__src-off" data-testid="library-sources-off">{sourcesOffText()}</div>
        ) : (
          <>
            {statuses.map((s) => {
              const isActive = active?.kind === 'source' && active.source === s.entry.source
              const removedReason = s.removed?.reason
              const warning =
                s.removed !== null
                  ? removedReason
                    ? t`The Hew catalog removed this source: ${removedReason}`
                    : t`The Hew catalog removed this source.`
                  : s.unreachable
                    ? t`Hew could not reach this source the last time it checked.`
                    : null
              return (
                <button
                  key={s.entry.source}
                  type="button"
                  className={`hwlib__src-row${isActive ? ' hwlib__src-row--active' : ''}`}
                  data-testid="library-source-row"
                  aria-current={isActive ? 'true' : undefined}
                  onClick={() => onOpenSource(s.entry.source)}
                >
                  <span className="hwlib__src-row-name">{s.entry.name}</span>
                  <span className="hwlib__src-marks">
                    {s.followed && (
                      <span className="hwlib__src-mark--followed" role="img" aria-label={t`Following`} title={t`Following`} data-testid="source-followed-mark">
                        {'↻'}
                      </span>
                    )}
                    {warning !== null && (
                      <span className="hwlib__src-mark--warning" role="img" aria-label={warning} title={warning} data-testid="source-warning-mark">
                        {'⚠'}
                      </span>
                    )}
                  </span>
                </button>
              )
            })}
            {catalogEnabled && (
              <button
                type="button"
                className={`hwlib__src-row hwlib__src-action${active?.kind === 'catalog' || active?.kind === 'preview' ? ' hwlib__src-row--active' : ''}`}
                data-testid="library-browse-catalog"
                onClick={onOpenCatalog}
              >
                <Trans>Browse the catalog</Trans>
              </button>
            )}
            <button type="button" className="hwlib__src-row hwlib__src-action" data-testid="library-add-source" onClick={onAddSource}>
              <Trans>Add a source…</Trans>
            </button>
          </>
        )}
      </div>
    </>
  )
}
