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
  /**
   * When true the OS provides a native menu bar (Tauri desktop).
   * The in-app bar hides the File/Edit dropdown menus and shows
   * only the document title + kernel version badge.
   */
  nativeMenuBar?: boolean
  onNew: () => void
  onOpen: () => void
  onSave: () => void
  onSaveAs: () => void
  onUndo: () => void
  onRedo: () => void
  canUndo: boolean
  canRedo: boolean
  /** The currently active tool name. */
  activeTool?: string
  /** Called when the user picks a tool from the menu. */
  onSelectTool?: (name: string) => void
  /** Whether the Model info pane is visible. */
  showModelInfo?: boolean
  /** Whether the Materials pane is visible. */
  showMaterials?: boolean
  /** Toggle the Model info pane. */
  onToggleModelInfo?: () => void
  /** Toggle the Materials pane. */
  onToggleMaterials?: () => void
}

type MenuId = 'file' | 'edit' | 'draw' | 'tools' | 'camera' | 'window' | null

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

interface CheckMenuItemProps {
  label: string
  shortcut?: string
  checked: boolean
  onClick: () => void
}

function CheckMenuItem({ label, shortcut, checked, onClick }: CheckMenuItemProps) {
  const [hovered, setHovered] = useState(false)
  return (
    <div
      style={{
        ...MENU_ITEM_STYLE(false),
        background: hovered ? '#3a5e9e' : 'transparent',
      }}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onMouseDown={(e) => {
        e.preventDefault()
        onClick()
      }}
    >
      <span style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
        <span style={{ width: '12px', fontSize: '10px', color: '#7aaaee' }}>{checked ? '✓' : ''}</span>
        {label}
      </span>
      {shortcut !== undefined && <span style={SHORTCUT_STYLE}>{shortcut}</span>}
    </div>
  )
}

export function MenuBar({
  title,
  kernelVersion,
  nativeMenuBar = false,
  onNew,
  onOpen,
  onSave,
  onSaveAs,
  onUndo,
  onRedo,
  canUndo,
  canRedo,
  activeTool,
  onSelectTool,
  showModelInfo = true,
  showMaterials = true,
  onToggleModelInfo,
  onToggleMaterials,
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
      {/* File and Edit menus — hidden when the OS provides a native menu bar */}
      {!nativeMenuBar && (
        <>
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

          {/* Draw menu */}
          <div style={{ position: 'relative' }}>
            <button
              style={MENU_TRIGGER_STYLE(openMenu === 'draw')}
              onClick={() => toggle('draw')}
            >
              Draw
            </button>
            {openMenu === 'draw' && (
              <div style={DROPDOWN_STYLE}>
                <CheckMenuItem
                  label="Rectangle"
                  shortcut={`${mod}K`}
                  checked={activeTool === 'Rectangle'}
                  onClick={withClose(() => onSelectTool?.('Rectangle'))}
                />
              </div>
            )}
          </div>

          {/* Tools menu */}
          <div style={{ position: 'relative' }}>
            <button
              style={MENU_TRIGGER_STYLE(openMenu === 'tools')}
              onClick={() => toggle('tools')}
            >
              Tools
            </button>
            {openMenu === 'tools' && (
              <div style={DROPDOWN_STYLE}>
                <CheckMenuItem
                  label="Select"
                  shortcut="Space"
                  checked={activeTool === 'Select'}
                  onClick={withClose(() => onSelectTool?.('Select'))}
                />
                <CheckMenuItem
                  label="Paint"
                  checked={activeTool === 'Paint'}
                  onClick={withClose(() => onSelectTool?.('Paint'))}
                />
                <CheckMenuItem
                  label="Move"
                  shortcut={`${mod}0`}
                  checked={activeTool === 'Move'}
                  onClick={withClose(() => onSelectTool?.('Move'))}
                />
                <CheckMenuItem
                  label="Rotate"
                  shortcut={`${mod}8`}
                  checked={activeTool === 'Rotate'}
                  onClick={withClose(() => onSelectTool?.('Rotate'))}
                />
                <CheckMenuItem
                  label="Scale"
                  shortcut={`${mod}9`}
                  checked={activeTool === 'Scale'}
                  onClick={withClose(() => onSelectTool?.('Scale'))}
                />
                <CheckMenuItem
                  label="Push/Pull"
                  shortcut={`${mod}=`}
                  checked={activeTool === 'Push/Pull'}
                  onClick={withClose(() => onSelectTool?.('Push/Pull'))}
                />
              </div>
            )}
          </div>

          {/* Camera menu */}
          <div style={{ position: 'relative' }}>
            <button
              style={MENU_TRIGGER_STYLE(openMenu === 'camera')}
              onClick={() => toggle('camera')}
            >
              Camera
            </button>
            {openMenu === 'camera' && (
              <div style={DROPDOWN_STYLE}>
                <CheckMenuItem
                  label="Orbit"
                  shortcut={`${mod}B`}
                  checked={activeTool === 'Orbit'}
                  onClick={withClose(() => onSelectTool?.('Orbit'))}
                />
                <CheckMenuItem
                  label="Pan"
                  shortcut={`${mod}R`}
                  checked={activeTool === 'Pan'}
                  onClick={withClose(() => onSelectTool?.('Pan'))}
                />
                <CheckMenuItem
                  label="Zoom"
                  shortcut={`${mod}\\`}
                  checked={activeTool === 'Zoom'}
                  onClick={withClose(() => onSelectTool?.('Zoom'))}
                />
              </div>
            )}
          </div>

          {/* Window menu */}
          <div style={{ position: 'relative' }}>
            <button
              style={MENU_TRIGGER_STYLE(openMenu === 'window')}
              onClick={() => toggle('window')}
            >
              Window
            </button>
            {openMenu === 'window' && (
              <div style={DROPDOWN_STYLE}>
                <CheckMenuItem
                  label="Model Info"
                  shortcut={`⇧${mod}I`}
                  checked={showModelInfo}
                  onClick={withClose(() => onToggleModelInfo?.())}
                />
                <CheckMenuItem
                  label="Materials"
                  shortcut={`⇧${mod}C`}
                  checked={showMaterials}
                  onClick={withClose(() => onToggleMaterials?.())}
                />
              </div>
            )}
          </div>
        </>
      )}

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
