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
  /** File ▸ Save to Library… — prompts for a name, then saves the whole
   *  document as a library item. Omitted/no-op-guarded the same way the
   *  other optional handlers are; `saveToLibraryDisabled` greys the item out
   *  on a platform with no library backend (the web build). */
  onSaveToLibrary?: () => void
  /** True on a platform with no library backend (web) — disables File ▸ Save
   *  to Library… rather than hiding it, matching Import…'s session-gating
   *  posture. */
  saveToLibraryDisabled?: boolean
  /** File ▸ Open on Phone… (docs/design/shop-mode.md §4) — starts the LAN
   *  QR-handoff dialog. `undefined` — and the item hidden — outside Tauri
   *  (only the desktop shell can run the LAN server); same optional-prop-
   *  gates-visibility convention as `onCheckForUpdates`. */
  onOpenOnPhone?: () => void
  /** Greys out File ▸ Open on Phone… when the document has nothing worth
   *  sharing (an empty scene) — mirrors `saveToLibraryDisabled`'s
   *  disabled-not-hidden posture, since the item's mere presence (not its
   *  enabled state) is what `onOpenOnPhone`'s own undefined-gates-
   *  visibility already controls. */
  openOnPhoneDisabled?: boolean
  /** Close the current window (desktop shells only — omitted on web, where
   *  the browser owns the window). */
  onClose?: () => void
  /** Quit the application (desktop shells only). macOS routes Quit through the
   *  native app menu instead, so this is the Windows/Linux path. */
  onExit?: () => void
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
  /** Whether the Library browser dialog is open. */
  showLibrary?: boolean
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
  /** Toggle the Library browser dialog (Window ▸ Library, ⇧⌘L). */
  onToggleLibrary?: () => void
  /** Whether the world axes are shown (View ▸ Axes). */
  showAxes?: boolean
  /** Whether the ground grid is shown (View ▸ Grid). */
  showGrid?: boolean
  /** Whether construction guides are shown (View ▸ Guides). */
  showGuides?: boolean
  /** Toggle the world axes. */
  onToggleAxes?: () => void
  /** Reset the movable drawing axes to world identity (View ▸ Reset Axes). */
  onResetAxes?: () => void
  /** Toggle the ground grid. */
  onToggleGrid?: () => void
  /** Toggle construction-guide visibility. */
  onToggleGuides?: () => void
  /** Delete every construction guide (Edit ▸ Delete Guide Lines). */
  onDeleteGuides?: () => void
  /** View ▸ Section Plane's check state: true only when a section is BOTH
   * placed and active — SketchUp's "Active Cut" — derived by the caller
   * from the section manager's own state (`getSectionState`), never a
   * locally-tracked shadow boolean. False (unchecked, and the item
   * disabled — see `sectionPlaneExists`) when no section is placed. */
  sectionPlaneChecked?: boolean
  /** Whether a section is placed at all, regardless of active/inactive —
   * gates the View ▸ Section Plane item's enabled state (nothing to toggle
   * with none placed). */
  sectionPlaneExists?: boolean
  /** Toggle the placed section plane's active (clipping) flag — View ▸
   * Section Plane, SketchUp's "Active Cut". Same `toggle-section-active`
   * dispatch this command has always used; only its menu location moved
   * (from Tools) in section-plane-polish. */
  onToggleSectionActive?: () => void
  /** Delete the current selection — whole Object/Group/Instance nodes only (Edit ▸ Delete). */
  onDelete?: () => void
  /** Run an Edit-menu object command by action id (edit-group, edit-union, …)
   *  — same dispatch ids the native macOS menu and the contextual dock use. */
  onEditAction?: (id: string) => void
  /** Selection-gated availability for the Edit-menu object commands. */
  editGates?: {
    canGroup: boolean
    canUngroup: boolean
    canMakeComponent: boolean
    canPlaceCopy: boolean
    canExplode: boolean
    canMakeUnique: boolean
    canBoolean: boolean
    /** False while an explode session is open (`ExplodeSessionScope`) —
     *  importing restructures the document, which a session can't fold
     *  back cleanly. Defaults to enabled (`true`) when omitted so callers
     *  that don't yet track session state see the pre-existing behavior. */
    canImport?: boolean
    /** False while an explode session is open, for the same reason as
     *  `canImport` — 3D Text placement is an instance placement. Defaults
     *  to enabled when omitted. */
    canDrawText?: boolean
  }
  /** Zoom the camera to fit all scene geometry (View → Zoom Extents). */
  onZoomExtents?: () => void
  /** Open the "3D Text…" dialog (Draw → 3D Text…, docs/design/3d-text.md). */
  onDrawText?: () => void
  /** Reposition the camera to a standard view (Camera → Standard Views). */
  onStandardView?: (view: StandardView) => void
  /** Camera ▸ Parallel Projection checkbox state (docs/design/camera.md §1). */
  parallelProjectionChecked?: boolean
  /** Toggle Camera ▸ Parallel Projection. */
  onToggleParallelProjection?: () => void
  /** Open document windows (Tauri multi-window only), rendered as a tail of
   *  focus-this-window entries at the end of the Window menu — the current
   *  window's entry is checked. `undefined` on plain web (single window, no
   *  shell to list from) and on macOS Tauri (the native menu renders its own
   *  tail shell-side; this only feeds the in-app bar used on Windows/Linux). */
  windowList?: { label: string; title: string; focused: boolean }[]
  /** Focus (raise) an open document window by its shell-assigned label. */
  onFocusWindow?: (label: string) => void
  /** Open the platform's Settings surface (the gear on the bar's trailing
   *  edge + Window → Settings…): the Fluent in-app page on Windows, the
   *  separate settings window on Linux, the modal on web. macOS reaches
   *  Settings through its native app menu instead (this bar renders nothing
   *  there). */
  onOpenSettings?: () => void
  /** Assemble and write a "Report Bug" bundle (Help → Report Bug…). */
  onReportBug?: () => void
  /** Trigger a manual update check (Help → Check for Updates…). Omitted — and
   *  the item hidden — on the web build and in package-manager desktop builds
   *  that compile the updater out. */
  onCheckForUpdates?: () => void
  /** Switch to Shop Mode (Window → Shop Mode): the fullscreen touch-first
   *  viewer/inspector shell (`shop/ShopApp.tsx`) — the mirror of ShopApp's
   *  own "Use full editor" entry. Omitted — and the item hidden — under
   *  Tauri (Shop Mode is web-only) and on any device that doesn't report a
   *  coarse pointer (`platform.isCoarsePointer()`): a touch-capable laptop
   *  with a mouse-shaped desktop session has no workbench use case for it.
   *  Same optional-prop-gates-visibility convention as `onCheckForUpdates`
   *  above. */
  onEnterShopMode?: () => void
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

