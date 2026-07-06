/**
 * MenuBar — top-of-screen app bar with File + Edit menus.
 *
 * Shows the document name + "Edited/Saved <relative time>" save-state
 * indicator centered in the bar, when not hidden.
 * Keyboard shortcuts for File operations are handled in App.tsx via global
 * keydown listeners; the menu items here are the visual/click-driven path.
 *
 * Under Tauri (nativeMenuBar=true, macOS only since) the OS owns the
 * menus AND the document title lives in the native title bar (set via
 * Tauri's window.setTitle — see App.tsx), so this component renders nothing
 * in that case. On Windows/Linux, `hideTitle` is set instead (the custom
 * `TitleBar` above shows the name + indicator there); only the web build
 * reaches this component's own centered name/indicator.
 */

import { useState, useEffect, useRef, useCallback } from 'react'
import { modLabel, isMac } from '../platform'
import { shortcutFor, type ToolName } from '../tools/toolRegistry'
import type { StandardView } from '../viewport/Viewport'

export interface MenuBarProps {
  /** Bare document name (no dirty mark, no " — Hew" suffix — `documentSession.ts`'s
   * `documentName()`). Only rendered in the web (non-native) bar. */
  name: string
  /** "Edited <relative time>" / "Saved <relative time>" / "" — `documentSession.ts`'s
   * `saveStateLabel()`. Only rendered alongside `name` in the web bar. */
  saveState: string
  /**
   * When true the OS provides a native menu bar (Tauri desktop) AND the
   * native title bar shows the document title — so this component renders
   * nothing at all in that case.
   */
  nativeMenuBar?: boolean
  /** Hide the centered document title (a custom TitleBar above shows it). */
  hideTitle?: boolean
  onNew: () => void
  onOpen: () => void
  onSave: () => void
  onSaveAs: () => void
  /** Import a model file (COLLADA / SketchUp / glTF — chosen in the file dialog). */
  onImport: () => void
  /** Open the unified Export dialog (format — glTF/GLB or STL — chosen there). */
  onExport: () => void
  /** Recent file paths (most-recent first), shown under File ▸ Open Recent. */
  recentFiles?: string[]
  /** Open a recent file by its path. */
  onOpenRecent?: (path: string) => void
  /** Clear the recent-files list. */
  onClearRecent?: () => void
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
  /** Whether the Tags pane is visible. */
  showTags?: boolean
  /** Whether the Object Info pane is visible. */
  showObjectInfo?: boolean
  /** Whether the Debug Log panel is visible. */
  showDebugLog?: boolean
  /** Toggle the Model info pane. */
  onToggleModelInfo?: () => void
  /** Toggle the Materials pane. */
  onToggleMaterials?: () => void
  /** Toggle the Tags pane. */
  onToggleTags?: () => void
  /** Toggle the Object Info pane. */
  onToggleObjectInfo?: () => void
  /** Toggle the Debug Log panel. */
  onToggleDebugLog?: () => void
  /** Whether the world axes/grid are shown (View ▸ Axes). */
  showAxes?: boolean
  /** Whether construction guides are shown (View ▸ Guides). */
  showGuides?: boolean
  /** Toggle the world axes/grid. */
  onToggleAxes?: () => void
  /** Toggle construction-guide visibility. */
  onToggleGuides?: () => void
  /** Delete every construction guide (Edit ▸ Delete Guide Lines). */
  onDeleteGuides?: () => void
  /** Delete the current selection — whole Object/Group/Instance nodes only (Edit ▸ Delete). */
  onDelete?: () => void
  /** Zoom the camera to fit all scene geometry (View → Zoom Extents). */
  onZoomExtents?: () => void
  /** Reposition the camera to a standard view (Camera → Standard Views). */
  onStandardView?: (view: StandardView) => void
  /** Open the Settings window/modal (Window → Settings…, web only — native uses the OS app menu). */
  onOpenSettings?: () => void
  /** Assemble and write a "Report Bug" bundle (Help → Report Bug…). */
  onReportBug?: () => void
}

