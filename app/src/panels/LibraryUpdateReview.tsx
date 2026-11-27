/**
 * LibraryUpdateReview — "Review…" on the Library notice bar (v1.5 OW-2): every
 * part of this model that came from the Library and differs from it now, with
 * its state in plain words and a checkbox. Parts the Library changed start
 * checked; parts changed on both sides ("your changes here would be
 * replaced") and parts Hew cannot order start unchecked, with a warning;
 * locked parts are shown but cannot be checked. After an update the same
 * dialog says what was left alone and why.
 */

import { useCallback, useEffect, useState } from 'react'
import { plural } from '@lingui/core/macro'
import { Trans, useLingui } from '@lingui/react/macro'
import { isComposingKey } from '../input/keyboard'
import {
  defaultChecked,
  keptCopy,
  outcomeSummary,
  reviewRows,
  rowBlockedReason,
  rowName,
  rowNote,
  skippedCopy,
  stateLabel,
  type LibraryLink,
  type ReviewRow,
  type UpdateOutcome,
} from '../library/libraryUpdates'
import { BUTTON_STYLE, PRIMARY_STYLE } from './FollowNoticeBar'

export interface LibraryUpdateReviewProps {
  links: readonly LibraryLink[]
  /** The name the Library shows for an item, when known. */
  itemName?: (item: string) => string | undefined
  /** What the last update did, shown above the list. */
  outcome: UpdateOutcome | null
  /** The links as they were when that update ran (for its summary count). */
  outcomeLinks: readonly LibraryLink[]
  busy: boolean
  onUpdateSelected: (definitions: string[]) => void
  onUpdateAllNewer: () => void
  onClose: () => void
}

const OVERLAY: React.CSSProperties = {
  position: 'fixed',
  inset: 0,
  background: 'var(--backdrop-dim, rgba(0,0,0,0.6))',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  zIndex: 2000,
}

const CARD: React.CSSProperties = {
  background: 'var(--surface-overlay, #2a2a2a)',
  border: '1px solid var(--border-strong, #4a4a4a)',
  borderRadius: 'var(--radius-control, 6px)',
  boxShadow: 'var(--shadow-palette, 0 8px 32px rgba(0,0,0,0.6))',
  padding: '20px 24px',
  width: '520px',
  maxWidth: '92vw',
  maxHeight: '80vh',
  display: 'flex',
  flexDirection: 'column',
  gap: '12px',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  color: 'var(--text-secondary, #ddd)',
}

