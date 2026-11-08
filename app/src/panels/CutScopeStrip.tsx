/**
 * CutScopeStrip — the small strip above the status bar that asks how far a
 * cut should reach when the part being cut has more than one linked copy.
 *
 * "Cut all N copies" is the default (Enter); "Only this one" unlinks that copy
 * first and cuts it alone; Escape cancels. The strip belongs to one boolean
 * and goes away when the selection changes.
 */

import { useEffect, useRef } from 'react'
import { useLingui } from '@lingui/react/macro'
import { cutAllCopies, onlyThisOne } from '../linkedCopyCopy'
import { isComposingKey } from '../input/keyboard'

export interface CutScopeStripProps {
  /** Placements sharing the part (always above one). */
  copies: number
  onChoose: (scope: 'all' | 'one') => void
  onCancel: () => void
}

const BUTTON_STYLE: React.CSSProperties = {
  padding: '3px 12px',
  background: 'var(--surface-input)',
  color: 'var(--text-primary)',
  border: '1px solid var(--border-strong, transparent)',
  borderRadius: 'var(--radius-control)',
  fontFamily: 'inherit',
  fontSize: 'var(--font-size-body, 12px)',
  cursor: 'pointer',
}

export function CutScopeStrip({ copies, onChoose, onCancel }: CutScopeStripProps) {
  const { t } = useLingui()
  const defaultRef = useRef<HTMLButtonElement>(null)
  useEffect(() => {
    defaultRef.current?.focus()
  }, [])
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isComposingKey(e)) return
      // Typing in a field, or working in a dialog or popover, is not about the strip.
      const target = e.target as HTMLElement | null
      if (
        target !== null &&
        typeof target.closest === 'function' &&
        (target.closest('input, textarea, select, [contenteditable="true"], [role="dialog"], [role="menu"]') !== null)
      ) return
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        onCancel()
      } else if (e.key === 'Enter') {
        // Enter on a focused button presses that button; anywhere else it is
        // the default choice.
        const target = e.target as HTMLElement | null
        if (target?.tagName === 'BUTTON' && target !== defaultRef.current) return
        e.preventDefault()
        e.stopPropagation()
        onChoose('all')
      }
    }
    document.addEventListener('keydown', onKey, true)
    return () => document.removeEventListener('keydown', onKey, true)
  }, [onChoose, onCancel])
  return (
    <div
      role="group"
      data-testid="cut-scope-strip"
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 'var(--space-3, 8px)',
        padding: '6px var(--space-6, 13px)',
        background: 'var(--surface-overlay)',
        borderTop: '1px solid var(--border-hairline)',
        fontFamily: 'var(--font-family-ui)',
        fontSize: 'var(--font-size-body, 12px)',
        flexShrink: 0,
      }}
    >
      <button
        ref={defaultRef}
        type="button"
        onClick={() => onChoose('all')}
        style={{ ...BUTTON_STYLE, background: 'var(--accent-base)', color: '#fff', border: 'none', fontWeight: 600 }}
      >
        {cutAllCopies(copies)}
      </button>
      <button type="button" onClick={() => onChoose('one')} style={BUTTON_STYLE}>
        {onlyThisOne()}
      </button>
      <button type="button" onClick={onCancel} style={BUTTON_STYLE} aria-label={t`Cancel`}>
        {'✕'}
      </button>
    </div>
  )
}
