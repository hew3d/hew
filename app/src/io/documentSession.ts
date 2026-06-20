/**
 * documentSession — pure document-lifecycle state helpers.
 *
 * This module holds the logic for tracking document state (currentRef, dirty)
 * and deriving the window title.  It is intentionally UI-free so it can be
 * unit-tested without a DOM or React.
 *
 * React-level integration is in App.tsx (useDocumentSession hook).
 */

import type { FileRef } from './fileHost'

/** Snapshot of document session state. */
export interface DocSessionState {
  /** Current file reference (null = unsaved / "Untitled"). */
  currentRef: FileRef | null
  /** True when the document has unsaved changes. */
  dirty: boolean
  /**
   * Display name used when currentRef is null but the document was imported
   * from a named file (e.g. from a .dae import).  Used by deriveTitle and
   * saveAsDocument so the imported filename appears in the title and the
   * Save As dialog's suggested name, without a backing .hew handle.
   */
  importedName?: string
}

/** Initial (blank) session state. */
export const INITIAL_SESSION: DocSessionState = {
  currentRef: null,
  dirty: false,
}

/**
 * Derive the window/document title for the current session state.
 *
 * Format: `[• ]<filename | importedName | 'Untitled'> — Hew`
 */
export function deriveTitle(state: DocSessionState): string {
  const name = state.currentRef?.name ?? state.importedName ?? 'Untitled'
  const dirtyMark = state.dirty ? '• ' : ''
  return `${dirtyMark}${name} — Hew`
}

/**
 * Return the state after a document mutation (marks dirty).
 */
export function afterMutation(state: DocSessionState): DocSessionState {
  if (state.dirty) return state // no change needed
  return { ...state, dirty: true }
}

/**
 * Return the state after a successful save.
 * `ref` is the FileRef returned by the host (may be a new ref for Save As).
 */
export function afterSave(ref: FileRef): DocSessionState {
  return { currentRef: ref, dirty: false }
}

/**
 * Return the state after opening a file or creating a new document.
 * `ref` is null for New.
 */
export function afterOpen(ref: FileRef | null): DocSessionState {
  return { currentRef: ref, dirty: false }
}

/**
 * Return the state after replacing the document with an imported .dae file.
 *
 * `name` is the basename of the .dae file (e.g. "Kitchen.dae").  The ".dae"
 * extension is stripped for display.
 *
 * Safety contract: currentRef is null so saveDocument() will call
 * save(bytes, null) → both WebFileHost and TauriFileHost treat a null ref as
 * "Save As" and always prompt the user.  This prevents silent overwrites of any
 * file handle.  The importedName flows through to deriveTitle (for the window
 * title) and to saveAsDocument's suggestedName (so the user sees a sensible
 * default filename in the Save As dialog).
 */
export function afterImport(name: string): DocSessionState {
  // Strip the .dae extension (case-insensitive) for the display name.
  const displayName = name.replace(/\.dae$/i, '')
  return { currentRef: null, dirty: true, importedName: displayName }
}
