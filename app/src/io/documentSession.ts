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
import { formatRelativeTime } from './relativeTime'

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
  /**
   * Epoch ms when the document most recently transitioned clean → dirty
   *. Null until the first edit of the session. Deliberately
   * NOT bumped on every subsequent mutation while already dirty — it marks
   * the start of the current unsaved-edit streak, matching `afterMutation`'s
   * existing no-op-when-already-dirty optimization (dragging Push/Pull etc.
   * can fire many mutations/sec; re-stamping this on each one would defeat
   * that and cause far more re-renders for a purely cosmetic label).
   */
  lastEditAt: number | null
  /** Epoch ms of the most recent successful save/open/import.
   * Null until the document has been saved (or opened/imported) at least once. */
  lastSavedAt: number | null
}

/** Initial (blank) session state. */
export const INITIAL_SESSION: DocSessionState = {
  currentRef: null,
  dirty: false,
  lastEditAt: null,
  lastSavedAt: null,
}

/** Just the bare document name (no dirty mark, no " — Hew" suffix) — used by
 * `TitleBar.tsx`/`MenuBar.tsx`'s Studio chrome, which render the
 * name and the `saveStateLabel` indicator as separate pieces rather than one
 * pre-formatted string. */
export function documentName(state: DocSessionState): string {
  return state.currentRef?.name ?? state.importedName ?? 'Untitled'
}

/**
 * Derive the window/document title for the current session state.
 *
 * Format: `[• ]<filename | importedName | 'Untitled'> — Hew`
 */
export function deriveTitle(state: DocSessionState): string {
  const dirtyMark = state.dirty ? '• ' : ''
  return `${dirtyMark}${documentName(state)} — Hew`
}

/**
 * Derive the passive save-state label shown beside the filename — replaces a
 * Save button as the primary save-state cue (`02_app_shell.md`):
 * "Edited <relative time>" while dirty, "Saved <relative time>" once clean.
 * Returns "" before the document has any edit/save history yet (a fresh
 * blank document — nothing to report).
 */
export function saveStateLabel(state: DocSessionState, now: number): string {
  if (state.dirty) {
    return state.lastEditAt === null ? 'Edited' : `Edited ${formatRelativeTime(state.lastEditAt, now)}`
  }
  return state.lastSavedAt === null ? '' : `Saved ${formatRelativeTime(state.lastSavedAt, now)}`
}

/**
 * Return the state after a document mutation (marks dirty).
 */
export function afterMutation(state: DocSessionState, now: number): DocSessionState {
  if (state.dirty) return state // no change needed — see lastEditAt's doc comment
  return { ...state, dirty: true, lastEditAt: now }
}

/**
 * Return the state after a successful save.
 * `ref` is the FileRef returned by the host (may be a new ref for Save As).
 */
export function afterSave(ref: FileRef, now: number): DocSessionState {
  return { currentRef: ref, dirty: false, lastEditAt: null, lastSavedAt: now }
}

/**
 * Return the state after opening a file or creating a new document.
 * `ref` is null for New.
 */
export function afterOpen(ref: FileRef | null, now: number): DocSessionState {
  return { currentRef: ref, dirty: false, lastEditAt: null, lastSavedAt: now }
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
export function afterImport(name: string, now: number): DocSessionState {
  // Strip the .dae extension (case-insensitive) for the display name.
  const displayName = name.replace(/\.dae$/i, '')
  return { currentRef: null, dirty: true, importedName: displayName, lastEditAt: now, lastSavedAt: null }
}
