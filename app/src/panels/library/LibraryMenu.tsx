/**
 * LibraryMenu — the tile's `⋯` button-anchored popover (frame 1f). Every
 * row here is also reachable from `LibraryDetailPane` directly (settled
 * decision: no action requires the menu) — "Add to collection" and "Rename"
 * simply select the item and close the menu, since the detail pane's
 * Collection dropdown and name field are always live for whatever item is
 * selected; the menu is a shortcut to get there, not a second code path.
 */

import { msg } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'
import { useLingui } from '@lingui/react/macro'
import { isMac, isWindows } from '../../platform'

/** "Reveal in the OS file manager" label, worded per platform. Exported so
 * `LibraryDetailPane`'s matching manage-row button can't drift from this
 * menu's wording — both sites reach for the same string instead of each
 * hardcoding "Reveal in Finder". */
export const revealLabel: MessageDescriptor = isMac
  ? msg`Reveal in Finder`
  : isWindows
    ? msg`Show in Explorer`
    : msg`Show in file manager`

export interface LibraryMenuProps {
  itemName: string
  /** Viewport position (from the anchor button's `getBoundingClientRect()`)
   * to pin the popover near. */
  anchor: { top: number; left: number; bottom: number }
  /** An unreadable item — only Reveal/Delete can act on it, so the other
   * entries are hidden rather than silently no-oping. */
  errored?: boolean
  canReveal: boolean
  /** Web: offer the item's bytes as a plain download (no Reveal there).
   * Hidden for errored items — there are no bytes to hand over. */
  canDownload: boolean
  /** Whether `meta.sourceDoc` is set — gates the "Remove Source Info" row
   * the same way the detail pane gates its own copy of this action. */
  hasSourceInfo: boolean
  /** Components and models (not materials, profiles or built-ins) can be
   * opened in place for editing. */
  canEdit: boolean
  /** The item came from a source (it carries a note). `canUpdate` is true
   * when a newer version can be taken now. Absent for an item with no note. */
  fromSource?: { name: string; canUpdate: boolean } | undefined
  onUpdateFromSource?: (() => void) | undefined
  onUnlinkFromSource?: (() => void) | undefined
  onClose: () => void
  onEdit: () => void
  onOpenAsDocument: () => void
  onAddToCollection: () => void
  onRename: () => void
  onRerenderThumbnail: () => void
  onRemoveSourceInfo: () => void
  onReveal: () => void
  onDownload: () => void
  onDeleteRequest: () => void
}

export function LibraryMenu({
  itemName,
  anchor,
  errored = false,
  canReveal,
  canDownload,
  hasSourceInfo,
  canEdit,
  fromSource,
  onUpdateFromSource,
  onUnlinkFromSource,
  onClose,
  onEdit,
  onOpenAsDocument,
  onAddToCollection,
  onRename,
  onRerenderThumbnail,
  onRemoveSourceInfo,
  onReveal,
  onDownload,
  onDeleteRequest,
}: LibraryMenuProps) {
  const { t, i18n } = useLingui()
  const sourceName = fromSource?.name ?? ''
  // Popover pinned just under the anchor button, left-aligned; both axes
  // are clamped to the viewport so a bottom-row (or right-click-anywhere)
  // anchor can't push the menu off-screen (adversarial review S14). The
  // height estimate errs high, which only pulls the menu up a little
  // early — never off the top (floor at 8).
  const estimatedHeight = errored ? 96 : 264 + (hasSourceInfo ? 32 : 0) + (canEdit ? 32 : 0) + (fromSource !== undefined ? 64 : 0)
  const style: React.CSSProperties = {
    position: 'fixed',
    top: Math.max(8, Math.min(anchor.bottom + 4, window.innerHeight - estimatedHeight)),
    left: Math.max(8, Math.min(anchor.left, window.innerWidth - 216)),
  }

  function item(label: string, onClick: () => void, extra?: { danger?: boolean }) {
    return (
      <button
        type="button"
        className={`hwlib__menu-item${extra?.danger ? ' hwlib__menu-item--danger' : ''}`}
        onClick={() => {
          onClick()
          onClose()
        }}
      >
        {label}
      </button>
    )
  }

  return (
    <>
      {/* Full-surface scrim: closes the menu on any outside click, without a
          document-level listener race against the dialog's own Escape
          handling (which owns closing this menu on Escape instead — see
          LibraryDialog's handleKeyDown). */}
      <div className="hwlib__menu-scrim" onClick={onClose} />
      <div className="hwlib__menu" style={style} role="menu" aria-label={t`${itemName} actions`}>
        {!errored && (
          <>
            {canEdit && item(t`Edit`, onEdit)}
            {item(t`Open as document`, onOpenAsDocument)}
            {item(t`Add to collection`, onAddToCollection)}
            {item(t`Rename`, onRename)}
            <div className="hwlib__menu-divider" />
            {item(t`Re-render thumbnail`, onRerenderThumbnail)}
            {hasSourceInfo && item(t`Remove Source Info`, onRemoveSourceInfo)}
            {fromSource !== undefined && (
              <>
                <div className="hwlib__menu-divider" />
                {fromSource.canUpdate && onUpdateFromSource !== undefined && item(t`Update from source`, onUpdateFromSource)}
                {onUnlinkFromSource !== undefined && item(t`Unlink from ${sourceName}`, onUnlinkFromSource)}
              </>
            )}
          </>
        )}
        {canReveal && item(i18n._(revealLabel), onReveal)}
        {canDownload && !errored && item(t`Download…`, onDownload)}
        {(!errored || canReveal || canDownload) && <div className="hwlib__menu-divider" />}
        {item(t`Delete from library…`, onDeleteRequest, { danger: true })}
      </div>
    </>
  )
}
