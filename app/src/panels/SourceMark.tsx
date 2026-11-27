/**
 * SourceMark — the small marker beside a Library item that has a newer
 * version at the source it came from (docs/design/shared-library-sources.md
 * 7.5). Drawn like `LibraryMark`: same size, border and colors, so the two
 * read as one family; a different glyph and its own tooltip. Never a
 * status-bar control. Only shown on the Library's own tile and row.
 */

import { useLingui } from '@lingui/react/macro'

export function SourceMark() {
  const { t } = useLingui()
  const tip = t`Newer at the source`
  return (
    <span
      data-testid="source-mark"
      title={tip}
      role="img"
      aria-label={tip}
      style={{
        fontSize: '10px',
        lineHeight: 1,
        flexShrink: 0,
        padding: '1px 4px',
        borderRadius: '8px',
        border: '1px solid var(--accent-border, #5b8cff)',
        color: 'var(--accent-text-on-tint, #9bb8ff)',
        background: 'none',
        fontFamily: 'inherit',
      }}
    >
      {'⇣'}
    </span>
  )
}
