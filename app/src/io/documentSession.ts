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
}

/** Initial (blank) session state. */
export const INITIAL_SESSION: DocSessionState = {
  currentRef: null,
  dirty: false,
}

/**
 * Derive the window/document title for the current session state.
 *
 * Format: `[• ]<filename | 'Untitled'> — Hew`
 */
export function deriveTitle(state: DocSessionState): string {
  const name = state.currentRef?.name ?? 'Untitled'
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
