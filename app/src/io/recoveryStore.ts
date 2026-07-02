/**
 * RecoveryStore — abstracts the platform's autosave/crash-recovery storage.
 *
 * Mirrors the FileHost seam (see ./fileHost.ts): WebRecoveryStore is statically
 * imported (always needed for the web build), while under Tauri the
 * TauriRecoveryStore is imported dynamically so its Tauri-only imports never
 * enter the web bundle.
 *
 * Reuses the existing `scene.save(): Uint8Array` wasm-api method — autosave
 * writes the same bytes a normal Save would, just to a recovery slot instead
 * of the user's chosen file.
 */

import type { DocSessionState } from './documentSession'

/** Metadata describing one autosaved recovery snapshot. */
export interface RecoveryMeta {
  version: 1
  /** Epoch ms when the snapshot was written. */
  savedAt: number
  /** Display name (currentRef.name | importedName | 'Untitled'). */
  name: string
  /** Original .hew absolute path (Tauri) or null (web / unsaved document). */
  path: string | null
}

/** A recovery snapshot read back from storage. */
export interface RecoverySnapshot {
  bytes: Uint8Array
  meta: RecoveryMeta
}

export interface RecoveryStore {
  /** Persist a snapshot, overwriting any previous one. */
  write(bytes: Uint8Array, meta: RecoveryMeta): Promise<void>
  /** Read back the most recent snapshot, or null if none exists. */
  read(): Promise<RecoverySnapshot | null>
  /** Discard the stored snapshot (no-op if none exists). */
  clear(): Promise<void>
}

// WebRecoveryStore is statically imported — it's always needed for the web build.
import { WebRecoveryStore } from './webRecoveryStore'

/**
 * True when running inside the Tauri desktop shell.
 * Re-derived here (rather than imported from fileHost.ts) to keep this module
 * self-contained — both checks read the same `window.__TAURI_INTERNALS__` flag.
 */
const isTauri: boolean =
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

/**
 * Return the appropriate RecoveryStore for the current platform.
 *
 * Under Tauri, TauriRecoveryStore is imported dynamically (lazy) so its
 * Tauri-only imports never appear in the web bundle.  makeRecoveryStore() is
 * called once (in a useRef) so we return a synchronous shim that defers to
 * TauriRecoveryStore once loaded.
 */
export function makeRecoveryStore(): RecoveryStore {
  if (isTauri) {
    const storePromise = import('./tauriRecoveryStore').then(
      (m) => new m.TauriRecoveryStore(),
    )
    return {
      write: (bytes, meta) => storePromise.then((s) => s.write(bytes, meta)),
      read: () => storePromise.then((s) => s.read()),
      clear: () => storePromise.then((s) => s.clear()),
    }
  }
  return new WebRecoveryStore()
}

/**
 * Format `savedAt` (epoch ms) as a short relative-time string, relative to
 * `now` (epoch ms).  Falls back to a date string beyond ~24h.
 *
 * Re-exported from `relativeTime.ts`, which also backs
 * documentSession.ts's "Edited/Saved <relative time>" indicator — kept as a
 * named alias here so this module's existing call sites (RecoveryDialog.tsx)
 * don't need to change.
 */
export { formatRelativeTime as formatRecoveryTime } from './relativeTime'

/**
 * True when the user should be prompted to recover `snapshot`.
 *
 * Only prompts when there IS a snapshot, AND nothing else was loaded at
 * startup (no currentRef, not dirty) — so a cold-start file-association open,
 * a freshly-restored document, or any other startup path that already
 * populated the session suppresses the prompt.
 */
export function shouldPromptRecovery(
  session: DocSessionState,
  snapshot: RecoverySnapshot | null,
): boolean {
  return snapshot !== null && session.currentRef === null && !session.dirty
}
