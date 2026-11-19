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
import { Trans, useLingui } from '@lingui/react/macro'
import { isComposingKey } from '../input/keyboard'

export interface SaveToLibraryPopoverProps {
  /** Pre-filled, fully-selected starting value for the name field. */
  defaultName: string
  /** 'dock' (default): anchored above the ContextualDock. 'modal': centered
   * over the viewport, for File ▸ Save to Library…, which has no dock verb
   * to anchor from. */
  variant?: 'dock' | 'modal'
  onSave: (name: string, opts?: { replace?: boolean }) => void
  onClose: () => void
  /** Heading, default "Save to Library" (a profile says "Save profile"; Make
   *  component asks for other names too). Each of `title`, `inputLabel` and
   *  `buttonLabel` defaults to the Save to Library wording; `hint` of `null`
   *  drops the footer that explains the insertion point. */
  title?: string
  inputLabel?: string
  buttonLabel?: string
  hint?: string | null
  /** Read-only facts shown under the name (a profile's detected width). */
  rows?: Array<{ label: string; value: string }>
  /** Drop the "inserts by its bottom center" note (profiles are not placed). */
  hideOriginHint?: boolean
  /** An item this document was opened from: offers "Replace '{name}'" to
   *  overwrite it in place instead of saving a new item. */
  replace?: { name: string }
  /** The selection is a part that came from a Library item this model has
   *  changed (v1.5 OW-2): leads with "Update “{name}” in the Library", and
   *  keeps "Save as a new item" beneath it. `confirm` is set when the Library
   *  has also changed since the part was inserted, which asks first;
   *  `cannotCompare` is set when Hew cannot tell which side is newer, which
   *  asks first too, in its own words. */
  updateInLibrary?: { name: string; confirm: boolean; cannotCompare?: boolean }
  /** The selection came from a Library item but is a linked copy, which cannot
   *  update it: no button, one line saying what to do instead. */
  updateInLibraryBlocked?: { name: string; linkedSet?: boolean }
  onUpdateInLibrary?: () => void
}

/** The placement tool's origin-marker yellow (`LibraryPlaceTool.ts`'s
 * `GHOST_ORIGIN_COLOR`) — a literal, not a token, because it names one
 * specific concrete color shared with a three.js material rather than a
 * themed semantic role. Kept in sync by hand; both sites are small and
 * unlikely to drift. */
const ORIGIN_DOT_COLOR = '#f5d76a'

const PRIMARY_BUTTON_STYLE: React.CSSProperties = {
  padding: '7px 12px',
  background: 'var(--accent-base)',
  border: 'none',
  borderRadius: 'var(--radius-control)',
  color: '#fff',
  fontFamily: 'var(--font-family-ui)',
  fontSize: 'var(--font-size-tool-row)',
  fontWeight: 600,
  cursor: 'pointer',
}

const SECONDARY_BUTTON_STYLE: React.CSSProperties = {
  padding: '7px 12px',
  background: 'var(--surface-input)',
  border: '1px solid var(--border-strong)',
  borderRadius: 'var(--radius-control)',
  color: 'var(--text-primary)',
  fontFamily: 'var(--font-family-ui)',
  fontSize: 'var(--font-size-tool-row)',
  cursor: 'pointer',
}

