/**
 * LibraryMark — the small marker beside a part that came from the Library
 * and no longer matches it (v1.5 OW-2): newer in the Library, changed here,
 * changed on both sides, or different with no telling which is newer (a part
 * only the Review list can reach). Its tooltip says which. Never a status-bar
 * control; it lives on a Components row or an Outliner row, next to the "×N"
 * badge. Every kind but "changed here" lets a click open Review.
 */

import { markedState, markTooltip, type LibraryLink, type MarkedState } from '../library/libraryUpdates'

const GLYPH: Record<MarkedState, string> = {
  library_newer: '↓',
  edited_here: '↑',
  both_changed: '⇅',
  different: '≠',
}

export function LibraryMark({ link, onReview }: { link: LibraryLink | undefined; onReview?: () => void }) {
  if (link === undefined || !markedState(link.state)) return null
  const state = link.state
  const tip = markTooltip(state)
  const style: React.CSSProperties = {
    fontSize: '10px',
    lineHeight: 1,
    flexShrink: 0,
    padding: '1px 4px',
    borderRadius: '8px',
    border: '1px solid var(--accent-border, #5b8cff)',
    color: 'var(--accent-text-on-tint, #9bb8ff)',
    background: 'none',
    fontFamily: 'inherit',
  }
  if (state !== 'edited_here' && onReview !== undefined) {
    return (
      <button
        type="button"
        data-testid="library-mark"
        data-state={state}
        title={tip}
        aria-label={tip}
        onClick={(e) => {
          e.stopPropagation()
          onReview()
        }}
        onPointerDown={(e) => e.stopPropagation()}
        style={{ ...style, cursor: 'pointer' }}
      >
        {GLYPH[state]}
      </button>
    )
  }
  return (
    <span data-testid="library-mark" data-state={state} title={tip} role="img" aria-label={tip} style={style}>
      {GLYPH[state]}
    </span>
  )
}
