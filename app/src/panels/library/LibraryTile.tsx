/**
 * LibraryTile — one grid cell of the Library browser (frames 1a/1b).
 *
 * Two visual shapes share one component: a rendered-thumbnail card for
 * components/models, and a flat swatch card for materials (the material
 * itself IS the thumbnail — color fill or texture image, translucent
 * entries composited over a checkerboard so partial opacity reads at a
 * glance). Both shapes share selection/hover/focus chrome, the in-model dot
 * badge, and the hover-revealed action row (Insert/Open/Paint button + ⋯).
 */

import { materialSubline } from '../../library/libraryModel'
import type { LibraryItem } from '../../library/types'
import { CubeGlyph, WarningGlyph } from './icons'

/** Stable DOM id for a tile, shared with the grid's `aria-activedescendant`
 * (S21) so the two sites can't drift. `encodeURIComponent` keeps the id a
 * valid HTML id token (no whitespace) regardless of what characters the
 * underlying file name contains. */
export function tileElementId(fileName: string): string {
  return `hwlib-tile-${encodeURIComponent(fileName)}`
}

export interface LibraryTileProps {
  item: LibraryItem
  thumbUrl: string | null
  textureUrl: string | null
  selected: boolean
  /** Live placement count in the open document (0 = not placed). */
  placementCount: number
  /** Materials only: whether this item's material already exists in the
   * document's palette. */
  inPalette: boolean
  /** Label for the hover-revealed primary action button (category default). */
  defaultActionLabel: string
  onSelect: () => void
  onActivateDefault: () => void
  onOpenMenu: (anchor: HTMLElement) => void
}

export function LibraryTile({
  item,
  thumbUrl,
  textureUrl,
  selected,
  placementCount,
  inPalette,
  defaultActionLabel,
  onSelect,
  onActivateDefault,
  onOpenMenu,
}: LibraryTileProps) {
  const isMaterial = item.category === 'material'
  const title = item.error ? `${item.displayName} — ${item.error}` : item.displayName

  return (
    <div
      id={tileElementId(item.file.fileName)}
      className={`hwlib__tile${isMaterial ? ' hwlib__tile--material' : ''}${selected ? ' hwlib__tile--selected' : ''}${item.error ? ' hwlib__tile--error' : ''}`}
      role="option"
      aria-selected={selected}
      tabIndex={-1}
      title={title}
      data-filename={item.file.fileName}
      onClick={onSelect}
      onDoubleClick={() => {
        if (!item.error) onActivateDefault()
      }}
      onContextMenu={(e) => {
        // Right-click opens the same item menu the ⋯ button does (native in
        // the desktop Library window, DOM popover on the web) — additional
        // to the button, never the only path (settled design decision #5).
        e.preventDefault()
        onSelect()
        onOpenMenu(e.currentTarget)
      }}
    >
      {/* The blue "in this model" dot: a placement count for components/
          models, or palette membership for materials (which aren't
          "placed" as instances — same treatment as Components, not a
          separate green pill). */}
      {(placementCount > 0 || (isMaterial && inPalette)) && <span className="hwlib__tile-dot" aria-hidden="true" />}

      {isMaterial ? (
        <div className="hwlib__tile-swatch hwlib__checker">
          {item.error ? (
            <div className="hwlib__tile-error-fill">
              <WarningGlyph />
            </div>
          ) : (
            <MaterialFill item={item} textureUrl={textureUrl} />
          )}
        </div>
      ) : (
        <div className="hwlib__tile-thumb">
          {item.error ? (
            <WarningGlyph />
          ) : thumbUrl ? (
            <img src={thumbUrl} alt="" className="hwlib__tile-thumb-img" />
          ) : (
            <CubeGlyph />
          )}
        </div>
      )}

      {!item.error && (
        <div className="hwlib__tile-actions">
          <button
            type="button"
            className="hwlib__tile-action-btn"
            onClick={(e) => {
              e.stopPropagation()
              onActivateDefault()
            }}
          >
            {defaultActionLabel}
          </button>
          <button
            type="button"
            className="hwlib__tile-menu-btn"
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
      )}

      <div className="hwlib__tile-name">{item.displayName}</div>
      <div className="hwlib__tile-sub">
        {item.error
          ? 'Failed to load'
          : isMaterial
            ? materialSubline(item.summary.material_entries[0] ?? emptyMaterial())
            : placementCount > 0
              ? `${placementCount} in this model`
              : ''}
      </div>
    </div>
  )
}

/** A neutral placeholder material row for a category-material item whose
 * summary has no entries at all — shared with `LibraryListRow`, whose rows
 * hit the same edge case. */
export function emptyMaterial() {
  return { name: '', color: [200, 200, 200, 255] as [number, number, number, number], texture_asset: null, texture_format: null, texture_world_size: null }
}

/** The color/texture layer painted over the checkerboard base — a plain
 * rgba fill for a flat color, or the fetched texture image at the
 * material's own opacity for a textured entry. Exported for `LibraryListRow`
 * so the two views render an identical swatch rather than duplicating this
 * logic. */
export function MaterialFill({ item, textureUrl }: { item: LibraryItem; textureUrl: string | null }) {
  const entry = item.summary.material_entries[0]
  if (!entry) return null
  const [r, g, b, a] = entry.color
  const alpha = a / 255
  if (entry.texture_asset !== null && textureUrl !== null) {
    return (
      <div
        className="hwlib__tile-swatch-fill"
        style={{ backgroundImage: `url(${textureUrl})`, backgroundSize: 'cover', opacity: alpha }}
      />
    )
  }
  return <div className="hwlib__tile-swatch-fill" style={{ background: `rgba(${r}, ${g}, ${b}, ${alpha})` }} />
}
