/**
 * SourceUpdateReview — "Review…" on the sources bar
 * (docs/design/shared-library-sources.md 5.2, 7.5). Modeled on
 * LibraryUpdateReview: every item with something newer at its source, its
 * source, the name it will take, and its result once an update has run. An
 * item you changed here is never replaced in bulk: its row asks "Replace your
 * changes" or "Keep mine". New items of a followed source are listed too.
 */

import { useEffect, useState } from 'react'
import { t as tCore } from '@lingui/core/macro'
import { Trans, useLingui } from '@lingui/react/macro'
import { isComposingKey } from '../input/keyboard'
import type { ReviewItem, ReviewState } from '../library/useSourceActions'

export interface SourceUpdateReviewProps {
  review: ReviewState
  onUpdateSelected: (keys: string[]) => void
  onDecide: (key: string, decision: 'replace' | 'keep') => void
  onClose: () => void
}

const OVERLAY: React.CSSProperties = {
  position: 'fixed',
  inset: 0,
  background: 'var(--backdrop-dim, rgba(0,0,0,0.6))',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  zIndex: 2100,
}

const CARD: React.CSSProperties = {
  background: 'var(--surface-overlay, #2a2a2a)',
  border: '1px solid var(--border-strong, #4a4a4a)',
  borderRadius: 'var(--radius-control, 6px)',
  boxShadow: 'var(--shadow-palette, 0 8px 32px rgba(0,0,0,0.6))',
  padding: '20px 24px',
  width: '560px',
  maxWidth: '92vw',
  maxHeight: '80vh',
  display: 'flex',
  flexDirection: 'column',
  gap: '12px',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  color: 'var(--text-secondary, #ddd)',
}

export function SourceUpdateReview({ review, onUpdateSelected, onDecide, onClose }: SourceUpdateReviewProps) {
  const { t } = useLingui()
  const [overrides, setOverrides] = useState<Record<string, boolean>>({})
  const done = (item: ReviewItem): boolean => review.results[item.key] !== undefined && review.results[item.key].kind !== 'needs_review'
  const isChecked = (item: ReviewItem): boolean => !item.edited && !done(item) && (overrides[item.key] ?? true)
  const checked = review.items.filter(isChecked)
  // Nothing is left to choose when every row is finished or waits on its own
  // two buttons: only Done remains, and it is the default.
  const canUpdate = review.items.some((i) => !i.edited && !done(i))

  useEffect(() => {
    function onKey(e: KeyboardEvent): void {
      if (isComposingKey(e) || e.key !== 'Escape') return
      e.preventDefault()
      e.stopPropagation()
      onClose()
    }
    document.addEventListener('keydown', onKey, true)
    return () => document.removeEventListener('keydown', onKey, true)
  }, [onClose])

  const heading = t`Source updates`
  const anyResult = Object.keys(review.results).length > 0
  return (
    <div style={OVERLAY} onClick={onClose}>
      <div style={CARD} role="dialog" aria-modal="true" aria-label={heading} data-testid="source-update-review" onClick={(e) => e.stopPropagation()}>
        <div style={{ fontSize: '15px', fontWeight: 600, color: 'var(--text-primary, #eee)' }}>{heading}</div>
        <div style={{ overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: '8px', minHeight: 0 }}>
          {review.items.length === 0 ? (
            <div style={{ fontSize: '13px', color: 'var(--text-faint, #888)' }}>
              <Trans>Everything from your sources is up to date.</Trans>
            </div>
          ) : (
            review.items.map((item) => {
              const result = review.results[item.key]
              const name = item.name
              const edited = item.edited && (result === undefined || result.kind === 'needs_review')
              const rename = item.kind === 'newer' && item.localName !== null && item.localName !== item.name
              const sourceName = item.sourceName
              const localName = item.localName ?? ''
              return (
                <div key={item.key} className={`hwlib__src-review-row${edited ? ' hwlib__src-review-row--warn' : ''}`} data-testid="source-review-row" data-kind={item.kind} data-edited={item.edited ? 'true' : 'false'}>
                  {!edited && result === undefined && (
                    <input
                      type="checkbox"
                      aria-label={item.kind === 'new' ? t`Add ${name}` : t`Update ${name}`}
                      checked={isChecked(item)}
                      onChange={(e) => setOverrides((o) => ({ ...o, [item.key]: e.target.checked }))}
                      style={{ marginTop: '3px' }}
                    />
                  )}
                  <span className="hwlib__src-review-body">
                    <span style={{ color: 'var(--text-primary, #eee)' }} data-testid="source-review-name">{name}</span>
                    <span className="hwlib__src-review-sub" data-testid="source-review-source">
                      {item.kind === 'new' ? t`New at ${sourceName}` : t`From ${sourceName}`}
                    </span>
                    {rename && <span className="hwlib__src-review-sub" data-testid="source-review-rename">{t`Now named “${localName}”. It will take the source’s name.`}</span>}
                    {edited && (
                      <>
                        <span className="hwlib__src-review-warn"><Trans>You changed this item here. Taking the newer version replaces your changes.</Trans></span>
                        <span className="hwlib__src-review-choices">
                          <button type="button" className="hwlib__btn-secondary" disabled={review.busy} onClick={() => onDecide(item.key, 'keep')}>
                            <Trans>Keep mine</Trans>
                          </button>
                          <button type="button" className="hwlib__btn-primary" disabled={review.busy} onClick={() => onDecide(item.key, 'replace')}>
                            <Trans>Replace your changes</Trans>
                          </button>
                        </span>
                      </>
                    )}
                    {result !== undefined && result.kind !== 'needs_review' && (
                      <span className={`hwlib__src-review-result${result.kind === 'failed' ? ' hwlib__src-review-result--failed' : ''}`} data-testid="source-review-result" data-result={result.kind}>
                        {result.kind === 'failed' ? result.message.text : resultText(result.kind)}
                      </span>
                    )}
                    {result !== undefined && result.kind !== 'failed' && result.note !== undefined && (
                      <span className="hwlib__src-review-result hwlib__src-review-result--failed" data-testid="source-review-note">
                        {result.note.text}
                      </span>
                    )}
                  </span>
                </div>
              )
            })
          )}
        </div>
        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '10px', flexWrap: 'wrap' }}>
          <button type="button" className={canUpdate ? 'hwlib__btn-secondary' : 'hwlib__btn-primary'} onClick={onClose} data-testid="source-review-close" autoFocus={!canUpdate}>
            {anyResult || !canUpdate ? t`Done` : t`Cancel`}
          </button>
          {canUpdate && (
            <button
              type="button"
              className="hwlib__btn-primary"
              disabled={review.busy || checked.length === 0}
              onClick={() => onUpdateSelected(checked.map((i) => i.key))}
              data-testid="source-review-update-selected"
            >
              {t`Update selected`}
            </button>
          )}
        </div>
      </div>
    </div>
  )
}

function resultText(kind: 'updated' | 'added' | 'replaced' | 'kept'): string {
  switch (kind) {
    case 'updated':
      return tCore`Updated`
    case 'added':
      return tCore`Added`
    case 'replaced':
      return tCore`Replaced with the source’s version`
    case 'kept':
      return tCore`Kept yours`
  }
}
