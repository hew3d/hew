/**
 * MenuBar — the in-app menu bar (File / Edit / Object / View / Camera /
 * Draw / Tools / Window / Help), used everywhere the OS menu bar isn't:
 * the web build and the Windows/Linux desktop shells. Menu structure
 * mirrors the native macOS menu built in shells/tauri/src-tauri/src/main.rs
 * — change them together.
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
import { Trans, useLingui } from '@lingui/react/macro'
import { modLabel, isMac } from '../platform'
import { shortcutFor, toolLabel, type ToolName } from '../tools/toolRegistry'
import type { StandardView } from '../viewport/Viewport'
import { MENU } from '../health/healthCopy'
import { isComposingKey } from '../input/keyboard'
import { appVersionString, isBetaBuild } from '../buildInfo'

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
  /** File ▸ Revert to Saved — after one confirmation, re-reads the saved
   *  file and throws away every change since the last save. Omitted by a
   *  host with no saved file to return to. */
  onRevertToSaved?: () => void
  /** Disables Revert to Saved: no saved file, or no changes since it. */
  revertToSavedDisabled?: boolean
  /** Import a model file (COLLADA / SketchUp / glTF — chosen in the file dialog). */
  onImport: () => void
  /** Place a picture (PNG/JPEG) in the current document — Place Image.
   *  Optional so hosts without it (Shop Mode) leave the item out. */
  onImportImage?: () => void
  /** Open the unified Export dialog (format — glTF/GLB or STL — chosen there). */
  onExport: () => void
  /** File ▸ Print… — the Print dialog (paper or PDF, standard or to scale;
   *  docs/design/printing.md). ⌘P / Ctrl+P. */
  onPrint: () => void
  /** File ▸ Save to Library… — prompts for a name, then saves the whole
   *  document as a library item. Omitted/no-op-guarded the same way the
   *  other optional handlers are; `saveToLibraryDisabled` greys the item out
   *  on a platform with no library backend (the web build). */
  onSaveToLibrary?: () => void
  /** True on a platform with no library backend (web) — disables File ▸ Save
   *  to Library… rather than hiding it, matching Import…'s session-gating
   *  posture. */
  saveToLibraryDisabled?: boolean
  /** File ▸ Purge Unused… — deletes every unused palette material and
   *  component definition, previewed and confirmed in a modal (v1.1 assets
   *  lane). Always enabled: an idle purge is a documented no-op toast, not
   *  a disabled state to compute. */
  onPurgeUnused?: () => void
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
  /** "Undo Push/Pull" / "Redo Move" — the top undo/redo entry's label
   * (Lane C, docs/design/v1.1-cycle.md). Defaults to the bare verb. */
  undoLabel?: string
  redoLabel?: string
  /** The currently active tool name. */
  activeTool?: string
  /** Called when the user picks a tool from the menu. */
  onSelectTool?: (name: string) => void
  /** Whether the Model info pane is visible. */
  showModelInfo?: boolean
  /** Whether the Materials pane is visible. */
  showMaterials?: boolean
  /** Whether the Components pane is visible. */
  showComponents?: boolean
  /** Whether the Tags pane is visible. */
  showTags?: boolean
  /** Whether the Scenes pane is visible (docs/design/scenes.md §5). */
  showScenes?: boolean
  /** Whether the Changes pane is visible (Lane C, docs/design/v1.1-cycle.md). */
  showChanges?: boolean
  /** Whether the Object Info pane is visible. */
  showObjectInfo?: boolean
  /** Whether the right-hand tray is shown (View ▸ Tray). */
  showTray?: boolean
  /** Show or hide the whole tray. */
  onToggleTray?: () => void
  /** Whether the tool rail is compacted to icons (View ▸ Compact Tool Rail). */
  compactRail?: boolean
  /** Compact the tool rail to icons, or bring its names back. */
  onToggleCompactRail?: () => void
  /** Whether the contextual dock is shown (View ▸ Contextual Dock). */
  showDock?: boolean
  /** Turn the contextual dock off or back on. */
  onToggleDock?: () => void
  /** Whether the Debug Log panel is visible. */
  showDebugLog?: boolean
  /** Whether the Library browser dialog is open. */
  showLibrary?: boolean
  /** Toggle the Model info pane. */
  onToggleModelInfo?: () => void
  /** Toggle the Materials pane. */
  onToggleMaterials?: () => void
  /** Toggle the Components pane. */
  onToggleComponents?: () => void
  /** Toggle the Tags pane. */
  onToggleTags?: () => void
  /** Toggle the Scenes pane. */
  onToggleScenes?: () => void
  /** Toggle the Changes pane. */
  onToggleChanges?: () => void
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
  /** Whether X-ray is on (View ▸ X-ray). */
  xray?: boolean
  /** Whether Wireframe is on (View ▸ Wireframe). */
  wireframe?: boolean
  onToggleXray?: () => void
  onToggleWireframe?: () => void
  /** Toggle the world axes. */
  onToggleAxes?: () => void
  /** Reset the movable drawing axes to world identity (View ▸ Reset Drawing Axes). */
  onResetAxes?: () => void
  /** Toggle the ground grid. */
  onToggleGrid?: () => void
  /** Toggle construction-guide visibility. */
  onToggleGuides?: () => void
  /** Delete every construction guide (Edit ▸ Delete Guide Lines). */
  onDeleteGuides?: () => void
  /** View ▸ Section Cut's check state: true only when a section is BOTH
   * placed and active — SketchUp's "Active Cut" — derived by the caller
   * from the section manager's own state (`getSectionState`), never a
   * locally-tracked shadow boolean. False (unchecked, and the item
   * disabled — see `sectionPlaneExists`) when no section is placed. */
  sectionPlaneChecked?: boolean
  /** Whether a section is placed at all, regardless of active/inactive —
   * gates the View ▸ Section Cut item's enabled state (nothing to toggle
   * with none placed). */
  sectionPlaneExists?: boolean
  /** Toggle the placed section plane's active (clipping) flag — View ▸
   * Section Plane, SketchUp's "Active Cut". Same `toggle-section-active`
   * dispatch this command has always used; only its menu location moved
   * (from Tools) in section-plane-polish. */
  onToggleSectionActive?: () => void
  /** Delete the current selection — whole Object/Group/Instance nodes only (Edit ▸ Delete). */
  onDelete?: () => void
  /** Run a menu command by action id — the Object menu's object commands
   *  (edit-group, edit-union, …; the `edit-` prefix is their historical wire
   *  id) plus Edit ▸ Select All — same dispatch ids the native macOS menu
   *  and the contextual dock use. */
  onEditAction?: (id: string) => void
  /** Selection-gated availability for the Object-menu commands. */
  editGates?: {
    canGroup: boolean
    canUngroup: boolean
    canMakeComponent: boolean
    canPlaceCopy: boolean
    canExplode: boolean
    canMakeUnique: boolean
    canBoolean: boolean
    /** One object, group, or instance selected, with a library backend —
     *  Object ▸ Save Selection to Library… (also the dock's verb). Defaults
     *  to disabled when omitted. */
    canSaveSelectionToLibrary?: boolean
    /** False while an explode session is open (`ExplodeSessionScope`) —
     *  importing restructures the document, which a session can't fold
     *  back cleanly. Defaults to enabled (`true`) when omitted so callers
     *  that don't yet track session state see the pre-existing behavior. */
    canImport?: boolean
    /** False while an explode session is open, for the same reason as
     *  `canImport` — 3D Text placement is an instance placement. Defaults
     *  to enabled when omitted. */
    canDrawText?: boolean
    /** Cut/Copy (Lane D): at least one structural (object/group/instance)
     *  node is selected. */
    hasStructuralSelection?: boolean
    /** Paste/Paste In Place (Lane D): this window's clipboard has content. */
    clipboardHasContent?: boolean
  }
  /** Zoom the camera to fit all scene geometry (View → Zoom Extents). */
  onZoomExtents?: () => void
  /** View ▸ Scenes ▸ Add Scene (docs/design/scenes.md §5). */
  onScenesAdd?: () => void
  /** View ▸ Scenes ▸ Update Scene — re-captures the active Scene's checked
   *  properties. Greyed out (`sceneUpdateEnabled`) with none active. */
  onScenesUpdate?: () => void
  sceneUpdateEnabled?: boolean
  /** View ▸ Scenes ▸ Next/Previous Scene (Page Down/Up's menu mirror). */
  onScenesNext?: () => void
  onScenesPrevious?: () => void
  /** Greys Next/Previous out when the document has no Scenes at all. */
  scenesNavEnabled?: boolean
  /** View ▸ Scenes ▸ Scene Transitions checkmark — a SPEC.md deviation: the
   *  design put this toggle in Settings; it lives here instead, at a fixed
   *  0.6s (`settings/sceneTransitions.ts`'s own doc comment on why). */
  sceneTransitionsChecked?: boolean
  onToggleSceneTransitions?: () => void
  /** Open the "3D Text…" dialog (Draw → 3D Text…, docs/design/3d-text.md). */
  onDrawText?: () => void
  /** Reposition the camera to a standard view (Camera → Standard Views). */
  onStandardView?: (view: StandardView) => void
  /** Camera ▸ Parallel Projection checkbox state (docs/design/camera.md §1). */
  parallelProjectionChecked?: boolean
  /** Toggle Camera ▸ Parallel Projection. */
  onToggleParallelProjection?: () => void
  /** Camera ▸ Fly check state — the LATCHED fly mode (docs/design/
   *  fly-navigation.md §10), for trackpads; a right-button hold flies
   *  under any tool without it. */
  flyChecked?: boolean
  /** Toggle Camera ▸ Fly. */
  onToggleFly?: () => void
  /** Open document windows (Tauri multi-window only), rendered as a tail of
   *  focus-this-window entries at the end of the Window menu — the current
   *  window's entry is checked. `undefined` on plain web (single window, no
   *  shell to list from) and on macOS Tauri (the native menu renders its own
   *  tail shell-side; this only feeds the in-app bar used on Windows/Linux). */
  windowList?: { label: string; title: string; focused: boolean }[]
  /** Focus (raise) an open document window by its shell-assigned label. */
  onFocusWindow?: (label: string) => void
  /** Open the platform's Settings surface (the gear on the bar's trailing
   *  edge): the Fluent in-app page on Windows, the
   *  separate settings window on Linux, the modal on web. macOS reaches
   *  Settings through its native app menu instead (this bar renders nothing
   *  there). */
  onOpenSettings?: () => void
  /** Assemble and write a "Report Bug" bundle (Help → Report Bug…). */
  onReportBug?: () => void
  /** Open Help → Sent Reports… (docs/design/follow-a-report.md §2.2). */
  onSentReports?: () => void
  /** Unread replies across followed reports, for the "(n new)" suffix. */
  sentReportsNew?: number
  /** Trigger a manual update check (Help → Check for Updates…). Omitted — and
   *  the item hidden — on the web build and in package-manager desktop builds
   *  that compile the updater out. */
  onCheckForUpdates?: () => void
  /** Switch to Shop Mode (View → Shop Mode): the fullscreen touch-first
   *  viewer/inspector shell (`shop/ShopApp.tsx`) — the mirror of ShopApp's
   *  own "Use full editor" entry. Omitted — and the item hidden — under
   *  Tauri (Shop Mode is web-only) and on any device that doesn't report a
   *  coarse pointer (`platform.isCoarsePointer()`): a touch-capable laptop
   *  with a mouse-shaped desktop session has no workbench use case for it.
   *  Same optional-prop-gates-visibility convention as `onCheckForUpdates`
   *  above. */
  onEnterShopMode?: () => void
  /** Open the online user guide (Help → Hew Help). */
  onOpenGuide?: () => void
  /** Open the command palette (View → Command Palette…) — the same surface
   *  `Cmd/Ctrl+/` opens; listed in the menu so it stays discoverable, matching
   *  the native macOS View menu. */
  onOpenPalette?: () => void
  /** Help → Search: hand the typed text to the command palette (the app's
   *  searchable index of every command). The native macOS Help menu gets the
   *  system search field instead (set_as_help_menu_for_nsapp, main.rs). */
  onHelpSearch?: (query: string) => void
}

