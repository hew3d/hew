import type { CSSProperties } from 'react'
import { PREVIEW_MARK_TIP, PREVIEW_TAG } from './copy'

const MARK_STYLE: CSSProperties = {
  fontSize: '10px',
  fontWeight: 700,
  textTransform: 'uppercase',
  letterSpacing: '0.08em',
  padding: '2px 7px',
  borderRadius: '999px',
  border: '1px solid var(--accent-base, #7aaaee)',
  color: 'var(--accent-text, #8fb0ff)',
  whiteSpace: 'nowrap',
  flexShrink: 0,
  cursor: 'default',
  userSelect: 'none',
}

/** The "Preview" mark in the window chrome of a preview build: a passive
 *  label, not a control. Callers render it only behind `previewBuild`. */
export function PreviewMark() {
  return (
    <span style={MARK_STYLE} title={PREVIEW_MARK_TIP} data-testid="preview-mark">
      {PREVIEW_TAG}
    </span>
  )
}
