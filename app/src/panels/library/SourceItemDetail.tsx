/**
 * SourceItemDetail — the detail pane for an item in a source's view
 * (docs/design/shared-library-sources.md 5.1, 5.2, 7.5): its picture, name,
 * state in words, the license (shown BEFORE either action), and the two
 * actions: "Add to my Library" and "Insert". A newer item also offers "Update
 * from source". Nothing here edits the item; it is not in the Library yet, or
 * its Library copy is edited in the Library's own pane.
 */

import { Trans, useLingui } from '@lingui/react/macro'
import { categoryTypeLabel, formatBytes } from '../../library/libraryModel'
import { rowLibraryItem, rowLicense, rowStateLabel } from '../../library/sourceModel'
import type { IndexSummary, ItemRow } from '../../library/sources'
import { CubeGlyph } from './icons'

export interface SourceItemDetailProps {
  row: ItemRow | null
  /** The source's header facts, for the license an item inherits. */
  summary: IndexSummary | null
  thumbUrl: string | null
  width: number
  native: boolean
  /** The catalog removed the source: nothing is added or updated from it. */
  removed: boolean
  /** The catalog's reason, shown in the pane when the source was removed. */
  removedText?: string | null | undefined
  /** Another action is running. */
  busy: boolean
  /** The message from the last action, if it failed. */
  error: string | null
  onAdd(): void
  onInsert(): void
  onUpdate(): void
}

export function SourceItemDetail({ row, summary, thumbUrl, width, native, removed, removedText = null, busy, error, onAdd, onInsert, onUpdate }: SourceItemDetailProps) {
  const { t } = useLingui()
  const widthStyle: React.CSSProperties = { flex: `0 0 ${width}px`, width }
  const btn = (styled: string): string => (native ? 'hwlib__native-btn' : styled)
  if (row === null) {
    return (
      <div className="hwlib__detail hwlib__detail--empty" style={widthStyle}>
        <p><Trans>Select an item to see its details.</Trans></p>
      </div>
    )
  }
  const item = rowLibraryItem(row)
  const inLibrary = row.file !== null
  // An id in the Library with no note (`in_library`) can still be added: the module
  // asks "Replace the item in your Library?" (design 5.2). Linked rows cannot.
  const canAdd = (row.state === 'not_in_library' || row.state === 'in_library') && !removed
  const canUpdate = row.state === 'newer' && !removed
  // Components and models can be placed; a material is painted from the
  // Library once it is there, and a profile is used from the Library.
  const canInsert = (row.category === 'Components' || row.category === 'Models') && (inLibrary || !removed) && row.state !== 'in_library_from_other'
  const fromOther = row.state === 'in_library_from_other'
  const license = rowLicense(row, summary)
  const keywords = row.entry?.keywords ?? []
  const collection = row.entry?.collection
  const stateLabel = rowStateLabel(row)
  const warn = row.state === 'gone' || fromOther

  return (
    <div className="hwlib__detail" style={widthStyle} data-testid="source-item-detail" data-state={row.state}>
      <div className="hwlib__detail-thumb">
        {thumbUrl ? <img src={thumbUrl} alt="" className="hwlib__detail-thumb-img" /> : <CubeGlyph />}
      </div>
      <div className="hwlib__detail-name">{row.name}</div>
      <div className={`hwlib__src-state${warn ? ' hwlib__src-state--warn' : ''}`} data-testid="source-item-state">
        {stateLabel}
      </div>
      <div className="hwlib__meta">
        <div data-testid="source-item-license">
          <Trans>License: {license}</Trans>
        </div>
        <div>
          {categoryTypeLabel(item.category)}
          {row.entry !== null ? ` · ${formatBytes(row.entry.bytes)}` : ''}
        </div>
        {collection != null && collection !== '' && <div><Trans>Collection: {collection}</Trans></div>}
      </div>
      {keywords.length > 0 && (
        <div className="hwlib__keywords" aria-label={t`Keywords`}>
          {keywords.map((kw) => (
            <span key={kw} className="hwlib__keyword-chip">{kw}</span>
          ))}
        </div>
      )}
      {removed && removedText !== null && <div className="hwlib__src-warning" data-testid="source-item-removed">{removedText}</div>}
      {error !== null && <div className="hwlib__action-error" role="alert">{error}</div>}

      <div className="hwlib__actions">
        <button type="button" className={btn('hwlib__btn-primary')} disabled={!canAdd || busy} onClick={onAdd}>
          <Trans>Add to my Library</Trans>
        </button>
        {canUpdate && (
          <button type="button" className={btn('hwlib__btn-secondary')} disabled={busy} onClick={onUpdate}>
            <Trans>Update from source</Trans>
          </button>
        )}
        {(row.category === 'Components' || row.category === 'Models') && (
          <button type="button" className={btn('hwlib__btn-secondary')} disabled={!canInsert || busy} onClick={onInsert}>
            <Trans>Insert</Trans>
          </button>
        )}
      </div>
    </div>
  )
}
