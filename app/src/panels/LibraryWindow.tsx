/**
 * LibraryWindow — the desktop shell's REAL native Library window (a
 * resizable Tauri webview at `index.html#library`, the Settings-window
 * pattern). Hosts `LibraryDialog` in its `'window'` variant and bridges it
 * to the ACTIVE DOCUMENT window, which owns the model:
 *
 * - Placements ("in this model" badges) arrive by push: every document
 *   window emits `library-placements` app-wide whenever its document
 *   changes or it gains focus; this window keeps the latest. On mount it
 *   emits `library-placements-request` so it never starts empty.
 * - Actions (Insert / Open / Paint / Add to palette) go through the
 *   `library_dispatch` command, which routes to the active document window
 *   exactly like a native menu action — and focuses it first for the
 *   flows that continue there (cursor placement, Paint).
 *
 * The browser's own listing/manage flows need no bridging: the library
 * store and the wasm item readers are window-independent.
 */

import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { emit, listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { readItemSummary } from '../library/itemFiles'
import type { LibraryItem } from '../library/types'
import { LibraryDialog } from './LibraryDialog'

/** The payload document windows push (and this window consumes). */
interface PlacementsPush {
  placements: Record<string, number>
  /** Content hashes of the document palette's live entries (decimal
   * strings) — the "in palette" badge's comparison set. */
  paletteHashes?: string[]
}

/** The action payload this window dispatches to the active document window
 *  (`library_dispatch` → `library-action`). Bytes are NOT sent — the
 *  document window re-reads the item from the shared store by name. */
export interface LibraryActionPayload {
  action: 'insert' | 'open' | 'paint' | 'add-palette'
  fileName: string
  displayName: string
  sourceId: string | null
  category: string
}

function dispatch(action: LibraryActionPayload['action'], item: LibraryItem, focusTarget: boolean): void {
  const payload: LibraryActionPayload = {
    action,
    fileName: item.file.fileName,
    displayName: item.displayName,
    sourceId: item.meta.id ?? null,
    category: item.category,
  }
  void invoke<boolean>('library_dispatch', {
    payload: JSON.stringify(payload),
    focusTarget,
  })
    .then((delivered) => {
      if (!delivered) {
        // All document windows are closed — say so natively rather than
        // silently doing nothing (adversarial review S4).
        void invoke('library_confirm', {
          title: 'Library',
          message: 'Open a document window first — library items are placed into a model.',
          actionLabel: 'OK',
        })
      }
    })
    .catch(() => {
      /* the document window surfaces its own errors; nothing to show here */
    })
}

export function LibraryWindow() {
  const [placements, setPlacements] = useState<Record<string, number>>({})
  const paletteHashesRef = useRef<Set<string>>(new Set())
  const requested = useRef(false)

  useEffect(() => {
    // Register the listener FIRST, then request — emitting before the
    // listener lands can drop the very answer the request asked for
    // (adversarial review). The shell forwards pushes as a JSON string
    // ('library-placements-json'), already gated to the active document
    // window.
    const unlisten = listen<string>('library-placements-json', (event) => {
      try {
        const parsed = JSON.parse(event.payload) as PlacementsPush
        setPlacements(parsed.placements ?? {})
        paletteHashesRef.current = new Set(parsed.paletteHashes ?? [])
      } catch {
        /* a malformed push is ignored; the next one wins */
      }
    })
    void unlisten.then(() => {
      if (!requested.current) {
        requested.current = true
        void emit('library-placements-request', {})
      }
    })
    return () => {
      void unlisten.then((u) => u())
    }
  }, [])

  return (
    <LibraryDialog
      open
      variant="window"
      onClose={() => {
        void getCurrentWindow().close()
      }}
      placements={placements}
      onInsert={(item) => dispatch('insert', item, true)}
      onOpenAsDocument={(item) => dispatch('open', item, true)}
      onPaintWith={(item) => dispatch('paint', item, true)}
      onAddToPalette={(item) => dispatch('add-palette', item, false)}
      // The badge compares kernel content hashes: the document window
      // pushes its palette's hashes; the item's ride in its own summary.
      materialInPalette={async (bytes) => {
        const summary = await readItemSummary(bytes)
        const have = paletteHashesRef.current
        return (
          summary.material_entries.length > 0 &&
          summary.material_entries.every((m) => have.has(m.content_hash))
        )
      }}
    />
  )
}
