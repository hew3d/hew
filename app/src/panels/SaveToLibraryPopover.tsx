/**
 * SaveToLibraryPopover — the "Save to Library" name prompt (design frame
 * 1d). One component backs both places it opens from:
 *
 *   - the ContextualDock's `save-to-library` verb (Object/Group/Instance
 *     selection) — `variant="dock"`, anchored above the dock, the default.
 *   - File ▸ Save to Library… (the whole document) — `variant="modal"`,
 *     centered like a small dialog.
 *
 * Same body either way, so the two save flows can never drift out of sync
 * with each other. The name input is pre-filled with `defaultName` and
 * fully selected, so typing immediately replaces it while pressing Enter
 * without editing still saves under the suggested name. Escape closes the
 * popover only — it must not also cancel whatever selection/session is live
 * underneath (App.tsx's own Escape handling, a component-session pop, …) —
 * so the keydown listener stops propagation, mirroring every other modal in
 * `dialogs.test.tsx`.
 */

import { useCallback, useEffect, useRef, useState } from 'react'

export interface SaveToLibraryPopoverProps {
  /** Pre-filled, fully-selected starting value for the name field. */
  defaultName: string
  /** 'dock' (default): anchored above the ContextualDock. 'modal': centered
   * over the viewport, for File ▸ Save to Library…, which has no dock verb
   * to anchor from. */
  variant?: 'dock' | 'modal'
  onSave: (name: string) => void
  onClose: () => void
}

/** The placement tool's origin-marker yellow (`LibraryPlaceTool.ts`'s
 * `GHOST_ORIGIN_COLOR`) — a literal, not a token, because it names one
 * specific concrete color shared with a three.js material rather than a
 * themed semantic role. Kept in sync by hand; both sites are small and
 * unlikely to drift. */
const ORIGIN_DOT_COLOR = '#f5d76a'

export function SaveToLibraryPopover({
  defaultName,
  variant = 'dock',
  onSave,
  onClose,
}: SaveToLibraryPopoverProps) {
  const [name, setName] = useState(defaultName)
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    inputRef.current?.focus()
    inputRef.current?.select()
  }, [])

  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        onClose()
      }
    },
    [onClose],
  )
  useEffect(() => {
    document.addEventListener('keydown', handleKeyDown)
    return () => document.removeEventListener('keydown', handleKeyDown)
  }, [handleKeyDown])

  function commit() {
    const trimmed = name.trim()
    onSave(trimmed !== '' ? trimmed : defaultName)
  }

  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 2001,
        background: variant === 'modal' ? 'var(--backdrop-dim)' : 'transparent',
        display: 'flex',
        alignItems: variant === 'modal' ? 'center' : 'flex-end',
        justifyContent: 'center',
        // Above the dock (bottom: 18px, ~54px tall) with a visible gap.
        paddingBottom: variant === 'dock' ? '90px' : 0,
      }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label="Save to Library"
        style={{
          width: '280px',
          display: 'flex',
          flexDirection: 'column',
          gap: '10px',
          padding: '14px',
          background: 'var(--surface-overlay)',
          border: '1px solid var(--border-strong)',
          borderRadius: 'var(--radius-panel-item)',
          boxShadow: 'var(--shadow-palette)',
          fontFamily: 'var(--font-family-ui)',
        }}
      >
        <div
          style={{
            fontFamily: 'var(--font-family-mono)',
            fontSize: 'var(--font-size-section-header)',
            fontWeight: 600,
            letterSpacing: '0.12em',
            textTransform: 'uppercase',
            color: 'var(--text-section)',
          }}
        >
          Save to Library
        </div>
        <input
          ref={inputRef}
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault()
              commit()
            }
          }}
          aria-label="Item name"
          style={{
            padding: '6px 8px',
            background: 'var(--surface-input)',
            border: '1px solid var(--border-panel)',
            borderRadius: 'var(--radius-control)',
            color: 'var(--text-primary)',
            fontFamily: 'var(--font-family-ui)',
            fontSize: '13px',
            outline: 'none',
          }}
        />
        <button
          type="button"
          onClick={commit}
          style={{
            padding: '7px 12px',
            background: 'var(--accent-base)',
            border: 'none',
            borderRadius: 'var(--radius-control)',
            color: '#fff',
            fontFamily: 'var(--font-family-ui)',
            fontSize: 'var(--font-size-tool-row)',
            fontWeight: 600,
            cursor: 'pointer',
          }}
        >
          Save
        </button>
        <div style={{ display: 'flex', alignItems: 'center', gap: '6px', fontSize: '10px', color: 'var(--text-faint)' }}>
          <span
            aria-hidden="true"
            style={{ width: '6px', height: '6px', borderRadius: '50%', background: ORIGIN_DOT_COLOR, flexShrink: 0 }}
          />
          Inserts by its bottom center — or by the drawing axes origin, if you've placed axes
        </div>
      </div>
    </div>
  )
}