/** Strip the constant " — Hew" app-name suffix a shell-pushed window title
 *  carries (`deriveTitle` in App.tsx/documentSession.ts) — the Window-menu
 *  list is already inside the Hew app, so repeating the app name on every
 *  entry is just noise. Purely cosmetic: if the title format ever changes
 *  this stops matching and the raw title shows instead (see the matching
 *  strip in the shell's own native Window-menu tail, main.rs). */
function windowMenuLabel(title: string): string {
  const stripped = title.replace(/ — Hew$/, '')
  return stripped === '' ? 'Untitled' : stripped
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

const MENU_TRIGGER_STYLE = (open: boolean, hovered: boolean): React.CSSProperties => ({
  padding: '0 12px',
  height: '100%',
  display: 'flex',
  alignItems: 'center',
  fontSize: 'var(--font-size-menu-item, 13px)',
  color: open ? 'var(--accent-text-on-tint, #fff)' : 'var(--text-tertiary, #ccc)',
  // Native menu bars highlight a trigger on plain hover, before any click.
  background: open || hovered ? 'var(--accent-tint-15, #3a5e9e)' : 'transparent',
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

/**
 * One top-level menu trigger, with native menu-bar semantics: plain hover
 * highlights the trigger; a click enters "menu mode" (drops the menu); while
 * any menu is open, hovering a different trigger switches the open menu to it
 * (no click needed); clicking the open trigger again — or anywhere outside
 * the bar (see the outside-mousedown effect), or Escape — leaves menu mode.
 */
function MenuTrigger({
  id,
  label,
  openMenu,
  onToggle,
  onActivate,
}: {
  id: MenuId
  label: string
  openMenu: MenuId
  onToggle: (id: MenuId) => void
  /** Hover-switch: make this the open menu, but only while one is open. */
  onActivate: (id: MenuId) => void
}) {
  const [hovered, setHovered] = useState(false)
  return (
    <button
      style={MENU_TRIGGER_STYLE(openMenu === id, hovered)}
      onClick={() => onToggle(id)}
      onMouseEnter={() => {
        setHovered(true)
        onActivate(id)
      }}
      onMouseLeave={() => setHovered(false)}
    >
      {label}
    </button>
  )
}

/** Gear button at the far right of the bar — opens the Settings surface
 *  (the Fluent settings page on Windows), per the modern convention of a
 *  settings gear on the menu bar's trailing edge. */
function GearButton({ onClick, shortcutHint }: { onClick: () => void; shortcutHint: string }) {
  const [hovered, setHovered] = useState(false)
  return (
    <button
      aria-label="Settings"
      title={`Settings (${shortcutHint})`}
      onClick={onClick}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      style={{
        width: '36px',
        height: '100%',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        border: 'none',
        background: hovered ? 'var(--accent-tint-15, #3a5e9e)' : 'transparent',
        color: 'var(--text-tertiary, #ccc)',
        cursor: 'pointer',
        padding: 0,
        flexShrink: 0,
      }}
    >
      <svg
        width="15"
        height="15"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden="true"
      >
        <circle cx="12" cy="12" r="3.2" />
        <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33h.08a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51h.08a1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v.08a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
      </svg>
    </button>
  )
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
  /** Mirrors `MenuItem`'s `disabled` — greys the row out and swallows
   * clicks. Used by View ▸ Section Plane while no section is placed
   * (D3, section-plane-polish): toggling is a documented no-op then, so
   * disabling reads better than an always-clickable unchecked box. */
  disabled?: boolean
  onClick: () => void
}

function CheckMenuItem({ label, shortcut, checked, disabled = false, onClick }: CheckMenuItemProps) {
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
        e.preventDefault()
        if (!disabled) onClick()
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
  onSaveToLibrary,
  saveToLibraryDisabled = false,
  onClose,
  onExit,
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
  showLibrary = false,
  onToggleModelInfo,
  onToggleMaterials,
  onToggleTags,
  onToggleObjectInfo,
  onToggleDebugLog,
  onToggleLibrary,
  showAxes = true,
  showGrid = true,
  showGuides = true,
  onToggleAxes,
  onResetAxes,
  onToggleGrid,
  onToggleGuides,
  onDeleteGuides,
  sectionPlaneChecked = false,
  sectionPlaneExists = false,
  onToggleSectionActive,
  onDelete,
  onEditAction,
  editGates,
  onZoomExtents,
  onDrawText,
  onStandardView,
  parallelProjectionChecked = false,
  onToggleParallelProjection,
  onOpenSettings,
  onReportBug,
  onCheckForUpdates,
  onOpenOnPhone,
  openOnPhoneDisabled = false,
  onEnterShopMode,
  windowList,
  onFocusWindow,
}: MenuBarProps) {
  const [openMenu, setOpenMenu] = useState<MenuId>(null)
  const barRef = useRef<HTMLDivElement>(null)

  const mod = modLabel

  const close = useCallback(() => setOpenMenu(null), [])

  const toggle = useCallback((id: MenuId) => {
    setOpenMenu((cur) => (cur === id ? null : id))
  }, [])

  // Native "menu mode" hover-switch: while some menu is open, pointing at a
  // different trigger opens that one — no click needed. With no menu open,
  // hover only highlights (MenuTrigger's local hover state).
  const activate = useCallback((id: MenuId) => {
    setOpenMenu((cur) => (cur === null ? null : id))
  }, [])

  // Leave menu mode on any click outside the menu bar, or on Escape.
  useEffect(() => {
    if (openMenu === null) return
    const onMouseDown = (e: MouseEvent) => {
      if (barRef.current && !barRef.current.contains(e.target as Node)) {
        close()
      }
    }
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation()
        close()
      }
    }
    document.addEventListener('mousedown', onMouseDown)
    document.addEventListener('keydown', onKeyDown)
    return () => {
      document.removeEventListener('mousedown', onMouseDown)
      document.removeEventListener('keydown', onKeyDown)
    }
  }, [openMenu, close])

  const withClose = (fn: () => void) => () => { close(); fn() }

  // Under Tauri the OS owns the menus AND the title bar shows the document
  // title (App.tsx calls the Tauri window setTitle) — so there is nothing
  // left for the in-app bar to render.
  if (nativeMenuBar) return null

  return (
    <div
      ref={barRef}
      style={BAR_STYLE}
      data-testid="menu-bar"
      // Clicking the EMPTY part of the bar leaves menu mode, like clicking
      // outside it (native menu bars treat their own dead space as outside).
      // Triggers/dropdowns/gear are children, so their clicks never have the
      // bar itself as target; only dead-space clicks do.
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) close()
      }}
    >
      {/* File menu */}
      <div style={{ position: 'relative', height: '100%' }}>
        <MenuTrigger id="file" label="File" openMenu={openMenu} onToggle={toggle} onActivate={activate} />
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
            <MenuItem
              label="Import…"
              disabled={editGates?.canImport === false}
              onClick={withClose(onImport)}
            />
            <MenuItem label="Export…" onClick={withClose(onExport)} />
            <div style={SEPARATOR_STYLE} />
            <MenuItem label="Save" shortcut={`${mod}S`} onClick={withClose(onSave)} />
            <MenuItem label="Save As…" shortcut={`${mod}⇧S`} onClick={withClose(onSaveAs)} />
            {onSaveToLibrary !== undefined && (
              <MenuItem
                label="Save to Library…"
                disabled={saveToLibraryDisabled}
                onClick={withClose(onSaveToLibrary)}
              />
            )}
            {onOpenOnPhone !== undefined && (
              <>
                <div style={SEPARATOR_STYLE} />
                <MenuItem
                  label="Open on Phone…"
                  disabled={openOnPhoneDisabled}
                  onClick={withClose(onOpenOnPhone)}
                />
              </>
            )}
            {(onClose !== undefined || onExit !== undefined) && (
              <div style={SEPARATOR_STYLE} />
            )}
            {onClose !== undefined && (
              <MenuItem label="Close" shortcut={`${mod}W`} onClick={withClose(onClose)} />
            )}
            {onExit !== undefined && (
              <MenuItem label="Exit" onClick={withClose(onExit)} />
            )}
          </div>
        )}
      </div>

      {/* Edit menu */}
      <div style={{ position: 'relative', height: '100%' }}>
        <MenuTrigger id="edit" label="Edit" openMenu={openMenu} onToggle={toggle} onActivate={activate} />
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
              label="Select All"
              shortcut={`${mod}A`}
              onClick={withClose(() => onEditAction?.('edit-select-all'))}
            />
            <MenuItem
              label="Delete"
              shortcut="⌫"
              onClick={withClose(() => onDelete?.())}
            />
            <MenuItem
              label="Delete Guide Lines"
              onClick={withClose(() => onDeleteGuides?.())}
            />
            <div style={SEPARATOR_STYLE} />
            <MenuItem
              label="Group"
              shortcut={`${mod}G`}
              disabled={!(editGates?.canGroup ?? false)}
              onClick={withClose(() => onEditAction?.('edit-group'))}
            />
            <MenuItem
              label="Ungroup"
              shortcut={`${mod}⇧G`}
              disabled={!(editGates?.canUngroup ?? false)}
              onClick={withClose(() => onEditAction?.('edit-ungroup'))}
            />
            <div style={SEPARATOR_STYLE} />
            <MenuItem
              label="Make Component"
              disabled={!(editGates?.canMakeComponent ?? false)}
              onClick={withClose(() => onEditAction?.('edit-make-component'))}
            />
            <MenuItem
              label="Place Copy"
              disabled={!(editGates?.canPlaceCopy ?? false)}
              onClick={withClose(() => onEditAction?.('edit-place-copy'))}
            />
            <MenuItem
              label="Explode"
              disabled={!(editGates?.canExplode ?? false)}
              onClick={withClose(() => onEditAction?.('edit-explode'))}
            />
            <MenuItem
              label="Make Unique"
              disabled={!(editGates?.canMakeUnique ?? false)}
              onClick={withClose(() => onEditAction?.('edit-make-unique'))}
            />
            <div style={SEPARATOR_STYLE} />
            <MenuItem
              label="Union"
              disabled={!(editGates?.canBoolean ?? false)}
              onClick={withClose(() => onEditAction?.('edit-union'))}
            />
            <MenuItem
              label="Subtract"
              disabled={!(editGates?.canBoolean ?? false)}
              onClick={withClose(() => onEditAction?.('edit-subtract'))}
            />
            <MenuItem
              label="Intersect"
              disabled={!(editGates?.canBoolean ?? false)}
              onClick={withClose(() => onEditAction?.('edit-intersect'))}
            />
          </div>
        )}
      </div>

      {/* View menu */}
      <div style={{ position: 'relative', height: '100%' }}>
        <MenuTrigger id="view" label="View" openMenu={openMenu} onToggle={toggle} onActivate={activate} />
        {openMenu === 'view' && (
          <div style={DROPDOWN_STYLE}>
            <CheckMenuItem
              label="Axes"
              checked={showAxes}
              onClick={withClose(() => onToggleAxes?.())}
            />
            <MenuItem
              label="Reset Axes"
              onClick={withClose(() => onResetAxes?.())}
            />
            <CheckMenuItem
              label="Grid"
              checked={showGrid}
              onClick={withClose(() => onToggleGrid?.())}
            />
            <CheckMenuItem
              label="Guides"
              checked={showGuides}
              onClick={withClose(() => onToggleGuides?.())}
            />
            <CheckMenuItem
              label="Section Plane"
              checked={sectionPlaneChecked}
              disabled={!sectionPlaneExists}
              onClick={withClose(() => onToggleSectionActive?.())}
            />
          </div>
        )}
      </div>

      {/* Draw menu */}
      <div style={{ position: 'relative', height: '100%' }}>
        <MenuTrigger id="draw" label="Draw" openMenu={openMenu} onToggle={toggle} onActivate={activate} />
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
              label="Polygon"
              shortcut={keyFor('Polygon')}
              checked={activeTool === 'Polygon'}
              onClick={withClose(() => onSelectTool?.('Polygon'))}
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
            <div style={{ borderTop: '1px solid var(--border-strong)', margin: '4px 0' }} />
            <MenuItem
              label="3D Text…"
              disabled={editGates?.canDrawText === false}
              onClick={withClose(() => onDrawText?.())}
            />
          </div>
        )}
      </div>

      {/* Tools menu */}
      <div style={{ position: 'relative', height: '100%' }}>
        <MenuTrigger id="tools" label="Tools" openMenu={openMenu} onToggle={toggle} onActivate={activate} />
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
              label="Position Texture"
              checked={activeTool === 'Position Texture'}
              onClick={withClose(() => onSelectTool?.('Position Texture'))}
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
            <CheckMenuItem
              label="Follow Me"
              checked={activeTool === 'Follow Me'}
              onClick={withClose(() => onSelectTool?.('Follow Me'))}
            />
            <CheckMenuItem
              label="Offset"
              shortcut={keyFor('Offset')}
              checked={activeTool === 'Offset'}
              onClick={withClose(() => onSelectTool?.('Offset'))}
            />
            <div style={SEPARATOR_STYLE} />
            <CheckMenuItem
              label="Tape Measure"
              shortcut={keyFor('Tape Measure')}
              checked={activeTool === 'Tape Measure'}
              onClick={withClose(() => onSelectTool?.('Tape Measure'))}
            />
            <CheckMenuItem
              label="Dimension"
              checked={activeTool === 'Dimension'}
              onClick={withClose(() => onSelectTool?.('Dimension'))}
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
              label="Section Plane"
              checked={activeTool === 'Section Plane'}
              onClick={withClose(() => onSelectTool?.('Section Plane'))}
            />
            <CheckMenuItem
              label="Edit Vertex"
              checked={activeTool === 'Edit Vertex'}
              onClick={withClose(() => onSelectTool?.('Edit Vertex'))}
            />
            <CheckMenuItem
              label="Axes"
              checked={activeTool === 'Axes'}
              onClick={withClose(() => onSelectTool?.('Axes'))}
            />
            <CheckMenuItem
              label="Text"
              checked={activeTool === 'Text'}
              onClick={withClose(() => onSelectTool?.('Text'))}
            />
          </div>
        )}
      </div>

      {/* Camera menu */}
      <div style={{ position: 'relative', height: '100%' }}>
        <MenuTrigger id="camera" label="Camera" openMenu={openMenu} onToggle={toggle} onActivate={activate} />
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
            {/* Position Camera / Walk / Look Around (docs/design/camera.md
                §4): the first REAL camera tools — Orbit/Pan/Zoom above are
                just OrbitControls mouse-button remaps. No default
                bare-letter shortcut in v1 (SketchUp ships none for these
                either), same as Zoom Window below. */}
            <CheckMenuItem
              label="Position Camera"
              checked={activeTool === 'Position Camera'}
              onClick={withClose(() => onSelectTool?.('Position Camera'))}
            />
            <CheckMenuItem
              label="Walk"
              checked={activeTool === 'Walk'}
              onClick={withClose(() => onSelectTool?.('Walk'))}
            />
            <CheckMenuItem
              label="Look Around"
              checked={activeTool === 'Look Around'}
              onClick={withClose(() => onSelectTool?.('Look Around'))}
            />
            <div style={SEPARATOR_STYLE} />
            {/* No standalone Field of View entry (camera-playtest2.md §2 —
                Kurt's playtest call): fov is reachable only through Zoom
                (typed-degree/mm entry, or Shift-drag/wheel — camera.md §2,
                camera-playtest2.md §3), never a separate menu item. */}
            <MenuItem
              label="Zoom Window"
              onClick={withClose(() => onSelectTool?.('Zoom Window'))}
            />
            <MenuItem
              label="Zoom Extents"
              onClick={withClose(() => onZoomExtents?.())}
            />
            <div style={SEPARATOR_STYLE} />
            <CheckMenuItem
              label="Parallel Projection"
              checked={parallelProjectionChecked}
              onClick={withClose(() => onToggleParallelProjection?.())}
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
        <MenuTrigger id="window" label="Window" openMenu={openMenu} onToggle={toggle} onActivate={activate} />
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
              label="Library"
              shortcut={`⇧${mod}L`}
              checked={showLibrary}
              onClick={withClose(() => onToggleLibrary?.())}
            />
            {onEnterShopMode !== undefined && (
              <>
                <div style={SEPARATOR_STYLE} />
                <MenuItem label="Shop Mode" onClick={withClose(() => onEnterShopMode())} />
              </>
            )}
            {windowList !== undefined && windowList.length > 0 && (
              <>
                <div style={SEPARATOR_STYLE} />
                {windowList.map((w) => (
                  <CheckMenuItem
                    key={w.label}
                    label={windowMenuLabel(w.title)}
                    checked={w.focused}
                    onClick={withClose(() => onFocusWindow?.(w.label))}
                  />
                ))}
              </>
            )}
          </div>
        )}
      </div>

      {/* Help menu */}
      <div style={{ position: 'relative', height: '100%' }}>
        <MenuTrigger id="help" label="Help" openMenu={openMenu} onToggle={toggle} onActivate={activate} />
        {openMenu === 'help' && (
          <div style={DROPDOWN_STYLE}>
            <CheckMenuItem
              label="Debug Log"
              checked={showDebugLog}
              onClick={withClose(() => onToggleDebugLog?.())}
            />
            <div style={SEPARATOR_STYLE} />
            <MenuItem
              label="Report Bug…"
              onClick={withClose(() => onReportBug?.())}
            />
            {onCheckForUpdates !== undefined && (
              <>
                <div style={SEPARATOR_STYLE} />
                <MenuItem
                  label="Check for Updates…"
                  onClick={withClose(() => onCheckForUpdates())}
                />
              </>
            )}
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

      {/* Spacer — dead space, so clicking it also leaves menu mode (it has
          zero height today, letting clicks fall through to the bar's own
          handler, but a future height would swallow them). (The resting
          command-palette field lived here on Windows/Linux/Web until moved
          to the top of the tool rail on every platform — see ToolRail.tsx.) */}
      <div style={{ flex: 1 }} onMouseDown={close} />

      {/* Settings gear on the trailing edge — the modern menu-bar convention
          for reaching Settings. Same target as Window ▸ Settings… */}
      {onOpenSettings !== undefined && (
        <GearButton onClick={onOpenSettings} shortcutHint={`${mod},`} />
      )}
    </div>
  )
}