export function LibraryUpdateReview({
  links,
  itemName,
  outcome,
  outcomeLinks,
  busy,
  onUpdateSelected,
  onUpdateAllNewer,
  onClose,
}: LibraryUpdateReviewProps) {
  const { t } = useLingui()
  const rows = reviewRows(links, itemName)
  const [overrides, setOverrides] = useState<Record<string, boolean>>({})
  const isChecked = (row: ReviewRow): boolean => !row.locked && (overrides[row.key] ?? defaultChecked(row))
  const checkedRows = rows.filter(isChecked)
  const anyNewer = rows.some((r) => r.state === 'library_newer' && !r.locked)

  const onKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (isComposingKey(e)) return
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        onClose()
      }
    },
    [onClose],
  )
  useEffect(() => {
    document.addEventListener('keydown', onKeyDown)
    return () => document.removeEventListener('keydown', onKeyDown)
  }, [onKeyDown])

  const heading = t`Library updates`
  return (
    <div style={OVERLAY} onClick={onClose}>
      <div style={CARD} role="dialog" aria-modal="true" aria-label={heading} data-testid="library-update-review" onClick={(e) => e.stopPropagation()}>
        <div style={{ fontSize: '15px', fontWeight: 600, color: 'var(--text-primary, #eee)' }}>{heading}</div>

        {outcome !== null && (
          <div data-testid="library-update-outcome" role="status" style={{ fontSize: '13px', lineHeight: 1.5 }}>
            <div style={{ color: 'var(--text-primary, #eee)' }}>{outcomeSummary(outcome, outcomeLinks)}</div>
            {outcome.kept.length > 0 && (
              <div data-testid="library-update-kept" style={{ marginTop: '4px', color: 'var(--text-tertiary, #ccc)' }}>
                {keptCopy(outcome, outcomeLinks).join(' ')}
              </div>
            )}
            {outcome.skipped.length > 0 && (
              <ul style={{ margin: '6px 0 0', paddingLeft: '18px', color: 'var(--text-tertiary, #ccc)' }}>
                {outcome.skipped.map((s) => (
                  <li key={s.definition} data-testid="library-update-skipped">
                    {skippedCopy(s)}
                  </li>
                ))}
              </ul>
            )}
          </div>
        )}

        <div style={{ overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: '8px', minHeight: 0 }}>
          {rows.length === 0 ? (
            <div style={{ fontSize: '13px', color: 'var(--text-faint, #888)' }}>
              <Trans>Every part from the Library matches it.</Trans>
            </div>
          ) : (
            rows.map((row) => {
              const blocked = rowBlockedReason(row)
              const note = rowNote(row)
              const name = rowName(row)
              const more = row.names.length - 1
              const warn = row.state !== 'library_newer'
              return (
                <label
                  key={row.key}
                  data-testid="library-review-row"
                  data-state={row.state}
                  style={{
                    display: 'flex',
                    gap: '10px',
                    alignItems: 'flex-start',
                    padding: '8px 10px',
                    border: `1px solid ${warn ? 'var(--warning-border, #8a6d1f)' : 'var(--border-panel, #444)'}`,
                    borderRadius: 'var(--radius-control, 4px)',
                    opacity: blocked !== null ? 0.6 : 1,
                  }}
                >
                  <input
                    type="checkbox"
                    aria-label={t`Update ${name}`}
                    checked={isChecked(row)}
                    disabled={blocked !== null}
                    onChange={(e) => setOverrides((o) => ({ ...o, [row.key]: e.target.checked }))}
                    style={{ marginTop: '3px' }}
                  />
                  <span style={{ display: 'flex', flexDirection: 'column', gap: '2px', minWidth: 0, flex: 1 }}>
                    <span style={{ fontSize: '13px', color: 'var(--text-primary, #eee)' }}>
                      {name}
                      {more > 0 && (
                        <span style={{ color: 'var(--text-faint, #888)' }}>
                          {' '}
                          {plural(more, { one: '+ # more part', other: '+ # more parts' })}
                        </span>
                      )}
                    </span>
                    <span style={{ fontSize: '12px', color: warn ? 'var(--warning-text, #e0b341)' : 'var(--text-tertiary, #ccc)' }}>
                      {stateLabel(row.state)}
                      {' · '}
                      {plural(row.copies, { one: '# copy in this model', other: '# copies in this model' })}
                    </span>
                    {note !== null && <span style={{ fontSize: '12px', color: 'var(--text-tertiary, #ccc)' }}>{note}</span>}
                    {blocked !== null && <span style={{ fontSize: '12px', color: 'var(--text-tertiary, #ccc)' }}>{blocked}</span>}
                  </span>
                </label>
              )
            })
          )}
        </div>

        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '10px', flexWrap: 'wrap' }}>
          <button type="button" style={BUTTON_STYLE} onClick={onClose} data-testid="library-review-close">
            {outcome !== null ? t`Done` : t`Cancel`}
          </button>
          <button
            type="button"
            style={BUTTON_STYLE}
            disabled={busy || !anyNewer}
            onClick={onUpdateAllNewer}
            data-testid="library-review-update-newer"
          >
            {t`Update all that changed in the Library`}
          </button>
          <button
            type="button"
            style={PRIMARY_STYLE}
            disabled={busy || checkedRows.length === 0}
            onClick={() => onUpdateSelected(checkedRows.flatMap((r) => r.definitions))}
            data-testid="library-review-update-selected"
          >
            {t`Update Selected`}
          </button>
        </div>
      </div>
    </div>
  )
}
