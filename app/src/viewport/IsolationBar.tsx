/**
 * The bar shown at the top center of the viewport while view-only isolation is
 * on: "Showing 1 object" with a Show everything button. Styled like the
 * "Editing <name>" session chip beside it.
 */
import { Trans } from '@lingui/react/macro'
import { isolationBarText } from './isolation'

interface Props {
  /** How many objects are shown. */
  count: number
  onShowEverything: () => void
  /** CSS top offset; the default sits below the selection-path strip's first row. */
  top?: string
  /** Width in px of a panel covering the viewport's right side (the Make Solid
   *  wizard); the bar centers in what is left. */
  rightInset?: number
}

export function IsolationBar({ count, onShowEverything, top = '54px', rightInset = 0 }: Props) {
  return (
    <div
      data-isolation-bar
      role="status"
      style={{
        position: 'absolute',
        top,
        left: rightInset > 0 ? `calc((100% - ${rightInset}px) / 2)` : '50%',
        transform: 'translateX(-50%)',
        display: 'flex',
        alignItems: 'center',
        gap: '10px',
        padding: '4px 6px 4px 12px',
        background: 'var(--surface-overlay)',
        border: '1px solid var(--border-hairline)',
        borderRadius: '7px',
        fontFamily: 'var(--font-family-ui)',
        fontSize: '11.5px',
        color: 'var(--text-secondary)',
        whiteSpace: 'nowrap',
        zIndex: 21,
      }}
    >
      <span data-isolation-text>{isolationBarText(count)}</span>
      <button
        type="button"
        onClick={onShowEverything}
        style={{
          background: 'none',
          border: '1px solid var(--border-strong)',
          borderRadius: '4px',
          color: 'var(--text-primary)',
          cursor: 'pointer',
          fontFamily: 'var(--font-family-ui)',
          fontSize: '11.5px',
          padding: '2px 8px',
        }}
      >
        <Trans>Show everything</Trans>
      </button>
    </div>
  )
}
