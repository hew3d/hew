/**
 * The tray's hide control and its way back. Shown, the chevron sits in a thin
 * row at the top of the tray, left-aligned to mirror the rail's. Hidden, the
 * tray is gone entirely, so the way back is a tab floating on the viewport's
 * right edge, vertically centered to stay clear of the measurement box (top
 * right) and the contextual dock (bottom center).
 *
 * Both take `focusOnMount`: when the tray is hidden or shown from a control
 * that had keyboard focus, focus follows to the control that replaced it
 * instead of falling back to the document body.
 */
import { useEffect, useRef, useState } from 'react'
import { useLingui } from '@lingui/react/macro'
import { useIsRtl } from '../i18n/direction'
import { InlineIcon } from './InlineIcon'
import chevronLeftSvg from '@material-symbols/svg-400/outlined/chevron_left.svg?raw'
import chevronRightSvg from '@material-symbols/svg-400/outlined/chevron_right.svg?raw'

interface TrayToggleProps {
  onClick: () => void
  focusOnMount?: boolean
}

/** Mount-only on purpose: the flag describes how THIS mount came about. */
function useFocusOnMount(focusOnMount: boolean) {
  const ref = useRef<HTMLButtonElement>(null)
  const initial = useRef(focusOnMount)
  useEffect(() => {
    if (initial.current) ref.current?.focus()
  }, [])
  return ref
}

export function TrayHideRow({ onClick, focusOnMount = false }: TrayToggleProps) {
  const { t } = useLingui()
  const rtl = useIsRtl()
  const [hovered, setHovered] = useState(false)
  const ref = useFocusOnMount(focusOnMount)
  return (
    <div style={{ display: 'flex', flexShrink: 0, padding: '6px 8px 0' }}>
      <button
        ref={ref}
        type="button"
        aria-label={t`Hide tray`}
        title={t`Hide tray`}
        onClick={onClick}
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
        style={{
          display: 'flex',
          padding: '2px',
          borderRadius: 'var(--radius-control)',
          border: 'none',
          cursor: 'pointer',
          color: hovered ? 'var(--text-secondary)' : 'var(--text-faint)',
          background: hovered ? 'var(--surface-hover)' : 'transparent',
        }}
      >
        {/* Points at the edge the tray leaves toward: right in English, left
            once the tray is docked on the left. */}
        <InlineIcon svg={rtl ? chevronLeftSvg : chevronRightSvg} size={18} />
      </button>
    </div>
  )
}

export function TrayShowTab({ onClick, focusOnMount = false }: TrayToggleProps) {
  const { t } = useLingui()
  const rtl = useIsRtl()
  const [hovered, setHovered] = useState(false)
  const ref = useFocusOnMount(focusOnMount)
  return (
    <button
      ref={ref}
      type="button"
      aria-label={t`Show tray`}
      data-tray-show-tab="true"
      title={t`Show tray`}
      onClick={onClick}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      style={{
        position: 'absolute',
        // The tab hangs on the tray's edge, which is the inline end in both
        // directions (right in English, left in Arabic).
        insetInlineEnd: 0,
        top: '50%',
        transform: 'translateY(-50%)',
        zIndex: 20,
        display: 'flex',
        padding: '10px 1px',
        border: '1px solid var(--border-hairline)',
        borderInlineEnd: 'none',
        // Rounded on the side away from the window edge: the inline start
        // corners (top-left/bottom-left in English, top-right/bottom-right
        // in Arabic).
        borderStartStartRadius: 'var(--radius-control)',
        borderEndStartRadius: 'var(--radius-control)',
        cursor: 'pointer',
        color: hovered ? 'var(--text-secondary)' : 'var(--text-faint)',
        // Opaque either way: a translucent hover wash would let the canvas
        // show through the tab.
        background: 'var(--surface-panel)',
      }}
    >
      {/* Points inward, toward the tray it brings back. */}
      <InlineIcon svg={rtl ? chevronRightSvg : chevronLeftSvg} size={18} />
    </button>
  )
}
