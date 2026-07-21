import { useEffect, useState, useCallback, useRef, useMemo } from 'react'
import { flushSync } from 'react-dom'
import { loadKernel, type Scene } from './wasm/loader'
import Viewport, { type ViewportApi, type InferenceInfo, type StandardView } from './viewport/Viewport'
import { InferenceTooltip } from './viewport/InferenceTooltip'
import { SnapDot } from './viewport/SnapDot'
import { MeasurementBox } from './viewport/MeasurementBox'
import { ViewportHUD } from './viewport/ViewportHUD'
import { DocumentTree } from './panels/DocumentTree'
import { MaterialPalette } from './panels/MaterialPalette'
import { MenuBar } from './panels/MenuBar'
import { TitleBar } from './TitleBar'
import { isLinux, isMac, isWindows } from './platform'
import { nextPaint } from './paint'
import { TagsPanel } from './panels/TagsPanel'
import { ObjectInfoPanel } from './panels/ObjectInfoPanel'
import { TraySection } from './panels/TraySection'
import { ToolRail } from './panels/ToolRail'
import { ContextualDock } from './panels/ContextualDock'
import { nextSelection, canBoolean as canBooleanHelper, canMakeComponent, canPlaceInstance, canExplodeInstance, canMakeUnique, canGroup as canGroupHelper, canUngroup as canUngroupHelper, nodeKey, nodeKindToNumber, nodeRefFromJs, resolveLabel, buildTreeIndexMap, collectLeafIds as collectLeafIdsShared, pruneDeadSelection, type NodeRef } from './panels/treeModel'
import { tagPathKey, isPathUnder } from './panels/tagModel'
import { LogPanel } from './log/LogPanel'
import * as LogStore from './log/LogStore'
import { installTestHarness } from './test/harness'
import { install as installConsoleCapture, restore as restoreConsoleCapture } from './log/consoleCapture'
import { MATERIAL_SENTINEL } from './tools/PaintTool'
import { makeFileHost, isTauri, type ImportReport, type ImportPick, type OpenPick } from './io/fileHost'
import {
  INITIAL_SESSION,
  deriveTitle,
  documentName,
  saveStateLabel,
  afterMutation,
  afterSave,
  afterOpen,
  afterImport,
  type DocSessionState,
} from './io/documentSession'
import { makeRecoveryStore, shouldPromptRecovery, type RecoveryListing, type RecoverySnapshot, type RecoveryMeta } from './io/recoveryStore'
import { ImportReportDialog } from './panels/ImportReportDialog'
import { ImportingOverlay } from './panels/ImportingOverlay'
import { RecoveryDialog } from './panels/RecoveryDialog'
import { WelcomeScreen, type SampleEntry } from './panels/WelcomeScreen'
import { getShowWelcome, setShowWelcome } from './settings/welcomeScreen'
import { getLengthUnit, setLengthUnit, homeFramingScale, subscribe as subscribeLengthUnit, type LengthFormat } from './settings/units'
import { StlExportDialog } from './panels/StlExportDialog'
import { StlUnitsDialog } from './panels/StlUnitsDialog'
import { setLastStlImportUnit } from './settings/stlImportUnit'
import { ExportDialog, type ExportFormat } from './panels/ExportDialog'
import { collectNonSolidObjects } from './io/exporters/stlExport'
import { friendlyErrorText, isErrorLevelCode } from './kernelErrors'
import { CommandPalette } from './palette/CommandPalette'
import { toolHint, toolActionId, type PaletteEntry } from './palette/registry'
import type { TagReveal } from './panels/TagsPanel'
import { SettingsWindow } from './settings/SettingsWindow'
import { FluentSettingsPage } from './settings/FluentSettingsPage'
import { getDebugMode, subscribe as subscribeDebugMode } from './settings/debugMode'
import { getTrayLayout, setTrayLayout, subscribe as subscribeTrayLayout } from './settings/trayLayout'
import * as diagnosticLog from './log/diagnosticLog'
import * as inputRecorder from './recording/inputRecorder'
import { generateBugReport } from './log/reportBug'
import { TOOLS, type ToolName } from './tools/toolRegistry'

/** Autosave tick interval (ms). */
const AUTOSAVE_INTERVAL_MS = 12000
/** Right-tray width: default/bounds (px) + localStorage persistence key.
 *  Default is sized so typical nested tag/outliner labels fit untruncated. */
const TRAY_WIDTH_DEFAULT = 304
const TRAY_WIDTH_MIN = 220
const TRAY_WIDTH_MAX = 560
const TRAY_WIDTH_KEY = 'hew.trayWidth'
const clampTrayWidth = (w: number): number =>
  Math.min(TRAY_WIDTH_MAX, Math.max(TRAY_WIDTH_MIN, Math.round(w)))
/** Refresh interval (ms) for the "Edited/Saved <relative time>" indicator. */
const SAVE_STATE_TICK_MS = 30000

interface AppState {
  kernelVersion: string
  scene: Scene
}

interface Toast {
  id: number
  message: string
  code?: string
  /** Error-level (red bubble, error log) vs warning — classified ONCE at
   * creation via kernelErrors' isErrorLevelCode, the single source. */
  isError: boolean
}

let toastCounter = 0

/** Tools that create new geometry — picking one clears a lingering
 * top-level selection (the contextual dock should switch to draw context,
 * not stay pinned to the old object). */
const DRAW_TOOLS: ReadonlySet<string> = new Set(['Line', 'Rectangle', 'Circle', 'Polygon', 'Arc'])

/** Tool name → native menu item id, for the macOS menu's radio checks.
 * Exported for the native-menu parity test (`nativeMenuParity.test.ts`):
 * every id here must exist as a real item in the Tauri shell's menu. */
export const TOOL_MENU_IDS: Record<string, string> = {
  Select: 'tool-select',
  Rectangle: 'draw-rectangle',
  Circle: 'draw-circle',
  Polygon: 'draw-polygon',
  Arc: 'draw-arc',
  Line: 'draw-line',
  'Push/Pull': 'tool-pushpull',
  'Follow Me': 'tool-follow-me',
  Offset: 'tool-offset',
  Paint: 'tool-paint',
  Move: 'tool-move',
  Rotate: 'tool-rotate',
  Scale: 'tool-scale',
  'Tape Measure': 'tool-tape-measure',
  Protractor: 'tool-protractor',
  Slice: 'tool-slice',
  'Section Plane': 'tool-section-plane',
  'Edit Vertex': 'tool-edit-vertex',
  Orbit: 'cam-orbit',
  Pan: 'cam-pan',
  Zoom: 'cam-zoom',
}

/** Extract the filename from an absolute path (cross-platform / or \). */
function basenameOf(path: string): string {
  return path.replace(/[/\\]+/g, '/').split('/').filter(Boolean).pop() ?? path
}

/** Strings that signal the Scene borrow-lock after a Rust panic. */
const PANIC_SIGNATURES = ['recursive use of an object', 'unreachable']

/** True when the document holds no entities at all (a pristine "Untitled").
 *  Used by File ▸ New (blank documents are reused instead of spawning a
 *  second empty window) and by the open-time Zoom Extents (nothing to frame). */
function isSceneEmpty(scene: Scene): boolean {
  return (
    scene.object_ids().length === 0 &&
    scene.group_ids().length === 0 &&
    scene.instance_ids().length === 0 &&
    scene.sketch_ids().length === 0
  )
}

/** True when a window holds nothing worth protecting — no named file, no
 *  unsaved edits, no geometry — so a File ▸ Open or File ▸ New pick may
 *  reuse it in place instead of opening a fresh window. Stricter than
 *  `isSceneEmpty` alone: a saved-then-emptied document (currentRef set, but
 *  the model has since been deleted down to nothing) still has a real file
 *  behind it and must not be silently abandoned by reusing the window. */
export function isPristineDocument(session: DocSessionState, scene: Scene): boolean {
  return session.currentRef === null && !session.dirty && isSceneEmpty(scene)
}

function isPanicError(message: string): boolean {
  const lower = message.toLowerCase()
  return PANIC_SIGNATURES.some((sig) => lower.includes(sig))
}