type MenuId = 'file' | 'edit' | 'object' | 'view' | 'draw' | 'tools' | 'camera' | 'window' | 'help' | null

/* oxlint-disable lingui/no-unlocalized-strings -- tool IDs, never displayed: labels resolve through toolLabel() */
const T = {
  Arc: 'Arc' as ToolName,
  Circle: 'Circle' as ToolName,
  Dimension: 'Dimension' as ToolName,
  DrawingAxes: 'Drawing Axes' as ToolName,
  EditVertex: 'Edit Vertex' as ToolName,
  FollowMe: 'Follow Me' as ToolName,
  Line: 'Line' as ToolName,
  LookAround: 'Look Around' as ToolName,
  Move: 'Move' as ToolName,
  Offset: 'Offset' as ToolName,
  Orbit: 'Orbit' as ToolName,
  Paint: 'Paint' as ToolName,
  Pan: 'Pan' as ToolName,
  Polygon: 'Polygon' as ToolName,
  PositionCamera: 'Position Camera' as ToolName,
  PositionTexture: 'Position Texture' as ToolName,
  Protractor: 'Protractor' as ToolName,
  PushPull: 'Push/Pull' as ToolName,
  Rectangle: 'Rectangle' as ToolName,
  Rotate: 'Rotate' as ToolName,
  Scale: 'Scale' as ToolName,
  SectionPlane: 'Section Plane' as ToolName,
  Select: 'Select' as ToolName,
  Slice: 'Slice' as ToolName,
  TapeMeasure: 'Tape Measure' as ToolName,
  Text: 'Text' as ToolName,
  Walk: 'Walk' as ToolName,
  Zoom: 'Zoom' as ToolName,
  ZoomWindow: 'Zoom Window' as ToolName,
}
/* oxlint-enable lingui/no-unlocalized-strings */

