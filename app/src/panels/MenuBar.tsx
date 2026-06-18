/**
 * MenuBar — top-of-screen app bar with File + Edit menus.
 *
 * Shows the document title (with dirty marker) in the center.
 * Keyboard shortcuts for File operations are handled in App.tsx via global
 * keydown listeners; the menu items here are the visual/click-driven path.
 */

import { useState, useEffect, useRef, useCallback } from 'react'

export interface MenuBarProps {
  /** Full document title (already includes dirty mark and " — Hew"). */
  title: string
  kernelVersion: string
  onNew: () => void
  onOpen: () => void
  onSave: () => void
  onSaveAs: () => void
  onUndo: () => void
  onRedo: () => void
  canUndo: boolean
  canRedo: boolean
}

type MenuId = 'file' | 'edit' | null

const BAR_STYLE: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  height: '32px',
  background: '#1e1e1e',
  borderBottom: '1px solid #3a3a3a',
  flexShrink: 0,
  userSelect: 'none',
  gap: 0,
  position: 'relative',
}

const MENU_TRIGGER_STYLE = (open: boolean): React.CSSProperties => ({
  padding: '0 12px',
  height: '100%',
  display: 'flex',
  alignItems: 'center',
  fontSize: '13px',
  color: open ? '#fff' : '#ccc',
  background: open ? '#3a5e9e' : 'transparent',
  cursor: 'pointer',
  border: 'none',
  fontFamily: 'system-ui, sans-serif',
  whiteSpace: 'nowrap',
})

const DROPDOWN_STYLE: React.CSSProperties = {
  position: 'absolute',
  top: '32px',
  left: 0,
  minWidth: '180px',
  background: '#2a2a2a',
  border: '1px solid #4a4a4a',
  borderRadius: '0 0 4px 4px',
  boxShadow: '0 4px 12px rgba(0,0,0,0.5)',
  zIndex: 1000,
  paddingTop: '4px',
  paddingBottom: '4px',
}

const MENU_ITEM_STYLE = (disabled: boolean): React.CSSProperties => ({
  display: 'flex',
  justifyContent: 'space-between',
  alignItems: 'center',
  padding: '5px 16px',
  fontSize: '13px',
  color: disabled ? '#666' : '#ddd',
  cursor: disabled ? 'default' : 'pointer',
  fontFamily: 'system-ui, sans-serif',
  gap: '32px',
})

const SEPARATOR_STYLE: React.CSSProperties = {
  height: '1px',
  background: '#444',
  margin: '4px 0',
}

const SHORTCUT_STYLE: React.CSSProperties = {
  fontSize: '11px',
  color: '#888',
  whiteSpace: 'nowrap',
}

interface MenuItemProps {
  label: string
  shortcut?: string
  disabled?: boolean
  onClick: () => void
}

function MenuItem({ label, shortcut, disabled = false, onClick }: MenuItemProps) {
  const [hovered, setHovered] = useState(false)
  return (
    <div
      style={{
        ...MENU_ITEM_STYLE(disabled),
        background: hovered && !disabled ? '#3a5e9e' : 'transparent',
      }}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onMouseDown={(e) => {
        e.preventDefault() // don't steal focus
        if (!disabled) onClick()
      }}
    >
      <span>{label}</span>
      {shortcut !== undefined && <span style={SHORTCUT_STYLE}>{shortcut}</span>}
    </div>
  )
}

export function MenuBar({
  title,
  kernelVersion,
  onNew,
  onOpen,
  onSave,
  onSaveAs,
  onUndo,
  onRedo,
  canUndo,
  canRedo,
}: MenuBarProps) {
  const [openMenu, setOpenMenu] = useState<MenuId>(null)
  const barRef = useRef<HTMLDivElement>(null)

  const isMac = typeof navigator !== 'undefined' && /Mac|iPod|iPhone|iPad/.test(navigator.platform)
  const mod = isMac ? '⌘' : 'Ctrl+'

  const close = useCallback(() => setOpenMenu(null), [])

  const toggle = useCallback((id: MenuId) => {
    setOpenMenu((cur) => (cur === id ? null : id))
  }, [])

  // Close when clicking outside the menu bar
  useEffect(() => {
    if (openMenu === null) return
    const handler = (e: MouseEvent) => {
      if (barRef.current && !barRef.current.contains(e.target as Node)) {
        close()
      }
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [openMenu, close])

  const withClose = (fn: () => void) => () => { close(); fn() }

  return (
    <div ref={barRef} style={BAR_STYLE} data-testid="menu-bar">
      {/* File menu */}
      <div style={{ position: 'relative' }}>
        <button
          style={MENU_TRIGGER_STYLE(openMenu === 'file')}
          onClick={() => toggle('file')}
        >
          File
        </button>
        {openMenu === 'file' && (
          <div style={DROPDOWN_STYLE}>
            <MenuItem label="New" shortcut={`${mod}N`} onClick={withClose(onNew)} />
            <MenuItem label="Open…" shortcut={`${mod}O`} onClick={withClose(onOpen)} />
            <div style={SEPARATOR_STYLE} />
            <MenuItem label="Save" shortcut={`${mod}S`} onClick={withClose(onSave)} />
            <MenuItem label="Save As…" shortcut={`${mod}⇧S`} onClick={withClose(onSaveAs)} />
          </div>
        )}
      </div>

      {/* Edit menu */}
      <div style={{ position: 'relative' }}>
        <button
          style={MENU_TRIGGER_STYLE(openMenu === 'edit')}
          onClick={() => toggle('edit')}
        >
          Edit
        </button>
        {openMenu === 'edit' && (
          <div style={DROPDOWN_STYLE}>
            <MenuItem
              label="Undo"
              shortcut={`${mod}Z`}
              disabled={!canUndo}
              onClick={withClose(onUndo)}
            />
            <MenuItem
              label="Redo"
              shortcut={`${mod}⇧Z`}
              disabled={!canRedo}
              onClick={withClose(onRedo)}
            />
          </div>
        )}
      </div>

      {/* Document title — centered in bar */}
      <div
        style={{
          position: 'absolute',
          left: '50%',
          transform: 'translateX(-50%)',
          fontSize: '13px',
          color: '#bbb',
          fontFamily: 'system-ui, sans-serif',
          pointerEvents: 'none',
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          maxWidth: '50%',
          textOverflow: 'ellipsis',
        }}
      >
        {title}
      </div>

      {/* Spacer */}
      <div style={{ flex: 1 }} />

      {/* Kernel version — muted, right side */}
      <span
        style={{
          fontSize: '10px',
          color: '#555',
          fontFamily: 'monospace',
          paddingRight: '12px',
          whiteSpace: 'nowrap',
        }}
        title={`Kernel version ${kernelVersion}`}
      >
        v{kernelVersion}
      </span>
    </div>
  )
}
