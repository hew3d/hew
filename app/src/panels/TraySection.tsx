/**
 * TraySection — a collapsible section for the docked right tray (
 * `06_docked_panels.md`). Replaces `FloatingPanel.tsx`'s role for Entity
 * Info / Outliner / Materials / Tags: same header+scrollable-body shape,
 * with FloatingPanel's drag/resize/snap/z-order logic stripped (a fixed
 * tray has no need for any of it) and a collapse/expand toggle in its place.
 *
 * Layout note (a deliberate simplification vs. the spec): rather than "only
 * the bottom-most section stretches, others size to content," every
 * currently-expanded section gets `flex: 1` and scrolls independently —
 * flexbox then splits the tray's remaining height evenly across however
 * many sections happen to be open (1, 2, 3, or 4). This is simpler and more
 * robust than tracking which section is "last expanded" and avoids any
 * section growing unboundedly tall.
 */
export interface TraySectionProps {
  title: string
  collapsed: boolean
  onToggle: () => void
  children: React.ReactNode
  /** Extra control rendered at the header's trailing edge, alongside the
   *  title/chevron toggle button rather than inside it (a nested `<button>`
   *  would be invalid HTML and would also fire `onToggle` on every click) —
   *  Scenes' header ⊕ Add Scene icon button (SPEC.md §1) is the first
   *  consumer. Omitted for every other section, so their header markup and
   *  behavior stay unchanged. */
  headerRight?: React.ReactNode
}

export function TraySection({ title, collapsed, onToggle, children, headerRight }: TraySectionProps) {
  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        borderBottom: '1px solid var(--border-hairline)',
        minHeight: 0,
        flex: collapsed ? '0 0 auto' : '1 1 0',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', flexShrink: 0 }}>
        <button
          type="button"
          onClick={onToggle}
          aria-expanded={!collapsed}
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            flex: '1 1 auto',
            minWidth: 0,
            padding: '11px 13px',
            background: 'transparent',
            border: 'none',
            cursor: 'pointer',
            fontFamily: 'var(--font-family-mono)',
            fontSize: 'var(--font-size-panel-header, 11px)',
            fontWeight: 600,
            textTransform: 'uppercase',
            letterSpacing: '0.08em',
            color: 'var(--text-secondary)',
          }}
        >
          <span>{title}</span>
          <span aria-hidden="true" style={{ color: 'var(--text-section)' }}>{collapsed ? '▸' : '▾'}</span>
        </button>
        {headerRight !== undefined && (
          <span style={{ flexShrink: 0, paddingRight: '11px' }}>{headerRight}</span>
        )}
      </div>
      {!collapsed && (
        <div style={{ flex: '1 1 0', minHeight: 0, overflowY: 'auto', padding: '0 13px 13px' }}>
          {children}
        </div>
      )}
    </div>
  )
}
