/**
 * LibraryListRow — one row of the Library browser's List view (playtest
 * round-3 finding #3: a real macOS Finder / Windows Explorer style list —
 * exactly three columns, Name/Type/Size, matching `LibraryListHead`'s
 * widths so header and body columns line up). Mirrors `LibraryTile`'s
 * interaction contract exactly — same selection model, same double-click/
 * Enter default action, same ⋯/right-click menu — just laid out as a
 * horizontal row instead of a card.
 */

import { categoryTypeLabel, formatBytes } from '../../library/libraryModel'
import type { LibraryItem } from '../../library/types'
import type { LibraryListColWidths } from './LibraryListHead'
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
  widths,
  onSelect,
  onActivateDefault,
  onOpenMenu,
}: LibraryListRowProps) {
  const isMaterial = item.category === 'material'
  const title = item.error ? `${item.displayName} — ${item.error}` : item.displayName

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
      </div>
      <div className="hwlib__row-cell hwlib__row-cell--type" style={{ flex: `0 0 ${widths.type}px`, width: widths.type }}>
        {item.error ? 'Failed to load' : categoryTypeLabel(item.category)}
      </div>
      <div className="hwlib__row-cell hwlib__row-cell--size" style={{ flex: '1 1 0', minWidth: 70 }}>
        {formatBytes(item.file.size)}
      </div>
      <button
        type="button"
        className="hwlib__row-menu-btn"
        aria-label={`${item.displayName} actions`}
        onClick={(e) => {
          e.stopPropagation()
          onSelect()
          onOpenMenu(e.currentTarget)
        }}
      >
        ⋯
      </button>
    </div>
  )
}