type MenuId = 'file' | 'edit' | 'view' | 'draw' | 'tools' | 'camera' | 'window' | 'help' | null

/** Shortcut display for a tool-registry entry — mac vs. the
 * Windows/Linux/Web bare-letter scheme, `undefined` when the tool has none. */
function keyFor(name: ToolName): string | undefined {
  const s = shortcutFor(name, isMac)
  return s === '' ? undefined : s
}

/** Filename portion of a path (handles / and \ separators). */
function baseName(path: string): string {
  return path.replace(/[/\\]+$/, '').split(/[/\\]/).pop() ?? path
}

// `02_app_shell.md`'s Windows/Linux menu-bar spec: 33px height, surface/bar.
const BAR_STYLE: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  height: '33px',
  background: 'var(--surface-bar, #1e1e1e)',
  borderBottom: '1px solid var(--border-hairline, #3a3a3a)',
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
  fontSize: 'var(--font-size-menu-item, 13px)',
  color: open ? 'var(--accent-text-on-tint, #fff)' : 'var(--text-tertiary, #ccc)',
  background: open ? 'var(--accent-tint-15, #3a5e9e)' : 'transparent',
  cursor: 'pointer',
  border: 'none',
  fontFamily: 'var(--font-family-ui)',
  whiteSpace: 'nowrap',
  borderRadius: 'var(--radius-control, 0)',
})

const DROPDOWN_STYLE: React.CSSProperties = {
  position: 'absolute',
  // Anchored to the trigger wrapper's bottom edge. The wrapper must be
  // height:100% (see the trigger divs below): the bar centers its flex items,
  // so an auto-height wrapper sits ~8px down and a pixel offset here would
  // open the menu detached from the bar (regression fixed).
  top: '100%',
  left: 0,
  minWidth: '180px',
  background: 'var(--surface-overlay, #2a2a2a)',
  backdropFilter: 'blur(12px)',
  border: '1px solid var(--border-strong, #4a4a4a)',
  borderRadius: '0 0 var(--radius-control, 4px) var(--radius-control, 4px)',
  boxShadow: 'var(--shadow-dock, 0 4px 12px rgba(0,0,0,0.5))',
  zIndex: 1000,
  paddingTop: '4px',
  paddingBottom: '4px',
}

const MENU_ITEM_STYLE = (disabled: boolean): React.CSSProperties => ({
  display: 'flex',
  justifyContent: 'space-between',
  alignItems: 'center',
  padding: '5px 16px',
  fontSize: 'var(--font-size-menu-item, 13px)',
  color: disabled ? 'var(--text-faint, #666)' : 'var(--text-secondary, #ddd)',
  cursor: disabled ? 'default' : 'pointer',
  fontFamily: 'var(--font-family-ui)',
  gap: '32px',
})

const SEPARATOR_STYLE: React.CSSProperties = {
  height: '1px',
  background: 'var(--border-hairline, #444)',
  margin: '4px 0',
}

const SHORTCUT_STYLE: React.CSSProperties = {
  fontFamily: 'var(--font-family-mono)',
  fontSize: 'var(--font-size-kbd, 11px)',
  color: 'var(--text-faint, #888)',
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
        background: hovered && !disabled ? 'var(--accent-tint-15, #3a5e9e)' : 'transparent',
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

/** A menu row that opens a nested flyout to its right on hover. */
function SubMenu({ label, children }: { label: string; children: React.ReactNode }) {
  const [hovered, setHovered] = useState(false)
  return (
    <div
      style={{ position: 'relative' }}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      <div
        style={{
          ...MENU_ITEM_STYLE(false),
          background: hovered ? 'var(--accent-tint-15, #3a5e9e)' : 'transparent',
        }}
      >
        <span>{label}</span>
        <span style={SHORTCUT_STYLE}>▸</span>
      </div>
      {hovered && (
        <div style={{ ...DROPDOWN_STYLE, top: -4, left: '100%', borderRadius: '4px' }}>
          {children}
        </div>
      )}
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
        background: hovered ? 'var(--accent-tint-15, #3a5e9e)' : 'transparent',
      }}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onMouseDown={(e) => {
        e.preventDefault()
        onClick()
      }}
    >
      <span style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
        <span style={{ width: '12px', fontSize: '10px', color: 'var(--accent-base, #7aaaee)' }}>{checked ? '✓' : ''}</span>
        {label}
      </span>
      {shortcut !== undefined && <span style={SHORTCUT_STYLE}>{shortcut}</span>}
    </div>
  )
}

