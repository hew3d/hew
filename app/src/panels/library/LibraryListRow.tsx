/**
 * LibraryListRow — one row of the Library browser's List view (playtest
 * round-3 finding #3: a real macOS Finder / Windows Explorer style list —
 * exactly three columns, Name/Type/Size, matching `LibraryListHead`'s
 * widths so header and body columns line up). Mirrors `LibraryTile`'s
 * interaction contract exactly — same selection model, same double-click/
 * Enter default action, same ⋯/right-click menu — just laid out as a
 * horizontal row instead of a card.
 */

import { useLingui } from '@lingui/react/macro'
import { categoryTypeLabel, formatBytes, isBuiltInItem } from '../../library/libraryModel'
import type { LibraryItem } from '../../library/types'
import type { LibraryListColWidths } from './LibraryListHead'
import type { MarkedState } from '../../library/libraryUpdates'
import { LibraryMark } from '../LibraryMark'
import { SourceMark } from '../SourceMark'
import { CubeGlyph, WarningGlyph } from './icons'
import { MaterialFill, tileElementId } from './LibraryTile'

export interface LibraryListRowProps {
  item: LibraryItem
  thumbUrl: string | null
  textureUrl: string | null
  selected: boolean
  /** Live placement count in the open document (0 = not placed). */
  placementCount: number
  /** Material items only: content-equal entry already in the palette. */
  inPalette: boolean
  /** Where the model's copies of this item stand against it, when they differ. */
  linkState?: MarkedState | undefined
  /** A newer version of this item exists at the source it came from. */
  sourceNewer?: boolean | undefined
  /** A source view's state label, shown after the name. */
  subText?: string | undefined
  /** No menu button: a source view's row, whose actions live in the detail pane. */
  noActions?: boolean | undefined
  /** Name/Type column widths, shared with `LibraryListHead` so the header
   * and every row line up under it. Size always takes the remainder. */
  widths: LibraryListColWidths
  onSelect: () => void
  onActivateDefault: () => void
  onOpenMenu: (anchor: HTMLElement) => void
}

export function LibraryListRow({
  item,
  thumbUrl,
  textureUrl,
  selected,
  placementCount,
  inPalette,
  linkState,
  sourceNewer = false,
  subText,
  noActions = false,
  widths,
  onSelect,
  onActivateDefault,
  onOpenMenu,
}: LibraryListRowProps) {
  const { t } = useLingui()
  const displayName = item.displayName
  const failure = item.error
  const isMaterial = item.category === 'material'
  const title = item.error ? t`${displayName} — ${failure}` : item.displayName

  return (
    <div
      id={tileElementId(item.file.fileName)}
      className={`hwlib__row${selected ? ' hwlib__row--selected' : ''}${item.error ? ' hwlib__row--error' : ''}`}
      role="row"
      aria-selected={selected}
      tabIndex={-1}
      title={title}
      data-filename={item.file.fileName}
      onClick={onSelect}
      onDoubleClick={() => {
        if (!item.error) onActivateDefault()
      }}
      onContextMenu={(e) => {
        e.preventDefault()
        onSelect()
        onOpenMenu(e.currentTarget)
      }}
    >
      <div className="hwlib__row-cell hwlib__row-cell--name" style={{ flex: `0 0 ${widths.name}px`, width: widths.name }}>
        {/* Same dot as the tile view: a placement count for components/
            models, or palette membership for materials — no separate green
            pill (matches Components' treatment exactly). */}
        {(placementCount > 0 || (isMaterial && inPalette)) && <span className="hwlib__row-dot" aria-hidden="true" />}
        <div className={`hwlib__row-thumb${isMaterial ? ' hwlib__checker' : ''}`}>
          {item.error ? (
            <WarningGlyph />
          ) : isMaterial ? (
            <MaterialFill item={item} textureUrl={textureUrl} />
          ) : thumbUrl ? (
            <img src={thumbUrl} alt="" className="hwlib__row-thumb-img" />
          ) : (
            <CubeGlyph />
          )}
        </div>
        <span className="hwlib__row-name">{item.displayName}</span>
        {linkState !== undefined && <span style={{ marginLeft: 6 }}><LibraryMark state={linkState} /></span>}
        {sourceNewer && <span style={{ marginLeft: 6 }}><SourceMark /></span>}
        {subText !== undefined && <span className="hwlib__row-state">{subText}</span>}
      </div>
      <div className="hwlib__row-cell hwlib__row-cell--type" style={{ flex: `0 0 ${widths.type}px`, width: widths.type }}>
        {item.error ? t`Failed to load` : isBuiltInItem(item) ? t`Profile (built-in)` : categoryTypeLabel(item.category)}
      </div>
      <div className="hwlib__row-cell hwlib__row-cell--size" style={{ flex: '1 1 0', minWidth: 70 }}>
        {isBuiltInItem(item) ? '\u2014' : formatBytes(item.file.size)}
      </div>
      {!isBuiltInItem(item) && !noActions && (
      <button
        type="button"
        className="hwlib__row-menu-btn"
        aria-label={t`${displayName} actions`}
        onClick={(e) => {
          e.stopPropagation()
          onSelect()
          onOpenMenu(e.currentTarget)
        }}
      >
        ⋯
      </button>
      )}
    </div>
  )
}