export default function App() {
  const [state, setState] = useState<AppState | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [toolName, setToolName] = useState<string>('Select')
  /** Live stage-aware guidance from the active tool (Tool.statusHint);
   *  null = fall back to the palette's static tool description. */
  const [toolStageHint, setToolStageHint] = useState<string | null>(null)
  /** Precision snapping (the Ctrl/⌘+Alt chord held) — modal state, so it
   * gets a status-bar chip rather than living only in the viewport. */
  const [precisionSnap, setPrecisionSnap] = useState(false)
  const [snapKind, setSnapKind] = useState<string | null>(null)
  const [measurement, setMeasurement] = useState<string>('')
  /** Live inference-cursor info for the tooltip chip. */
  const [inferenceInfo, setInferenceInfo] = useState<InferenceInfo | null>(null)
  const [toasts, setToasts] = useState<Toast[]>([])
  /** Per-object watertight map (bigint key) */
  const [watertightMap, setWatertightMap] = useState<Map<bigint, boolean>>(new Map())
  /** Tool driven from toolbar clicks */
  const [activeTool, setActiveTool] = useState<ToolName>('Select')
  /** Sticky banner: true once a kernel panic / borrow-lock is detected */
  const [kernelPanicked, setKernelPanicked] = useState(false)
  /** Selected nodes (ordered; index 0 = primary). */
  const [selectedIds, setSelectedIds] = useState<NodeRef[]>([])
  /** Selected construction guide, mutually exclusive with node
   * selection. Deleted via the same Edit ▸ Delete / Delete-key path as nodes. */
  const [selectedGuide, setSelectedGuide] = useState<bigint | null>(null)
  /** Session-only hidden node set (keyed by nodeKey). Cleared on load/new. */
  const [hiddenKeys, setHiddenKeys] = useState<Set<string>>(new Set())
  /** Active context path. Empty = top level. */
  const [activeContext, setActiveContext] = useState<NodeRef[]>([])
  /** Bumped on any document change so the tree re-queries entity lists. */
  const [docRev, setDocRev] = useState(0)
  /** Currently selected material id for the Paint tool. */
  const [currentMaterialId, setCurrentMaterialId] = useState<bigint>(MATERIAL_SENTINEL)
  // Picking a material makes it current AND activates the Paint tool, so the
  // next click paints with it (Ctrl/Cmd-click fills the whole object). This is
  // the whole-object path that the removed "Fill selected object" button used
  // to serve — the status bar spells out the Ctrl/Cmd-click shortcut.
  const handleSelectMaterial = (id: bigint) => {
    setCurrentMaterialId(id)
    setActiveTool('Paint')
  }
  /** Document session: currentRef + dirty flag. */
  const [docSession, setDocSession] = useState<DocSessionState>(INITIAL_SESSION)
  /** Ticks every SAVE_STATE_TICK_MS purely to refresh the "Edited/Saved
   * <relative time>" indicator — nothing else reads this state.
   * Coarse (30s) since the label only needs minute-level freshness. */
  const [nowTick, setNowTick] = useState(() => Date.now())
  /** Tray-section expanded state ( sections; the showX names predate the
   * tray — they used to mean floating-panel visibility). Initialized from and
   * written back to the trayLayout singleton so the layout survives
   * relaunches; the setters, shortcuts, and Window-menu checkmarks are
   * untouched. */
  /** Pane visibility: Model info (DocumentTree) */
  const [showModelInfo, setShowModelInfo] = useState(() => getTrayLayout().modelInfo)
  /** Pane visibility: Materials (MaterialPalette) */
  const [showMaterials, setShowMaterials] = useState(() => getTrayLayout().materials)
  /** Pane visibility: Tags */
  const [showTags, setShowTags] = useState(() => getTrayLayout().tags)
  /** Pane visibility: Object Info */
  const [showObjectInfo, setShowObjectInfo] = useState(() => getTrayLayout().objectInfo)
  /** Debug Log panel visibility (default hidden — opt-in via Window menu only). */
  const [showDebugLog, setShowDebugLog] = useState(false)
  /** View ▸ Axes / Grid / Guides visibility. Default all shown. */
  const [showAxes, setShowAxes] = useState(true)
  const [showGrid, setShowGrid] = useState(true)
  const [showGuides, setShowGuides] = useState(true)
  /** View ▸ Section Plane's check/enabled state — a RENDER CACHE of the
   * section manager's own truth, never toggled directly by this component.
   * Populated only by `handleSectionChanged` re-reading
   * `viewportApi.current?.getSectionState()` (D3, section-plane-polish);
   * unlike showAxes/showGrid/showGuides (which this component itself owns
   * and pushes DOWN into the viewport), the section's existence/active flag
   * is owned by the viewport's session-only SectionManager and pulled UP
   * here, so it can never independently drift from kernel/session truth. */
  const [sectionPlaneMenuState, setSectionPlaneMenuState] = useState<{ checked: boolean; exists: boolean }>({
    checked: false,
    exists: false,
  })
  /** Settings modal visibility — web fallback only (Tauri opens a real OS window). */
  const [showSettingsModal, setShowSettingsModal] = useState(false)
  // Windows desktop settings surface (Fluent in-app page; see openSettings).
  const [showFluentSettings, setShowFluentSettings] = useState(false)
  /** Command palette visibility (⌘K / Ctrl-K). */
  const [paletteOpen, setPaletteOpen] = useState(false)
  /** True while the user is pointer-dragging the camera (orbit/pan/dolly) —
   * fades the contextual dock out of the way. Fed by Viewport's
   * OrbitControls start/end events; never persisted. */
  const [cameraDragging, setCameraDragging] = useState(false)
  /** True while the cursor is aimed at a live sketch's extrudable region AND
   * nothing is selected ( hover-dock) — Viewport polls+throttles this
   * via SketchHoverGate and only calls back on a true/false transition. An
   * explicit selection's dock always wins; ContextualDock only consults this
   * when the derived context is 'empty'. */
  const [hoveringSketchRegion, setHoveringSketchRegion] = useState(false)
  /** Tag-path hide set: each entry is tagPathKey(path). Cleared on load/new. */
  const [hiddenTagPaths, setHiddenTagPaths] = useState<Set<string>>(new Set())
  /** Import report to display (null = no dialog). */
  const [importReport, setImportReport] = useState<ImportReport | null>(null)
  /** True while import_dae is running (blocks main thread). */
  const [isImporting, setIsImporting] = useState(false)
  /** Display name of the file being imported (shown in the overlay). */
  const [importingName, setImportingName] = useState('')
  /** STL units-chooser modal: the picked file's name, or null when closed. */
  const [pendingStlImport, setPendingStlImport] = useState<{ name: string } | null>(null)
  /** Recovery snapshot to offer at startup (null = no dialog). */
  const [recoveryPrompt, setRecoveryPrompt] = useState<RecoveryListing[] | null>(null)
  /** Welcome screen on a bare launch (startup-handoff effect decides). */
  const [welcomeOpen, setWelcomeOpen] = useState(false)
  /** True once any document has loaded — vetoes a late welcome-screen offer. */
  const documentLoadedRef = useRef(false)
  /** Mirror of welcomeOpen for the window-level key handlers (some have
   *  empty dep arrays, so they can't read the state directly). */
  const welcomeOpenRef = useRef(false)
  welcomeOpenRef.current = welcomeOpen
  /** Mirror of the persisted "show welcome on startup" flag, so the
   *  dialog's checkbox re-renders on toggle. */
  const [showWelcomeSetting, setShowWelcomeSetting] = useState(getShowWelcome)
  // The welcome screen's unit dropdown mirrors the units singleton; kept in
  // React state so the select re-renders, and re-synced from external
  // changes (the Settings surface) while the screen is up.
  const [welcomeUnit, setWelcomeUnit] = useState<LengthFormat>(getLengthUnit)
  useEffect(() => subscribeLengthUnit(setWelcomeUnit), [])
  /** Right-tray width (px), user-resizable via the drag handle; persisted. */
  const [trayWidth, setTrayWidth] = useState<number>(() => {
    const raw = window.localStorage.getItem(TRAY_WIDTH_KEY)
    const n = raw !== null ? Number(raw) : NaN
    return Number.isFinite(n) ? clampTrayWidth(n) : TRAY_WIDTH_DEFAULT
  })
  /** Open document windows (Tauri multi-window only) — the Window menu's
   *  tail of focus-this-window entries. Populated by `list_windows` at
   *  mount and kept fresh by the shell's `window-list` broadcast (window
   *  create/destroy/focus/title-change). Always empty on the web build
   *  (single window, no shell to ask) and on macOS Tauri, where the native
   *  menu renders its own tail directly — this only feeds the in-app
   *  MenuBar (Windows/Linux). */
  const [windowList, setWindowList] = useState<{ label: string; title: string; focused: boolean }[]>([])

  /** Imperative handle into the viewport (e.g. running a boolean). */
  const viewportApi = useRef<ViewportApi | null>(null)

  // Stable ref to the Scene for undo/redo button state queries
  const sceneRef = useRef<Scene | null>(null)
  // Bytes of a fresh blank scene, captured once after kernel load, used for New.
  const blankBytesRef = useRef<Uint8Array | null>(null)
  // Stable file host instance.
  const fileHostRef = useRef(makeFileHost())
  // Resolver for the in-flight StlUnitsDialog promise (see promptStlUnits);
  // null when no chooser is open.
  const stlUnitsResolveRef = useRef<((unitScale: number | null) => void) | null>(null)
  // Guards openDocument/importDocument against re-entry: a second gesture
  // started while the first is mid-flight (e.g. Ctrl+K ▸ "import" ▸ Enter
  // behind the units modal) would otherwise orphan the first, hanging it
  // forever. Shared between both entry points — they converge on the same
  // post-pick import steps (runImportPick) and must not race each other. A
  // ref (not state) so the guard is synchronous, before the first await.
  const openInFlightRef = useRef(false)
  // Mirror of docSession kept up-to-date so callbacks can read current state
  // without capturing stale closures or causing impure updater functions.
  const docSessionRef = useRef<DocSessionState>(INITIAL_SESSION)
  // When true, handleDocumentChanged suppresses the dirty-marking setState
  // (used during programmatic loads so the post-load afterOpen wins).
  const suppressDirtyRef = useRef(false)
  // Stable recovery-store instance (autosave / crash recovery).
  const recoveryStoreRef = useRef(makeRecoveryStore())
  // pushUnionHidden is defined further down (it depends on `state`); reach it
  // through a ref (kept current beside its definition) like reconcileRef /
  // applyLoadedBytesRef, so applyLoadedBytes and importDocument — both defined
  // earlier — can push freshly-seeded hidden state without a stale closure.
  const pushUnionHiddenRef = useRef<(nextHiddenKeys: Set<string>, nextHiddenTagPaths: Set<string>) => void>(() => {})
  // Re-seeds the session's hidden-tag set from the document's tag registry
  // and re-pushes the visibility union. Undo/redo can change the registry
  // (delete tag restores/removes hidden flags with it), so handleUndo/
  // handleRedo — defined before the seeding helpers — reach the latest
  // closure through this ref, same pattern as pushUnionHiddenRef.
  const resyncTagVisibilityRef = useRef<() => void>(() => {})
  // Latest tag handlers for the test harness — it installs once on mount
  // (before these are defined further down), so it reaches them through
  // refs, the same pattern as reconcileRef/applyLoadedBytesRef.
  const toggleTagPathRef = useRef<(path: string[]) => void>(() => {})
  const deleteTagRef = useRef<(path: string[]) => void>(() => {})
  // True when a mutation has occurred since the last successful autosave tick
  // (or since the last explicit Save, which also resets it). Avoids redundant
  // writes when the document hasn't changed between ticks.
  const dirtySinceAutosaveRef = useRef(false)
  // The autosave write currently in flight (never rejects), or null before
  // the first write. clearRecoverySnapshot awaits it so a discard's clear
  // can't interleave with a write of the very snapshot being discarded.
  const autosaveWriteRef = useRef<Promise<void> | null>(null)
  // Resolved once this window's menu-open-path listener is registered (or
  // registration failed) — the startup-handoff effect awaits it before
  // telling the shell this webview is ready for live open delivery. Created
  // during render so the promise exists before any effect runs.
  const openListenerReadyRef = useRef<{ promise: Promise<void>; resolve: () => void } | null>(null)
  if (openListenerReadyRef.current === null) {
    let resolve!: () => void
    const promise = new Promise<void>((r) => { resolve = r })
    openListenerReadyRef.current = { promise, resolve }
  }

  // Install console capture on mount, restore on unmount.
  useEffect(() => {
    installConsoleCapture()
    return () => {
      restoreConsoleCapture()
    }
  }, [])

  useEffect(() => {
    loadKernel()
      .then((kernel) => {
        const kernelVersion = kernel.version()
        const scene = kernel.newScene()
        sceneRef.current = scene
        // Snapshot blank scene bytes for "New" resets.
        blankBytesRef.current = new Uint8Array(scene.save())
        setState({ kernelVersion, scene })
        LogStore.log.info('app', `Kernel loaded — version ${kernelVersion}`)
      })
      .catch((err: unknown) => {
        const msg = String(err)
        setError(msg)
        LogStore.log.error('app', `Kernel load failed: ${msg}`)
      })
  }, [])

  // ---------------------------------------------------------------- Debug Mode
  // Apply the GLOBAL half of Debug Mode's effects (file logging + input
  // recording + torture mode on the CURRENT scene) once on mount and on every
  // subsequent toggle. The PER-SCENE half — a freshly-created Scene (New/Open)
  // inheriting the current mode — is covered by wasm/loader.ts's newScene().
  useEffect(() => {
    const apply = (on: boolean): void => {
      diagnosticLog.setFileLogging(on)
      if (on) {
        inputRecorder.start()
      } else {
        inputRecorder.stop()
      }
      sceneRef.current?.set_torture_mode(on)
    }
    apply(getDebugMode())
    return subscribeDebugMode(apply)
  }, [])

  // ---------------------------------------------------------------- tray layout persistence
  // Write the four section flags back to the singleton whenever any of them
  // changes (also fires once on mount, writing the just-restored values —
  // harmless). The subscription mirrors the other settings singletons and
  // covers cross-window/tab changes; for locally-originated changes it echoes
  // the values React already has, so the setState calls bail out.
  useEffect(() => {
    setTrayLayout({
      modelInfo: showModelInfo,
      objectInfo: showObjectInfo,
      materials: showMaterials,
      tags: showTags,
    })
  }, [showModelInfo, showObjectInfo, showMaterials, showTags])
  useEffect(() => {
    return subscribeTrayLayout((layout) => {
      setShowModelInfo(layout.modelInfo)
      setShowObjectInfo(layout.objectInfo)
      setShowMaterials(layout.materials)
      setShowTags(layout.tags)
    })
  }, [])

  // Keep the ref in sync so side-effect callbacks can read current session state.
  useEffect(() => {
    docSessionRef.current = docSession
  }, [docSession])

  // ---------------------------------------------------------------- autosave
  // Periodically snapshot the scene to the RecoveryStore so a crash or forced
  // quit doesn't lose work. Only writes when the document is dirty AND has
  // mutated since the last successful write (avoids redundant IO on an idle,
  // already-autosaved document). Runs once for the component's lifetime.
  useEffect(() => {
    const interval = setInterval(() => {
      const scene = sceneRef.current
      const session = docSessionRef.current
      if (scene === null || !session.dirty || !dirtySinceAutosaveRef.current) return
      const bytes = new Uint8Array(scene.save())
      const meta: RecoveryMeta = {
        version: 1,
        name: session.currentRef?.name ?? session.importedName ?? 'Untitled',
        path: typeof session.currentRef?.handle === 'string' ? session.currentRef.handle : null,
        savedAt: Date.now(),
      }
      // Keep the in-flight write observable so clearRecoverySnapshot can
      // wait it out — the shell does not order recovery_write against
      // recovery_clear, so a clear overlapping a write can leave the
      // snapshot (or half of it) on disk. The stored promise never rejects.
      const write = recoveryStoreRef.current.write(bytes, meta).then(() => {
        // Only mark clean while this write is still the latest — a stale
        // disarm could suppress the re-write that follows a superseding
        // edit-plus-clear interleave.
        if (autosaveWriteRef.current === write) dirtySinceAutosaveRef.current = false
      }).catch(() => { /* ignore — try again next tick */ })
      autosaveWriteRef.current = write
    }, AUTOSAVE_INTERVAL_MS)
    return () => clearInterval(interval)
  }, [])

  // ---------------------------------------------------------------- save-state indicator tick
  // The "Edited/Saved <relative time>" text in TitleBar/MenuBar needs to
  // advance even when nothing else changes (e.g. sitting idle after an edit,
  // "Edited just now" should become "Edited 2 minutes ago"). Runs once for
  // the component's lifetime.
  useEffect(() => {
    const interval = setInterval(() => setNowTick(Date.now()), SAVE_STATE_TICK_MS)
    return () => clearInterval(interval)
  }, [])

  // (The startup recovery check lives in the startup-handoff effect below —
  // it must run strictly AFTER any pending file-association open or recovery
  // handoff has been consumed, or the dialog races the open and can appear
  // over, then replace, a document the user explicitly double-clicked.)

  // Update document.title whenever session state changes. Under Tauri, also
  // push the title to the shell via `set_window_title` — the in-app MenuBar
  // no longer renders a title for the native case, so the title now lives
  // solely in the OS title bar. `set_window_title` (rather than a bare
  // window.setTitle) also refreshes the shell's open-window list/menu, so
  // the Window-menu entry for this window tracks its document name and
  // dirty mark live.
  useEffect(() => {
    const title = deriveTitle(docSession)
    document.title = title
    if (isTauri) {
      import('@tauri-apps/api/core').then(({ invoke }) => {
        invoke('set_window_title', { title }).catch(() => { /* ignore */ })
      }).catch(() => { /* ignore */ })
    }
  }, [docSession])

  // Warn before unload when there are unsaved changes.
  useEffect(() => {
    const handler = (e: BeforeUnloadEvent) => {
      if (docSession.dirty) {
        e.preventDefault()
        // Legacy support
        e.returnValue = ''
      }
    }
    window.addEventListener('beforeunload', handler)
    return () => window.removeEventListener('beforeunload', handler)
  }, [docSession.dirty])

  const handleStatusChange = useCallback((name: string, kind: string | null) => {
    setToolName(name)
    setSnapKind(kind)
  }, [])

  const handleMeasurement = useCallback((text: string) => {
    setMeasurement(text)
  }, [])

  const handleInferenceChange = useCallback((info: InferenceInfo | null) => {
    setInferenceInfo(info)
  }, [])

  const handleSceneChange = useCallback((wtMap: Map<bigint, boolean>) => {
    setWatertightMap(new Map(wtMap))
  }, [])

  const handleSelect = useCallback((node: NodeRef | null, additive: boolean) => {
    // Node and guide selection are mutually exclusive.
    setSelectedGuide(null)
    setSelectedIds((cur) => nextSelection(cur, node, additive))
  }, [])

  /** Lift a multi-node selection (marquee, Select All) from the viewport.
   * Non-additive replaces; additive (shift-drag) merges without duplicates. */
  const handleSelectMany = useCallback((nodes: NodeRef[], additive: boolean) => {
    setSelectedGuide(null)
    setSelectedIds((cur) => {
      if (!additive) return nodes
      const seen = new Set(cur.map(nodeKey))
      return [...cur, ...nodes.filter((n) => !seen.has(nodeKey(n)))]
    })
  }, [])

  /** Replace the selection outright (Object Info's "(N instances)" click). */
  const handleReplaceSelection = useCallback(
    (nodes: NodeRef[]) => handleSelectMany(nodes, false),
    [handleSelectMany],
  )

  /** Lift a guide pick from the viewport; clears node selection. */
  const handleSelectGuide = useCallback((id: bigint | null) => {
    setSelectedGuide(id)
    if (id !== null) setSelectedIds([])
  }, [])

  const handleEnterContext = useCallback((node: NodeRef) => {
    setActiveContext((prev) => [...prev, node])
    setSelectedIds([node])
  }, [])

  const handleExitContext = useCallback(() => {
    setActiveContext((prev) => prev.slice(0, -1))
  }, [])

  /** Truncate the context path to `depth` entries. */
  const handleSetContextDepth = useCallback((depth: number) => {
    setActiveContext((prev) => prev.slice(0, depth))
  }, [])

  /** Validate and trim the context path when the document changes. */
  const trimContextPath = useCallback((scene: Scene, path: NodeRef[]): NodeRef[] => {
    const objectIds = new Set(Array.from(scene.object_ids()))
    const groupIds = new Set(Array.from(scene.group_ids()))
    const instanceIds = new Set(Array.from(scene.instance_ids()))

    let trimIndex = path.length
    for (let i = 0; i < path.length; i++) {
      const node = path[i]
      let alive: boolean
      if (node.kind === 'object') {
        alive = objectIds.has(node.id)
      } else if (node.kind === 'instance') {
        alive = instanceIds.has(node.id)
      } else {
        alive = groupIds.has(node.id)
      }
      if (!alive) {
        trimIndex = i
        break
      }
    }
    return path.slice(0, trimIndex)
  }, [])

  // Bump the tree's revision; trim stale context path entries; mark dirty.
  // Every scene mutation flows through here via Viewport's handleSceneRefresh.
  // (The first-sketch auto-zoom that used to fire here was removed by
  // maintainer decision: the welcome screen's unit choice already sets a
  // sensible initial framing, and yanking the camera on the first draw was
  // more irritating than helpful. Zoom Extents still covers sketches.)
  const handleDocumentChanged = useCallback(() => {
    setDocRev((r) => r + 1)
    setActiveContext((ctx) => {
      const scene = sceneRef.current
      if (scene === null) return ctx
      return trimContextPath(scene, ctx)
    })
    // Prune dead handles out of the selection. Undo/redo is the headline
    // case (the maintainer's repro: undo an array copy and Object Info kept
    // saying "3 selected" while the dock sat in Multi mode), but every
    // mutation that can kill nodes funnels through here — deletes, booleans
    // consuming operands, extrusions consuming sketches — so they all
    // reconcile at this one choke point. pruneDeadSelection returns the
    // same array when nothing died, so this setState is a no-op then.
    setSelectedIds((sel) => {
      const scene = sceneRef.current
      if (scene === null) return sel
      return pruneDeadSelection(scene, sel)
    })
    // Mark the document dirty on any mutation — but NOT during programmatic
    // loads (suppressDirtyRef is true while applyLoadedBytes calls notifyLoaded).
    if (!suppressDirtyRef.current) {
      setDocSession((s) => afterMutation(s, Date.now()))
      dirtySinceAutosaveRef.current = true
    }
  }, [trimContextPath])

  // Re-derive the View ▸ Section Plane menu state from the section
  // manager's own truth (`getSectionState`) — called by the viewport
  // whenever a section is placed/offset-committed/toggled/deleted, or a
  // fresh document clears it (Viewport's `onSectionChanged`). Deliberately
  // NOT a toggle: this always re-READS live state rather than flipping a
  // local boolean, so it can never drift from what the session actually
  // holds (D3, section-plane-polish).
  const handleSectionChanged = useCallback(() => {
    const state = viewportApi.current?.getSectionState() ?? null
    setSectionPlaneMenuState({ checked: state !== null && state.active, exists: state !== null })
  }, [])

  // Semantic test harness `window.__hew_test`, installed only in
  // debug/test builds (dev server, or a build with VITE_HEW_TEST=1 for the
  // Playwright E2E build). Live values are read through refs so the harness
  // object is installed once and never goes stale.
  const reconcileRef = useRef(handleDocumentChanged)
  reconcileRef.current = handleDocumentChanged
  const selectedIdsRef = useRef(selectedIds)
  selectedIdsRef.current = selectedIds
  // applyLoadedBytes is defined further down (it depends on later state setters);
  // the harness installs once on mount, so reach it through a ref (kept current
  // below, beside that definition) like reconcileRef.
  const applyLoadedBytesRef = useRef<((bytes: Uint8Array) => boolean) | null>(null)
  useEffect(() => {
    if (!(import.meta.env.DEV || import.meta.env.VITE_HEW_TEST === '1')) return
    return installTestHarness({
      getScene: () => sceneRef.current,
      getViewportApi: () => viewportApi.current,
      reconcile: () => reconcileRef.current(),
      getSelection: () => selectedIdsRef.current,
      setSelectedObjects: (ids) =>
        setSelectedIds(ids.map((id) => ({ kind: 'object' as const, id }))),
      setSelection: (nodes) => {
        setSelectedGuide(null)
        setSelectedIds(nodes)
      },
      loadBytes: (bytes) => applyLoadedBytesRef.current?.(bytes) ?? false,
      toggleTagPath: (path) => toggleTagPathRef.current(path),
      deleteTag: (path) => deleteTagRef.current(path),
    })
  }, [])

  const handleToast = useCallback((message: string, code?: string) => {
    // Severity lives beside the copy table in kernelErrors.ts (one source; a
    // new code gets its level where it gets its message).
    const isError = code !== undefined && isErrorLevelCode(code)
    const level = isError ? 'error' : 'warn'
    const logMessage = code !== undefined ? `[${code}] ${message}` : message
    LogStore.log[level]('tool', logMessage)

    if (isPanicError(message)) {
      setKernelPanicked(true)
    }

    const id = ++toastCounter
    setToasts((prev) => [...prev, { id, message, code, isError }])
    setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id))
    }, 4000)
  }, [])

  const dismissToast = useCallback((id: number) => {
    setToasts((prev) => prev.filter((t) => t.id !== id))
  }, [])

  // Report Bug — assemble the bundle and tell the user what happened.
  // The write is otherwise silent (Tauri saves a file, web downloads), so
  // without a toast it looks like the menu item does nothing.
  const handleReportBug = useCallback(() => {
    const scene = sceneRef.current
    if (scene === null) {
      handleToast('Report Bug: the model is still loading — try again in a moment.')
      return
    }
    handleToast('Generating bug report…')
    void generateBugReport(scene, 'user-report').then((result) => {
      if (!result.ok) {
        handleToast('Report Bug failed — see the diagnostic log for details.')
      } else if (result.path !== null) {
        handleToast(`Bug report saved: ${result.path}`)
      } else {
        handleToast('Bug report downloaded.')
      }
    })
  }, [handleToast])

  // Whether this desktop build carries the auto-updater (package-manager
  // builds compile it out — see the shell's `updater` feature). The in-app
  // menu bar (Windows/Linux) shows "Check for Updates" only when true; macOS
  // reaches it through the native menu instead. Stays false on the web build,
  // where the invoke simply never resolves.
  const [updaterAvailable, setUpdaterAvailable] = useState(false)
  useEffect(() => {
    if (!isTauri) return
    let cancelled = false
    import('@tauri-apps/api/core')
      .then(({ invoke }) => invoke<boolean>('updater_available'))
      .then((ok) => { if (!cancelled) setUpdaterAvailable(ok) })
      .catch(() => { /* command absent (updater compiled out) — leave hidden */ })
    return () => { cancelled = true }
  }, [])

  // Manual "Check for Updates" — hands off to the shell, which runs the whole
  // flow (check → native confirm → download → restart prompt).
  const handleCheckForUpdates = useCallback(() => {
    import('@tauri-apps/api/core')
      .then(({ invoke }) => invoke('check_for_updates'))
      .catch(() => { /* ignore */ })
  }, [])

  useEffect(() => {
    const unsub = LogStore.subscribe((entries) => {
      const latest = entries[entries.length - 1]
      if (latest === undefined) return
      if (latest.source === 'console' && latest.level === 'error') {
        if (isPanicError(latest.message)) {
          setKernelPanicked(true)
        }
      }
    })
    return unsub
  }, [])

  // Compute the lit set for isolation.
  // When the context is non-empty, compute the leaf objects of the deepest context node.
  // When entering an object (deepest is an object), lit = {that object}.
  // When entering an instance (deepest is an instance), lit = member objects of the def.
  // NOTE: this is a hook, so it must run on every render — keep it above the
  // early returns below (Rules of Hooks). It guards `state === null` itself.
  const activeLitSet: Set<bigint> | null = useMemo(() => {
    if (state === null || activeContext.length === 0) return null
    const deepest = activeContext[activeContext.length - 1]
    if (deepest.kind === 'instance') {
      // Light all definition member objects when inside a component
      const componentId = state.scene.instance_def(deepest.id)
      if (componentId === undefined) return null
      const members = Array.from(state.scene.component_member_objects(componentId))
      return new Set(members)
    }
    const kind = deepest.kind === 'group' ? 1 : 0
    const leaves = Array.from(state.scene.node_leaf_objects(kind, deepest.id))
    return new Set(leaves)
  }, [activeContext, state])

  // Build the set of tag path keys marked hidden-by-default in the document's
  // tag metadata registry (scene.tag_meta_paths()/tag_meta_hidden()) — this
  // covers tags no node carries (e.g. an imported .skp layer list, empty
  // layers included), so a hidden empty layer still comes up hidden.
  const seedHiddenTagPathsFromRegistry = (scene: Scene): Set<string> => {
    const paths = scene.tag_meta_paths()
    const hidden = scene.tag_meta_hidden()
    const seeded = new Set<string>()
    for (let i = 0; i < paths.length; i++) {
      if (hidden[i] !== 1) continue
      const path = paths[i].split('/').map((s) => s.trim()).filter((s) => s.length > 0)
      if (path.length > 0) seeded.add(tagPathKey(path))
    }
    return seeded
  }

  // Build the set of hiddenKeys (nodeKey strings) from the document's
  // persisted USER-hidden registry (scene.user_hidden_kinds()/
  // user_hidden_ids(), manifest v6) — this is how imported .skp hidden
  // groups/components/instances (and a re-opened .hew with nodes previously
  // eye-toggled) arrive, since hiddenKeys itself is session-only React state
  // that gets reset on every load.
  const seedHiddenKeysFromRegistry = (scene: Scene): Set<string> => {
    const kinds = scene.user_hidden_kinds()
    const ids = scene.user_hidden_ids()
    const kindNames: NodeRef['kind'][] = ['object', 'group', 'instance']
    const seeded = new Set<string>()
    for (let i = 0; i < kinds.length; i++) {
      const kind = kindNames[kinds[i]]
      if (kind === undefined) continue
      seeded.add(nodeKey({ kind, id: ids[i] }))
    }
    return seeded
  }

  // Selection-dependent command availability — one derivation shared by the
  // render-body handlers, the native Edit menu (sync_menu_state), and the
  // in-app MenuBar, so every surface agrees on what's currently possible.
  // NOTE: hook — must stay above the early returns (Rules of Hooks).
  const menuGates = useMemo(() => {
    const scene = state?.scene
    if (scene == null) return null
    const objectIdSet = new Set(Array.from(scene.object_ids()))
    const groupIdSet = new Set(Array.from(scene.group_ids()))
    // Booleans take plain solids AND whole groups (boolean_nodes; the kernel
    // owns eligibility — solidity, instances — and refuses typed). Liveness
    // here; top-level-ness in canBooleanHelper below.
    const isBooleanOperand = (n: NodeRef) =>
      (n.kind === 'object' && objectIdSet.has(n.id)) ||
      (n.kind === 'group' && groupIdSet.has(n.id))
    const booleanOperands = selectedIds.filter(isBooleanOperand)
    const isSketchKind = (n: NodeRef) =>
      n.kind === 'sketch' ||
      n.kind === 'sketch-island' ||
      n.kind === 'sketch-curve' ||
      n.kind === 'sketch-edge'
    const parentOf = (n: NodeRef) => {
      if (isSketchKind(n)) return undefined
      const k = n.kind === 'group' ? 1 : n.kind === 'instance' ? 2 : 0
      return scene.node_parent(k, n.id)
    }
    // Sketch-scoped selections have no kernel NodeId — any in the selection
    // disqualifies the node-level commands.
    const hasSketch = selectedIds.some(isSketchKind)
    return {
      booleanOperands,
      // Same top-level rule the kernel enforces (GroupedOperand): a nested
      // node picked in the Outliner must not light the commands up only to
      // be refused on commit.
      canBoolean:
        activeContext.length === 0 &&
        canBooleanHelper(selectedIds, parentOf, isBooleanOperand),
      canGroup: !hasSketch && canGroupHelper(selectedIds, parentOf),
      canUngroup: !hasSketch && canUngroupHelper(selectedIds),
      canMakeComponent:
        activeContext.length === 0 && canMakeComponent(selectedIds, parentOf),
      canPlaceCopy: canPlaceInstance(selectedIds),
      canExplode: canExplodeInstance(selectedIds),
      canMakeUnique: canMakeUnique(selectedIds),
    }
    // docRev: entity lists change on every mutation without changing identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state, selectedIds, activeContext, docRev])

  // Choosing a Draw tool at top level clears the selection: the user is
  // about to create geometry, so a still-selected object/group/component
  // would pin the contextual dock (and Object Info) to stale context.
  // In-context (edit-mode) selections are the drawing target and are kept.
  useEffect(() => {
    if (!DRAW_TOOLS.has(activeTool)) return
    if (activeContext.length > 0) return
    setSelectedIds((cur) => (cur.length > 0 ? [] : cur))
  }, [activeTool, activeContext])

  // ---------------------------------------------------------------- palette: dynamic Model entries
  // Object/group/component/tag names as searchable palette entries. Selecting
  // one "jumps" there: nodes get selected + revealed in the Outliner/Entity
  // Info; tags get revealed + flashed in the Tags panel. Labels match the
  // Outliner exactly (same resolveLabel fallbacks).
  const paletteModelEntries = useMemo((): PaletteEntry[] => {
    const scene = state?.scene
    if (scene == null) return []
    const entries: PaletteEntry[] = []

    // Positional fallback indices ("Object N") must be the Outliner's — the
    // tree numbers nodes within their parent container, not within the flat
    // per-kind id lists these loops iterate.
    const treeIndex = buildTreeIndexMap(
      scene.top_level_nodes().map(nodeRefFromJs),
      (groupId) => scene.group_members(groupId).map(nodeRefFromJs),
    )
    const indexOf = (kind: NodeRef['kind'], id: bigint) =>
      treeIndex.get(nodeKey({ kind, id })) ?? 0

    const pushNode = (kind: NodeRef['kind'], id: bigint, label: string, kindLabel: string) => {
      entries.push({
        id: `jump-node:${kind}:${id}`,
        label: `${kindLabel}: ${label}`,
        description: `Select it and reveal it in the Outliner.`,
        group: 'Model',
        synonyms: [label],
      })
    }
    Array.from(scene.object_ids()).forEach((id) => {
      pushNode('object', id, resolveLabel(scene.object_name(id), undefined, 'object', indexOf('object', id)), 'Object')
    })
    Array.from(scene.group_ids()).forEach((id) => {
      pushNode('group', id, resolveLabel(scene.group_name(id), undefined, 'group', indexOf('group', id)), 'Group')
    })
    Array.from(scene.instance_ids()).forEach((id) => {
      const def = scene.instance_def(id)
      const defName = def !== undefined ? scene.component_name(def) : undefined
      pushNode('instance', id, resolveLabel(scene.instance_name(id), defName, 'instance', indexOf('instance', id)), 'Component')
    })

    // Unique tag paths across all nodes (same walk the Tags panel does).
    const seenTags = new Set<string>()
    const allNodes: { kindNum: number; id: bigint }[] = [
      ...Array.from(scene.object_ids()).map((id) => ({ kindNum: 0, id })),
      ...Array.from(scene.group_ids()).map((id) => ({ kindNum: 1, id })),
      ...Array.from(scene.instance_ids()).map((id) => ({ kindNum: 2, id })),
    ]
    for (const { kindNum, id } of allNodes) {
      for (const rawTag of scene.node_tags(kindNum, id)) {
        const path = rawTag.split('/').map((s) => s.trim()).filter((s) => s.length > 0)
        if (path.length === 0) continue
        const key = tagPathKey(path)
        if (seenTags.has(key)) continue
        seenTags.add(key)
        entries.push({
          id: `jump-tag:${key}`,
          label: `Tag: ${path.join(' / ')}`,
          description: 'Reveal this tag in the Tags panel.',
          group: 'Model',
          synonyms: path,
        })
      }
    }
    return entries
    // docRev: names/tags change on every mutation without changing identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state, docRev])

  /** Active Tags-panel reveal (palette jump); cleared after a short flash. */
  const [revealTag, setRevealTag] = useState<TagReveal | null>(null)
  const revealNonceRef = useRef(0)

  // ---------------------------------------------------------------- shared reset helper
  // Used by New, Open — loads bytes into the scene and resets all UI state.
  const applyLoadedBytes = useCallback((bytes: Uint8Array): boolean => {
    const scene = sceneRef.current
    if (scene === null) return false
    try {
      scene.load(bytes)
    } catch (err) {
      const raw = err instanceof Error ? err.message : String(err)
      if (isPanicError(raw)) {
        setKernelPanicked(true)
      }
      handleToast(friendlyErrorText(err))
      return false
    }
    // Any successful load supersedes the welcome screen — e.g. a warm
    // file-association open delivered while it was showing. The ref covers
    // the other ordering: a load that lands while the startup-handoff effect
    // is still awaiting its recovery check must also veto the offer it makes
    // at the end (the state setters alone can't — the later one wins).
    documentLoadedRef.current = true
    setWelcomeOpen(false)
    setSelectedIds([])
    setActiveContext([])
    // Seed from the just-loaded document's registries rather than clearing —
    // hidden .skp layers/nodes (or a re-opened .hew with tags/nodes previously
    // hidden via the eye toggle) must come up hidden on first render, not
    // visible.
    const seededHiddenKeys = seedHiddenKeysFromRegistry(scene)
    setHiddenKeys(seededHiddenKeys)
    const seededHiddenTagPaths = seedHiddenTagPathsFromRegistry(scene)
    setHiddenTagPaths(seededHiddenTagPaths)
    // Also clear the renderer's hidden set: it keys by dense ids that the new
    // document reuses, so stale ids would silently hide (and un-pick) unrelated
    // objects after a load. (No-op if the viewport isn't mounted yet.)
    viewportApi.current?.setHidden([], [])
    // Suppress dirty-marking while notifyLoaded triggers handleDocumentChanged;
    // the caller will commit the authoritative afterOpen state (dirty=false).
    suppressDirtyRef.current = true
    try {
      viewportApi.current?.notifyLoaded()
    } finally {
      suppressDirtyRef.current = false
    }
    // Push the seeded hides now that the scene is tessellated (notifyLoaded
    // above), so hidden-by-default tags/nodes take effect on first render
    // instead of waiting for the user to touch an eye toggle.
    pushUnionHiddenRef.current(seededHiddenKeys, seededHiddenTagPaths)
    // Frame the freshly-loaded model (Open / Recover / drag-drop / file
    // association all funnel through here). Empty documents (File ▸ New's
    // blank bytes) keep the default framing.
    if (!isSceneEmpty(scene)) {
      requestAnimationFrame(() => viewportApi.current?.zoomExtents())
    }
    return true
  }, [handleToast])
  // Keep the harness's Open path ( __hew_test.load) pointed at the latest
  // applyLoadedBytes closure.
  applyLoadedBytesRef.current = applyLoadedBytes

  // ---------------------------------------------------------------- discard guard
  // Returns true if it's safe to proceed (no unsaved changes, or user confirms).
  // Reads current session state from docSessionRef to stay pure-function-safe.
  const confirmDiscard = useCallback(async (): Promise<boolean> => {
    if (!docSessionRef.current.dirty) return true
    const message = 'You have unsaved changes. Discard them?'
    if (isTauri) {
      const { ask } = await import('@tauri-apps/plugin-dialog')
      return ask(message, { title: 'Unsaved Changes', kind: 'warning' })
    }
    return window.confirm(message)
  }, [])

  // Drop this window's autosave snapshot once its document has actually been
  // discarded (replaced in place, or the window is closing) — a discarded
  // document must not resurface in the next launch's recovery prompt. Called
  // only AFTER the discard is irreversible, never at confirm time: a
  // confirmed-then-cancelled Open leaves the dirty document (and its crash
  // snapshot) untouched. Replace-in-place call sites additionally gate on
  // the pre-replace dirty flag — replacing a clean document discards
  // nothing, and clearing anyway could delete an Escape-deferred startup
  // snapshot on the web's single, unprotected slot (desktop slots are
  // claim-protected by the shell).
  //
  // Disarms the autosave tick first, then waits out any write already in
  // flight — the shell does not order recovery_write against recovery_clear,
  // so an overlapping clear could strand the discarded snapshot (or half of
  // it) on disk. Also used after Save/Save As: the same ordering guarantee
  // applies to clearing the snapshot of a just-saved document.
  const clearRecoverySnapshot = useCallback(async (): Promise<void> => {
    dirtySinceAutosaveRef.current = false
    const inFlight = autosaveWriteRef.current
    await inFlight
    // A newer write while we awaited means the document changed under us
    // (e.g. a first edit to the replacement document re-armed the tick) —
    // the slot no longer holds the discarded document, so leave it alone.
    if (autosaveWriteRef.current !== inFlight) return
    await recoveryStoreRef.current.clear().catch(() => { /* best effort */ })
  }, [])

  // ---------------------------------------------------------------- document lifecycle

  const newDocument = useCallback(async () => {
    const scene = sceneRef.current
    if (scene === null) return
    if (!isPristineDocument(docSessionRef.current, scene)) {
      // Non-pristine (a named file, unsaved edits, or existing geometry):
      // never overwrite it. On the desktop, File ▸ New opens a fresh
      // document window (the macOS staple); the current window keeps its
      // state untouched, so no discard prompt is needed. Only the web
      // build, which can't open OS windows, falls back to replace-in-place.
      // (Note: this is stricter than a bare isSceneEmpty check — a
      // saved-then-emptied document, currentRef set but nothing left in the
      // model, is still not pristine and must not be reused silently.)
      if (isTauri) {
        try {
          const { invoke } = await import('@tauri-apps/api/core')
          await invoke('new_window')
          return
        } catch {
          // fall through to the in-window reset below
        }
      }
      if (!(await confirmDiscard())) return
    }
    // Pristine (or web fallback): reset in place — a pristine document is
    // reused rather than spawning a second empty one.
    const blank = blankBytesRef.current
    if (blank === null) return
    const discardsUnsaved = docSessionRef.current.dirty
    if (applyLoadedBytes(blank)) {
      setDocSession(afterOpen(null, Date.now()))
      if (discardsUnsaved) void clearRecoverySnapshot()
    }
  }, [confirmDiscard, applyLoadedBytes, clearRecoverySnapshot])

  // Show the STL units-chooser modal and resolve with the chosen unit_scale,
  // or null if the user cancels (Escape / backdrop click / Cancel button).
  // A promise-plus-ref-resolver bridges the imperative runImportPick flow
  // below to the declarative modal rendered near the bottom of this
  // component (see the `pendingStlImport` block).
  const promptStlUnits = useCallback((fileName: string): Promise<number | null> => {
    return new Promise((resolve) => {
      // Defense in depth: if a chooser is somehow already open, fail its
      // promise cleanly (null) before taking over the resolver slot, so a
      // clobbered import bails rather than hanging forever.
      if (stlUnitsResolveRef.current !== null) {
        stlUnitsResolveRef.current(null)
      }
      stlUnitsResolveRef.current = resolve
      setPendingStlImport({ name: fileName })
    })
  }, [])

  // The post-pick import steps, shared by openDocument (the unified Open
  // dialog's non-hew branch) and importDocument (File ▸ Import…'s explicit,
  // import-only dialog) — both converge here once a pick is in hand. The
  // caller has already confirmed discard and holds openInFlightRef for the
  // whole gesture; `scene` is the live Scene it captured at the top of that
  // gesture.
  const runImportPick = useCallback(async (scene: Scene, pick: ImportPick) => {
    // STL carries no units of its own — ask before doing any blocking work
    // (unlike the other formats, which get no prompt at all). Cancelling
    // here leaves the current document untouched, same as cancelling the
    // file picker.
    let stlUnitScale = 0.001 // unused for non-STL kinds
    if (pick.kind === 'stl') {
      const chosen = await promptStlUnits(pick.name)
      if (chosen === null) return
      stlUnitScale = chosen
    }

    // Show the overlay BEFORE any blocking work.
    //
    // flushSync forces a synchronous DOM commit so the overlay card is in the
    // DOM immediately, rather than waiting for React's next async render cycle.
    // nextPaint() then waits two animation frames before we freeze the main
    // thread.  One rAF is NOT enough: rAF callbacks run before their frame is
    // painted, so resolving there would let the awaiting continuation start
    // the synchronous import ahead of the paint and the overlay would not
    // show until partway through the freeze.  See nextPaint() in paint.ts.
    //
    // NOTE: import_dae runs synchronously on the main thread, so the CSS
    // spinner animation will freeze while it parses.  The text message still
    // communicates progress.  True smooth animation would require running the
    // import in a Web Worker (future work — needs a SharedArrayBuffer channel
    // to the WASM module).
    flushSync(() => {
      setImportingName(pick.name)
      setIsImporting(true)
    })
    await nextPaint()

    let report: ImportReport
    try {
      // Reset to a blank document first (replace semantics).
      //
      // applyLoadedBytes(blank) calls scene.load(), clears selection/context,
      // and calls notifyLoaded() under suppressDirtyRef so the dirty mark is
      // suppressed — the afterImport state below owns the dirty flag.
      // We must bail if the blank load fails (should never happen in practice).
      const blank = blankBytesRef.current
      if (blank === null) return
      const discardsUnsaved = docSessionRef.current.dirty
      const blankOk = applyLoadedBytes(blank)
      if (!blankOk) return
      // The previous document is gone as of the blank load above (even if
      // the import below fails) — drop its autosave snapshot with it.
      if (discardsUnsaved) void clearRecoverySnapshot()

      // Import into the now-empty document, dispatched by format. STL has no
      // internal object names, so name its Objects from the file stem (the
      // picked basename minus the .stl extension); a blank stem falls back
      // to "Imported" in the kernel.
      const stlStem = pick.name.replace(/\.stl$/i, '').trim() || undefined
      report = (
        pick.kind === 'gltf'
          ? scene.import_gltf(pick.bytes)
          : pick.kind === 'skp'
            ? scene.import_skp(pick.bytes)
            : pick.kind === 'stl'
              ? scene.import_stl(pick.bytes, stlUnitScale, stlStem)
              : scene.import_dae(
                  pick.bytes,
                  Object.keys(pick.images).length > 0 ? pick.images : null,
                )
      ) as ImportReport
    } catch (err: unknown) {
      handleToast(`Import failed: ${friendlyErrorText(err)}`)
      // The blank-document replace above already committed (applyLoadedBytes
      // mutates the live scene directly, with no rollback), even though the
      // import into it just failed — the live scene really is blank now.
      // Reflect that in docSession (as a fresh untitled document, exactly
      // like newDocument's blank reset) rather than leaving it pointing at
      // whatever file/handle was open before this gesture: without this, the
      // next Save would write the now-blank scene straight to that stale
      // path with no prompt, silently destroying a perfectly good file.
      setDocSession(afterOpen(null, Date.now()))
      return
    } finally {
      // Always clear the overlay — even on throw, so it can never get stuck.
      setIsImporting(false)
    }

    // Tessellate the imported objects and update session state.
    //
    // notifyLoaded() calls handleSceneRefresh() which tessellates the new
    // objects and bumps docRev.  suppressDirtyRef is false here so the dirty
    // mark would normally fire — but we immediately set afterImport() which
    // owns dirty=true, so the net effect is correct.
    viewportApi.current?.notifyLoaded()

    // Re-seed hidden tags and hidden nodes from the registries the import just
    // populated — the applyLoadedBytes(blank) call above seeded from an empty
    // document, so hidden-by-default tags/nodes (e.g. a hidden .skp layer or
    // hidden group/component) only show up now that scene.import_* has
    // registered them.
    const seededHiddenKeys = seedHiddenKeysFromRegistry(scene)
    setHiddenKeys(seededHiddenKeys)
    const seededHiddenTagPaths = seedHiddenTagPathsFromRegistry(scene)
    setHiddenTagPaths(seededHiddenTagPaths)
    pushUnionHiddenRef.current(seededHiddenKeys, seededHiddenTagPaths)

    // Commit the session state.
    //
    // afterImport() sets currentRef=null (so Save always prompts — no silent
    // overwrite risk on either WebFileHost or TauriFileHost) and dirty=true.
    // The importedName is used by deriveTitle (window title) and by
    // saveAsDocument's suggested filename.
    setDocSession(afterImport(pick.name, Date.now()))

    setImportReport(report)
    const fmt =
      pick.kind === 'gltf' ? 'glTF' : pick.kind === 'skp' ? 'SKP' : pick.kind === 'stl' ? 'STL' : 'DAE'
    LogStore.log.info('app', `Imported ${fmt}: ${report.objects_created} objects (${report.watertight} solid, ${report.leaky} leaky)`)
    requestAnimationFrame(() => { viewportApi.current?.zoomExtents() })
  }, [handleToast, applyLoadedBytes, clearRecoverySnapshot, promptStlUnits])

  // The unified Open dialog: ONE picker accepting `.hew` plus every import
  // format, dispatching on the extension the user picked (see
  // fileHost.ts's openAny doc comment). A `.hew` pick applies directly; every
  // other kind converges on runImportPick, the same post-pick steps
  // importDocument's explicit, import-only dialog feeds.
  const openDocument = useCallback(async () => {
    const scene = sceneRef.current
    if (scene === null) return
    // Re-entrancy guard: shared with importDocument (see openInFlightRef) —
    // a second gesture started while this one is mid-flight (e.g. Ctrl+K
    // behind the STL units modal) would otherwise orphan it. A ref (not
    // state) so the check is synchronous, before the first await.
    if (openInFlightRef.current) return
    openInFlightRef.current = true
    try {
      // Reuse THIS window only when it's pristine (see isPristineDocument);
      // otherwise the pick lands in a fresh window on the desktop (Tauri),
      // leaving the current document completely untouched. Only the web
      // build, which can't open OS windows, falls back to a guarded
      // replace-in-place — and only that fallback needs the discard prompt:
      // a pristine reuse discards nothing (dirty is false by definition),
      // and the new-window path never touches this window at all.
      const pristine = isPristineDocument(docSessionRef.current, scene)
      const opensNewWindow = !pristine && isTauri
      if (!pristine && !opensNewWindow) {
        if (!(await confirmDiscard())) return
      }

      let pick: OpenPick | null
      try {
        pick = await fileHostRef.current.openAny()
      } catch (err: unknown) {
        handleToast(`Open failed: ${friendlyErrorText(err)}`)
        return
      }
      if (pick === null) return // user cancelled — current document unchanged

      if (opensNewWindow) {
        // Every Tauri pick (hew or import) carries a real filesystem path —
        // hew's `handle`, import kinds' `path` — so this should always
        // resolve; the fallback below only matters against an older shell.
        const path = pick.kind === 'hew' ? (pick.handle as string | null) : (pick.path ?? null)
        if (typeof path === 'string') {
          try {
            const { invoke } = await import('@tauri-apps/api/core')
            await invoke('open_in_new_window', { path })
            return
          } catch {
            // fall through to the guarded in-place replace below
          }
        }
        if (!(await confirmDiscard())) return
      }

      if (pick.kind === 'hew') {
        const discardsUnsaved = docSessionRef.current.dirty
        const ok = applyLoadedBytes(pick.bytes)
        if (!ok) return
        setDocSession(afterOpen({ name: pick.name, handle: pick.handle }, Date.now()))
        if (discardsUnsaved) void clearRecoverySnapshot()
        if (isTauri && typeof pick.handle === 'string') {
          import('@tauri-apps/api/core').then(({ invoke }) =>
            invoke('push_recent', { path: pick.handle as string })
          ).catch(() => { /* ignore */ })
        }
        return
      }

      await runImportPick(scene, pick)
    } finally {
      openInFlightRef.current = false
    }
  }, [confirmDiscard, applyLoadedBytes, handleToast, clearRecoverySnapshot, runImportPick])

  // File ▸ Import…'s explicit, import-only dialog (no `.hew` in its filter) —
  // kept alongside the unified Open dialog above for users who specifically
  // want an import-labeled entry point. Converges on the same runImportPick
  // steps once a pick is in hand.
  const importDocument = useCallback(async () => {
    const scene = sceneRef.current
    if (scene === null) return
    // Re-entrancy guard: shared with openDocument (see openInFlightRef) — a
    // second gesture started while this one is mid-flight (e.g. Ctrl+K ▸
    // "import" ▸ Enter behind the units modal) would otherwise orphan it,
    // hanging it forever and silently discarding its work. Refuse it. A ref
    // (not state) so the check is synchronous, before the first await.
    if (openInFlightRef.current) return
    openInFlightRef.current = true
    try {
      // Same pristine-else-new-window rule as openDocument (see
      // isPristineDocument): reuse THIS window only when it's pristine;
      // otherwise the pick lands in a fresh window on the desktop (Tauri),
      // leaving the current document completely untouched. Only the web
      // build, which can't open OS windows, falls back to a guarded
      // replace-in-place — and only that fallback needs the discard prompt
      // (a pristine reuse discards nothing, and the new-window path never
      // touches this window at all).
      const pristine = isPristineDocument(docSessionRef.current, scene)
      const opensNewWindow = !pristine && isTauri
      if (!pristine && !opensNewWindow) {
        if (!(await confirmDiscard())) return
      }

      // Known limitation: openForImport() couples the picker with the file read
      // (and, for COLLADA on the web, a texture-directory picker AFTER the
      // read), so reading a large file happens before the overlay below can
      // appear.  Separating "picker closed" from "bytes read" needs a FileHost
      // API change; until then the overlay covers everything from here on.
      let pick: ImportPick | null
      try {
        pick = await fileHostRef.current.openForImport()
      } catch (err: unknown) {
        handleToast(`Import failed: ${friendlyErrorText(err)}`)
        return
      }
      if (pick === null) return // user cancelled — current document unchanged

      if (opensNewWindow) {
        // Every Tauri import pick carries a real filesystem path (see
        // openDocument's identical comment) — the fallback below only
        // matters against an older shell.
        if (typeof pick.path === 'string') {
          try {
            const { invoke } = await import('@tauri-apps/api/core')
            await invoke('open_in_new_window', { path: pick.path })
            return
          } catch {
            // fall through to the guarded in-place replace below
          }
        }
        if (!(await confirmDiscard())) return
      }

      await runImportPick(scene, pick)
    } finally {
      openInFlightRef.current = false
    }
  }, [confirmDiscard, handleToast, runImportPick])

  const saveDocument = useCallback(() => {
    const scene = sceneRef.current
    if (scene === null) return
    const bytes = new Uint8Array(scene.save())
    const ref = docSession.currentRef
    fileHostRef.current.save(bytes, ref).then((newRef) => {
      if (newRef === null) return // user cancelled
      setDocSession(afterSave(newRef, Date.now()))
      LogStore.log.info('app', `Saved: ${newRef.name}`)
      // The work is now safely on disk — drop the autosave snapshot.
      void clearRecoverySnapshot()
      if (isTauri && typeof newRef.handle === 'string') {
        import('@tauri-apps/api/core').then(({ invoke }) =>
          invoke('push_recent', { path: newRef.handle as string })
        ).catch(() => { /* ignore */ })
      }
    }).catch((err: unknown) => {
      handleToast(`Save failed: ${friendlyErrorText(err)}`)
    })
  }, [docSession.currentRef, handleToast, clearRecoverySnapshot])

  const saveAsDocument = useCallback(() => {
    const scene = sceneRef.current
    if (scene === null) return
    const bytes = new Uint8Array(scene.save())
    // When saving an imported model (currentRef=null, importedName set), suggest
    // the imported filename with a .hew extension so the user sees a sensible
    // default in the Save As dialog.
    const baseName = docSession.currentRef?.name ?? docSession.importedName ?? 'Untitled'
    const suggestedName = baseName.endsWith('.hew') ? baseName : baseName + '.hew'
    fileHostRef.current.saveAs(bytes, suggestedName).then((newRef) => {
      if (newRef === null) return // user cancelled
      setDocSession(afterSave(newRef, Date.now()))
      LogStore.log.info('app', `Saved as: ${newRef.name}`)
      // The work is now safely on disk — drop the autosave snapshot.
      void clearRecoverySnapshot()
      if (isTauri && typeof newRef.handle === 'string') {
        import('@tauri-apps/api/core').then(({ invoke }) =>
          invoke('push_recent', { path: newRef.handle as string })
        ).catch(() => { /* ignore */ })
      }
    }).catch((err: unknown) => {
      handleToast(`Save As failed: ${friendlyErrorText(err)}`)
    })
  }, [docSession.currentRef, docSession.importedName, handleToast, clearRecoverySnapshot])

  // ---------------------------------------------------------------- open by path (Tauri only)
  // Reads the file at `path` and applies it to the CURRENT window
  // unconditionally — no pristine check, no discard guard. Callers must have
  // already established it's safe: openPath below (the pristine-else-
  // new-window branch) after confirming pristine-ness or an explicit discard,
  // or the guaranteed-fresh window a queued open_in_new_window/take_pending_open
  // delivery lands in. Drag-drop, recents, and file-association/second-instance
  // opens only ever hand openPath `.hew` paths (each filters/records only that
  // extension upstream), but a pick delivered to a new window can be any
  // format the unified Open dialog accepts, so this dispatches on extension
  // the same way openDocument does.
  const openPathInPlace = useCallback(async (path: string) => {
    const scene = sceneRef.current
    if (scene === null) return
    const { invoke } = await import('@tauri-apps/api/core')
    if (/\.hew$/i.test(path)) {
      const buf = await invoke<ArrayBuffer>('read_file', { path })
      const bytes = new Uint8Array(buf)
      const discardsUnsaved = docSessionRef.current.dirty
      if (applyLoadedBytes(bytes)) {
        setDocSession(afterOpen({ name: basenameOf(path), handle: path }, Date.now()))
        if (discardsUnsaved) void clearRecoverySnapshot()
        invoke('push_recent', { path }).catch(() => { /* ignore */ })
      }
      return
    }
    const buf = await invoke<ArrayBuffer>('read_file', { path })
    const bytes = new Uint8Array(buf)
    const { resolveImportPickFromPath } = await import('./io/tauriFileHost')
    const pick = await resolveImportPickFromPath(path, bytes)
    await runImportPick(scene, pick)
  }, [applyLoadedBytes, clearRecoverySnapshot, runImportPick])

  // Tauri-only. The entry point for every "open this path" gesture that can
  // land on a window the user may already be working in: native/in-app "Open
  // Recent", native drag-drop, and a live file-association or second-instance
  // open delivered via the `menu-open-path` listener below (which also covers
  // a warm macOS "open document" Apple event and a native recent-file click —
  // see that listener's own doc comment). It also covers the two
  // guaranteed-fresh-window deliveries in the startup-handoff effect
  // (open_in_new_window's queued path, and a cold-start take_pending_open
  // path): a freshly spawned window is pristine by construction, so the
  // pristine check below reduces to the same in-place apply those deliveries
  // always had — this is not a special case, it just falls out of the rule.
  //
  // Follows the same pristine-else-new-window rule as openDocument (see
  // isPristineDocument): a non-pristine window is never silently replaced —
  // the path opens in a fresh window instead, leaving this window's document
  // completely untouched. Only a pristine reuse (or an invoke failure against
  // an older shell) proceeds to replace in place; the discard prompt is
  // reachable ONLY from that fallback, never from an ordinary new-window open
  // (nothing is discarded when a new window opens).
  const openPath = useCallback(async (path: string) => {
    const scene = sceneRef.current
    if (scene === null) return
    const pristine = isPristineDocument(docSessionRef.current, scene)
    if (!pristine) {
      try {
        const { invoke } = await import('@tauri-apps/api/core')
        await invoke('open_in_new_window', { path })
        return
      } catch {
        // Older shell without the command, or the invoke itself failed —
        // fall through to a guarded in-place replace, same as openDocument.
      }
      if (!(await confirmDiscard())) return
    }
    await openPathInPlace(path)
  }, [confirmDiscard, openPathInPlace])

  // ---------------------------------------------------------------- Open Recent (in-app menu; Linux web menu,  port)
  // The recents list is owned by the Rust shell (recents.json). The native
  // macOS/Windows menu has its own "Open Recent" submenu; the in-app web menu
  // (used on Linux + web) reads the list via `get_recents` and re-fetches after
  // any open/save/import (those change the session AND call `push_recent`).
  const [recentFiles, setRecentFiles] = useState<string[]>([])
  useEffect(() => {
    if (!isTauri) return
    let cancelled = false
    import('@tauri-apps/api/core')
      .then(({ invoke }) => invoke<string[]>('get_recents'))
      .then((list) => { if (!cancelled) setRecentFiles(list) })
      .catch(() => { /* ignore */ })
    return () => { cancelled = true }
  }, [docSession.currentRef, docSession.importedName])

  // A recent entry can go stale (file moved or deleted since); surface the
  // failure instead of letting the rejection vanish as an unhandled promise —
  // "clicking did nothing" is the worst outcome for a welcome-screen row.
  const openRecent = useCallback((path: string) => {
    openPath(path).catch(() => {
      handleToast(`Couldn't open ${basenameOf(path)} — the file may have been moved or deleted.`)
    })
  }, [openPath, handleToast])

  // ---------------------------------------------------------------- bundled samples
  // Fetched from the app's own assets (app/public/samples/, generated by
  // `cargo run -p kernel --example build_samples`). A sample opens like an
  // import: no file handle, so Save always prompts for a location.
  const openSample = useCallback(async (sample: SampleEntry) => {
    try {
      const res = await fetch(`samples/${sample.file}`)
      if (!res.ok) throw new Error(`HTTP ${res.status}`)
      const bytes = new Uint8Array(await res.arrayBuffer())
      if (applyLoadedBytes(bytes)) {
        setDocSession(afterImport(sample.title, Date.now()))
        LogStore.log.info('app', `Opened sample ${sample.file}`)
      }
    } catch (err: unknown) {
      handleToast(`Couldn't open the sample: ${friendlyErrorText(err)}`)
    }
  }, [applyLoadedBytes, handleToast])
  const clearRecent = useCallback(() => {
    import('@tauri-apps/api/core')
      .then(({ invoke }) => invoke('clear_recent'))
      .then(() => setRecentFiles([]))
      .catch(() => { /* ignore */ })
  }, [])

  // ---------------------------------------------------------------- recovery prompt actions

  /** Load a claimed snapshot into THIS window's document and session. Shared
   *  by the startup dialog (first/newest snapshot) and by recovery windows
   *  spawned for the remaining snapshots (take_pending_recovery below). */
  const adoptSnapshot = useCallback((snapshot: RecoverySnapshot): boolean => {
    const ok = applyLoadedBytes(snapshot.bytes)
    if (!ok) return false
    const { meta } = snapshot
    setDocSession({
      currentRef: meta.path !== null ? { name: meta.name, handle: meta.path } : null,
      dirty: true,
      importedName: meta.path !== null ? undefined : meta.name,
      // meta.savedAt is when the autosave snapshot was written — the best
      // available lower bound for "edits existed as of here".
      // lastSavedAt stays null: this snapshot was never actually written to
      // the real file yet, only to the recovery slot.
      lastEditAt: meta.savedAt,
      lastSavedAt: null,
    })
    // The recovered document still only exists in the recovery snapshot —
    // claim() re-homed it to this window's own slot (the next autosave tick
    // refreshes it in place); mark dirty-since-autosave so a tick will
    // actually fire if nothing else changes.
    dirtySinceAutosaveRef.current = true
    return true
  }, [applyLoadedBytes])
  const adoptSnapshotRef = useRef(adoptSnapshot)
  useEffect(() => { adoptSnapshotRef.current = adoptSnapshot }, [adoptSnapshot])

  const handleRecover = useCallback(() => {
    const listings = recoveryPrompt
    if (listings === null || listings.length === 0) return
    void (async () => {
      // One snapshot loads here; every other one opens in its own window
      // (new_window parks the slot for the new webview to claim at mount).
      // Claiming re-homes each snapshot, so nothing is shadowed, overwritten,
      // or silently discarded — with N crashed documents, all N come back.
      //
      // This window prefers its OWN slot when one exists (the dialog only
      // runs in the window labeled "main"): claiming a different slot while
      // recovery-main.hew still holds an unclaimed document would force the
      // shell's no-clobber fallbacks instead of the clean rename path.
      const ownIdx = listings.findIndex((l) => l.slot === 'main')
      const ordered =
        ownIdx > 0
          ? [listings[ownIdx], ...listings.filter((_, i) => i !== ownIdx)]
          : listings
      const [first, ...rest] = ordered
      try {
        const snapshot = await recoveryStoreRef.current.claim(first.slot)
        if (snapshot !== null) adoptSnapshot(snapshot)
      } catch {
        /* claim failed — leave this window blank rather than guessing */
      }
      if (isTauri && rest.length > 0) {
        try {
          const { invoke } = await import('@tauri-apps/api/core')
          for (const listing of rest) {
            await invoke('new_window', { recoverSlot: listing.slot })
          }
        } catch {
          /* window spawn failed — the slots stay on disk for next launch */
        }
      }
    })()
    setRecoveryPrompt(null)
  }, [recoveryPrompt, adoptSnapshot])

  const handleDiscardRecovery = useCallback(() => {
    // The dialog listed every snapshot, so this is an informed Discard All —
    // per-window saves use clear(), which drops only that window's own slot.
    recoveryStoreRef.current.discardAll().catch(() => { /* ignore */ })
    setRecoveryPrompt(null)
  }, [])

  // Escape closes the prompt WITHOUT clearing — the snapshot survives and is
  // re-offered next launch, so an accidental keypress can't lose work.
  const handleDismissRecovery = useCallback(() => {
    setRecoveryPrompt(null)
  }, [])

  // ---------------------------------------------------------------- undo/redo for Edit menu
  // Tag-visibility resync is NOT called here: it hangs off the viewport's
  // onHistoryChanged (handleHistoryChanged below), the choke point shared by
  // every undo/redo entry point — menu, palette, AND the viewport's own
  // Cmd+Z/Cmd+Shift+Z keydown, which never passes through these handlers.
  const handleUndo = useCallback(() => {
    viewportApi.current?.runUndo()
  }, [])

  const handleRedo = useCallback(() => {
    viewportApi.current?.runRedo()
  }, [])

  // Fired by the viewport after ANY successful undo/redo. Undoing/redoing a
  // tag deletion changes the tag registry (and its hidden flags): re-seed so
  // content hidden solely via a restored hidden tag re-hides — and stays
  // unpickable — no matter which entry point drove the history step.
  const handleHistoryChanged = useCallback(() => {
    resyncTagVisibilityRef.current()
  }, [])

  // ---------------------------------------------------------------- zoom extents
  const handleZoomExtents = useCallback(() => {
    viewportApi.current?.zoomExtents()
  }, [])

  // ---------------------------------------------------------------- glTF export
  const exportGltf = useCallback(async () => {
    const api = viewportApi.current
    if (api === null) {
      handleToast('Export failed: viewport not ready.')
      return
    }
    let bytes: Uint8Array | null
    try {
      bytes = await api.exportGlb()
    } catch (err: unknown) {
      handleToast(`Export failed: ${friendlyErrorText(err)}`)
      return
    }
    if (bytes === null) {
      handleToast('Nothing to export — the model has no solids.')
      return
    }
    // Suggest a name derived from the current document, dropping any .hew suffix.
    const rawBase = docSession.currentRef?.name ?? docSession.importedName ?? 'Untitled'
    const base = rawBase.replace(/\.hew$/i, '')
    try {
      const ok = await fileHostRef.current.exportBinary(bytes, base, {
        description: 'glTF Binary',
        ext: 'glb',
        mime: 'model/gltf-binary',
      })
      if (ok) {
        handleToast('Exported glTF.')
        LogStore.log.info('app', `Exported glTF (${bytes.length} bytes)`)
      }
    } catch (err: unknown) {
      handleToast(`Export failed: ${friendlyErrorText(err)}`)
    }
  }, [docSession.currentRef, docSession.importedName, handleToast])

  // ------------------------------------------------- slicer exports (STL/3MF)
  // Non-solid objects pending the solid-gating confirmation, plus which
  // format's export is waiting on it; null = no dialog. STL and 3MF feed
  // slicers, so both warn — never repair (rule 4) — when any exported object
  // is not a watertight solid.
  const [solidWarning, setSolidWarning] = useState<
    { format: 'stl' | '3mf'; names: string[] } | null
  >(null)

  /** Curve resolution (segments per full turn; 0 = stored facets) chosen in
   *  the Export dialog, held across the solid-gating confirmation step. */
  const stlSegmentsRef = useRef(48)

  /** The actual export — runs directly when all objects are solid, or after
   *  "Export Anyway" in the gating dialog. */
  const doExportStl = useCallback(async () => {
    const api = viewportApi.current
    if (api === null) {
      handleToast('Export failed: viewport not ready.')
      return
    }
    let result: Awaited<ReturnType<typeof api.exportStl>>
    try {
      result = await api.exportStl(stlSegmentsRef.current)
    } catch (err: unknown) {
      handleToast(`Export failed: ${friendlyErrorText(err)}`)
      return
    }
    if (result === null) {
      handleToast('Nothing to export — the model has no solids.')
      return
    }
    // Suggest a name derived from the current document, dropping any .hew suffix.
    const rawBase = docSession.currentRef?.name ?? docSession.importedName ?? 'Untitled'
    const base = rawBase.replace(/\.hew$/i, '')
    try {
      const ok = await fileHostRef.current.exportBinary(result.bytes, base, {
        description: 'STL (Binary)',
        ext: 'stl',
        mime: 'model/stl',
      })
      if (ok) {
        handleToast('Exported STL.')
        LogStore.log.info(
          'app',
          `Exported STL (${result.triangleCount} triangles, ${result.bytes.length} bytes` +
            (result.skippedDegenerate > 0
              ? `, ${result.skippedDegenerate} degenerate triangles skipped)`
              : ')'),
        )
      }
    } catch (err: unknown) {
      handleToast(`Export failed: ${friendlyErrorText(err)}`)
    }
  }, [docSession.currentRef, docSession.importedName, handleToast])

  /** The actual 3MF export — runs directly when all objects are solid, or
   *  after "Export Anyway" in the gating dialog. */
  const doExport3mf = useCallback(async () => {
    const api = viewportApi.current
    if (api === null) {
      handleToast('Export failed: viewport not ready.')
      return
    }
    let result: Awaited<ReturnType<typeof api.export3mf>>
    try {
      result = await api.export3mf()
    } catch (err: unknown) {
      handleToast(`Export failed: ${friendlyErrorText(err)}`)
      return
    }
    if (result === null) {
      handleToast('Nothing to export — the model has no solids.')
      return
    }
    // Suggest a name derived from the current document, dropping any .hew suffix.
    const rawBase = docSession.currentRef?.name ?? docSession.importedName ?? 'Untitled'
    const base = rawBase.replace(/\.hew$/i, '')
    try {
      const ok = await fileHostRef.current.exportBinary(result.bytes, base, {
        description: '3MF',
        ext: '3mf',
        mime: 'model/3mf',
      })
      if (ok) {
        handleToast('Exported 3MF.')
        LogStore.log.info(
          'app',
          `Exported 3MF (${result.objectCount} parts, ${result.triangleCount} triangles, ` +
            `${result.bytes.length} bytes` +
            (result.skippedDegenerate > 0
              ? `, ${result.skippedDegenerate} degenerate triangles skipped)`
              : ')'),
        )
      }
    } catch (err: unknown) {
      handleToast(`Export failed: ${friendlyErrorText(err)}`)
    }
  }, [docSession.currentRef, docSession.importedName, handleToast])

  /** Entry point (Export dialog, STL/3MF formats): gate on solid status
   *  first. STL carries its chosen curve resolution (segments per turn)
   *  into `stlSegmentsRef`, held across the confirmation step. */
  const exportSolidGated = useCallback(async (format: 'stl' | '3mf', stlSegmentsPerTurn?: number) => {
    if (stlSegmentsPerTurn !== undefined) stlSegmentsRef.current = stlSegmentsPerTurn
    const scene = sceneRef.current
    const offenders = scene !== null ? collectNonSolidObjects(scene) : []
    if (offenders.length > 0) {
      setSolidWarning({ format, names: offenders.map((o) => o.name) })
      return
    }
    await (format === 'stl' ? doExportStl() : doExport3mf())
  }, [doExportStl, doExport3mf])

  // ---------------------------------------------------------------- unified Export dialog
  // File ▸ Export… opens ONE dialog with a Format select (glTF/STL/3MF) —
  // the dialog's Export dispatches to the picked format; the slicer formats'
  // solid-gating dialog remains the follow-on step (chain unchanged).
  const [exportDialogOpen, setExportDialogOpen] = useState(false)

  const handleExportFormat = useCallback((format: ExportFormat, stlSegmentsPerTurn: number) => {
    setExportDialogOpen(false)
    if (format === 'glb') {
      void exportGltf()
    } else {
      void exportSolidGated(format, format === 'stl' ? stlSegmentsPerTurn : undefined)
    }
  }, [exportGltf, exportSolidGated])

  // ---------------------------------------------------------------- settings window
  // Per-platform settings surface:
  //  - Windows desktop: a full-window in-app page in the Windows 11 app-
  //    settings idiom (back arrow + settings cards, like Notepad/Paint) —
  //    FluentSettingsPage below.
  //  - macOS/Linux desktop: a separate, free-floating OS webview window
  //    (macOS HIG expects a standalone settings window).
  //  - Web: no concept of a second OS window — an in-app modal
  //    (showSettingsModal below).
  const openSettings = useCallback(() => {
    if (isTauri && isWindows) {
      setShowFluentSettings(true)
      return
    }
    if (isTauri) {
      // Created by the shell (open_settings_window) rather than the JS
      // WebviewWindow API, so the capability set carries no window-creation
      // grants — a compromised webview must not be able to mint windows.
      import('@tauri-apps/api/core')
        .then(({ invoke }) => invoke('open_settings_window'))
        .catch(() => { /* ignore */ })
      return
    }
    setShowSettingsModal(true)
  }, [])

  // ---------------------------------------------------------------- stable refs for Tauri event listeners
  // These refs always track the latest callbacks so Tauri event handlers
  // (registered once) don't capture stale closures.
  const newDocumentRef = useRef(newDocument)
  const openDocumentRef = useRef(openDocument)
  const importDocumentRef = useRef(importDocument)
  const saveDocumentRef = useRef(saveDocument)
  const saveAsDocumentRef = useRef(saveAsDocument)
  const handleUndoRef = useRef(handleUndo)
  const handleRedoRef = useRef(handleRedo)
  const handleZoomExtentsRef = useRef(handleZoomExtents)
  const openPathRef = useRef(openPath)
  const openSettingsRef = useRef(openSettings)
  useEffect(() => { newDocumentRef.current = newDocument }, [newDocument])
  useEffect(() => { openDocumentRef.current = openDocument }, [openDocument])
  useEffect(() => { importDocumentRef.current = importDocument }, [importDocument])
  useEffect(() => { saveDocumentRef.current = saveDocument }, [saveDocument])
  useEffect(() => { saveAsDocumentRef.current = saveAsDocument }, [saveAsDocument])
  useEffect(() => { handleUndoRef.current = handleUndo }, [handleUndo])
  useEffect(() => { handleRedoRef.current = handleRedo }, [handleRedo])
  useEffect(() => { handleZoomExtentsRef.current = handleZoomExtents }, [handleZoomExtents])
  useEffect(() => { openPathRef.current = openPath }, [openPath])
  useEffect(() => { openSettingsRef.current = openSettings }, [openSettings])

  // ---------------------------------------------------------------- native menu-action dispatch
  // The dispatch switch lives in a ref refreshed every render. The listener
  // below is registered ONCE (so it survives StrictMode and re-renders without
  // re-subscribing), but Vite Fast Refresh re-renders this component WITHOUT
  // re-running a []-deps effect — so a switch captured inside that effect would
  // be frozen at its first-mount version and silently drop any case added later
  // (e.g. a newly-added pane toggle). Routing through this ref keeps it current.
  // Whole-node delete: deletes every currently-selected node
  // (Object/Group/Instance) and clears the selection. Shared by the Edit ▸
  // Delete menu item and the Delete/Backspace key handler below — both
  // dispatch through `menuActionRef.current('edit-delete')` so this always
  // sees the current `selectedIds` (the ref is reassigned fresh every render;
  // see the Fast-Refresh note above `menuActionRef`).
  const deleteSelection = () => {
    // A selected guide deletes via the same path; mutually exclusive
    // with node selection.
    if (selectedGuide !== null) {
      viewportApi.current?.runDeleteGuide(selectedGuide)
      setSelectedGuide(null)
      setDocRev((r) => r + 1)
      return
    }
    if (selectedIds.length === 0) return
    viewportApi.current?.runDelete(selectedIds)
    setSelectedIds([])
    setDocRev((r) => r + 1)
  }

  const menuActionRef = useRef<(payload: string) => void>(() => {})
  menuActionRef.current = (payload: string) => {
    // Palette "jump" entries (dynamic Model group) carry their target in the
    // id. Nodes: select + reveal in the Outliner/Object Info (the tree
    // scrolls its primary selection into view). Tags: reveal + flash in the
    // Tags panel.
    if (payload.startsWith('jump-node:')) {
      const [, kindStr, idStr] = payload.split(':')
      const kind = kindStr as NodeRef['kind']
      if ((kind === 'object' || kind === 'group' || kind === 'instance') && /^\d+$/.test(idStr)) {
        setSelectedGuide(null)
        setSelectedIds([{ kind, id: BigInt(idStr) }])
        setShowModelInfo(true)
        setShowObjectInfo(true)
      }
      return
    }
    if (payload.startsWith('jump-tag:')) {
      const key = payload.slice('jump-tag:'.length)
      setShowTags(true)
      const nonce = ++revealNonceRef.current
      setRevealTag({ key, nonce })
      // Let the highlight fade back out unless another jump superseded it.
      setTimeout(() => {
        setRevealTag((cur) => (cur?.nonce === nonce ? null : cur))
      }, 2000)
      return
    }
    switch (payload) {
      case 'new':      newDocumentRef.current(); break
      case 'open':     openDocumentRef.current(); break
      case 'import':   importDocumentRef.current(); break
      case 'export':   setExportDialogOpen(true); break
      case 'save':     saveDocumentRef.current(); break
      case 'save-as':  saveAsDocumentRef.current(); break
      case 'undo':     handleUndoRef.current(); break
      case 'redo':     handleRedoRef.current(); break
      case 'edit-select-all': viewportApi.current?.selectAll(); break
      case 'edit-delete': deleteSelection(); break
      case 'close':
        // Trigger the beforeunload / close-guard path by emitting the
        // Tauri window close request — handled by the close guard effect.
        import('@tauri-apps/api/window').then(({ getCurrentWindow }) => {
          getCurrentWindow().close().catch(() => { /* ignore */ })
        }).catch(() => { /* ignore */ })
        break
      // Tool activations from native menu
      case 'tool-select':    setActiveTool('Select'); break
      case 'tool-rectangle': setActiveTool('Rectangle'); break
      case 'tool-circle':    setActiveTool('Circle'); break
      case 'tool-polygon':   setActiveTool('Polygon'); break
      case 'tool-arc':       setActiveTool('Arc'); break
      case 'tool-line':      setActiveTool('Line'); break
      case 'tool-pushpull':  setActiveTool('Push/Pull'); break
      case 'tool-follow-me': setActiveTool('Follow Me'); break
      case 'tool-offset':    setActiveTool('Offset'); break
      case 'tool-paint':     setActiveTool('Paint'); break
      case 'tool-move':      setActiveTool('Move'); break
      case 'tool-rotate':    setActiveTool('Rotate'); break
      case 'tool-scale':     setActiveTool('Scale'); break
      case 'tool-tape-measure': setActiveTool('Tape Measure'); break
      case 'tool-protractor': setActiveTool('Protractor'); break
      case 'tool-slice':     setActiveTool('Slice'); break
      case 'tool-section-plane': setActiveTool('Section Plane'); break
      case 'tool-edit-vertex': setActiveTool('Edit Vertex'); break
      case 'tool-orbit':     setActiveTool('Orbit'); break
      case 'tool-pan':       setActiveTool('Pan'); break
      case 'tool-zoom':      setActiveTool('Zoom'); break
      // Window pane toggles — must use functional updaters (StrictMode safe)
      case 'toggle-model-info':   setShowModelInfo((v) => !v); break
      case 'toggle-materials':    setShowMaterials((v) => !v); break
      case 'toggle-tags':         setShowTags((v) => !v); break
      case 'toggle-object-info':  setShowObjectInfo((v) => !v); break
      case 'toggle-debug-log':    setShowDebugLog((v) => !v); break
      case 'toggle-axes':         setShowAxes((v) => !v); break
      case 'toggle-grid':         setShowGrid((v) => !v); break
      case 'toggle-guides':       setShowGuides((v) => !v); break
      case 'toggle-section-active': viewportApi.current?.toggleSectionActive(); break
      case 'edit-delete-guides':  viewportApi.current?.deleteAllGuides(); break
      case 'zoom-extents':        handleZoomExtentsRef.current(); break
      case 'view-top':            viewportApi.current?.setStandardView('top'); break
      case 'view-bottom':         viewportApi.current?.setStandardView('bottom'); break
      case 'view-front':          viewportApi.current?.setStandardView('front'); break
      case 'view-back':           viewportApi.current?.setStandardView('back'); break
      case 'view-left':           viewportApi.current?.setStandardView('left'); break
      case 'view-right':          viewportApi.current?.setStandardView('right'); break
      case 'view-iso':            viewportApi.current?.setStandardView('iso'); break
      case 'open-settings':       openSettingsRef.current(); break
      case 'report-bug': handleReportBug(); break
      case 'open-palette':        setPaletteOpen(true); break
      // Contextual dock only — these need the current selection, not
      // just a bare trigger, so unlike every case above they aren't also
      // reachable from a static native-menu item; the dock and (once
      // adds outliner actions there too) other selection-aware UI are the
      // only callers. handleUngroup/handleMakeUnique already read
      // `selectedIds` from their own closure (no args needed); this case is
      // defined before their `const` declarations later in this render, but
      // the switch only ever *runs* on a later click, by which point this
      // render has fully executed and the closure sees the real function.
      case 'enter-context':
        if (selectedIds.length === 1) handleEnterContext(selectedIds[0])
        break
      case 'ungroup': handleUngroup(); break
      case 'make-unique': handleMakeUnique(); break
      case 'explode-instance': handleExplodeInstance(); break
      // Edit-menu object commands (also reachable from the contextual dock
      // via the aliases above). Selection-gated: the native menu items are
      // enabled/disabled through sync_menu_state, and the handlers themselves
      // re-check the selection, so a stale click is a no-op.
      case 'edit-group': handleGroup(); break
      case 'edit-ungroup': handleUngroup(); break
      case 'edit-make-component': handleMakeComponent(); break
      case 'edit-place-copy': handlePlaceInstance(); break
      case 'edit-explode': handleExplodeInstance(); break
      case 'edit-make-unique': handleMakeUnique(); break
      case 'edit-union': handleBoolean(0); break
      case 'edit-subtract': handleBoolean(1); break
      case 'edit-intersect': handleBoolean(2); break
    }
  }

  // ---------------------------------------------------------------- native menu-action listener (Tauri only)
  // Registered once; dispatches through menuActionRef so the handler is always
  // the latest one (HMR/StrictMode safe).
  useEffect(() => {
    if (!isTauri) return
    let unlisten: (() => void) | undefined
    let cancelled = false
    // Window-scoped listener (NOT the module-level `listen`, whose Any
    // target also receives events emit_to'd at OTHER document windows —
    // with File ▸ New's multi-window support, a menu action must only run
    // in the window it was routed to).
    import('@tauri-apps/api/webviewWindow').then(({ getCurrentWebviewWindow }) => {
      return getCurrentWebviewWindow().listen<string>('menu-action', (event) => {
        menuActionRef.current(event.payload)
      })
    }).then((fn) => { if (cancelled) fn(); else unlisten = fn }).catch(() => { /* ignore if not in Tauri */ })
    return () => { cancelled = true; unlisten?.() }
  }, [])

  // ---------------------------------------------------------------- menu-open-path listener (Tauri only)
  // Emitted by Rust when a recent-file menu item is clicked, or when a file
  // is opened via the macOS "open document" Apple event (warm case). The
  // startup-handoff effect below waits for this registration before telling
  // the shell the window is ready for live delivery.
  useEffect(() => {
    if (!isTauri) return
    let unlisten: (() => void) | undefined
    let cancelled = false
    // Window-scoped for the same multi-window reason as menu-action above.
    import('@tauri-apps/api/webviewWindow').then(({ getCurrentWebviewWindow }) => {
      return getCurrentWebviewWindow().listen<string>('menu-open-path', (event) => {
        openPathRef.current(event.payload)
      })
    }).then((fn) => {
      if (cancelled) { fn(); return }
      unlisten = fn
      openListenerReadyRef.current?.resolve()
    }).catch(() => {
      // Registration failed — resolve anyway so the handoff effect (which
      // only needs the poll path in that case) can proceed.
      openListenerReadyRef.current?.resolve()
    })
    return () => { cancelled = true; unlisten?.() }
  }, []) // openPath accessed via openPathRef — no dep needed

  // ---------------------------------------------------------------- open-window list (Tauri only)
  // Feeds the in-app MenuBar's Window-menu tail (Windows/Linux — macOS's
  // native menu renders its own tail shell-side). `list_windows` seeds this
  // window's initial view; the shell's `window-list` broadcast (emitted to
  // every window on create/destroy/focus/title-change — see
  // refresh_window_list in main.rs) keeps it live after that. A plain
  // (non-window-scoped) listener is correct here, unlike menu-action/
  // menu-open-path above: the list is identical for every window and
  // reading it is never destructive.
  useEffect(() => {
    if (!isTauri) return
    let unlisten: (() => void) | undefined
    let cancelled = false
    // Register the listener BEFORE doing any fetch. The reverse order (fetch
    // and listen started in parallel) raced this window's OWN startup
    // delivery: a freshly spawned window's docSession-title-push effect
    // (below) can invoke `set_window_title` — broadcasting the corrected
    // title — before this window's own `window-list` listener has finished
    // its dynamic import + subscribe round trip. That broadcast would then
    // be silently dropped (nobody listening yet), with nothing to correct it
    // until the next focus/create/destroy event fires a fresh one — exactly
    // the "stays Untitled until focused" symptom. Listening first means
    // nothing between mount and "listener ready" can be missed: anything
    // already committed shell-side by then is picked up by the catch-up
    // fetch below, and anything after is captured live by the listener.
    let broadcastSeenSinceListening = false
    import('@tauri-apps/api/event').then(({ listen }) =>
      listen<{ label: string; title: string; focused: boolean }[]>('window-list', (event) => {
        broadcastSeenSinceListening = true
        setWindowList(event.payload)
      }),
    ).then((fn) => {
      if (cancelled) { fn(); return }
      unlisten = fn
      // Catch-up fetch, now that the listener is definitely live. Guarded
      // the same way a fetch-vs-broadcast race is always guarded here: a
      // broadcast landing in the gap between "listener registered" and
      // "this fetch resolves" must win, not be clobbered by a slower,
      // now-stale snapshot.
      import('@tauri-apps/api/core')
        .then(({ invoke }) => invoke<{ label: string; title: string; focused: boolean }[]>('list_windows'))
        .then((list) => { if (!cancelled && !broadcastSeenSinceListening) setWindowList(list) })
        .catch(() => { /* older shell without the command — ignore */ })
    }).catch(() => { /* not in Tauri, or registration failed */ })
    return () => { cancelled = true; unlisten?.() }
  }, [])

  const focusWindow = useCallback((label: string) => {
    import('@tauri-apps/api/core')
      .then(({ invoke }) => invoke('focus_window', { label }))
      .catch(() => { /* ignore */ })
  }, [])

  // ---------------------------------------------------------------- startup handoff: recovery slot, buffered open, recovery prompt
  // Runs once, after the scene first becomes available (adopting a snapshot
  // or opening a buffered path into a still-loading kernel would silently
  // no-op), in strict order:
  //
  //  1. take_pending_recovery — this window was spawned to recover a
  //     specific crash snapshot; claim it and skip everything else.
  //  2. take_pending_window_open — this window was spawned by
  //     open_in_new_window (a File ▸ Open pick onto a non-pristine window);
  //     claim its queued path and skip everything else.
  //  3. take_pending_open — a cold-start file association buffered a path
  //     before this webview could listen; open it and skip the prompt (the
  //     user explicitly asked for that document).
  //  4. frontend_ready — from here on the shell delivers opens live (the
  //     open-path listener registration is awaited above, and the polls
  //     have drained the buffer, so nothing can be delivered twice or lost).
  //  5. Only then, and only in the primary window with nothing loaded, the
  //     crash-recovery offer. Ordering it after 1–3 (rather than racing
  //     them from a separate effect) is what keeps the dialog from
  //     appearing over — and its Recover then replacing — a document the
  //     user just double-clicked.
  //
  // Secondary document windows never prompt: a sibling window's live
  // autosave must not be "recovered" into them.
  const pendingHandoffCheckedRef = useRef(false)
  useEffect(() => {
    if (state === null || pendingHandoffCheckedRef.current) return
    pendingHandoffCheckedRef.current = true
    void (async () => {
      if (isTauri) {
        try {
          await openListenerReadyRef.current?.promise
          const { invoke } = await import('@tauri-apps/api/core')
          const slot = await invoke<string | null>('take_pending_recovery')
          const windowOpenPath =
            slot == null ? await invoke<string | null>('take_pending_window_open') : null
          const path =
            slot == null && windowOpenPath == null
              ? await invoke<string | null>('take_pending_open')
              : null
          invoke('frontend_ready').catch(() => { /* older shell — ignore */ })
          if (slot != null) {
            const snapshot = await recoveryStoreRef.current.claim(slot)
            if (snapshot !== null) adoptSnapshotRef.current(snapshot)
            return
          }
          if (windowOpenPath != null) {
            await openPathRef.current(windowOpenPath)
            return
          }
          if (path != null) {
            await openPathRef.current(path)
            return
          }
          const { getCurrentWindow } = await import('@tauri-apps/api/window')
          if (getCurrentWindow().label !== 'main') return
        } catch {
          /* not fatal — fall through to the recovery check */
        }
      }
      try {
        const listings = await recoveryStoreRef.current.list()
        if (shouldPromptRecovery(docSessionRef.current, listings)) {
          setRecoveryPrompt(listings)
          return
        }
      } catch {
        /* ignore — no recovery prompt */
      }
      // Bare launch confirmed: nothing pending, nothing to recover, primary
      // window. Offer the welcome screen (unless the user turned it off, or
      // a live file-open landed while the recovery check above was in flight).
      if (getShowWelcome() && !documentLoadedRef.current) setWelcomeOpen(true)
    })()
  }, [state]) // runs once, after the kernel is ready; handlers accessed via refs

  // ---------------------------------------------------------------- close guard (Tauri only)
  // Intercepts the native window close to warn about unsaved changes.
  //
  // Tauri v2 cannot honor a non-prevent decision made *after* an await: by the
  // time the async handler resolves, the synchronous prevent window has closed.
  // Fix: always call event.preventDefault() immediately (synchronously), then
  // explicitly call win.destroy() when we decide to close.  win.destroy() force-
  // closes the window bypassing onCloseRequested, so there is no re-entrancy loop.
  useEffect(() => {
    if (!isTauri) return
    let unlisten: (() => void) | undefined
    let cancelled = false
    import('@tauri-apps/api/window').then(({ getCurrentWindow }) => {
      const win = getCurrentWindow()
      return win.onCloseRequested(async (event) => {
        // Always prevent the default close; we decide explicitly below.
        event.preventDefault()
        if (docSessionRef.current.dirty) {
          const { ask } = await import('@tauri-apps/plugin-dialog')
          const ok = await ask(
            'You have unsaved changes. Discard them and close?',
            { title: 'Unsaved Changes', kind: 'warning' },
          )
          if (!ok) return // keep the window open
          // The user chose to discard — the close makes it irreversible, so
          // drop this window's autosave snapshot before the webview (and its
          // ability to invoke the shell) is destroyed.
          await clearRecoverySnapshot()
        }
        // Force-close, bypassing onCloseRequested (no loop).
        await win.destroy()
      })
    }).then((fn) => { if (cancelled) fn(); else unlisten = fn }).catch(() => { /* ignore */ })
    return () => { cancelled = true; unlisten?.() }
    // docSessionRef is always current; clearRecoverySnapshot has [] deps (stable).
  }, [clearRecoverySnapshot])

  // ---------------------------------------------------------------- native drag-drop (Tauri only)
  // The OS delivers file drops to Tauri's webview event bus rather than the
  // browser's dataTransfer API, so React onDrop never fires in the desktop build.
  // This effect subscribes to the Tauri webview drag-drop event and forwards
  // the first .hew path to openPath.  The existing React onDrop handlers are
  // kept for the web build and remain unchanged.
  useEffect(() => {
    if (!isTauri) return
    let unlisten: (() => void) | undefined
    let cancelled = false
    import('@tauri-apps/api/webview')
      .then(({ getCurrentWebview }) =>
        getCurrentWebview().onDragDropEvent((event) => {
          if (event.payload.type === 'drop') {
            const hew = event.payload.paths.find((p) => p.toLowerCase().endsWith('.hew'))
            if (hew) openPathRef.current(hew)
          }
        }),
      )
      .then((fn) => { if (cancelled) fn(); else unlisten = fn })
      .catch(() => { /* not in Tauri */ })
    return () => { cancelled = true; unlisten?.() }
  }, []) // openPath accessed via openPathRef — no dep needed

  // ---------------------------------------------------------------- global keyboard shortcuts
  useEffect(() => {
    // On macOS Tauri the native menu bar owns every MODIFIER shortcut
    // (Cmd-combos) — the JS handler must not double-fire those. But the
    // bare-letter tool shortcuts below have no native accelerator on macOS
    // (a native bare-letter accelerator would fire even while typing), so
    // they are handled here on EVERY platform — this is what makes the
    // shortcuts the tool rail advertises (C for Circle, B for Paint, …)
    // actually work on macOS. Windows/Linux Tauri and the web build use
    // this handler for everything.
    const nativeMenuOwnsModCombos = isTauri && isMac

    const onKeyDown = (ev: KeyboardEvent) => {
      const isMod = ev.metaKey || ev.ctrlKey

      // The welcome screen is modal: its overlay blocks pointer events but
      // not the keyboard, so app shortcuts must not fire behind it (a stray
      // 'r' silently switching tools, Ctrl+K stacking the palette invisibly
      // underneath the higher-z overlay). Mod-combos are still swallowed so
      // the browser doesn't act on them either (Ctrl+S "save page").
      if (welcomeOpenRef.current) {
        if (isMod && !nativeMenuOwnsModCombos) ev.preventDefault()
        return
      }

      // Don't fire shortcuts while typing in an input/textarea
      const target = ev.target as HTMLElement
      const isTyping = target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable

      // A tool mid-measurement-entry (VCB typed buffer) owns the keyboard:
      // letters may be unit suffixes ("1cm", "5' 2-1/4\""), not tool switches.
      // Per-key: a tool may own only some keys (Move's armed array window
      // takes digits/x/*// but never Space — Space always resets to Select).
      const toolIsTyping = viewportApi.current?.isCapturingInput?.(ev.key) ?? false

      if (!isMod && !isTyping && !toolIsTyping) {
        // Space → Select (no modifier required; guard against typing contexts).
        if (ev.key === ' ') {
          ev.preventDefault()
          setActiveTool('Select')
          return
        }
        // SketchUp-for-Windows bare-letter tool shortcuts — the
        // letter-keyed tools from `tools/toolRegistry.ts`'s `winKey`s. Real
        // SketchUp reserves these as unmodified keys; Hew's other tools
        // (Protractor/Slice/Edit Vertex/camera) keep their existing
        // Ctrl-combo shortcuts below instead — the design spec doesn't cover
        // bare letters for them.
        const key = ev.key.toLowerCase()
        if (key === 'l') { ev.preventDefault(); setActiveTool('Line'); return }
        if (key === 'r') { ev.preventDefault(); setActiveTool('Rectangle'); return }
        if (key === 'c') { ev.preventDefault(); setActiveTool('Circle'); return }
        if (key === 'a') { ev.preventDefault(); setActiveTool('Arc'); return }
        if (key === 'p') { ev.preventDefault(); setActiveTool('Push/Pull'); return }
        if (key === 'f') { ev.preventDefault(); setActiveTool('Offset'); return }
        if (key === 'm') { ev.preventDefault(); setActiveTool('Move'); return }
        if (key === 'q') { ev.preventDefault(); setActiveTool('Rotate'); return }
        if (key === 's') { ev.preventDefault(); setActiveTool('Scale'); return }
        if (key === 't') { ev.preventDefault(); setActiveTool('Tape Measure'); return }
        if (key === 'b') { ev.preventDefault(); setActiveTool('Paint'); return }
        // Camera tools: SketchUp's real O / H / Z — replaces the old
        // Ctrl+B / Ctrl+R / Ctrl+\ Hew inventions so the rail, the menus,
        // and actual dispatch all advertise the same keys.
        if (key === 'o') { ev.preventDefault(); setActiveTool('Orbit'); return }
        if (key === 'h') { ev.preventDefault(); setActiveTool('Pan'); return }
        if (key === 'z') { ev.preventDefault(); setActiveTool('Zoom'); return }
      }

      // (Delete/Backspace handled by a dedicated always-on effect below.)

      if (!isMod) return
      if (isTyping) return
      // macOS Tauri: the native menu's accelerators own all Cmd-combos.
      if (nativeMenuOwnsModCombos) return

      if (ev.key === 's' && !ev.shiftKey) {
        ev.preventDefault()
        saveDocument()
        return
      }
      if (ev.key === 's' && ev.shiftKey) {
        ev.preventDefault()
        saveAsDocument()
        return
      }
      if (ev.key.toLowerCase() === 'o' && !ev.shiftKey) {
        ev.preventDefault()
        openDocument()
        return
      }
      if (ev.key === 'n') {
        ev.preventDefault()
        newDocument()
        return
      }
      // Group / Ungroup. The in-app Edit menu has always advertised these
      // accelerators, but nothing dispatched them outside the macOS native
      // menu — the shortcut was display-only on Windows/Linux/web. Route
      // through the same menuActionRef dispatch the menu items use; the
      // handlers no-op on an ineligible selection, matching a disabled item.
      if (ev.key.toLowerCase() === 'g') {
        ev.preventDefault()
        menuActionRef.current(ev.shiftKey ? 'edit-ungroup' : 'edit-group')
        return
      }
      // (Camera tools moved from Ctrl+B/R/\ to SketchUp's bare O / H / Z in
      //  — see the bare-letter block above.)
      // Window pane toggles. Note: with Shift held, ev.key is the UPPERCASE
      // letter, so compare case-insensitively (else these never fire).
      if (ev.key.toLowerCase() === 'i' && ev.shiftKey) {
        ev.preventDefault()
        setShowModelInfo((v) => !v)
        return
      }
      if (ev.key.toLowerCase() === 'c' && ev.shiftKey) {
        ev.preventDefault()
        setShowMaterials((v) => !v)
        return
      }
      if (ev.key.toLowerCase() === 't' && ev.shiftKey) {
        ev.preventDefault()
        setShowTags((v) => !v)
        return
      }
      if (ev.key.toLowerCase() === 'o' && ev.shiftKey) {
        ev.preventDefault()
        setShowObjectInfo((v) => !v)
        return
      }
      if (ev.key === ',') {
        ev.preventDefault()
        openSettingsRef.current()
        return
      }
      // Command palette. Ctrl+K on Windows/Linux/web — free to
      // use here since moved Rectangle off Ctrl+K onto a bare
      // 'R'. macOS instead binds Cmd+/ via the native menu (see main.rs) —
      // Cmd+K there is still Rectangle's native accelerator, unchanged.
      if (ev.key.toLowerCase() === 'k' && !ev.shiftKey) {
        ev.preventDefault()
        setPaletteOpen(true)
        return
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [saveDocument, saveAsDocument, openDocument, newDocument])

  // Delete / Backspace → delete the current selection (guides).
  // Registered SEPARATELY from the global-shortcut effect above because that one
  // is disabled under Tauri (the native menu owns accelerators) — but Edit ▸
  // Delete has *no* native accelerator (a bare Delete/Backspace would bypass the
  // typing guard and collide with the tools' VCB Backspace), so the key must be
  // handled in JS on BOTH web and desktop. It deletes from any tool; only a tool
  // mid-VCB entry (isCapturingInput) keeps Backspace for the typed buffer.
  useEffect(() => {
    const onDeleteKey = (ev: KeyboardEvent) => {
      if (ev.key !== 'Delete' && ev.key !== 'Backspace') return
      const target = ev.target as HTMLElement
      const isTyping =
        target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable
      if (isTyping) return // let the focused field edit text normally
      // ALWAYS swallow Delete/Backspace outside text fields: the webview
      // otherwise treats Backspace as "navigate back", which silently wedges
      // the whole app until restart.
      ev.preventDefault()
      // …but never delete anything from behind the modal welcome screen.
      if (welcomeOpenRef.current) return
      // Delete the current selection — from ANY tool and however it was selected
      // (viewport or Object list), NOT just the Select tool. The one exception
      // is a tool mid-VCB entry (e.g. typing a Move distance), where Backspace
      // must edit the typed buffer instead; the Viewport routes the key to that
      // tool and reports it here via isCapturingInput so we don't also delete.
      if (!(viewportApi.current?.isCapturingInput?.(ev.key) ?? false)) {
        menuActionRef.current('edit-delete')
      }
    }
    window.addEventListener('keydown', onDeleteKey)
    return () => window.removeEventListener('keydown', onDeleteKey)
  }, [])

  // Cmd/Ctrl+A → Select All. Like Delete above, this is handled in JS on
  // BOTH web and desktop: the native menu item deliberately carries no
  // accelerator, because a native CmdOrCtrl+A fires even while typing in a
  // text field and would hijack select-all-text into a scene-wide selection.
  // Here a focused text field keeps the browser's own select-all.
  useEffect(() => {
    const onSelectAllKey = (ev: KeyboardEvent) => {
      if (!(ev.metaKey || ev.ctrlKey) || ev.shiftKey || ev.altKey) return
      if (ev.key.toLowerCase() !== 'a') return
      // Modal welcome screen: swallow (no scene select-all, no page-wide
      // text selection of the app behind the overlay).
      if (welcomeOpenRef.current) { ev.preventDefault(); return }
      const target = ev.target as HTMLElement
      const isTyping =
        target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable
      if (isTyping) return // let the focused field select its own text
      ev.preventDefault()
      menuActionRef.current('edit-select-all')
    }
    window.addEventListener('keydown', onSelectAllKey)
    return () => window.removeEventListener('keydown', onSelectAllKey)
  }, [])

  // Mirror the View ▸ Axes / Grid / Guides toggles into the viewport. The
  // viewport API ref is populated once the viewport mounts; all default to
  // visible so an early run before the ref is ready is a harmless no-op.
  useEffect(() => { viewportApi.current?.setAxesVisible(showAxes) }, [showAxes])
  useEffect(() => { viewportApi.current?.setGridVisible(showGrid) }, [showGrid])
  useEffect(() => { viewportApi.current?.setGuidesVisible(showGuides) }, [showGuides])

  // ---------------------------------------------------------------- native menu state sync (macOS)
  // Reflect UI state into the native menu bar: the active tool's radio
  // check, the View/Window toggles' check marks, and the enabled state of
  // the selection-gated Edit commands. macOS is the only platform that
  // attaches the native menu (Linux/Windows use the in-app MenuBar, which
  // reads this state directly as props).
  //
  // There is ONE menu bar shared by every document window, so the state it
  // shows must follow focus: gaining focus re-pushes this window's state
  // (menuFocusTick), and the shell ignores pushes from unfocused windows —
  // without this, switching from a window with a selection to one without
  // left the old window's check marks and Edit gates on the menu.
  const [menuFocusTick, setMenuFocusTick] = useState(0)
  useEffect(() => {
    if (!isTauri || !isMac) return
    let unlisten: (() => void) | undefined
    let cancelled = false
    import('@tauri-apps/api/window').then(({ getCurrentWindow }) =>
      getCurrentWindow().onFocusChanged(({ payload: focused }) => {
        if (focused) setMenuFocusTick((t) => t + 1)
      }),
    ).then((fn) => { if (cancelled) fn(); else unlisten = fn }).catch(() => { /* ignore */ })
    return () => { cancelled = true; unlisten?.() }
  }, [])
  useEffect(() => {
    if (!isTauri || !isMac) return
    const checked: Record<string, boolean> = {
      'view-axes': showAxes,
      'view-grid': showGrid,
      'view-guides': showGuides,
      'view-section-plane': sectionPlaneMenuState.checked,
      'win-model-info': showModelInfo,
      'win-materials': showMaterials,
      'win-tags': showTags,
      'win-object-info': showObjectInfo,
      'win-debug-log': showDebugLog,
    }
    for (const [tool, id] of Object.entries(TOOL_MENU_IDS)) {
      checked[id] = tool === activeTool
    }
    const enabled: Record<string, boolean> = {
      'edit-delete': selectedIds.length > 0 || selectedGuide !== null,
      'edit-group': menuGates?.canGroup ?? false,
      'edit-ungroup': menuGates?.canUngroup ?? false,
      'edit-make-component': menuGates?.canMakeComponent ?? false,
      'edit-place-copy': menuGates?.canPlaceCopy ?? false,
      'edit-explode': menuGates?.canExplode ?? false,
      'edit-make-unique': menuGates?.canMakeUnique ?? false,
      'edit-union': menuGates?.canBoolean ?? false,
      'edit-subtract': menuGates?.canBoolean ?? false,
      'edit-intersect': menuGates?.canBoolean ?? false,
      'view-section-plane': sectionPlaneMenuState.exists,
    }
    import('@tauri-apps/api/core')
      .then(({ invoke }) => invoke('sync_menu_state', { checked, enabled }))
      .catch(() => { /* shell without the command (older build) — ignore */ })
  }, [
    activeTool,
    showAxes,
    showGrid,
    showGuides,
    sectionPlaneMenuState,
    showModelInfo,
    showMaterials,
    showTags,
    showObjectInfo,
    showDebugLog,
    selectedIds,
    selectedGuide,
    menuGates,
    menuFocusTick,
  ])

  // ---------------------------------------------------------------- drag-drop open
  const handleDragOver = useCallback((ev: React.DragEvent) => {
    ev.preventDefault()
    ev.dataTransfer.dropEffect = 'copy'
  }, [])

  const handleDrop = useCallback(async (ev: React.DragEvent) => {
    ev.preventDefault()
    const file = ev.dataTransfer.files[0]
    if (file == null || !file.name.endsWith('.hew')) return
    if (!(await confirmDiscard())) return
    file.arrayBuffer().then((buf) => {
      const discardsUnsaved = docSessionRef.current.dirty
      const ok = applyLoadedBytes(new Uint8Array(buf))
      if (ok) {
        setDocSession(afterOpen({ name: file.name, handle: null }, Date.now()))
        if (discardsUnsaved) void clearRecoverySnapshot()
      }
    }).catch((err: unknown) => {
      handleToast(`Drop open failed: ${friendlyErrorText(err)}`)
    })
  }, [confirmDiscard, applyLoadedBytes, handleToast, clearRecoverySnapshot])

  // ── Hide/Show + Tags (must be declared BEFORE the early returns below so the
  // hook count is stable across the loading and loaded renders — Rules of Hooks).
  /**
   * Collect all leaf object and instance ids for a node (recurse into
   * groups). Thin wrapper over the shared `collectLeafIds` in treeModel —
   * supplies the wasm `group_members` lookup as plain NodeRefs.
   */
  const collectLeafIds = (node: NodeRef): { objectIds: bigint[]; instanceIds: bigint[] } =>
    collectLeafIdsShared(node, (groupId) =>
      state!.scene.group_members(groupId).map((m) => ({ kind: m.kind as NodeRef['kind'], id: m.id })),
    )

  /**
   * Derive the union hidden id sets from both manual-hide and tag-hide sources,
   * then push the result to the renderer.
   *
   * Both sets are passed in explicitly so callers can pass the *next* set
   * (before setState is applied) without waiting for a re-render.
   */
  const pushUnionHidden = useCallback(
    (nextHiddenKeys: Set<string>, nextHiddenTagPaths: Set<string>) => {
      const scene = state?.scene
      if (scene === undefined) return

      const hiddenObjectIds: bigint[] = []
      const hiddenInstanceIds: bigint[] = []

      // --- (a) manual per-node hides ---
      for (const k of nextHiddenKeys) {
        const colonIdx = k.indexOf(':')
        const kind = k.slice(0, colonIdx) as NodeRef['kind']
        const id = BigInt(k.slice(colonIdx + 1))
        const n: NodeRef = { kind, id }
        const { objectIds, instanceIds } = collectLeafIds(n)
        hiddenObjectIds.push(...objectIds)
        hiddenInstanceIds.push(...instanceIds)
      }

      // --- (b) tag-path hides ---
      if (nextHiddenTagPaths.size > 0) {
        // Build the current tag list from the scene using first-class tag data.
        const allNodes = [
          ...Array.from(scene.object_ids()).map((id) => ({ kind: 'object' as const, id })),
          ...Array.from(scene.group_ids()).map((id) => ({ kind: 'group' as const, id })),
          ...Array.from(scene.instance_ids()).map((id) => ({ kind: 'instance' as const, id })),
        ]
        const tagged: { node: NodeRef; path: string[] }[] = []
        for (const raw of allNodes) {
          const node: NodeRef = raw as NodeRef
          const kindNum = node.kind === 'object' ? 0 : node.kind === 'group' ? 1 : 2
          const rawTags = scene.node_tags(kindNum, node.id)
          for (const rawTag of rawTags) {
            const path = rawTag.split('/').map((s) => s.trim()).filter((s) => s.length > 0)
            if (path.length > 0) {
              tagged.push({ node, path })
            }
          }
        }

        // For each hidden tag path, collect all nodes whose tag path is at or
        // under it.  We iterate once over all tagged nodes for efficiency.
        const hiddenAnchorPaths: string[][] = []
        for (const key of nextHiddenTagPaths) {
          try {
            const parsed = JSON.parse(key)
            if (Array.isArray(parsed)) hiddenAnchorPaths.push(parsed as string[])
          } catch { /* invalid key — skip */ }
        }

        // A node is covered if its tag path is at or under any hidden anchor path.
        // isPathUnder handles both exact matches and descendant paths, so hiding
        // a parent tag automatically covers all nodes tagged further down.
        for (const { node, path } of tagged) {
          const covered = hiddenAnchorPaths.some((anchor) => isPathUnder(path, anchor))
          if (!covered) continue
          const { objectIds, instanceIds } = collectLeafIds(node)
          hiddenObjectIds.push(...objectIds)
          hiddenInstanceIds.push(...instanceIds)
        }
      }

      // Deduplicate (a leaf may be covered by multiple hidden paths/tags).
      const objIds = [...new Set(hiddenObjectIds)]
      const instIds = [...new Set(hiddenInstanceIds)]
      // (1) Renderer: hide the meshes. (2) Kernel inference: drop the hidden
      // geometry so snap/pick_face skip it — otherwise you'd still snap to and
      // be unable to click past a hidden solid's edges/faces.
      viewportApi.current?.setHidden(objIds, instIds)
      scene.set_hidden(new BigUint64Array(objIds), new BigUint64Array(instIds))
    },
    // state?.scene changes on every render; we capture it fresh via the closure.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [state],
  )
  // Keep the ref pointed at the latest pushUnionHidden closure so callbacks
  // defined earlier (applyLoadedBytes, importDocument) never call a stale one.
  pushUnionHiddenRef.current = pushUnionHidden

  const handleToggleHidden = (node: NodeRef) => {
    const key = nodeKey(node)
    // Compute the next hidden set purely (no setState updater side effects).
    const next = new Set(hiddenKeys)
    const nowHidden = !next.has(key)
    if (next.has(key)) {
      next.delete(key)
    } else {
      next.add(key)
    }
    setHiddenKeys(next)
    pushUnionHidden(next, hiddenTagPaths)
    // Persist the choice in the document's USER-hidden registry (view state,
    // not undoable) so it survives save/load, mirroring how
    // handleToggleTagPath persists via set_tag_hidden.
    const kindNum = nodeKindToNumber(node.kind)
    if (kindNum >= 0) sceneRef.current?.set_node_user_hidden(kindNum, node.id, nowHidden)
  }

  const handleToggleTagPath = useCallback((path: string[]) => {
    const key = tagPathKey(path)
    const nowHidden = !hiddenTagPaths.has(key)
    const next = new Set(hiddenTagPaths)
    if (next.has(key)) {
      next.delete(key)
    } else {
      next.add(key)
    }
    setHiddenTagPaths(next)
    pushUnionHidden(hiddenKeys, next)
    // Persist the choice in the document's tag registry (view state, not
    // undoable) so it survives save/load, in addition to the session-only
    // hiddenTagPaths set above (which drives this render's UI immediately).
    sceneRef.current?.set_tag_hidden(path.join('/'), nowHidden)
  }, [hiddenTagPaths, hiddenKeys, pushUnionHidden])
  toggleTagPathRef.current = handleToggleTagPath

  // Re-seed hiddenTagPaths from the document's registry and re-push the
  // union — the shared tail of every operation that can change the registry
  // out from under the session set (delete tag, undo/redo of a deletion).
  const resyncTagVisibility = useCallback(() => {
    const scene = sceneRef.current
    if (scene === null) return
    const seeded = seedHiddenTagPathsFromRegistry(scene)
    setHiddenTagPaths(seeded)
    pushUnionHidden(hiddenKeys, seeded)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hiddenKeys, pushUnionHidden])
  resyncTagVisibilityRef.current = resyncTagVisibility

  // Delete a tag everywhere (undoable, kernel-side): unassigns it — and its
  // sub-tags — from every node and drops the registry entries. Geometry is
  // never deleted; content hidden solely via the deleted tag becomes visible
  // again through the registry re-seed.
  const handleDeleteTag = useCallback((path: string[]) => {
    const scene = sceneRef.current
    if (scene === null) return
    try {
      scene.delete_tag(path.join('/'))
    } catch (err: unknown) {
      handleToast(`Delete tag failed: ${friendlyErrorText(err)}`)
      return
    }
    resyncTagVisibility()
    handleDocumentChanged()
  }, [resyncTagVisibility, handleDocumentChanged, handleToast])
  deleteTagRef.current = handleDeleteTag

  if (error !== null) {
    return (
      <main style={{ fontFamily: 'sans-serif', padding: '1rem', color: 'var(--danger-base, red)' }}>
        <h1>Hew — kernel load error</h1>
        <pre>{error}</pre>
      </main>
    )
  }

  if (state === null) {
    return (
      <main style={{ fontFamily: 'sans-serif', padding: '1rem' }}>
        <p>Loading kernel…</p>
      </main>
    )
  }

  // Compute watertight summary for badge display
  const objectCount = watertightMap.size
  const allWatertight = objectCount === 0 || Array.from(watertightMap.values()).every(Boolean)
  const leakyCount = Array.from(watertightMap.values()).filter((v) => !v).length

  // Undo/redo availability (queried from scene each render for menu state)
  const canUndo = sceneRef.current?.can_scene_undo() ?? false
  const canRedo = sceneRef.current?.can_scene_redo() ?? false

  // Selection-gated command availability — see the menuGates memo above.
  const booleanOperands = menuGates?.booleanOperands ?? []
  const canBoolean = menuGates?.canBoolean ?? false
  const canGroupNow = menuGates?.canGroup ?? false
  const canUngroupNow = menuGates?.canUngroup ?? false
  const canMakeComp = menuGates?.canMakeComponent ?? false
  const canPlace = menuGates?.canPlaceCopy ?? false
  const canExplode = menuGates?.canExplode ?? false
  const canUnique = menuGates?.canMakeUnique ?? false
  const handleBoolean = (op: number) => {
    // Re-check the full gate (the accelerator path dispatches here
    // unconditionally, like handleGroup): exactly two operands, each a plain
    // solid or a group, at the top level.
    if (!(menuGates?.canBoolean ?? false)) return
    const result = viewportApi.current?.runBoolean(op, booleanOperands[0], booleanOperands[1])
    if (result != null) {
      setSelectedIds([result])
      setDocRev((r) => r + 1)
    }
  }

  const handleGroup = () => {
    // Re-check eligibility: the keyboard accelerator (Ctrl+G) dispatches here
    // unconditionally, unlike the menu items sync_menu_state disables, so the
    // handler must enforce the same gate — ≥2 distinct siblings, no sketches.
    // Without this a single selected node becomes a silent 1-member group
    // (the kernel accepts any non-empty sibling list).
    if (!(menuGates?.canGroup ?? false)) return
    const newGroupId = viewportApi.current?.runGroup(selectedIds)
    if (newGroupId != null) {
      setSelectedIds([{ kind: 'group', id: newGroupId }])
      setDocRev((r) => r + 1)
    }
  }

  const handleUngroup = () => {
    if (selectedIds.length === 1 && selectedIds[0].kind === 'group') {
      viewportApi.current?.runUngroup(selectedIds[0].id)
      setSelectedIds([])
      setDocRev((r) => r + 1)
    }
  }

  const handleMakeComponent = () => {
    // Same re-check handleGroup does: dispatchers without a disabled state
    // (dock verbs are hidden, but keyboard/palette paths aren't) must not
    // hand the kernel an ineligible selection.
    if (!(menuGates?.canMakeComponent ?? false)) return
    const instanceId = viewportApi.current?.runMakeComponent(selectedIds)
    if (instanceId != null) {
      setSelectedIds([{ kind: 'instance', id: instanceId }])
      setDocRev((r) => r + 1)
    }
  }

  const handlePlaceInstance = () => {
    if (selectedIds.length === 1 && selectedIds[0].kind === 'instance') {
      const newId = viewportApi.current?.runPlaceInstance(selectedIds[0].id)
      if (newId != null) {
        setSelectedIds([{ kind: 'instance', id: newId }])
        setDocRev((r) => r + 1)
      }
    }
  }

  const handleExplodeInstance = () => {
    if (selectedIds.length === 1 && selectedIds[0].kind === 'instance') {
      const objectIds = viewportApi.current?.runExplodeInstance(selectedIds[0].id)
      if (objectIds != null && objectIds.length > 0) {
        setSelectedIds(objectIds.map((id) => ({ kind: 'object' as const, id })))
        setDocRev((r) => r + 1)
      } else if (objectIds != null) {
        setSelectedIds([])
        setDocRev((r) => r + 1)
      }
    }
  }

  const handleMakeUnique = () => {
    if (selectedIds.length === 1 && selectedIds[0].kind === 'instance') {
      const kept = viewportApi.current?.runMakeUnique(selectedIds[0].id)
      if (kept != null) {
        // Keep the same instance selected — its id is unchanged.
        setDocRev((r) => r + 1)
      }
    }
  }

  return (
    <main
      style={{
        fontFamily: 'var(--font-family-ui, sans-serif)',
        position: 'relative',
        display: 'flex',
        flexDirection: 'column',
        height: '100%',
        boxSizing: 'border-box',
        overflow: 'hidden',
        background: 'var(--surface-canvas-page, #1a1a1a)',
      }}
    >
      {/* Linux desktop shell only: borderless window → draw our own title bar
          (KWin/WebKitGTK won't repaint the native one after setTitle). Windows
          and macOS keep native decorations, so their OS caption (which reflects
          setTitle) is the title bar — no custom one. */}
      {isTauri && isLinux && (
        <TitleBar name={documentName(docSession)} saveState={saveStateLabel(docSession, nowTick)} />
      )}

      {/* App bar / menu bar.
          On macOS Tauri the native OS menu bar owns File/Edit (this renders
          nothing). On the web build and the Linux/Windows borderless shells,
          the in-app bar renders the menus (Linux settled on in-app chrome
          over the native GTK menubar — trialed and rejected); the
          centered title is shown by TitleBar on Linux/Windows so it is
          hidden here in that case. */}
      <MenuBar
        name={documentName(docSession)}
        saveState={saveStateLabel(docSession, nowTick)}
        nativeMenuBar={isTauri && isMac}
        hideTitle={isTauri && (isLinux || isWindows)}
        onNew={newDocument}
        onOpen={openDocument}
        onSave={saveDocument}
        onSaveAs={saveAsDocument}
        onImport={importDocument}
        onExport={() => setExportDialogOpen(true)}
        onClose={
          isTauri && !isMac
            ? () => {
                import('@tauri-apps/api/window')
                  .then(({ getCurrentWindow }) => getCurrentWindow().close())
                  .catch(() => { /* ignore */ })
              }
            : undefined
        }
        onExit={
          isTauri && !isMac
            ? () => {
                import('@tauri-apps/api/window')
                  .then(async ({ getAllWindows }) => {
                    // Close every window (each document window's close-guard
                    // still prompts for its own unsaved changes); the app exits
                    // once the last one is gone.
                    for (const w of await getAllWindows()) {
                      await w.close().catch(() => { /* keep closing the rest */ })
                    }
                  })
                  .catch(() => { /* ignore */ })
              }
            : undefined
        }
        recentFiles={recentFiles}
        onOpenRecent={openRecent}
        onClearRecent={clearRecent}
        onUndo={handleUndo}
        onRedo={handleRedo}
        canUndo={canUndo}
        canRedo={canRedo}
        activeTool={activeTool}
        onSelectTool={(name) => setActiveTool(name as ToolName)}
        showModelInfo={showModelInfo}
        showMaterials={showMaterials}
        showTags={showTags}
        showObjectInfo={showObjectInfo}
        showDebugLog={showDebugLog}
        onToggleModelInfo={() => setShowModelInfo((v) => !v)}
        onToggleMaterials={() => setShowMaterials((v) => !v)}
        onToggleTags={() => setShowTags((v) => !v)}
        onToggleObjectInfo={() => setShowObjectInfo((v) => !v)}
        onToggleDebugLog={() => setShowDebugLog((v) => !v)}
        showAxes={showAxes}
        showGrid={showGrid}
        showGuides={showGuides}
        onToggleAxes={() => setShowAxes((v) => !v)}
        onToggleGrid={() => setShowGrid((v) => !v)}
        onToggleGuides={() => setShowGuides((v) => !v)}
        onDeleteGuides={() => viewportApi.current?.deleteAllGuides()}
        sectionPlaneChecked={sectionPlaneMenuState.checked}
        sectionPlaneExists={sectionPlaneMenuState.exists}
        onToggleSectionActive={() => viewportApi.current?.toggleSectionActive()}
        onDelete={deleteSelection}
        onEditAction={(id) => menuActionRef.current(id)}
        editGates={{
          canGroup: canGroupNow,
          canUngroup: canUngroupNow,
          canMakeComponent: canMakeComp,
          canPlaceCopy: canPlace,
          canExplode: canExplode,
          canMakeUnique: canUnique,
          canBoolean,
        }}
        onZoomExtents={handleZoomExtents}
        onStandardView={(view) => viewportApi.current?.setStandardView(view)}
        onOpenSettings={openSettings}
        onReportBug={handleReportBug}
        onCheckForUpdates={updaterAvailable ? handleCheckForUpdates : undefined}
        windowList={isTauri ? windowList : undefined}
        onFocusWindow={focusWindow}
      />

      {/* Kernel panic sticky banner */}
      {kernelPanicked && (
        <div
          style={{
            background: '#8b0000',
            color: '#fff',
            padding: '10px 16px',
            display: 'flex',
            alignItems: 'center',
            gap: '12px',
            fontFamily: 'monospace',
            fontSize: '13px',
            zIndex: 200,
            flexShrink: 0,
          }}
        >
          <span style={{ flex: 1 }}>
            A kernel error occurred — the session is no longer usable. Reload the page to recover.
          </span>
          <button
            onClick={() => window.location.reload()}
            style={{
              padding: '4px 14px',
              background: '#fff',
              color: '#8b0000',
              border: 'none',
              borderRadius: '3px',
              fontFamily: 'monospace',
              fontSize: '13px',
              fontWeight: 'bold',
              cursor: 'pointer',
            }}
          >
            Reload
          </button>
        </div>
      )}

      {/* Body row: labeled left tool rail (`03_tool_rail.md`), the
          viewport, and the docked right tray (`06_docked_panels.md`)
          — the app-shell's full 3-column layout (`02_app_shell.md`). */}
      <div style={{ flex: 1, minHeight: 0, display: 'flex' }}>
        <ToolRail
          activeTool={activeTool}
          onSelectTool={(name) => setActiveTool(name)}
          // The resting palette field lives at the top of the rail on every
          // platform ( — macOS forced it here since it has no in-window
          // menu bar, and the rest follow for consistency). The shortcut on
          // macOS desktop is Cmd+/ (Cmd+K stays Rectangle's native
          // accelerator — Refinement); everywhere else Ctrl+K
          // (metaKey ⌘K on mac web) via the global keydown handler above.
          onOpenPalette={() => setPaletteOpen(true)}
          paletteKbd={isTauri && isMac ? '⌘/' : isMac ? '⌘K' : 'Ctrl K'}
        />
        <div
          style={{ flex: 1, minWidth: 0, position: 'relative' }}
          onDragOver={handleDragOver}
          onDrop={handleDrop}
          // The inference cursor (snap dot + tooltip chip) tracks the pointer
          // inside the viewport; once the pointer leaves for the rail/tray it
          // would otherwise freeze at the edge and its label could sit on top
          // of panel content. Clear it on the way out.
          onPointerLeave={() => setInferenceInfo(null)}
        >
          <Viewport
            wasmScene={state.scene}
            onStatusChange={handleStatusChange}
            onSceneChange={handleSceneChange}
            onToast={handleToast}
            onToolHint={setToolStageHint}
            onPrecisionChange={setPrecisionSnap}
            activeTool={activeTool}
            activeContext={activeContext}
            selectedIds={selectedIds}
            activeLitSet={activeLitSet}
            onSelect={handleSelect}
            onSelectMany={handleSelectMany}
            onSelectGuide={handleSelectGuide}
            selectedGuide={selectedGuide}
            onEnterContext={handleEnterContext}
            onExitContext={handleExitContext}
            onDocumentChanged={handleDocumentChanged}
            onSectionChanged={handleSectionChanged}
            onHistoryChanged={handleHistoryChanged}
            apiRef={viewportApi}
            onMeasurement={handleMeasurement}
            onInferenceChange={handleInferenceChange}
            onCameraDragChange={setCameraDragging}
            onHoverSketchRegionChange={setHoveringSketchRegion}
            currentMaterialId={currentMaterialId}
          />

          {/* Inference & viewport feedback (`07_inference_feedback.md`)
              — all net-new DOM overlays; Viewport.tsx rendered none of this
              before this milestone. */}
          <SnapDot info={inferenceInfo} />
          <InferenceTooltip info={inferenceInfo} />
          {/* Overlays that live INSIDE the viewport container: hovering them
              also hides the inference cursor (the container's pointerleave
              can't see moves into its own children). display:contents keeps
              them out of the layout while still catching the bubbled events. */}
          <div style={{ display: 'contents' }} onPointerOver={() => setInferenceInfo(null)}>
            <MeasurementBox toolName={toolName} value={measurement} />
            <ViewportHUD
              onSelectView={(view: StandardView) => viewportApi.current?.setStandardView(view)}
              onOrbit={() => setActiveTool('Orbit')}
            />
          </div>

          {/* Contextual dock — bottom-center, self-hides only when
              there's no curated verb set for the current selection (a
              construction guide — a selected sketch gets its own 'sketch'
              context as of), and fades out while the camera is being
              dragged. Reuses the same menuActionRef dispatch the
              palette and every menu item already go through.
              activeToolId tells the dock which verb, if any,
              is the ACTUAL active tool — so it never shows a stale "selected"
              verb (e.g. Rectangle) while a different tool (e.g. Arc) is live.
              hoveringSketchRegion previews the Push/Pull verb when
              nothing is selected and the cursor is aimed at a sketch region
              — an explicit selection's dock always wins over this hint. */}
          <div style={{ display: 'contents' }} onPointerOver={() => setInferenceInfo(null)}>
            <ContextualDock
              selectedIds={selectedIds}
              selectedGuide={selectedGuide}
              hidden={cameraDragging}
              activeToolId={toolActionId(activeTool)}
              hoveringSketchRegion={hoveringSketchRegion}
              gates={{ canGroup: canGroupNow, canMakeComponent: canMakeComp }}
              onRun={(id) => menuActionRef.current(id)}
            />
          </div>

          {/* Toast stack — positioned inside the viewport container. */}
          <div
            style={{
              position: 'absolute',
              bottom: '16px',
              left: '50%',
              transform: 'translateX(-50%)',
              display: 'flex',
              flexDirection: 'column',
              gap: '6px',
              pointerEvents: 'none',
              zIndex: 100,
            }}
          >
            {toasts.map((toast) => (
              <div
                key={toast.id}
                onClick={() => dismissToast(toast.id)}
                style={{
                  padding: '8px 16px',
                  background: toast.isError ? '#cc3322' : '#333',
                  color: '#fff',
                  borderRadius: '4px',
                  fontFamily: 'monospace',
                  fontSize: '13px',
                  cursor: 'pointer',
                  pointerEvents: 'auto',
                  boxShadow: '0 2px 8px rgba(0,0,0,0.4)',
                  maxWidth: '400px',
                  textAlign: 'center',
                }}
              >
                {toast.code !== undefined && (
                  <strong style={{ marginRight: '6px', opacity: 0.8 }}>[{toast.code}]</strong>
                )}
                {toast.message}
              </div>
            ))}
          </div>
        </div>

        {/* Docked right tray (`06_docked_panels.md`) — permanently
            present, collapsible sections; replaces the old floating,
            draggable panels entirely (FloatingPanel.tsx deleted). Default
            order per spec: Object Info -> Outliner -> Materials, plus Tags
            as a 4th section (not in the spec's default list, but a real
            shipped Hew feature — kept rather than dropped). The showX/setShowX
            state pairs are unchanged from the floating-panel era; they now
            mean "expanded" instead of "visible," so every existing keyboard
            shortcut (Shift+Cmd+I/C/T/O) and Window-menu checkbox keeps
            working with its original meaning. */}
        {/* Tray resize handle — drag to adjust the tray width; the width is
            clamped and persisted so complex models' tag/outliner labels can
            be given room once and keep it across launches. */}
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize panels"
          onPointerDown={(ev) => {
            ev.preventDefault()
            const startX = ev.clientX
            const startWidth = trayWidth
            const el = ev.currentTarget
            el.setPointerCapture(ev.pointerId)
            const drag = { width: startWidth }
            const onMove = (mv: PointerEvent) => {
              drag.width = clampTrayWidth(startWidth + (startX - mv.clientX))
              setTrayWidth(drag.width)
            }
            // One teardown for every way the drag can end. pointerup alone
            // is not enough: an interrupted drag (pointercancel, or the
            // browser revoking capture) would otherwise leave the move
            // listener attached with its stale startX — merely hovering the
            // handle afterwards keeps resizing with no button held, and the
            // width never persists.
            const endDrag = () => {
              el.removeEventListener('pointermove', onMove)
              el.removeEventListener('pointerup', endDrag)
              el.removeEventListener('pointercancel', endDrag)
              el.removeEventListener('lostpointercapture', endDrag)
              try {
                el.releasePointerCapture(ev.pointerId)
              } catch {
                /* capture already gone (lostpointercapture path) */
              }
              window.localStorage.setItem(TRAY_WIDTH_KEY, String(drag.width))
            }
            el.addEventListener('pointermove', onMove)
            el.addEventListener('pointerup', endDrag)
            el.addEventListener('pointercancel', endDrag)
            el.addEventListener('lostpointercapture', endDrag)
          }}
          style={{
            width: '5px',
            marginRight: '-5px',
            flexShrink: 0,
            cursor: 'col-resize',
            zIndex: 5,
            // Invisible grab strip riding on the tray's hairline border.
            background: 'transparent',
          }}
        />
        <div
          style={{
            width: `${trayWidth}px`,
            flexShrink: 0,
            display: 'flex',
            flexDirection: 'column',
            minHeight: 0,
            background: 'var(--surface-panel)',
            borderLeft: '1px solid var(--border-hairline)',
          }}
        >
          <TraySection title="Object Info" collapsed={!showObjectInfo} onToggle={() => setShowObjectInfo((v) => !v)}>
            <ObjectInfoPanel
              scene={state.scene}
              docRev={docRev}
              selectedIds={selectedIds}
              onDocumentChanged={handleDocumentChanged}
              onSelectMany={handleReplaceSelection}
              onToast={handleToast}
            />
          </TraySection>
          <TraySection title="Outliner" collapsed={!showModelInfo} onToggle={() => setShowModelInfo((v) => !v)}>
            <DocumentTree
              scene={state.scene}
              docRev={docRev}
              watertightMap={watertightMap}
              selectedIds={selectedIds}
              activeContext={activeContext}
              onSelect={handleSelect}
              onEnterContext={handleEnterContext}
              onExitContext={handleExitContext}
              onSetContextDepth={handleSetContextDepth}
              hiddenKeys={hiddenKeys}
              onToggleHidden={handleToggleHidden}
            />
          </TraySection>
          <TraySection title="Materials" collapsed={!showMaterials} onToggle={() => setShowMaterials((v) => !v)}>
            <MaterialPalette
              scene={state.scene}
              docRev={docRev}
              currentMaterialId={currentMaterialId}
              onSelectMaterial={handleSelectMaterial}
              onMaterialCreated={setCurrentMaterialId}
              onDocumentChanged={handleDocumentChanged}
              onAlphaCommitted={() => viewportApi.current?.syncMaterialOpacity()}
            />
          </TraySection>
          <TraySection title="Tags" collapsed={!showTags} onToggle={() => setShowTags((v) => !v)}>
            <TagsPanel
              scene={state.scene}
              docRev={docRev}
              hiddenTagPaths={hiddenTagPaths}
              onToggleTagPath={handleToggleTagPath}
              onDeleteTag={handleDeleteTag}
              revealTag={revealTag}
            />
          </TraySection>
        </div>
      </div>

      {/* Status bar — the Studio instructor line (`02_app_shell.md`): active
          tool name in text/tertiary, a dim ·, then a one-line hint in
          text/faint. Snap state moved to the on-cursor inference dot and live
          Measurements to the top-right VCB + near-cursor readout, so both are
          dropped here (Refinement pass, issue E). Every color is now a theme
          token — the old bar hardcoded near-white text (`#eee`), which was
          unreadable on the light theme's light bar (the dark/light divergence
          the user flagged). Extra bottom padding avoids descender clipping
          under macOS Tahoe's rounded window corners. */}
      <div
        style={{
          height: 30,
          boxSizing: 'border-box',
          padding: '0 var(--space-6, 13px) 6px',
          background: 'var(--surface-bar)',
          fontFamily: 'var(--font-family-ui)',
          fontSize: 'var(--font-size-body, 12px)',
          borderTop: '1px solid var(--border-hairline)',
          display: 'flex',
          alignItems: 'center',
          gap: 'var(--space-3, 8px)',
          flexShrink: 0,
        }}
      >
        <span style={{ color: 'var(--text-tertiary)', fontWeight: 600 }}>{toolName}</span>
        {/* Stage-aware guidance from the active tool when it provides one
            (Tool.statusHint — "Click the opposite corner…"), else the
            palette's static description; the app always says what to do
            next. */}
        {(toolStageHint ?? toolHint(toolName)) !== '' && (
          <>
            <span style={{ color: 'var(--text-section)' }} aria-hidden="true">·</span>
            <span style={{ color: 'var(--text-faint)' }}>{toolStageHint ?? toolHint(toolName)}</span>
          </>
        )}
        {/* Precision-snapping chip — the only cue that the chord is doing
            anything, and the mode is invisible otherwise (it changes which
            candidate wins, not what is drawn). Sits with the tool guidance
            rather than right-aligned with the document-level watertight badge,
            because it is about the gesture in progress. */}
        {precisionSnap && (
          <>
            <span style={{ color: 'var(--text-section)' }} aria-hidden="true">·</span>
            <span
              style={{
                padding: '2px 8px',
                fontSize: 'var(--font-size-dock-chip, 11px)',
                borderRadius: 4,
                background: 'var(--surface-raised, rgba(127,127,127,0.18))',
                color: 'var(--text-primary)',
                fontWeight: 600,
              }}
            >
              Precision snap ({isMac ? '⌘⌥' : 'Ctrl+Alt'})
            </span>
          </>
        )}
        {/* Watertight badge — aggregate solids feedback; no other single-glance
            home (per-object status lives in the Outliner). Right-aligned. */}
        {objectCount > 0 && (
          <span
            style={{
              marginLeft: 'auto',
              padding: '2px 8px',
              fontSize: 'var(--font-size-dock-chip, 11px)',
              borderRadius: 4,
              background: allWatertight ? 'var(--status-solid-bg)' : 'var(--status-leaky-bg)',
              color: '#fff',
            }}
          >
            {allWatertight
              ? `${objectCount} object${objectCount !== 1 ? 's' : ''} ✓ solid`
              : `${leakyCount} leaky`}
          </span>
        )}
      </div>

      {/* Debug Log panel — opt-in via Window menu, default hidden. */}
      {showDebugLog && <LogPanel panelHeight={160} />}

      {/* Importing overlay — shown while import_dae blocks the main thread.
          The overlay is painted before the blocking call via a double rAF in
          importDocument.  isImporting is always cleared in a finally block,
          so a thrown import error can never leave the overlay stuck. */}
      {isImporting && <ImportingOverlay fileName={importingName} />}

      {/* STL units-chooser — shown before the overlay above, since STL
          carries no unit information (see promptStlUnits). */}
      {pendingStlImport !== null && (
        <StlUnitsDialog
          fileName={pendingStlImport.name}
          onChoose={(unitScale, value) => {
            setLastStlImportUnit(value)
            setPendingStlImport(null)
            stlUnitsResolveRef.current?.(unitScale)
            stlUnitsResolveRef.current = null
          }}
          onCancel={() => {
            setPendingStlImport(null)
            stlUnitsResolveRef.current?.(null)
            stlUnitsResolveRef.current = null
          }}
        />
      )}

      {/* Import report modal — shown after a successful model import */}
      {importReport !== null && (
        <ImportReportDialog
          report={importReport}
          onClose={() => setImportReport(null)}
        />
      )}

      {/* Unified Export dialog  — File ▸ Export…'s one entry
          point; the Format select decides glTF vs. STL vs. 3MF. The slicer
          formats' solid-gating confirmation below remains the follow-on
          step. */}
      {exportDialogOpen && (
        <ExportDialog
          onExport={handleExportFormat}
          onCancel={() => setExportDialogOpen(false)}
        />
      )}

      {/* Slicer-format solid-gating confirmation — shown before an STL/3MF
          export when any exported object is not a watertight solid; never
          silent repair (rule 4). */}
      {solidWarning !== null && (
        <StlExportDialog
          offenders={solidWarning.names}
          formatLabel={solidWarning.format === 'stl' ? 'STL' : '3MF'}
          onExport={() => {
            const format = solidWarning.format
            setSolidWarning(null)
            void (format === 'stl' ? doExportStl() : doExport3mf())
          }}
          onCancel={() => setSolidWarning(null)}
        />
      )}

      {/* Recovery prompt — shown once at startup when autosaved snapshots
          exist and nothing else was loaded yet. Lists every crashed
          document; Recover opens each beyond the first in its own window. */}
      {recoveryPrompt !== null && recoveryPrompt.length > 0 && (
        <RecoveryDialog
          listings={recoveryPrompt}
          onRecover={handleRecover}
          onDiscard={handleDiscardRecovery}
          onDismiss={handleDismissRecovery}
        />
      )}

      {/* Welcome screen — bare launches only (the startup-handoff effect
          gates on: no pending open, no recovery, primary window, and the
          persisted "show on startup" flag). Any successful load closes it. */}
      {welcomeOpen && (
        <WelcomeScreen
          recentFiles={recentFiles}
          onClose={() => setWelcomeOpen(false)}
          onOpen={() => {
            setWelcomeOpen(false)
            openDocumentRef.current()
          }}
          onOpenRecent={(path) => {
            setWelcomeOpen(false)
            openRecent(path)
          }}
          onOpenSample={(sample) => {
            setWelcomeOpen(false)
            void openSample(sample)
          }}
          showOnStartup={showWelcomeSetting}
          onShowOnStartupChange={(show) => {
            setShowWelcome(show)
            setShowWelcomeSetting(show)
          }}
          unit={welcomeUnit}
          onUnitChange={(format) => {
            // Persists + broadcasts; the subscribe effect above updates
            // welcomeUnit, so no local setState needed.
            setLengthUnit(format)
            // A small-scale unit implies a small model: re-frame the (still
            // blank) scene closer so the first shape lands at a sensible
            // size. Guarded on emptiness — choosing a unit must never move
            // the camera over real work.
            const scene = sceneRef.current
            if (scene !== null && isSceneEmpty(scene)) {
              viewportApi.current?.setHomeFraming(homeFramingScale(format))
            }
          }}
        />
      )}

      {/* Command palette (⌘K / Ctrl-K / Cmd+/ on macOS). Selecting a
          result reuses the exact same dispatch the native menu and every
          menu click already go through. */}
      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        onRun={(id) => menuActionRef.current(id)}
        extraEntries={paletteModelEntries}
        gates={{
          selection: selectedIds.length > 0 || selectedGuide !== null,
          canGroup: menuGates?.canGroup ?? false,
          canUngroup: menuGates?.canUngroup ?? false,
          canMakeComponent: menuGates?.canMakeComponent ?? false,
          canPlaceCopy: menuGates?.canPlaceCopy ?? false,
          canExplode: menuGates?.canExplode ?? false,
          canMakeUnique: menuGates?.canMakeUnique ?? false,
          canBoolean: menuGates?.canBoolean ?? false,
        }}
      />

      {/* Settings modal — web-only fallback (Tauri opens a real, separate OS
          window instead; see openSettings in App.tsx). */}
      {showSettingsModal && (
        <div
          onClick={() => setShowSettingsModal(false)}
          style={{
            position: 'fixed',
            inset: 0,
            background: 'var(--backdrop-dim)',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            zIndex: 300,
          }}
        >
          <div
            onClick={(e) => e.stopPropagation()}
            style={{
              position: 'relative',
              width: '520px',
              height: '380px',
              maxHeight: '80vh',
              overflow: 'hidden',
              background: 'var(--surface-window, #1a1a1a)',
              color: 'var(--text-secondary, #ddd)',
              border: '1px solid var(--border-hairline, #333)',
              borderRadius: '10px',
              boxShadow: 'var(--shadow-palette, 0 8px 32px rgba(0,0,0,0.6))',
            }}
          >
            {/* Same tabbed Settings UI the desktop's dedicated window shows. */}
            <SettingsWindow />
            <button
              aria-label="Close settings"
              onClick={() => setShowSettingsModal(false)}
              style={{
                position: 'absolute',
                top: '8px',
                right: '10px',
                background: 'none',
                border: 'none',
                color: 'var(--text-faint, #888)',
                cursor: 'pointer',
                fontSize: '16px',
                lineHeight: 1,
              }}
            >
              ×
            </button>
          </div>
        </div>
      )}

      {/* Windows desktop settings surface — a full-window page in the
          Windows 11 app-settings idiom (back arrow returns to the document;
          see openSettings). Rendered last so it overlays every panel. */}
      {showFluentSettings && (
        <FluentSettingsPage onBack={() => setShowFluentSettings(false)} />
      )}
    </main>
  )
}