/* oxlint-disable lingui/no-unlocalized-strings -- keyboard shortcut glyphs, not translated (docs/LOCALIZATION.md) */
const scModA = `${modLabel}A`
const scModC = `${modLabel}C`
const scModG = `${modLabel}G`
const scModN = `${modLabel}N`
const scModO = `${modLabel}O`
const scModP = `${modLabel}P`
const scModS = `${modLabel}S`
const scModShiftG = `${modLabel}⇧G`
const scModShiftS = `${modLabel}⇧S`
const scModShiftZ = `${modLabel}⇧Z`
const scModSlash = `${modLabel}/`
const scModV = `${modLabel}V`
const scModW = `${modLabel}W`
const scModX = `${modLabel}X`
const scModZ = `${modLabel}Z`
const scPageDown = 'Page Down'
const scPageUp = 'Page Up'
const scShiftModA = `⇧${modLabel}A`
const scShiftModC = `⇧${modLabel}C`
const scShiftModI = `⇧${modLabel}I`
const scShiftModL = `⇧${modLabel}L`
const scShiftModM = `⇧${modLabel}M`
const scShiftModO = `⇧${modLabel}O`
const scShiftModT = `⇧${modLabel}T`
const scShiftModV = `⇧${modLabel}V`
/* oxlint-enable lingui/no-unlocalized-strings */

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
  return title.replace(/ — Hew$/, '')
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

const BETA_CHIP_STYLE: React.CSSProperties = {
  padding: '1px 6px',
  marginRight: '8px',
  fontSize: '10px',
  fontWeight: 600,
  letterSpacing: '0.08em',
  lineHeight: '14px',
  color: 'var(--text-tertiary, #ccc)',
  border: '1px solid var(--border-hairline, #3a3a3a)',
  borderRadius: '3px',
  cursor: 'default',
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
  // Inline start of the trigger, so a dropdown opens toward the reading
  // direction: left-aligned under its trigger in English, right-aligned in
  // Arabic.
  insetInlineStart: 0,
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
  // Native menus never wrap a row; the dropdown widens instead (its
  // absolutely-positioned box otherwise clamps to DROPDOWN_STYLE's minWidth
  // and long rows like "Command Palette…" fold onto two lines).
  whiteSpace: 'nowrap',
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
  // The keys are physical and are read in the order they are pressed, so the
  // hint stays left-to-right inside a mirrored menu — and isolated, so an
  // Arabic menu label beside it cannot drag the chord's punctuation across
  // the boundary (index.css `.hew-ltr` holds the same rules).
  direction: 'ltr',
  unicodeBidi: 'isolate',
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
  const { t } = useLingui()
  const [hovered, setHovered] = useState(false)
  return (
    <button
      aria-label={t`Settings`}
      title={t`Settings (${shortcutHint})`}
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
      {/* Empty check gutter, same width as CheckMenuItem's — every row is
          inset so labels align down the menu's left edge, whether or not
          the row can carry a checkmark. */}
      <span style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
        <span style={{ width: '12px' }} aria-hidden="true" />
        {label}
      </span>
      {shortcut !== undefined && <span style={SHORTCUT_STYLE}>{shortcut}</span>}
    </div>
  )
}

/** Help ▸ Search input — macOS-style: the first COMMITTED keystroke hands
 * the text to the command palette (the app's searchable command index), so
 * this stays one search, not two. Local state + composition guard: during
 * IME composition nothing fires; the composed text hands off on
 * compositionend. */