export function SaveToLibraryPopover({
  defaultName,
  variant = 'dock',
  onSave,
  onClose,
  title,
  inputLabel,
  buttonLabel,
  hint,
  rows = [],
  hideOriginHint = false,
  replace,
  updateInLibrary,
  onUpdateInLibrary,
  updateInLibraryBlocked,
}: SaveToLibraryPopoverProps) {
  const { t } = useLingui()
  const heading = title ?? t`Save to Library`
  const replaceName = replace?.name ?? ''
  const updateName = updateInLibrary?.name ?? ''
  const blockedName = updateInLibraryBlocked?.name ?? ''
  const [confirming, setConfirming] = useState(false)
  const [name, setName] = useState(defaultName)
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    inputRef.current?.focus()
    inputRef.current?.select()
  }, [])

  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (isComposingKey(e)) return
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
        aria-label={heading}
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
          {title ?? <Trans>Save to Library</Trans>}
        </div>
        {updateInLibraryBlocked !== undefined && (
          <div data-testid="update-in-library-blocked" style={{ fontSize: '12px', color: 'var(--text-secondary)', lineHeight: 1.4 }}>
            {updateInLibraryBlocked.linkedSet === false ? (
              <Trans>This part can’t update “{blockedName}” in the Library as it is. Make it a component first.</Trans>
            ) : (
              <Trans>
                This part is a linked copy, so it can’t update “{blockedName}” in the Library. Make it a component or unlink it first.
              </Trans>
            )}
          </div>
        )}
        {updateInLibrary !== undefined && onUpdateInLibrary !== undefined && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: '6px' }}>
            {confirming ? (
              <>
                <div data-testid="update-in-library-confirm" style={{ fontSize: '12px', color: 'var(--text-secondary)', lineHeight: 1.4 }}>
                  {updateInLibrary.cannotCompare === true ? (
                    <Trans>The Library copy and this part have both changed or can’t be compared. Replacing it will lose the Library’s version.</Trans>
                  ) : (
                    <Trans>The Library has also changed since you inserted “{updateName}”. Replacing it will lose those changes.</Trans>
                  )}
                </div>
                <div style={{ display: 'flex', gap: '8px' }}>
                  <button type="button" onClick={() => setConfirming(false)} style={{ ...SECONDARY_BUTTON_STYLE, flex: 1 }}>
                    <Trans>Cancel</Trans>
                  </button>
                  <button type="button" data-testid="update-in-library-replace" onClick={onUpdateInLibrary} style={{ ...PRIMARY_BUTTON_STYLE, flex: 1 }}>
                    <Trans>Replace it</Trans>
                  </button>
                </div>
              </>
            ) : (
              <>
                <button
                  type="button"
                  data-testid="update-in-library"
                  onClick={() => (updateInLibrary.confirm ? setConfirming(true) : onUpdateInLibrary())}
                  style={PRIMARY_BUTTON_STYLE}
                >
                  <Trans>Update “{updateName}” in the Library</Trans>
                </button>
                <div style={{ fontSize: '10px', color: 'var(--text-faint)' }}>
                  <Trans>Other models that use it will be offered the update.</Trans>
                </div>
              </>
            )}
          </div>
        )}
        <input
          ref={inputRef}
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => {
            if (isComposingKey(e)) return
            if (e.key === 'Enter') {
              e.preventDefault()
              commit()
            }
          }}
          aria-label={inputLabel ?? t`Item name`}
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
        {rows.map((r) => (
          <div
            key={r.label}
            style={{ display: 'flex', justifyContent: 'space-between', fontSize: '12px', color: 'var(--text-secondary)' }}
          >
            <span>{r.label}</span>
            <span style={{ fontFamily: 'var(--font-family-mono)' }}>{r.value}</span>
          </div>
        ))}
        {replace !== undefined && (
          <button
            type="button"
            onClick={() => {
              const trimmed = name.trim()
              onSave(trimmed !== '' ? trimmed : defaultName, { replace: true })
            }}
            style={{
              padding: '7px 12px',
              background: 'var(--surface-input)',
              border: '1px solid var(--border-strong)',
              borderRadius: 'var(--radius-control)',
              color: 'var(--text-primary)',
              fontFamily: 'var(--font-family-ui)',
              fontSize: 'var(--font-size-tool-row)',
              cursor: 'pointer',
            }}
          >
            <Trans>Replace “{replaceName}”</Trans>
          </button>
        )}
        <button type="button" onClick={commit} style={updateInLibrary !== undefined ? SECONDARY_BUTTON_STYLE : PRIMARY_BUTTON_STYLE}>
          {buttonLabel ?? (updateInLibrary !== undefined ? <Trans>Save as a new item</Trans> : <Trans>Save</Trans>)}
        </button>
        {hint !== null && !hideOriginHint && (
          <div style={{ display: 'flex', alignItems: 'center', gap: '6px', fontSize: '10px', color: 'var(--text-faint)' }}>
            {hint === undefined && (
              <span
                aria-hidden="true"
                style={{ width: '6px', height: '6px', borderRadius: '50%', background: ORIGIN_DOT_COLOR, flexShrink: 0 }}
              />
            )}
            {hint ?? <Trans>Inserts by its bottom center — or by the drawing axes origin, if you've placed axes</Trans>}
          </div>
        )}
      </div>
    </div>
  )
}
