/**
 * modelClipboard — in-memory clipboard for Copy/Cut/Paste/Paste In Place of
 * structural selections (docs/design/v1.1-cycle.md Lane D).
 *
 * Content is `Scene::extract_item` bytes of the copied selection —
 * `wrap_as_component: false`, deliberately NOT Save to Library's wrap, so a
 * pasted plain box stays a plain box rather than becoming an instance of a
 * freshly-minted one-off component. `insert_item` (the paste side) is the
 * same lossless graft Library placement uses, provenance-stamped with a
 * `sourceId`/`contentHash` derived from the bytes themselves so repeated
 * pastes of the SAME copy reuse whatever definitions the first paste
 * created rather than minting duplicates.
 *
 * Component instances stay LINKED within a document: Copy passes the
 * window's nonce as `extract_item`'s `origin`, which stamps each carried
 * definition with the definition it was copied from; Paste passes the same
 * nonce to `insert_item`, and the kernel places the ORIGINAL definition
 * when the stamp matches (SketchUp's copy/paste — editing the paste edits
 * the original, Make Unique breaks the link). The scene folds its document
 * epoch into the origin, so bytes copied before File ▸ Open/New in this
 * window, or in another window, paste as an ordinary copy: a new
 * definition the first time, reused by provenance on repeat pastes.
 *
 * Held at module scope (survives tool switches/re-renders within one tab).
 * Under Tauri, also mirrored into the Rust app-state clipboard
 * (`clipboard_set`/`clipboard_get`, shared across every open document
 * window) so a copy in one window pastes in another — `refreshFromShell`
 * pulls the shared state in whenever this window regains focus, treating
 * the shell as the cross-window source of truth at that moment while
 * leaving the richer same-window copy (`count`/`kinds`) alone when it's
 * still the freshest thing around (bytes-equal check). On the web build
 * there is no cross-tab channel — the clipboard is per-tab only, a stated
 * limitation, not a gap to close.
 */

import { isTauri } from '../io/fileHost'
import { sha256Hex } from '../library/itemFiles'

export interface ClipboardContent {
  bytes: Uint8Array
  /** How many structural nodes were copied — informational (the "Copied N
   *  objects" toast); `-1` when reconstructed from the shared shell state
   *  with no local record of the original selection (cross-window paste). */
  count: number
  /** Node kinds present in the copy (e.g. `['object']`, `['object','group']`) —
   *  informational only, same caveat as `count`. */
  kinds: string[]
  /** Best-effort origin tag for the copy — informational; unused by
   *  `insert_item` itself, which works from `bytes` alone. */
  sourceDocId: string
  /** sha256 hex of `bytes` — half of the `insert_item` provenance pair. */
  contentHash: string
  /** `insert_item`'s `source_id`: derived from `contentHash` (clipboard
   *  content has no other stable identity), so identical clipboard bytes
   *  always dedupe to the same definitions across repeated pastes. */
  sourceId: string
  /**
   * The item→world affine (row-major 3×4, 12 numbers — the same shape
   * `insert_item`'s own `affine` parameter takes) Paste In Place must pass
   * to `insert_item` for the pasted copy to overlap the original exactly,
   * or `null` when unknown (a cross-window paste with no local record of
   * the source selection, or the source selection resolved to no visible
   * content). Read from `Scene.extract_item_placement` at Copy time
   * (App.tsx's `doCopy`), since Paste In Place has no access to the
   * ORIGINAL selection by the time it runs — the kernel computes this,
   * never re-derived here, because it depends on internal `extract_item`
   * behavior a JS-side approximation could silently drift from:
   * `Document::extract_item` re-origins an item's content to its VISIBLE
   * bottom-center whenever the drawing axes are identity (the
   * overwhelmingly common case — see that function's "Re-origin" doc
   * comment), in which case this is a pure translation by that point; when
   * the axes are moved, `extract_item` does NOT re-origin (the deliberately
   * placed frame IS the chosen insertion point) and this is instead the
   * axes frame's own forward transform — a bare identity affine would
   * reproduce the original position in NEITHER case.
   */
  placementAffine: number[] | null
}

let current: ClipboardContent | null = null

function bytesEqual(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false
  }
  return true
}

/** Build the provenance pair from raw bytes — shared by `setClipboardBytes`
 *  and the shell-refresh reconstruction path below. */
async function withProvenance(
  bytes: Uint8Array,
  count: number,
  kinds: string[],
  sourceDocId: string,
  placementAffine: number[] | null,
): Promise<ClipboardContent> {
  const contentHash = await sha256Hex(bytes)
  return { bytes, count, kinds, sourceDocId, contentHash, sourceId: `clipboard:${contentHash}`, placementAffine }
}

/** Copy: build clipboard content from freshly-extracted bytes and a known
 *  selection (count/kinds/placementAffine are meaningful), storing it
 *  locally and — under Tauri — pushing it to the shared shell state. */
export async function setClipboardBytes(
  bytes: Uint8Array,
  count: number,
  kinds: string[],
  sourceDocId: string,
  placementAffine: number[] | null,
): Promise<ClipboardContent> {
  const content = await withProvenance(bytes, count, kinds, sourceDocId, placementAffine)
  current = content
  await pushToShell(bytes)
  return content
}

/** Synchronous read of whatever this window currently holds (may be stale
 *  relative to another window's more recent copy until the next
 *  `refreshFromShell` — see the module doc). */
export function getClipboard(): ClipboardContent | null {
  return current
}

export function hasClipboardContent(): boolean {
  return current !== null
}

/** Clear (nothing to push to the shell — an empty clipboard is simply never
 *  written there; Cut doesn't clear on delete, it only ever sets). */
export function clearClipboard(): void {
  current = null
}

async function pushToShell(bytes: Uint8Array): Promise<void> {
  if (!isTauri) return
  try {
    const { invoke } = await import('@tauri-apps/api/core')
    await invoke('clipboard_set', { bytes: Array.from(bytes) })
  } catch {
    /* shell without the command (older build) — the local copy still works */
  }
}

/**
 * Pull the shared shell clipboard and make it authoritative for this
 * window — called on window focus (App.tsx) so switching into a window
 * after copying somewhere else sees the fresh content. A no-op on the web
 * build (no shell to ask). When the shell's bytes match what's already
 * held locally, the local (richer) copy is left untouched rather than
 * replaced with a `count: -1` reconstruction.
 */
export async function refreshClipboardFromShell(): Promise<void> {
  if (!isTauri) return
  try {
    const { invoke } = await import('@tauri-apps/api/core')
    const raw = await invoke<number[] | null>('clipboard_get')
    if (raw === null || raw.length === 0) {
      current = null
      return
    }
    const bytes = new Uint8Array(raw)
    if (current !== null && bytesEqual(current.bytes, bytes)) return
    current = await withProvenance(bytes, -1, [], '', null)
  } catch {
    /* shell without the command — leave whatever's local alone */
  }
}