export function MenuBar({
  name,
  saveState,
  nativeMenuBar = false,
  hideTitle = false,
  onNew,
  onOpen,
  onSave,
  onSaveAs,
  onImport,
  onExport,
  recentFiles,
  onOpenRecent,
  onClearRecent,
  onUndo,
  onRedo,
  canUndo,
  canRedo,
  activeTool,
  onSelectTool,
  showModelInfo = true,
  showMaterials = true,
  showTags = false,
  showObjectInfo = false,
  showDebugLog = false,
  onToggleModelInfo,
  onToggleMaterials,
  onToggleTags,
  onToggleObjectInfo,
  onToggleDebugLog,
  showAxes = true,
  showGuides = true,
  onToggleAxes,
  onToggleGuides,
  onDeleteGuides,
  onDelete,
  onZoomExtents,
  onStandardView,
  onOpenSettings,
  onReportBug,
}: MenuBarProps) {
  const [openMenu, setOpenMenu] = useState<MenuId>(null)
  const barRef = useRef<HTMLDivElement>(null)

  const mod = modLabel

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

  // Under Tauri the OS owns the menus AND the title bar shows the document
  // title (App.tsx calls the Tauri window setTitle) — so there is nothing
  // left for the in-app bar to render.
  if (nativeMenuBar) return null

  return (
    <div ref={barRef} style={BAR_STYLE} data-testid="menu-bar">
      {/* File menu */}
      <div style={{ position: 'relative', height: '100%' }}>
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
            {recentFiles !== undefined && (
              <SubMenu label="Open Recent">
                {recentFiles.length === 0 ? (
                  <MenuItem label="No Recent Files" disabled onClick={() => {}} />
                ) : (
                  <>
                    {recentFiles.map((p) => (
                      <MenuItem
                        key={p}
                        label={baseName(p)}
                        onClick={withClose(() => onOpenRecent?.(p))}
                      />
                    ))}
                    <div style={SEPARATOR_STYLE} />
                    <MenuItem label="Clear Recent" onClick={withClose(() => onClearRecent?.())} />
                  </>
                )}
              </SubMenu>
            )}
            <div style={SEPARATOR_STYLE} />
            <MenuItem label="Import…" onClick={withClose(onImport)} />
            <MenuItem label="Export…" onClick={withClose(onExport)} />
            <div style={SEPARATOR_STYLE} />
            <MenuItem label="Save" shortcut={`${mod}S`} onClick={withClose(onSave)} />
            <MenuItem label="Save As…" shortcut={`${mod}⇧S`} onClick={withClose(onSaveAs)} />
          </div>
        )}
      </div>

      {/* Edit menu */}
      <div style={{ position: 'relative', height: '100%' }}>
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
            <div style={SEPARATOR_STYLE} />
            <MenuItem
              label="Delete"
              shortcut="⌫"
              onClick={withClose(() => onDelete?.())}
            />
            <MenuItem
              label="Delete Guide Lines"
              onClick={withClose(() => onDeleteGuides?.())}
            />
          </div>
        )}
      </div>

      {/* View menu */}
      <div style={{ position: 'relative', height: '100%' }}>
        <button
          style={MENU_TRIGGER_STYLE(openMenu === 'view')}
          onClick={() => toggle('view')}
        >
          View
        </button>
        {openMenu === 'view' && (
          <div style={DROPDOWN_STYLE}>
            <CheckMenuItem
              label="Axes"
              checked={showAxes}
              onClick={withClose(() => onToggleAxes?.())}
            />
            <CheckMenuItem
              label="Guides"
              checked={showGuides}
              onClick={withClose(() => onToggleGuides?.())}
            />
          </div>
        )}
      </div>

      {/* Draw menu */}
      <div style={{ position: 'relative', height: '100%' }}>
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
              shortcut={keyFor('Rectangle')}
              checked={activeTool === 'Rectangle'}
              onClick={withClose(() => onSelectTool?.('Rectangle'))}
            />
            <CheckMenuItem
              label="Circle"
              shortcut={keyFor('Circle')}
              checked={activeTool === 'Circle'}
              onClick={withClose(() => onSelectTool?.('Circle'))}
            />
            <CheckMenuItem
              label="Arc"
              shortcut={keyFor('Arc')}
              checked={activeTool === 'Arc'}
              onClick={withClose(() => onSelectTool?.('Arc'))}
            />
            <div style={{ borderTop: '1px solid var(--border-strong)', margin: '4px 0' }} />
            <CheckMenuItem
              label="Line"
              shortcut={keyFor('Line')}
              checked={activeTool === 'Line'}
              onClick={withClose(() => onSelectTool?.('Line'))}
            />
          </div>
        )}
      </div>

      {/* Tools menu */}
      <div style={{ position: 'relative', height: '100%' }}>
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
              shortcut={keyFor('Select')}
              checked={activeTool === 'Select'}
              onClick={withClose(() => onSelectTool?.('Select'))}
            />
            <CheckMenuItem
              label="Paint"
              shortcut={keyFor('Paint')}
              checked={activeTool === 'Paint'}
              onClick={withClose(() => onSelectTool?.('Paint'))}
            />
            <CheckMenuItem
              label="Move"
              shortcut={keyFor('Move')}
              checked={activeTool === 'Move'}
              onClick={withClose(() => onSelectTool?.('Move'))}
            />
            <CheckMenuItem
              label="Rotate"
              shortcut={keyFor('Rotate')}
              checked={activeTool === 'Rotate'}
              onClick={withClose(() => onSelectTool?.('Rotate'))}
            />
            <CheckMenuItem
              label="Scale"
              shortcut={keyFor('Scale')}
              checked={activeTool === 'Scale'}
              onClick={withClose(() => onSelectTool?.('Scale'))}
            />
            <CheckMenuItem
              label="Push/Pull"
              shortcut={keyFor('Push/Pull')}
              checked={activeTool === 'Push/Pull'}
              onClick={withClose(() => onSelectTool?.('Push/Pull'))}
            />
            <div style={SEPARATOR_STYLE} />
            <CheckMenuItem
              label="Tape Measure"
              shortcut={keyFor('Tape Measure')}
              checked={activeTool === 'Tape Measure'}
              onClick={withClose(() => onSelectTool?.('Tape Measure'))}
            />
            <CheckMenuItem
              label="Protractor"
              checked={activeTool === 'Protractor'}
              onClick={withClose(() => onSelectTool?.('Protractor'))}
            />
            <CheckMenuItem
              label="Slice"
              checked={activeTool === 'Slice'}
              onClick={withClose(() => onSelectTool?.('Slice'))}
            />
            <CheckMenuItem
              label="Edit Vertex"
              checked={activeTool === 'Edit Vertex'}
              onClick={withClose(() => onSelectTool?.('Edit Vertex'))}
            />
          </div>
        )}
      </div>

      {/* Camera menu */}
      <div style={{ position: 'relative', height: '100%' }}>
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
              shortcut={keyFor('Orbit')}
              checked={activeTool === 'Orbit'}
              onClick={withClose(() => onSelectTool?.('Orbit'))}
            />
            <CheckMenuItem
              label="Pan"
              shortcut={keyFor('Pan')}
              checked={activeTool === 'Pan'}
              onClick={withClose(() => onSelectTool?.('Pan'))}
            />
            <CheckMenuItem
              label="Zoom"
              shortcut={keyFor('Zoom')}
              checked={activeTool === 'Zoom'}
              onClick={withClose(() => onSelectTool?.('Zoom'))}
            />
            <div style={SEPARATOR_STYLE} />
            <MenuItem
              label="Zoom Extents"
              onClick={withClose(() => onZoomExtents?.())}
            />
            <div style={SEPARATOR_STYLE} />
            <SubMenu label="Standard Views">
              <MenuItem label="Top" onClick={withClose(() => onStandardView?.('top'))} />
              <MenuItem label="Bottom" onClick={withClose(() => onStandardView?.('bottom'))} />
              <MenuItem label="Front" onClick={withClose(() => onStandardView?.('front'))} />
              <MenuItem label="Back" onClick={withClose(() => onStandardView?.('back'))} />
              <MenuItem label="Left" onClick={withClose(() => onStandardView?.('left'))} />
              <MenuItem label="Right" onClick={withClose(() => onStandardView?.('right'))} />
              <div style={SEPARATOR_STYLE} />
              <MenuItem label="Iso" onClick={withClose(() => onStandardView?.('iso'))} />
            </SubMenu>
          </div>
        )}
      </div>

      {/* Window menu */}
      <div style={{ position: 'relative', height: '100%' }}>
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
            <CheckMenuItem
              label="Tags"
              shortcut={`⇧${mod}T`}
              checked={showTags}
              onClick={withClose(() => onToggleTags?.())}
            />
            <CheckMenuItem
              label="Object Info"
              shortcut={`⇧${mod}O`}
              checked={showObjectInfo}
              onClick={withClose(() => onToggleObjectInfo?.())}
            />
            <CheckMenuItem
              label="Debug Log"
              checked={showDebugLog}
              onClick={withClose(() => onToggleDebugLog?.())}
            />
            <div style={SEPARATOR_STYLE} />
            <MenuItem
              label="Settings…"
              shortcut={`${mod},`}
              onClick={withClose(() => onOpenSettings?.())}
            />
          </div>
        )}
      </div>

      {/* Help menu */}
      <div style={{ position: 'relative', height: '100%' }}>
        <button
          style={MENU_TRIGGER_STYLE(openMenu === 'help')}
          onClick={() => toggle('help')}
        >
          Help
        </button>
        {openMenu === 'help' && (
          <div style={DROPDOWN_STYLE}>
            <MenuItem
              label="Report Bug…"
              onClick={withClose(() => onReportBug?.())}
            />
          </div>
        )}
      </div>

      {/* Document name + save-state indicator — centered in bar (hidden when
          a custom TitleBar above already shows it, e.g. the Linux/Windows
          borderless shells). Web build only reaches here; it follows the
          same Windows/Linux name+indicator split, `02_app_shell.md`. */}
      {!hideTitle && (
        <div
          style={{
            position: 'absolute',
            left: '50%',
            transform: 'translateX(-50%)',
            display: 'flex',
            alignItems: 'baseline',
            gap: 'var(--space-2, 6px)',
            fontFamily: 'var(--font-family-ui)',
            pointerEvents: 'none',
            whiteSpace: 'nowrap',
            overflow: 'hidden',
            maxWidth: '50%',
          }}
        >
          <span
            style={{
              fontSize: 'var(--font-size-titlebar-filename, 13px)',
              fontWeight: 600,
              color: 'var(--text-primary, #eee)',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
            }}
          >
            {name}
          </span>
          {saveState !== '' && (
            <span
              style={{
                fontSize: 'var(--font-size-titlebar-meta, 11px)',
                color: 'var(--text-section, #888)',
                flexShrink: 0,
              }}
            >
              {saveState}
            </span>
          )}
        </div>
      )}

      {/* Spacer. (The resting command-palette field lived here on
          Windows/Linux/Web until moved it to the top of the tool rail on
          every platform — see ToolRail.tsx.) */}
      <div style={{ flex: 1 }} />
    </div>
  )
}
