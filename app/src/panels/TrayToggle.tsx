/**
 * The tray's put-away control and its way back. Open, the chevron sits in a
 * thin row at the top of the tray, left-aligned to mirror the rail's. Put
 * away, the tray is gone entirely, so the way back is a tab floating on the
 * viewport's right edge, vertically centered to stay clear of the ViewCube
 * and the measurement box.
 */
import { useState } from 'react'
import { InlineIcon } from './ToolRail'
import chevronLeftSvg from '@material-symbols/svg-400/outlined/chevron_left.svg?raw'
import chevronRightSvg from '@material-symbols/svg-400/outlined/chevron_right.svg?raw'

export function TrayCollapseRow({ onCollapse }: { onCollapse: () => void }) {
  const [hovered, setHovered] = useState(false)
  return (
    <div style={{ display: 'flex', flexShrink: 0, padding: '6px 8px 0' }}>
      <button
        type="button"
        aria-label="Collapse tray"
        title="Collapse tray"
        onClick={onCollapse}
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
        style={{
          display: 'flex',
          padding: '2px',
          borderRadius: 'var(--radius-control)',
          border: 'none',
          cursor: 'pointer',
          color: 'var(--text-faint, #888)',
          background: hovered ? 'rgba(255,255,255,0.04)' : 'transparent',
        }}
      >
        <InlineIcon svg={chevronRightSvg} size={18} />
      </button>
    </div>
  )
}

export function TrayExpandTab({ onExpand }: { onExpand: () => void }) {
  const [hovered, setHovered] = useState(false)
  return (
    <button
      type="button"
      aria-label="Expand tray"
      title="Expand tray"
      onClick={onExpand}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      style={{
        position: 'absolute',
        right: 0,
        top: '50%',
        transform: 'translateY(-50%)',
        zIndex: 20,
        display: 'flex',
        padding: '10px 1px',
        border: '1px solid var(--border-hairline)',
        borderRight: 'none',
        borderRadius: 'var(--radius-control) 0 0 var(--radius-control)',
        cursor: 'pointer',
        color: hovered ? 'var(--text-secondary)' : 'var(--text-faint, #888)',
        background: 'var(--surface-panel)',
      }}
    >
      <InlineIcon svg={chevronLeftSvg} size={18} />
    </button>
  )
}