function HelpSearchField({ onSearch }: { onSearch: (query: string) => void }) {
  const { t } = useLingui()
  const [text, setText] = useState('')
  const composing = useRef(false)
  const handOff = (q: string) => {
    if (q !== '') onSearch(q)
  }
  return (
    <div style={{ padding: '2px 12px 6px' }}>
      <input
        aria-label={t`Search commands and help`}
        placeholder={t`Search`}
        autoFocus
        value={text}
        onCompositionStart={() => {
          composing.current = true
        }}
        onCompositionEnd={(e) => {
          composing.current = false
          handOff((e.target as HTMLInputElement).value)
        }}
        onChange={(e) => {
          setText(e.target.value)
          if (!composing.current) handOff(e.target.value)
        }}
        style={{
          width: '100%',
          boxSizing: 'border-box',
          padding: '3px 8px',
          fontSize: 'var(--font-size-menu-item, 13px)',
          fontFamily: 'var(--font-family-ui)',
          color: 'var(--text-secondary, #ddd)',
          background: 'var(--surface-bar, #1e1e1e)',
          border: '1px solid var(--border-strong, #4a4a4a)',
          borderRadius: 'var(--radius-control, 4px)',
          outline: 'none',
        }}
      />
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
        {/* Same empty check gutter as MenuItem — see its comment. */}
        <span style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
          <span style={{ width: '12px' }} aria-hidden="true" />
          {label}
        </span>
        {/* The flyout arrow points at the edge its submenu opens toward, so
            it mirrors with the menu (index.css `.hew-flip-icon`). */}
        <span className="hew-flip-icon" style={SHORTCUT_STYLE}>▸</span>
      </div>
      {hovered && (
        <div style={{ ...DROPDOWN_STYLE, top: -4, insetInlineStart: '100%', borderRadius: '4px' }}>
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
   * clicks. Used by View ▸ Section Cut while no section is placed
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
  onRevertToSaved,
  revertToSavedDisabled = false,
  onImport,
  onImportImage,
  onExport,
  onPrint,
  onSaveToLibrary,
  saveToLibraryDisabled = false,
  onPurgeUnused,
  onClose,
  onExit,
  recentFiles,
  onOpenRecent,
  onClearRecent,
  onUndo,
  onRedo,
  canUndo,
  canRedo,
  undoLabel,
  redoLabel,
  activeTool,
  onSelectTool,
  showModelInfo = true,
  showMaterials = true,
  showComponents = false,
  showTags = false,
  showScenes = false,
  showChanges = false,
  showObjectInfo = false,
  showTray = true,
  onToggleTray,
  compactRail = false,
  onToggleCompactRail,
  showDock = true,
  onToggleDock,
  showDebugLog = false,
  showLibrary = false,
  onToggleModelInfo,
  onToggleMaterials,
  onToggleComponents,
  onToggleTags,
  onToggleScenes,
  onToggleChanges,
  onToggleObjectInfo,
  onToggleDebugLog,
  onToggleLibrary,
  showAxes = true,
  showGrid = true,
  showGuides = true,
  onToggleAxes,
  onResetAxes,
  onToggleGrid,
  xray = false,
  wireframe = false,
  onToggleXray,
  onToggleWireframe,
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
  onScenesAdd,
  onScenesUpdate,
  sceneUpdateEnabled = false,
  onScenesNext,
  onScenesPrevious,
  scenesNavEnabled = false,
  sceneTransitionsChecked = true,
  onToggleSceneTransitions,
  onStandardView,
  parallelProjectionChecked = false,
  onToggleParallelProjection,
  flyChecked,
  onToggleFly,
  onOpenSettings,
  onReportBug,
  onSentReports,
  sentReportsNew = 0,
  onCheckForUpdates,
  onOpenOnPhone,
  openOnPhoneDisabled = false,
  onEnterShopMode,
  onOpenGuide,
  onOpenPalette,
  onHelpSearch,
  windowList,
  onFocusWindow,
}: MenuBarProps) {
  const { t } = useLingui()
  const betaVersion = appVersionString()
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
      if (isComposingKey(e)) return
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
        <MenuTrigger id="file" label={t`File`} openMenu={openMenu} onToggle={toggle} onActivate={activate} />
        {openMenu === 'file' && (
          <div style={DROPDOWN_STYLE}>
            <MenuItem label={t`New`} shortcut={scModN} onClick={withClose(onNew)} />
            <MenuItem label={t`Open…`} shortcut={scModO} onClick={withClose(onOpen)} />
            {recentFiles !== undefined && (
              <SubMenu label={t`Open Recent`}>
                {recentFiles.length === 0 ? (
                  <MenuItem label={t`No Recent Files`} disabled onClick={() => {}} />
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
                    <MenuItem label={t`Clear Recent`} onClick={withClose(() => onClearRecent?.())} />
                  </>
                )}
              </SubMenu>
            )}
            <div style={SEPARATOR_STYLE} />
            {/* HIG File order (mirrors the native macOS menu): Close + the
                save block, then interchange (Import/Export), then sharing,
                then Print… — with Exit still the absolute last item on
                Windows/Linux. */}
            {onClose !== undefined && (
              <MenuItem label={t`Close`} shortcut={scModW} onClick={withClose(onClose)} />
            )}
            <MenuItem label={t`Save`} shortcut={scModS} onClick={withClose(onSave)} />
            <MenuItem label={t`Save As…`} shortcut={scModShiftS} onClick={withClose(onSaveAs)} />
            {onRevertToSaved !== undefined && (
              <MenuItem
                label={t`Revert to Saved`}
                disabled={revertToSavedDisabled}
                onClick={withClose(onRevertToSaved)}
              />
            )}
            {onSaveToLibrary !== undefined && (
              <MenuItem
                label={t`Save to Library…`}
                disabled={saveToLibraryDisabled}
                onClick={withClose(onSaveToLibrary)}
              />
            )}
            {onPurgeUnused !== undefined && (
              <>
                <div style={SEPARATOR_STYLE} />
                <MenuItem label={t`Purge Unused…`} onClick={withClose(onPurgeUnused)} />
              </>
            )}
            <div style={SEPARATOR_STYLE} />
            <MenuItem
              label={t`Import…`}
              disabled={editGates?.canImport === false}
              onClick={withClose(onImport)}
            />
            {onImportImage !== undefined && (
              <MenuItem
                label={t`Import Image…`}
                disabled={editGates?.canImport === false}
                onClick={withClose(onImportImage)}
              />
            )}
            <MenuItem label={t`Export…`} onClick={withClose(onExport)} />
            {onOpenOnPhone !== undefined && (
              <>
                <div style={SEPARATOR_STYLE} />
                <MenuItem
                  label={t`Open on Phone…`}
                  disabled={openOnPhoneDisabled}
                  onClick={withClose(onOpenOnPhone)}
                />
              </>
            )}
            <div style={SEPARATOR_STYLE} />
            <MenuItem label={t`Print…`} shortcut={scModP} onClick={withClose(onPrint)} />
            {onExit !== undefined && (
              <>
                <div style={SEPARATOR_STYLE} />
                <MenuItem label={t`Exit`} onClick={withClose(onExit)} />
              </>
            )}
          </div>
        )}
      </div>

      {/* Edit menu */}
      <div style={{ position: 'relative', height: '100%' }}>
        <MenuTrigger id="edit" label={t`Edit`} openMenu={openMenu} onToggle={toggle} onActivate={activate} />
        {openMenu === 'edit' && (
          <div style={DROPDOWN_STYLE}>
            <MenuItem
              label={canUndo && undoLabel !== undefined ? undoLabel : t`Undo`}
              shortcut={scModZ}
              disabled={!canUndo}
              onClick={withClose(onUndo)}
            />
            <MenuItem
              label={canRedo && redoLabel !== undefined ? redoLabel : t`Redo`}
              shortcut={scModShiftZ}
              disabled={!canRedo}
              onClick={withClose(onRedo)}
            />
            <div style={SEPARATOR_STYLE} />
            <MenuItem
              label={t`Cut`}
              shortcut={scModX}
              disabled={!(editGates?.hasStructuralSelection ?? false)}
              onClick={withClose(() => onEditAction?.('edit-cut'))}
            />
            <MenuItem
              label={t`Copy`}
              shortcut={scModC}
              disabled={!(editGates?.hasStructuralSelection ?? false)}
              onClick={withClose(() => onEditAction?.('edit-copy'))}
            />
            <MenuItem
              label={t`Paste`}
              shortcut={scModV}
              disabled={!(editGates?.clipboardHasContent ?? false)}
              onClick={withClose(() => onEditAction?.('edit-paste'))}
            />
            <MenuItem
              label={t`Paste In Place`}
              shortcut={scShiftModV}
              disabled={!(editGates?.clipboardHasContent ?? false)}
              onClick={withClose(() => onEditAction?.('edit-paste-in-place'))}
            />
            <div style={SEPARATOR_STYLE} />
            <MenuItem
              label={t`Delete`}
              shortcut="⌫"
              onClick={withClose(() => onDelete?.())}
            />
            <MenuItem
              label={t`Select All`}
              shortcut={scModA}
              onClick={withClose(() => onEditAction?.('edit-select-all'))}
            />
            <MenuItem
              label={t`Select None`}
              shortcut={scShiftModA}
              onClick={withClose(() => onEditAction?.('edit-select-none'))}
            />
            <MenuItem
              label={t`Invert Selection`}
              onClick={withClose(() => onEditAction?.('edit-invert-selection'))}
            />
            <div style={SEPARATOR_STYLE} />
            <MenuItem
              label={t`Delete Guide Lines`}
              onClick={withClose(() => onDeleteGuides?.())}
            />
          </div>
        )}
      </div>

      {/* Object menu — commands acting on the selected Objects/Groups/
          Components (containers, component lifecycle, booleans). Split out
          of Edit so the HIG-standard Edit menu stays small and future object
          commands have an obvious home; action ids keep their historical
          `edit-` prefix (wire ids, not user-visible text). Mirrors the
          native macOS Object menu. */}
      <div style={{ position: 'relative', height: '100%' }}>
        <MenuTrigger id="object" label={t`Object`} openMenu={openMenu} onToggle={toggle} onActivate={activate} />
        {openMenu === 'object' && (
          <div style={DROPDOWN_STYLE}>
            <MenuItem
              label={t`Group`}
              shortcut={scModG}
              disabled={!(editGates?.canGroup ?? false)}
              onClick={withClose(() => onEditAction?.('edit-group'))}
            />
            <MenuItem
              label={t`Ungroup`}
              shortcut={scModShiftG}
              disabled={!(editGates?.canUngroup ?? false)}
              onClick={withClose(() => onEditAction?.('edit-ungroup'))}
            />
            <div style={SEPARATOR_STYLE} />
            <MenuItem
              label={t`Make Component`}
              disabled={!(editGates?.canMakeComponent ?? false)}
              onClick={withClose(() => onEditAction?.('edit-make-component'))}
            />
            <MenuItem
              label={t`Make Unique`}
              disabled={!(editGates?.canMakeUnique ?? false)}
              onClick={withClose(() => onEditAction?.('edit-make-unique'))}
            />
            <MenuItem
              label={t`Place Copy`}
              disabled={!(editGates?.canPlaceCopy ?? false)}
              onClick={withClose(() => onEditAction?.('edit-place-copy'))}
            />
            <MenuItem
              label={t`Explode`}
              disabled={!(editGates?.canExplode ?? false)}
              onClick={withClose(() => onEditAction?.('edit-explode'))}
            />
            <MenuItem
              label={t`Save Selection to Library…`}
              disabled={!(editGates?.canSaveSelectionToLibrary ?? false)}
              onClick={withClose(() => onEditAction?.('save-to-library'))}
            />
            <div style={SEPARATOR_STYLE} />
            <MenuItem
              label={t`Union`}
              disabled={!(editGates?.canBoolean ?? false)}
              onClick={withClose(() => onEditAction?.('edit-union'))}
            />
            <MenuItem
              label={t`Subtract`}
              disabled={!(editGates?.canBoolean ?? false)}
              onClick={withClose(() => onEditAction?.('edit-subtract'))}
            />
            <MenuItem
              label={t`Intersect`}
              disabled={!(editGates?.canBoolean ?? false)}
              onClick={withClose(() => onEditAction?.('edit-intersect'))}
            />
            <div style={SEPARATOR_STYLE} />
            <MenuItem
              label={MENU.makeSolid}
              onClick={withClose(() => onEditAction?.('make-solid'))}
            />
            <MenuItem
              label={MENU.modelHealthItem}
              onClick={withClose(() => onEditAction?.('model-health'))}
            />
          </div>
        )}
      </div>

      {/* View menu */}
      <div style={{ position: 'relative', height: '100%' }}>
        <MenuTrigger id="view" label={t`View`} openMenu={openMenu} onToggle={toggle} onActivate={activate} />
        {openMenu === 'view' && (
          <div style={DROPDOWN_STYLE}>
            <CheckMenuItem
              label={t`Axes`}
              checked={showAxes}
              onClick={withClose(() => onToggleAxes?.())}
            />
            <MenuItem
              label={t`Reset Drawing Axes`}
              onClick={withClose(() => onResetAxes?.())}
            />
            <CheckMenuItem
              label={t`Grid`}
              checked={showGrid}
              onClick={withClose(() => onToggleGrid?.())}
            />
            <CheckMenuItem
              label={t`Guides`}
              checked={showGuides}
              onClick={withClose(() => onToggleGuides?.())}
            />
            {/* "Section Cut", not "Section Plane": the TOOL that places the
                plane keeps that name under Tools; this toggle clips/unclips
                the placed cut — SketchUp's own View ▸ Section Cuts vs
                Tools ▸ Section Plane naming split. */}
            <CheckMenuItem
              label={t`Section Cut`}
              checked={sectionPlaneChecked}
              disabled={!sectionPlaneExists}
              onClick={withClose(() => onToggleSectionActive?.())}
            />
            <CheckMenuItem
              label={t`X-ray`}
              checked={xray}
              onClick={withClose(() => onToggleXray?.())}
            />
            <CheckMenuItem
              label={t`Wireframe`}
              checked={wireframe}
              onClick={withClose(() => onToggleWireframe?.())}
            />
            <div style={SEPARATOR_STYLE} />
            {/* Panel toggles — HIG puts show/hide of interface panels in the
                View menu (Window holds actual windows). The Scenes panel
                lives inside the Scenes submenu below, keeping that whole
                feature in one place. The window's chrome comes first — the
                tool rail, the tray that frames the sections below, and the
                contextual dock. None takes an accelerator: they're
                set-and-forget layout choices, and SketchUp reserves no key
                for its trays either. */}
            <CheckMenuItem
              label={t`Compact Tool Rail`}
              checked={compactRail}
              onClick={withClose(() => onToggleCompactRail?.())}
            />
            <CheckMenuItem
              label={t`Tray`}
              checked={showTray}
              onClick={withClose(() => onToggleTray?.())}
            />
            <CheckMenuItem
              label={t`Contextual Dock`}
              checked={showDock}
              onClick={withClose(() => onToggleDock?.())}
            />
            <div style={SEPARATOR_STYLE} />
            <CheckMenuItem
              label={t`Model Info`}
              shortcut={scShiftModI}
              checked={showModelInfo}
              onClick={withClose(() => onToggleModelInfo?.())}
            />
            <CheckMenuItem
              label={t`Object Info`}
              shortcut={scShiftModO}
              checked={showObjectInfo}
              onClick={withClose(() => onToggleObjectInfo?.())}
            />
            <CheckMenuItem
              label={t`Materials`}
              shortcut={scShiftModC}
              checked={showMaterials}
              onClick={withClose(() => onToggleMaterials?.())}
            />
            <CheckMenuItem
              label={t`Components`}
              shortcut={scShiftModM}
              checked={showComponents}
              onClick={withClose(() => onToggleComponents?.())}
            />
            <CheckMenuItem
              label={t`Tags`}
              shortcut={scShiftModT}
              checked={showTags}
              onClick={withClose(() => onToggleTags?.())}
            />
            <div style={SEPARATOR_STYLE} />
            <SubMenu label={t`Scenes`}>
              <CheckMenuItem
                label={t`Scenes Panel`}
                checked={showScenes}
                onClick={withClose(() => onToggleScenes?.())}
              />
              <div style={SEPARATOR_STYLE} />
              <MenuItem label={t`Add Scene`} onClick={withClose(() => onScenesAdd?.())} />
              <MenuItem
                label={t`Update Scene`}
                disabled={!sceneUpdateEnabled}
                onClick={withClose(() => onScenesUpdate?.())}
              />
              <div style={SEPARATOR_STYLE} />
              <MenuItem
                label={t`Next Scene`}
                shortcut={scPageDown}
                disabled={!scenesNavEnabled}
                onClick={withClose(() => onScenesNext?.())}
              />
              <MenuItem
                label={t`Previous Scene`}
                shortcut={scPageUp}
                disabled={!scenesNavEnabled}
                onClick={withClose(() => onScenesPrevious?.())}
              />
              <div style={SEPARATOR_STYLE} />
              <CheckMenuItem
                label={t`Scene Transitions`}
                checked={sceneTransitionsChecked}
                onClick={withClose(() => onToggleSceneTransitions?.())}
              />
            </SubMenu>
            <CheckMenuItem
              label={t`Changes`}
              checked={showChanges}
              onClick={withClose(() => onToggleChanges?.())}
            />
            {(onOpenPalette !== undefined || onEnterShopMode !== undefined) && (
              <div style={SEPARATOR_STYLE} />
            )}
            {onOpenPalette !== undefined && (
              <MenuItem
                label={t`Command Palette…`}
                shortcut={scModSlash}
                onClick={withClose(onOpenPalette)}
              />
            )}
            {onEnterShopMode !== undefined && (
              <MenuItem label={t`Shop Mode`} onClick={withClose(() => onEnterShopMode())} />
            )}
          </div>
        )}
      </div>

      {/* Camera menu */}
      <div style={{ position: 'relative', height: '100%' }}>
        <MenuTrigger id="camera" label={t`Camera`} openMenu={openMenu} onToggle={toggle} onActivate={activate} />
        {openMenu === 'camera' && (
          <div style={DROPDOWN_STYLE}>
            <CheckMenuItem
              label={toolLabel(T.Orbit)}
              shortcut={keyFor(T.Orbit)}
              checked={activeTool === T.Orbit}
              onClick={withClose(() => onSelectTool?.(T.Orbit))}
            />
            <CheckMenuItem
              label={toolLabel(T.Pan)}
              shortcut={keyFor(T.Pan)}
              checked={activeTool === T.Pan}
              onClick={withClose(() => onSelectTool?.(T.Pan))}
            />
            <CheckMenuItem
              label={toolLabel(T.Zoom)}
              shortcut={keyFor(T.Zoom)}
              checked={activeTool === T.Zoom}
              onClick={withClose(() => onSelectTool?.(T.Zoom))}
            />
            <div style={SEPARATOR_STYLE} />
            {/* Position Camera / Walk / Look Around (docs/design/camera.md
                §4): the first REAL camera tools — Orbit/Pan/Zoom above are
                just OrbitControls mouse-button remaps. No default
                bare-letter shortcut in v1 (SketchUp ships none for these
                either), same as Zoom Window below. */}
            <CheckMenuItem
              label={toolLabel(T.PositionCamera)}
              checked={activeTool === T.PositionCamera}
              onClick={withClose(() => onSelectTool?.(T.PositionCamera))}
            />
            <CheckMenuItem
              label={toolLabel(T.Walk)}
              checked={activeTool === T.Walk}
              onClick={withClose(() => onSelectTool?.(T.Walk))}
            />
            <CheckMenuItem
              label={toolLabel(T.LookAround)}
              checked={activeTool === T.LookAround}
              onClick={withClose(() => onSelectTool?.(T.LookAround))}
            />
            {/* Fly (docs/design/fly-navigation.md): hold the right mouse
                button to fly under any tool; this entry LATCHES the same
                flight on for trackpads (§10) — a check, not a tool. */}
            <CheckMenuItem
              label={t`Fly`}
              checked={flyChecked ?? false}
              onClick={withClose(() => onToggleFly?.())}
            />
            <div style={SEPARATOR_STYLE} />
            {/* No standalone Field of View entry (camera-playtest2.md §2 —
                Kurt's playtest call): fov is reachable only through Zoom
                (typed-degree/mm entry, or Shift-drag/wheel — camera.md §2,
                camera-playtest2.md §3), never a separate menu item. */}
            <MenuItem
              label={toolLabel(T.ZoomWindow)}
              onClick={withClose(() => onSelectTool?.(T.ZoomWindow))}
            />
            <MenuItem
              label={t`Zoom Extents`}
              onClick={withClose(() => onZoomExtents?.())}
            />
            <div style={SEPARATOR_STYLE} />
            <CheckMenuItem
              label={t`Parallel Projection`}
              checked={parallelProjectionChecked}
              onClick={withClose(() => onToggleParallelProjection?.())}
            />
            <div style={SEPARATOR_STYLE} />
            <SubMenu label={t`Standard Views`}>
              <MenuItem label={t`Top`} onClick={withClose(() => onStandardView?.('top'))} />
              <MenuItem label={t`Bottom`} onClick={withClose(() => onStandardView?.('bottom'))} />
              <MenuItem label={t`Front`} onClick={withClose(() => onStandardView?.('front'))} />
              <MenuItem label={t`Back`} onClick={withClose(() => onStandardView?.('back'))} />
              <MenuItem label={t`Left`} onClick={withClose(() => onStandardView?.('left'))} />
              <MenuItem label={t`Right`} onClick={withClose(() => onStandardView?.('right'))} />
              <div style={SEPARATOR_STYLE} />
              <MenuItem label={t`Iso`} onClick={withClose(() => onStandardView?.('iso'))} />
            </SubMenu>
          </div>
        )}
      </div>

      {/* Draw menu */}
      <div style={{ position: 'relative', height: '100%' }}>
        <MenuTrigger id="draw" label={t`Draw`} openMenu={openMenu} onToggle={toggle} onActivate={activate} />
        {openMenu === 'draw' && (
          <div style={DROPDOWN_STYLE}>
            {/* Stroke tools (open paths), then closed shapes — future curve
                tools join the first group, new primitives the second. */}
            <CheckMenuItem
              label={toolLabel(T.Line)}
              shortcut={keyFor(T.Line)}
              checked={activeTool === T.Line}
              onClick={withClose(() => onSelectTool?.(T.Line))}
            />
            <CheckMenuItem
              label={toolLabel(T.Arc)}
              shortcut={keyFor(T.Arc)}
              checked={activeTool === T.Arc}
              onClick={withClose(() => onSelectTool?.(T.Arc))}
            />
            <div style={SEPARATOR_STYLE} />
            <CheckMenuItem
              label={toolLabel(T.Rectangle)}
              shortcut={keyFor(T.Rectangle)}
              checked={activeTool === T.Rectangle}
              onClick={withClose(() => onSelectTool?.(T.Rectangle))}
            />
            <CheckMenuItem
              label={toolLabel(T.Circle)}
              shortcut={keyFor(T.Circle)}
              checked={activeTool === T.Circle}
              onClick={withClose(() => onSelectTool?.(T.Circle))}
            />
            <CheckMenuItem
              label={toolLabel(T.Polygon)}
              shortcut={keyFor(T.Polygon)}
              checked={activeTool === T.Polygon}
              onClick={withClose(() => onSelectTool?.(T.Polygon))}
            />
            <div style={SEPARATOR_STYLE} />
            <MenuItem
              label={t`3D Text…`}
              disabled={editGates?.canDrawText === false}
              onClick={withClose(() => onDrawText?.())}
            />
          </div>
        )}
      </div>

      {/* Tools menu */}
      <div style={{ position: 'relative', height: '100%' }}>
        <MenuTrigger id="tools" label={t`Tools`} openMenu={openMenu} onToggle={toggle} onActivate={activate} />
        {openMenu === 'tools' && (
          <div style={DROPDOWN_STYLE}>
            {/* Grouped by job (mirrors the native menu): select, transform,
                shape geometry, measure & annotate, materials, model-space
                utilities — future tools slot into their group. */}
            <CheckMenuItem
              label={toolLabel(T.Select)}
              shortcut={keyFor(T.Select)}
              checked={activeTool === T.Select}
              onClick={withClose(() => onSelectTool?.(T.Select))}
            />
            <div style={SEPARATOR_STYLE} />
            <CheckMenuItem
              label={toolLabel(T.Move)}
              shortcut={keyFor(T.Move)}
              checked={activeTool === T.Move}
              onClick={withClose(() => onSelectTool?.(T.Move))}
            />
            <CheckMenuItem
              label={toolLabel(T.Rotate)}
              shortcut={keyFor(T.Rotate)}
              checked={activeTool === T.Rotate}
              onClick={withClose(() => onSelectTool?.(T.Rotate))}
            />
            <CheckMenuItem
              label={toolLabel(T.Scale)}
              shortcut={keyFor(T.Scale)}
              checked={activeTool === T.Scale}
              onClick={withClose(() => onSelectTool?.(T.Scale))}
            />
            <div style={SEPARATOR_STYLE} />
            <CheckMenuItem
              label={toolLabel(T.PushPull)}
              shortcut={keyFor(T.PushPull)}
              checked={activeTool === T.PushPull}
              onClick={withClose(() => onSelectTool?.(T.PushPull))}
            />
            <CheckMenuItem
              label={toolLabel(T.FollowMe)}
              checked={activeTool === T.FollowMe}
              onClick={withClose(() => onSelectTool?.(T.FollowMe))}
            />
            <CheckMenuItem
              label={toolLabel(T.Offset)}
              shortcut={keyFor(T.Offset)}
              checked={activeTool === T.Offset}
              onClick={withClose(() => onSelectTool?.(T.Offset))}
            />
            <CheckMenuItem
              label={toolLabel(T.Slice)}
              checked={activeTool === T.Slice}
              onClick={withClose(() => onSelectTool?.(T.Slice))}
            />
            <CheckMenuItem
              label={toolLabel(T.EditVertex)}
              checked={activeTool === T.EditVertex}
              onClick={withClose(() => onSelectTool?.(T.EditVertex))}
            />
            <div style={SEPARATOR_STYLE} />
            <CheckMenuItem
              label={toolLabel(T.TapeMeasure)}
              shortcut={keyFor(T.TapeMeasure)}
              checked={activeTool === T.TapeMeasure}
              onClick={withClose(() => onSelectTool?.(T.TapeMeasure))}
            />
            <CheckMenuItem
              label={toolLabel(T.Protractor)}
              checked={activeTool === T.Protractor}
              onClick={withClose(() => onSelectTool?.(T.Protractor))}
            />
            <CheckMenuItem
              label={toolLabel(T.Dimension)}
              checked={activeTool === T.Dimension}
              onClick={withClose(() => onSelectTool?.(T.Dimension))}
            />
            <CheckMenuItem
              label={toolLabel(T.Text)}
              checked={activeTool === T.Text}
              onClick={withClose(() => onSelectTool?.(T.Text))}
            />
            <div style={SEPARATOR_STYLE} />
            <CheckMenuItem
              label={toolLabel(T.Paint)}
              shortcut={keyFor(T.Paint)}
              checked={activeTool === T.Paint}
              onClick={withClose(() => onSelectTool?.(T.Paint))}
            />
            <CheckMenuItem
              label={toolLabel(T.PositionTexture)}
              checked={activeTool === T.PositionTexture}
              onClick={withClose(() => onSelectTool?.(T.PositionTexture))}
            />
            <div style={SEPARATOR_STYLE} />
            <CheckMenuItem
              label={toolLabel(T.DrawingAxes)}
              checked={activeTool === T.DrawingAxes}
              onClick={withClose(() => onSelectTool?.(T.DrawingAxes))}
            />
            <CheckMenuItem
              label={toolLabel(T.SectionPlane)}
              checked={activeTool === T.SectionPlane}
              onClick={withClose(() => onSelectTool?.(T.SectionPlane))}
            />
          </div>
        )}
      </div>

      {/* Window menu — actual windows (HIG): the Library (visually and
          behaviorally a window, not a tray pane — a real second window on
          desktop) and the open-windows list pushed by the desktop shell.
          The pane toggles that used to live here are in View now; with
          neither a Library nor a windowList the menu vanishes entirely. */}
      {(onToggleLibrary !== undefined || (windowList !== undefined && windowList.length > 0)) && (
        <div style={{ position: 'relative', height: '100%' }}>
          <MenuTrigger id="window" label={t`Window`} openMenu={openMenu} onToggle={toggle} onActivate={activate} />
          {openMenu === 'window' && (
            <div style={DROPDOWN_STYLE}>
              {onToggleLibrary !== undefined && (
                <CheckMenuItem
                  label={t`Library`}
                  shortcut={scShiftModL}
                  checked={showLibrary}
                  onClick={withClose(() => onToggleLibrary())}
                />
              )}
              {windowList !== undefined && windowList.length > 0 && (
                <>
                  {onToggleLibrary !== undefined && <div style={SEPARATOR_STYLE} />}
                  {windowList.map((w) => (
                    <CheckMenuItem
                      key={w.label}
                      label={windowMenuLabel(w.title) || t`Untitled`}
                      checked={w.focused}
                      onClick={withClose(() => onFocusWindow?.(w.label))}
                    />
                  ))}
                </>
              )}
            </div>
          )}
        </div>
      )}

      {/* Help menu */}
      <div style={{ position: 'relative', height: '100%' }}>
        <MenuTrigger id="help" label={t`Help`} openMenu={openMenu} onToggle={toggle} onActivate={activate} />
        {openMenu === 'help' && (
          <div style={DROPDOWN_STYLE}>
            {onHelpSearch !== undefined && (
              <>
                <HelpSearchField
                  onSearch={(q) => {
                    close()
                    onHelpSearch(q)
                  }}
                />
                <div style={SEPARATOR_STYLE} />
              </>
            )}
            {onOpenGuide !== undefined && (
              <>
                <MenuItem label={t`Hew Help`} onClick={withClose(onOpenGuide)} />
                <div style={SEPARATOR_STYLE} />
              </>
            )}
            <MenuItem
              label={t`Report Bug…`}
              onClick={withClose(() => onReportBug?.())}
            />
            <MenuItem
              label={sentReportsNew > 0 ? t`Sent Reports (${sentReportsNew} new)…` : t`Sent Reports…`}
              onClick={withClose(() => onSentReports?.())}
            />
            <CheckMenuItem
              label={t`Debug Log`}
              checked={showDebugLog}
              onClick={withClose(() => onToggleDebugLog?.())}
            />
            {onCheckForUpdates !== undefined && (
              <>
                <div style={SEPARATOR_STYLE} />
                <MenuItem
                  label={t`Check for Updates…`}
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

      {/* Beta build cue (beta web app only): a passive label, not a control —
          no handler, not focusable. The tooltip carries the full version. */}
      {isBetaBuild() && (
        <span style={BETA_CHIP_STYLE} title={betaVersion} data-testid="beta-chip">
          <Trans>BETA</Trans>
        </span>
      )}

      {/* Settings gear on the trailing edge — the modern menu-bar convention
          for reaching Settings (macOS reaches it via the app menu). */}
      {onOpenSettings !== undefined && (
        <GearButton onClick={onOpenSettings} shortcutHint={`${mod},`} />
      )}
    </div>
  )
}
