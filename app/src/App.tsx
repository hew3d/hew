import { useEffect, useState, useCallback, useRef, useMemo } from 'react'
import { flushSync } from 'react-dom'
import { loadKernel, type Scene } from './wasm/loader'
import Viewport, { type ViewportApi } from './viewport/Viewport'
import { DocumentTree } from './panels/DocumentTree'
import { MaterialPalette } from './panels/MaterialPalette'
import { MenuBar } from './panels/MenuBar'
import { TitleBar } from './TitleBar'
import { isLinux } from './platform'
import { TagsPanel } from './panels/TagsPanel'
import { ObjectInfoPanel } from './panels/ObjectInfoPanel'
import { FloatingPanel } from './panels/FloatingPanel'
import { nextSelection, canMakeComponent, canPlaceInstance, canExplodeInstance, canMakeUnique, nodeKey, type NodeRef } from './panels/treeModel'
import { tagPathKey, isPathUnder } from './panels/tagModel'
import { LogPanel } from './log/LogPanel'
import * as LogStore from './log/LogStore'
import { install as installConsoleCapture, restore as restoreConsoleCapture } from './log/consoleCapture'
import { MATERIAL_SENTINEL } from './tools/PaintTool'
import { makeFileHost, isTauri, type ImportReport } from './io/fileHost'
import {
  INITIAL_SESSION,
  deriveTitle,
  afterMutation,
  afterSave,
  afterOpen,
  afterImport,
  type DocSessionState,
} from './io/documentSession'
import { makeRecoveryStore, shouldPromptRecovery, type RecoverySnapshot, type RecoveryMeta } from './io/recoveryStore'
import { ImportReportDialog } from './panels/ImportReportDialog'
import { ImportingOverlay } from './panels/ImportingOverlay'
import { RecoveryDialog } from './panels/RecoveryDialog'
import { UnitsPane } from './settings/UnitsPane'
import { TOOL_ICON_SVG } from './tools/toolIcons'
import { modLabel } from './platform'

/** Autosave tick interval (ms). */
const AUTOSAVE_INTERVAL_MS = 12000

interface AppState {
  kernelVersion: string
  scene: Scene
}

interface Toast {
  id: number
  message: string
  code?: string
}

let toastCounter = 0

/** Extract the filename from an absolute path (cross-platform / or \). */
function basenameOf(path: string): string {
  return path.replace(/[/\\]+/g, '/').split('/').filter(Boolean).pop() ?? path
}

const TOOLS = ['Select', 'Rectangle', 'Push/Pull', 'Paint', 'Move', 'Rotate', 'Scale', 'Tape Measure', 'Protractor', 'Slice', 'Orbit', 'Pan', 'Zoom'] as const
type ToolName = (typeof TOOLS)[number]
// Canonical shortcuts use the ⌘ glyph; the toolbar-button tooltip swaps it for
// `modLabel` ('Ctrl+') on non-Mac hosts (e.g. Linux/WebKitGTK).
const TOOL_KEYS: Record<ToolName, string> = {
  'Select': 'Spc',
  'Rectangle': '⌘K',
  'Push/Pull': '⌘=',
  'Paint': '4',
  'Move': '⌘0',
  'Rotate': '⌘8',
  'Scale': '⌘9',
  'Tape Measure': '⌘D',
  'Protractor': '',
  'Slice': '',
  'Orbit': '⌘B',
  'Pan': '⌘R',
  'Zoom': '⌘\\',
}

/** Strings that signal the Scene borrow-lock after a Rust panic. */
const PANIC_SIGNATURES = ['recursive use of an object', 'unreachable']

function isPanicError(message: string): boolean {
  const lower = message.toLowerCase()
  return PANIC_SIGNATURES.some((sig) => lower.includes(sig))
}

/** Inline Material Symbols icon. The source SVGs carry no `fill`
 * attribute, so `fill="currentColor"` is spliced onto the root `<svg>` tag
 * here — letting the button's `color` style (active vs. idle) drive icon
 * color without a stylesheet (this codebase is inline-styles-only). */
function ToolIcon({ name }: { name: ToolName }) {
  // Material Symbols SVGs carry intrinsic width="48" height="48" attributes.
  // Strip those and inject the size we actually want, otherwise the glyph
  // renders at 48px inside an 18px span and bleeds into neighboring buttons.
  const svg = TOOL_ICON_SVG[name]
    .replace(/\swidth="[^"]*"/, '')
    .replace(/\sheight="[^"]*"/, '')
    .replace('<svg ', '<svg fill="currentColor" width="18" height="18" ')
  return (
    <span
      aria-hidden="true"
      style={{ width: '18px', height: '18px', display: 'block', overflow: 'hidden' }}
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  )
}

export default function App() {
  const [state, setState] = useState<AppState | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [toolName, setToolName] = useState<string>('Select')
  const [snapKind, setSnapKind] = useState<string | null>(null)
  const [measurement, setMeasurement] = useState<string>('')
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
  /** Document session: currentRef + dirty flag. */
  const [docSession, setDocSession] = useState<DocSessionState>(INITIAL_SESSION)
  /** Pane visibility: Model info (DocumentTree) */
  const [showModelInfo, setShowModelInfo] = useState(true)
  /** Pane visibility: Materials (MaterialPalette) */
  const [showMaterials, setShowMaterials] = useState(false)
  /** Pane visibility: Tags */
  const [showTags, setShowTags] = useState(false)
  /** Pane visibility: Object Info */
  const [showObjectInfo, setShowObjectInfo] = useState(true)
  /** Debug Log panel visibility (default hidden — opt-in via Window menu only). */
  const [showDebugLog, setShowDebugLog] = useState(false)
  /** View ▸ Axes / Guides visibility. Default both shown. */
  const [showAxes, setShowAxes] = useState(true)
  const [showGuides, setShowGuides] = useState(true)
  /** Settings modal visibility — web fallback only (Tauri opens a real OS window). */
  const [showSettingsModal, setShowSettingsModal] = useState(false)
  /**
   * Floating-panel z-order (draggable overlay panels). Each panel's
   * z-index is its position in this array (later = on top); clicking a panel
   * moves its id to the end. Base values stay well below the toast stack
   * (z 100) and other fixed overlays.
   */
  const [panelOrder, setPanelOrder] = useState<string[]>(['modelInfo', 'materials', 'tags', 'objectInfo'])
  const bringPanelToFront = useCallback((id: string) => {
    setPanelOrder((cur) => (cur[cur.length - 1] === id ? cur : [...cur.filter((p) => p !== id), id]))
  }, [])
  /** Tag-path hide set: each entry is tagPathKey(path). Cleared on load/new. */
  const [hiddenTagPaths, setHiddenTagPaths] = useState<Set<string>>(new Set())
  /** Import report to display (null = no dialog). */
  const [importReport, setImportReport] = useState<ImportReport | null>(null)
  /** True while import_dae is running (blocks main thread). */
  const [isImporting, setIsImporting] = useState(false)
  /** Display name of the file being imported (shown in the overlay). */
  const [importingName, setImportingName] = useState('')
  /** Recovery snapshot to offer at startup (null = no dialog). */
  const [recoveryPrompt, setRecoveryPrompt] = useState<RecoverySnapshot | null>(null)

  /** Imperative handle into the viewport (e.g. running a boolean). */
  const viewportApi = useRef<ViewportApi | null>(null)

  // Stable ref to the Scene for undo/redo button state queries
  const sceneRef = useRef<Scene | null>(null)
  // Bytes of a fresh blank scene, captured once after kernel load, used for New.
  const blankBytesRef = useRef<Uint8Array | null>(null)
  // Stable file host instance.
  const fileHostRef = useRef(makeFileHost())
  // Mirror of docSession kept up-to-date so callbacks can read current state
  // without capturing stale closures or causing impure updater functions.
  const docSessionRef = useRef<DocSessionState>(INITIAL_SESSION)
  // When true, handleDocumentChanged suppresses the dirty-marking setState
  // (used during programmatic loads so the post-load afterOpen wins).
  const suppressDirtyRef = useRef(false)
  // Stable recovery-store instance (autosave / crash recovery).
  const recoveryStoreRef = useRef(makeRecoveryStore())
  // True when a mutation has occurred since the last successful autosave tick
  // (or since the last explicit Save, which also resets it). Avoids redundant
  // writes when the document hasn't changed between ticks.
  const dirtySinceAutosaveRef = useRef(false)
  // Guards the startup recovery check so it runs exactly once even under
  // StrictMode's double-invoke or Vite HMR re-renders.
  const recoveryCheckedRef = useRef(false)

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
      recoveryStoreRef.current.write(bytes, meta).then(() => {
        dirtySinceAutosaveRef.current = false
      }).catch(() => { /* ignore — try again next tick */ })
    }, AUTOSAVE_INTERVAL_MS)
    return () => clearInterval(interval)
  }, [])

  // ---------------------------------------------------------------- startup recovery check
  // Runs once, after the scene first becomes available. Guarded by
  // recoveryCheckedRef so StrictMode's double-invoke / Vite HMR re-renders
  // can't trigger it twice or re-prompt after the user has already decided.
  // shouldPromptRecovery suppresses the prompt if anything else (e.g. a
  // cold-start file-association open) already populated the session.
  useEffect(() => {
    if (state === null) return
    if (recoveryCheckedRef.current) return
    recoveryCheckedRef.current = true
    recoveryStoreRef.current.read().then((snapshot) => {
      if (shouldPromptRecovery(docSessionRef.current, snapshot)) {
        setRecoveryPrompt(snapshot)
      }
    }).catch(() => { /* ignore — no recovery prompt */ })
  }, [state])

  // Update document.title whenever session state changes. Under Tauri, also
  // push the title to the native OS title bar — the in-app MenuBar no longer
  // renders a title for the native case, so the title now lives solely there.
  useEffect(() => {
    const title = deriveTitle(docSession)
    document.title = title
    if (isTauri) {
      import('@tauri-apps/api/window').then(({ getCurrentWindow }) => {
        getCurrentWindow().setTitle(title).catch(() => { /* ignore */ })
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

  const handleSceneChange = useCallback((wtMap: Map<bigint, boolean>) => {
    setWatertightMap(new Map(wtMap))
  }, [])

  const handleSelect = useCallback((node: NodeRef | null, additive: boolean) => {
    // Node and guide selection are mutually exclusive.
    setSelectedGuide(null)
    setSelectedIds((cur) => nextSelection(cur, node, additive))
  }, [])

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
  const handleDocumentChanged = useCallback(() => {
    setDocRev((r) => r + 1)
    setActiveContext((ctx) => {
      const scene = sceneRef.current
      if (scene === null) return ctx
      return trimContextPath(scene, ctx)
    })
    // Mark the document dirty on any mutation — but NOT during programmatic
    // loads (suppressDirtyRef is true while applyLoadedBytes calls notifyLoaded).
    if (!suppressDirtyRef.current) {
      setDocSession((s) => afterMutation(s))
      dirtySinceAutosaveRef.current = true
    }
  }, [trimContextPath])

  const handleToast = useCallback((message: string, code?: string) => {
    const isError = code !== undefined &&
      ['WouldVanish', 'NonManifoldResult', 'ObjectNotSolid', 'DegenerateGeometry',
        'OperandNotSolid', 'DegenerateContact', 'EmptyResult', 'SingularTransform'].includes(code)
    const level = isError ? 'error' : 'warn'
    const logMessage = code !== undefined ? `[${code}] ${message}` : message
    LogStore.log[level]('tool', logMessage)

    if (isPanicError(message)) {
      setKernelPanicked(true)
    }

    const id = ++toastCounter
    setToasts((prev) => [...prev, { id, message, code }])
    setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id))
    }, 4000)
  }, [])

  const dismissToast = useCallback((id: number) => {
    setToasts((prev) => prev.filter((t) => t.id !== id))
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
      handleToast(raw)
      return false
    }
    setSelectedIds([])
    setActiveContext([])
    setHiddenKeys(new Set())
    setHiddenTagPaths(new Set())
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
    return true
  }, [handleToast])

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

  // ---------------------------------------------------------------- document lifecycle

  const newDocument = useCallback(async () => {
    if (!(await confirmDiscard())) return
    const blank = blankBytesRef.current
    if (blank === null) return
    if (applyLoadedBytes(blank)) setDocSession(afterOpen(null))
  }, [confirmDiscard, applyLoadedBytes])

  const openDocument = useCallback(async () => {
    if (!(await confirmDiscard())) return
    fileHostRef.current.open().then((result) => {
      if (result === null) return // user cancelled
      const ok = applyLoadedBytes(result.bytes)
      if (!ok) return
      setDocSession(afterOpen(result.ref))
      if (isTauri && typeof result.ref.handle === 'string') {
        import('@tauri-apps/api/core').then(({ invoke }) =>
          invoke('push_recent', { path: result.ref.handle as string })
        ).catch(() => { /* ignore */ })
      }
    }).catch((err: unknown) => {
      handleToast(`Open failed: ${String(err)}`)
    })
  }, [confirmDiscard, applyLoadedBytes, handleToast])

  const importDocument = useCallback(async () => {
    const scene = sceneRef.current
    if (scene === null) return

    // Step 1: guard unsaved changes BEFORE showing any file dialog.
    // If the user cancels the discard prompt, we leave the current document
    // completely untouched.
    if (!(await confirmDiscard())) return

    // Step 2: show the file-open dialog.  If the user cancels (null), we
    // return immediately without touching the current document.
    let result: Awaited<ReturnType<typeof fileHostRef.current.openForImport>>
    try {
      result = await fileHostRef.current.openForImport()
    } catch (err: unknown) {
      handleToast(`Import failed: ${String(err)}`)
      return
    }
    if (result === null) return // user cancelled — current document unchanged

    // Step 3: show the overlay BEFORE any blocking work.
    //
    // flushSync forces a synchronous DOM commit so the overlay card is in the
    // DOM immediately, rather than waiting for React's next async render cycle.
    // After the commit we wait one requestAnimationFrame so the browser has a
    // chance to actually paint the committed DOM before we freeze the main thread.
    //
    // NOTE: import_dae runs synchronously on the main thread, so the CSS
    // spinner animation will freeze while it parses.  The text message still
    // communicates progress.  True smooth animation would require running the
    // import in a Web Worker (future work — needs a SharedArrayBuffer channel
    // to the WASM module).
    flushSync(() => {
      setImportingName(result!.name)
      setIsImporting(true)
    })
    await new Promise<void>((r) => requestAnimationFrame(() => r()))

    let report: ImportReport
    try {
      // Step 4: reset to a blank document first (replace semantics).
      //
      // applyLoadedBytes(blank) calls scene.load(), clears selection/context,
      // and calls notifyLoaded() under suppressDirtyRef so the dirty mark is
      // suppressed — the afterImport state below owns the dirty flag.
      // We must bail if the blank load fails (should never happen in practice).
      const blank = blankBytesRef.current
      if (blank === null) return
      const blankOk = applyLoadedBytes(blank)
      if (!blankOk) return

      // Step 5: import into the now-empty document, dispatched by format.
      report = (result!.kind === 'gltf'
        ? scene.import_gltf(result!.bytes)
        : scene.import_dae(
            result!.bytes,
            Object.keys(result!.images).length > 0 ? result!.images : null,
          )) as ImportReport
    } catch (err: unknown) {
      const raw = err instanceof Error ? err.message : String(err)
      handleToast(`Import failed: ${raw}`)
      return
    } finally {
      // Always clear the overlay — even on throw, so it can never get stuck.
      setIsImporting(false)
    }

    // Step 6: tessellate the imported objects and update session state.
    //
    // notifyLoaded() calls handleSceneRefresh() which tessellates the new
    // objects and bumps docRev.  suppressDirtyRef is false here so the dirty
    // mark would normally fire — but we immediately set afterImport() which
    // owns dirty=true, so the net effect is correct.
    viewportApi.current?.notifyLoaded()

    // Step 7: commit the session state.
    //
    // afterImport() sets currentRef=null (so Save always prompts — no silent
    // overwrite risk on either WebFileHost or TauriFileHost) and dirty=true.
    // The importedName is used by deriveTitle (window title) and by
    // saveAsDocument's suggested filename.
    setDocSession(afterImport(result!.name))

    setImportReport(report)
    const fmt = result!.kind === 'gltf' ? 'glTF' : 'DAE'
    LogStore.log.info('app', `Imported ${fmt}: ${report.objects_created} objects (${report.watertight} solid, ${report.leaky} leaky)`)
    requestAnimationFrame(() => { viewportApi.current?.zoomExtents() })
  }, [confirmDiscard, handleToast, applyLoadedBytes])

  const saveDocument = useCallback(() => {
    const scene = sceneRef.current
    if (scene === null) return
    const bytes = new Uint8Array(scene.save())
    const ref = docSession.currentRef
    fileHostRef.current.save(bytes, ref).then((newRef) => {
      if (newRef === null) return // user cancelled
      setDocSession(afterSave(newRef))
      LogStore.log.info('app', `Saved: ${newRef.name}`)
      // The work is now safely on disk — drop the autosave snapshot.
      recoveryStoreRef.current.clear().catch(() => { /* ignore */ })
      dirtySinceAutosaveRef.current = false
      if (isTauri && typeof newRef.handle === 'string') {
        import('@tauri-apps/api/core').then(({ invoke }) =>
          invoke('push_recent', { path: newRef.handle as string })
        ).catch(() => { /* ignore */ })
      }
    }).catch((err: unknown) => {
      handleToast(`Save failed: ${String(err)}`)
    })
  }, [docSession.currentRef, handleToast])

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
      setDocSession(afterSave(newRef))
      LogStore.log.info('app', `Saved as: ${newRef.name}`)
      // The work is now safely on disk — drop the autosave snapshot.
      recoveryStoreRef.current.clear().catch(() => { /* ignore */ })
      dirtySinceAutosaveRef.current = false
      if (isTauri && typeof newRef.handle === 'string') {
        import('@tauri-apps/api/core').then(({ invoke }) =>
          invoke('push_recent', { path: newRef.handle as string })
        ).catch(() => { /* ignore */ })
      }
    }).catch((err: unknown) => {
      handleToast(`Save As failed: ${String(err)}`)
    })
  }, [docSession.currentRef, docSession.importedName, handleToast])

  // ---------------------------------------------------------------- open by path (Tauri only — used by drag-drop, recents, and file association)
  // Reads the file at `path` via Tauri invoke, applies it, and sets session state.
  const openPath = useCallback(async (path: string) => {
    if (!(await confirmDiscard())) return
    const { invoke } = await import('@tauri-apps/api/core')
    const raw: number[] = await invoke('read_file', { path })
    const bytes = new Uint8Array(raw)
    if (applyLoadedBytes(bytes)) {
      setDocSession(afterOpen({ name: basenameOf(path), handle: path }))
      invoke('push_recent', { path }).catch(() => { /* ignore */ })
    }
  }, [confirmDiscard, applyLoadedBytes])

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

  const openRecent = useCallback((path: string) => { void openPath(path) }, [openPath])
  const clearRecent = useCallback(() => {
    import('@tauri-apps/api/core')
      .then(({ invoke }) => invoke('clear_recent'))
      .then(() => setRecentFiles([]))
      .catch(() => { /* ignore */ })
  }, [])

  // ---------------------------------------------------------------- recovery prompt actions
  const handleRecover = useCallback(() => {
    const snapshot = recoveryPrompt
    if (snapshot === null) return
    const ok = applyLoadedBytes(snapshot.bytes)
    if (ok) {
      const { meta } = snapshot
      setDocSession({
        currentRef: meta.path !== null ? { name: meta.name, handle: meta.path } : null,
        dirty: true,
        importedName: meta.path !== null ? undefined : meta.name,
      })
      // The recovered document still only exists in the recovery snapshot —
      // leave it in place (the next autosave tick refreshes it) and mark
      // dirty-since-autosave so a tick will actually fire if nothing else changes.
      dirtySinceAutosaveRef.current = true
    }
    setRecoveryPrompt(null)
  }, [recoveryPrompt, applyLoadedBytes])

  const handleDiscardRecovery = useCallback(() => {
    recoveryStoreRef.current.clear().catch(() => { /* ignore */ })
    setRecoveryPrompt(null)
  }, [])

  // Escape closes the prompt WITHOUT clearing — the snapshot survives and is
  // re-offered next launch, so an accidental keypress can't lose work.
  const handleDismissRecovery = useCallback(() => {
    setRecoveryPrompt(null)
  }, [])

  // ---------------------------------------------------------------- undo/redo for Edit menu
  const handleUndo = useCallback(() => {
    viewportApi.current?.runUndo()
  }, [])

  const handleRedo = useCallback(() => {
    viewportApi.current?.runRedo()
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
      handleToast(`Export failed: ${String(err)}`)
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
      handleToast(`Export failed: ${String(err)}`)
    }
  }, [docSession.currentRef, docSession.importedName, handleToast])

  // ---------------------------------------------------------------- settings window
  // Under Tauri: a separate, free-floating OS webview window (movable outside
  // the main Hew window). On web: there's no concept of a second OS window,
  // so we fall back to an in-app modal (showSettingsModal below).
  const openSettings = useCallback(() => {
    if (isTauri) {
      import('@tauri-apps/api/webviewWindow').then(async ({ WebviewWindow }) => {
        const existing = await WebviewWindow.getByLabel('settings')
        if (existing !== null) {
          await existing.setFocus()
          return
        }
        new WebviewWindow('settings', {
          url: 'index.html#settings',
          title: 'Settings',
          width: 520,
          height: 380,
          resizable: true,
        })
      }).catch(() => { /* ignore */ })
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
  const exportGltfRef = useRef(exportGltf)
  const openPathRef = useRef(openPath)
  const openSettingsRef = useRef(openSettings)
  // Tracks the latest active tool name for the keydown effect below (fixed
  // dep array — see its Delete/Backspace handling), so it never sees a stale
  // tool from the render it mounted in.
  const activeToolRef = useRef(activeTool)
  useEffect(() => { newDocumentRef.current = newDocument }, [newDocument])
  useEffect(() => { openDocumentRef.current = openDocument }, [openDocument])
  useEffect(() => { importDocumentRef.current = importDocument }, [importDocument])
  useEffect(() => { saveDocumentRef.current = saveDocument }, [saveDocument])
  useEffect(() => { saveAsDocumentRef.current = saveAsDocument }, [saveAsDocument])
  useEffect(() => { handleUndoRef.current = handleUndo }, [handleUndo])
  useEffect(() => { handleRedoRef.current = handleRedo }, [handleRedo])
  useEffect(() => { handleZoomExtentsRef.current = handleZoomExtents }, [handleZoomExtents])
  useEffect(() => { exportGltfRef.current = exportGltf }, [exportGltf])
  useEffect(() => { openPathRef.current = openPath }, [openPath])
  useEffect(() => { openSettingsRef.current = openSettings }, [openSettings])
  useEffect(() => { activeToolRef.current = activeTool }, [activeTool])

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
    switch (payload) {
      case 'new':      newDocumentRef.current(); break
      case 'open':     openDocumentRef.current(); break
      case 'import':   importDocumentRef.current(); break
      case 'export':   exportGltfRef.current(); break
      case 'save':     saveDocumentRef.current(); break
      case 'save-as':  saveAsDocumentRef.current(); break
      case 'undo':     handleUndoRef.current(); break
      case 'redo':     handleRedoRef.current(); break
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
      case 'tool-pushpull':  setActiveTool('Push/Pull'); break
      case 'tool-paint':     setActiveTool('Paint'); break
      case 'tool-move':      setActiveTool('Move'); break
      case 'tool-rotate':    setActiveTool('Rotate'); break
      case 'tool-scale':     setActiveTool('Scale'); break
      case 'tool-tape-measure': setActiveTool('Tape Measure'); break
      case 'tool-protractor': setActiveTool('Protractor'); break
      case 'tool-slice':     setActiveTool('Slice'); break
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
      case 'toggle-guides':       setShowGuides((v) => !v); break
      case 'edit-delete-guides':  viewportApi.current?.deleteAllGuides(); break
      case 'zoom-extents':        handleZoomExtentsRef.current(); break
      case 'open-settings':       openSettingsRef.current(); break
    }
  }

  // ---------------------------------------------------------------- native menu-action listener (Tauri only)
  // Registered once; dispatches through menuActionRef so the handler is always
  // the latest one (HMR/StrictMode safe).
  useEffect(() => {
    if (!isTauri) return
    let unlisten: (() => void) | undefined
    let cancelled = false
    import('@tauri-apps/api/event').then(({ listen }) => {
      return listen<string>('menu-action', (event) => {
        menuActionRef.current(event.payload)
      })
    }).then((fn) => { if (cancelled) fn(); else unlisten = fn }).catch(() => { /* ignore if not in Tauri */ })
    return () => { cancelled = true; unlisten?.() }
  }, [])

  // ---------------------------------------------------------------- menu-open-path listener (Tauri only)
  // Emitted by Rust when a recent-file menu item is clicked, or when a file
  // is opened via the macOS "open document" Apple event (warm case).
  useEffect(() => {
    if (!isTauri) return
    let unlisten: (() => void) | undefined
    let cancelled = false
    import('@tauri-apps/api/event').then(({ listen }) => {
      return listen<string>('menu-open-path', (event) => {
        openPathRef.current(event.payload)
      })
    }).then((fn) => { if (cancelled) fn(); else unlisten = fn }).catch(() => { /* ignore */ })
    return () => { cancelled = true; unlisten?.() }
  }, []) // openPath accessed via openPathRef — no dep needed

  // ---------------------------------------------------------------- cold-start file association (Tauri only)
  // On first mount, check whether Rust buffered a file path from a cold-start
  // "open with" (macOS Apple event before the webview listener existed, or
  // argv on Windows/Linux).
  useEffect(() => {
    if (!isTauri) return
    import('@tauri-apps/api/core').then(({ invoke }) => {
      return invoke<string | null>('take_pending_open')
    }).then((path) => {
      if (path != null) openPathRef.current(path)
    }).catch(() => { /* ignore */ })
  }, []) // runs once on mount; openPath accessed via openPathRef

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
        }
        // Force-close, bypassing onCloseRequested (no loop).
        await win.destroy()
      })
    }).then((fn) => { if (cancelled) fn(); else unlisten = fn }).catch(() => { /* ignore */ })
    return () => { cancelled = true; unlisten?.() }
  }, []) // reads docSessionRef (always current) — no dep needed

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
    // macOS/Windows Tauri use the native menu bar, which owns all keyboard
    // shortcuts — the JS handler must not double-fire them. On Linux the shell
    // is borderless with the in-app web menu (no native accelerators), so the
    // JS handler is the only shortcut source there (as on the web).
    if (isTauri && !isLinux) return

    const onKeyDown = (ev: KeyboardEvent) => {
      const isMod = ev.metaKey || ev.ctrlKey

      // Don't fire shortcuts while typing in an input/textarea
      const target = ev.target as HTMLElement
      const isTyping = target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable

      // Space → Select (no modifier required; guard against typing contexts)
      if (!isMod && ev.key === ' ' && !isTyping) {
        ev.preventDefault()
        setActiveTool('Select')
        return
      }

      // (Delete/Backspace handled by a dedicated always-on effect below, since
      // this whole handler is disabled under Tauri.)

      if (!isMod) return
      if (isTyping) return

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
      // Tool shortcuts
      if (ev.key === 'k' && !ev.shiftKey) {
        ev.preventDefault()
        setActiveTool('Rectangle')
        return
      }
      if (ev.key === '0') {
        ev.preventDefault()
        setActiveTool('Move')
        return
      }
      if (ev.key === '8') {
        ev.preventDefault()
        setActiveTool('Rotate')
        return
      }
      if (ev.key === '9') {
        ev.preventDefault()
        setActiveTool('Scale')
        return
      }
      if (ev.key === '=') {
        ev.preventDefault()
        setActiveTool('Push/Pull')
        return
      }
      if (ev.key.toLowerCase() === 'd' && !ev.shiftKey) {
        ev.preventDefault()
        setActiveTool('Tape Measure')
        return
      }
      if (ev.key === 'b' && !ev.shiftKey) {
        ev.preventDefault()
        setActiveTool('Orbit')
        return
      }
      if (ev.key === 'r' && !ev.shiftKey) {
        ev.preventDefault()
        setActiveTool('Pan')
        return
      }
      if (ev.key === '\\') {
        ev.preventDefault()
        setActiveTool('Zoom')
        return
      }
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
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [saveDocument, saveAsDocument, openDocument, newDocument])

  // Delete / Backspace → delete the current selection (guides).
  // Registered SEPARATELY from the global-shortcut effect above because that one
  // is disabled under Tauri (the native menu owns accelerators) — but Edit ▸
  // Delete has *no* native accelerator (a bare Delete/Backspace would bypass the
  // typing + Select-tool guards and collide with the tools' VCB Backspace), so
  // the key must be handled in JS on BOTH web and desktop. Gating to the Select
  // tool keeps it from stealing Backspace mid-typed-entry in other tools.
  useEffect(() => {
    const onDeleteKey = (ev: KeyboardEvent) => {
      if (ev.key !== 'Delete' && ev.key !== 'Backspace') return
      const target = ev.target as HTMLElement
      const isTyping =
        target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable
      if (isTyping) return // let the focused field edit text normally
      // ALWAYS swallow Delete/Backspace outside text fields: the webview
      // otherwise treats Backspace as "navigate back", which silently wedges
      // the whole app until restart. (This is why delete "stopped working" after
      // a stray Backspace in a non-Select tool.) Only the Select tool deletes.
      ev.preventDefault()
      if (activeToolRef.current === 'Select') {
        menuActionRef.current('edit-delete')
      }
    }
    window.addEventListener('keydown', onDeleteKey)
    return () => window.removeEventListener('keydown', onDeleteKey)
  }, [])

  // Mirror the View ▸ Axes / Guides toggles into the viewport. The
  // viewport API ref is populated once the viewport mounts; both default to
  // visible so an early run before the ref is ready is a harmless no-op.
  useEffect(() => { viewportApi.current?.setAxesVisible(showAxes) }, [showAxes])
  useEffect(() => { viewportApi.current?.setGuidesVisible(showGuides) }, [showGuides])

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
      const ok = applyLoadedBytes(new Uint8Array(buf))
      if (ok) {
        setDocSession(afterOpen({ name: file.name, handle: null }))
      }
    }).catch((err: unknown) => {
      handleToast(`Drop open failed: ${String(err)}`)
    })
  }, [confirmDiscard, applyLoadedBytes, handleToast])

  // ── Hide/Show + Tags (must be declared BEFORE the early returns below so the
  // hook count is stable across the loading and loaded renders — Rules of Hooks).
  /** Collect all leaf object and instance ids for a node (recurse into groups). */
  const collectLeafIds = (node: NodeRef): { objectIds: bigint[]; instanceIds: bigint[] } => {
    if (node.kind === 'object') return { objectIds: [node.id], instanceIds: [] }
    if (node.kind === 'instance') return { objectIds: [], instanceIds: [node.id] }
    // Group: recurse into members; group_members() returns {kind, id} values
    // via nodeRefFromJs. We recurse manually so instances inside groups are caught.
    const members = state!.scene.group_members(node.id)
    const objectIds: bigint[] = []
    const instanceIds: bigint[] = []
    for (let i = 0; i < members.length; i++) {
      const m = members[i]
      const mKind: NodeRef['kind'] = m.kind as NodeRef['kind']
      const child: NodeRef = { kind: mKind, id: m.id }
      const { objectIds: os, instanceIds: is_ } = collectLeafIds(child)
      objectIds.push(...os)
      instanceIds.push(...is_)
    }
    return { objectIds, instanceIds }
  }

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

  const handleToggleHidden = (node: NodeRef) => {
    const key = nodeKey(node)
    // Compute the next hidden set purely (no setState updater side effects).
    const next = new Set(hiddenKeys)
    if (next.has(key)) {
      next.delete(key)
    } else {
      next.add(key)
    }
    setHiddenKeys(next)
    pushUnionHidden(next, hiddenTagPaths)
  }

  const handleToggleTagPath = useCallback((path: string[]) => {
    const key = tagPathKey(path)
    const next = new Set(hiddenTagPaths)
    if (next.has(key)) {
      next.delete(key)
    } else {
      next.add(key)
    }
    setHiddenTagPaths(next)
    pushUnionHidden(hiddenKeys, next)
  }, [hiddenTagPaths, hiddenKeys, pushUnionHidden])

  if (error !== null) {
    return (
      <main style={{ fontFamily: 'sans-serif', padding: '1rem', color: 'red' }}>
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

  // Booleans require exactly two selected OBJECTS at top level.
  const objectIdSet = new Set(Array.from(state.scene.object_ids()))
  const booleanOperands = selectedIds.filter(
    (n) => n.kind === 'object' && objectIdSet.has(n.id),
  )
  const canBoolean = activeContext.length === 0 && booleanOperands.length === 2

  // Make Component: sibling multi-selection of objects/groups (no instances).
  const parentOf = (n: NodeRef) => {
    const k = n.kind === 'group' ? 1 : n.kind === 'instance' ? 2 : 0
    return state.scene.node_parent(k, n.id)
  }
  const canMakeComp = activeContext.length === 0 && canMakeComponent(selectedIds, parentOf)
  const canPlace = canPlaceInstance(selectedIds)
  const canExplode = canExplodeInstance(selectedIds)
  const canUnique = canMakeUnique(selectedIds)
  const handleBoolean = (op: number) => {
    if (booleanOperands.length === 2) {
      viewportApi.current?.runBoolean(op, booleanOperands[0].id, booleanOperands[1].id)
    }
  }

  const handleGroup = () => {
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
        fontFamily: 'sans-serif',
        position: 'relative',
        display: 'flex',
        flexDirection: 'column',
        height: '100%',
        boxSizing: 'border-box',
        overflow: 'hidden',
        background: '#1a1a1a',
      }}
    >
      {/* Linux desktop shell: borderless window → draw our own title bar
          (KWin won't repaint the native one after setTitle). */}
      {isTauri && isLinux && <TitleBar title={deriveTitle(docSession)} />}

      {/* App bar / menu bar.
          On macOS/Windows Tauri the native OS menu bar owns File/Edit (this
          renders nothing). On the web and the Linux borderless shell, the in-app
          bar renders the menus; the centered title is shown by TitleBar on Linux
          so it is hidden here in that case. */}
      <MenuBar
        title={deriveTitle(docSession)}
        nativeMenuBar={isTauri && !isLinux}
        hideTitle={isTauri && isLinux}
        onNew={newDocument}
        onOpen={openDocument}
        onSave={saveDocument}
        onSaveAs={saveAsDocument}
        onImport={importDocument}
        onExport={exportGltf}
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
        showGuides={showGuides}
        onToggleAxes={() => setShowAxes((v) => !v)}
        onToggleGuides={() => setShowGuides((v) => !v)}
        onDeleteGuides={() => viewportApi.current?.deleteAllGuides()}
        onDelete={deleteSelection}
        onZoomExtents={handleZoomExtents}
        onOpenSettings={openSettings}
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

      {/* Toolbar — tools + watertight badge */}
      <div
        style={{
          display: 'flex',
          gap: '6px',
          padding: '4px 6px',
          background: '#2a2a2a',
          borderBottom: '1px solid #3a3a3a',
          alignItems: 'center',
          flexShrink: 0,
        }}
      >
        {TOOLS.map((t) => (
          <button
            key={t}
            onClick={() => setActiveTool(t)}
            title={TOOL_KEYS[t] === '' ? t : `${t} (${TOOL_KEYS[t].replace('⌘', modLabel)})`}
            style={{
              width: '30px',
              height: '30px',
              padding: 0,
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              cursor: 'pointer',
              background: activeTool === t ? '#5588cc' : '#444',
              color: activeTool === t ? '#fff' : '#ccc',
              border: activeTool === t ? '1px solid #7aaaee' : '1px solid #555',
              borderRadius: '3px',
            }}
          >
            <ToolIcon name={t} />
          </button>
        ))}

        <div style={{ flex: 1 }} />

        {/* Watertight badge */}
        {objectCount > 0 && (
          <span
            style={{
              padding: '2px 8px',
              fontSize: '11px',
              borderRadius: '3px',
              background: allWatertight ? '#1a7a3a' : '#cc3322',
              color: '#fff',
              fontFamily: 'monospace',
            }}
          >
            {allWatertight
              ? `${objectCount} object${objectCount !== 1 ? 's' : ''} ✓ solid`
              : `${leakyCount} leaky`}
          </span>
        )}
      </div>

      {/* Viewport — fills the full width of the main area; floating panels
          overlay it instead of sharing a flex row. */}
      <div style={{ flex: 1, minHeight: 0, display: 'flex', padding: '8px 8px 0 8px' }}>
        <div
          style={{ flex: 1, minWidth: 0, position: 'relative' }}
          onDragOver={handleDragOver}
          onDrop={handleDrop}
        >
          <Viewport
            wasmScene={state.scene}
            onStatusChange={handleStatusChange}
            onSceneChange={handleSceneChange}
            onToast={handleToast}
            activeTool={activeTool}
            activeContext={activeContext}
            selectedIds={selectedIds}
            activeLitSet={activeLitSet}
            onSelect={handleSelect}
            onSelectGuide={handleSelectGuide}
            selectedGuide={selectedGuide}
            onEnterContext={handleEnterContext}
            onExitContext={handleExitContext}
            onDocumentChanged={handleDocumentChanged}
            apiRef={viewportApi}
            onMeasurement={handleMeasurement}
            currentMaterialId={currentMaterialId}
          />

          {/* Floating panels — overlaid on the viewport. Free-drag with
              magnetic docking against the container edges and sibling
              panels (see FloatingPanel.tsx). z-index stays below the toast
              stack (100) and other fixed overlays. */}
          {showModelInfo && (
            <FloatingPanel
              panelId="modelInfo"
              title="Model Info"
              defaultPosition={{ x: window.innerWidth - 300, y: 16 }}
              width={280}
              zIndex={10 + panelOrder.indexOf('modelInfo')}
              onFocus={() => bringPanelToFront('modelInfo')}
              onClose={() => setShowModelInfo(false)}
            >
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
                canBoolean={canBoolean}
                onBoolean={handleBoolean}
                onGroup={handleGroup}
                onUngroup={handleUngroup}
                canMakeComponent={canMakeComp}
                onMakeComponent={handleMakeComponent}
                canPlaceInstance={canPlace}
                onPlaceInstance={handlePlaceInstance}
                canExplodeInstance={canExplode}
                onExplodeInstance={handleExplodeInstance}
                canMakeUnique={canUnique}
                onMakeUnique={handleMakeUnique}
                hiddenKeys={hiddenKeys}
                onToggleHidden={handleToggleHidden}
              />
            </FloatingPanel>
          )}
          {showObjectInfo && (
            <FloatingPanel
              panelId="objectInfo"
              title="Object Info"
              // Directly below Model Info by default, so the two stack vertically.
              defaultPosition={{ x: window.innerWidth - 300, y: 250 }}
              width={280}
              zIndex={10 + panelOrder.indexOf('objectInfo')}
              onFocus={() => bringPanelToFront('objectInfo')}
              onClose={() => setShowObjectInfo(false)}
            >
              <ObjectInfoPanel
                scene={state.scene}
                docRev={docRev}
                selectedIds={selectedIds}
                onDocumentChanged={handleDocumentChanged}
              />
            </FloatingPanel>
          )}
          {showMaterials && (
            <FloatingPanel
              panelId="materials"
              title="Materials"
              defaultPosition={{ x: 16, y: 16 }}
              width={260}
              zIndex={10 + panelOrder.indexOf('materials')}
              onFocus={() => bringPanelToFront('materials')}
              onClose={() => setShowMaterials(false)}
            >
              <MaterialPalette
                scene={state.scene}
                docRev={docRev}
                currentMaterialId={currentMaterialId}
                onSelectMaterial={setCurrentMaterialId}
                onDocumentChanged={handleDocumentChanged}
                selectedIds={selectedIds}
              />
            </FloatingPanel>
          )}
          {showTags && (
            <FloatingPanel
              panelId="tags"
              title="Tags"
              defaultPosition={{ x: 300, y: 16 }}
              width={260}
              zIndex={10 + panelOrder.indexOf('tags')}
              onFocus={() => bringPanelToFront('tags')}
              onClose={() => setShowTags(false)}
            >
              <TagsPanel
                scene={state.scene}
                docRev={docRev}
                hiddenTagPaths={hiddenTagPaths}
                onToggleTagPath={handleToggleTagPath}
              />
            </FloatingPanel>
          )}

          {/* Toast stack — positioned inside the viewport container, above
              floating panels (z 100 vs panel z 10-13). */}
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
                  background: toast.code !== undefined && ['WouldVanish', 'NonManifoldResult', 'ObjectNotSolid'].includes(toast.code)
                    ? '#cc3322'
                    : '#333',
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
      </div>

      {/* Status bar — bottom strip, above the Debug Log panel when shown.
          Extra bottom padding avoids descender clipping under macOS Tahoe's
          rounded window corners. */}
      <div
        style={{
          padding: '4px 8px 10px 8px',
          background: '#222',
          color: '#eee',
          fontFamily: 'monospace',
          fontSize: '12px',
          borderTop: '1px solid #3a3a3a',
          display: 'flex',
          gap: '1.5rem',
          flexShrink: 0,
        }}
      >
        <span>Tool: <strong>{toolName}</strong></span>
        <span>Snap: {snapKind ?? '—'}</span>
        <span>Measurements: {measurement !== '' ? measurement : '—'}</span>
        {activeTool === 'Paint' && (
          <span style={{ color: '#aaa', fontSize: '11px' }}>
            Click: paint face | Cmd/Ctrl+click: fill whole object
          </span>
        )}
        <span style={{ color: '#888', fontSize: '11px' }}>
          Middle-drag: orbit | Shift+Middle: pan | Scroll: zoom
        </span>
        <span style={{ color: '#888', fontSize: '11px', marginLeft: 'auto' }}>
          Undo: Cmd/Ctrl+Z | Redo: Shift+Cmd/Ctrl+Z
        </span>
      </div>

      {/* Debug Log panel — opt-in via Window menu, default hidden. */}
      {showDebugLog && <LogPanel panelHeight={160} />}

      {/* Importing overlay — shown while import_dae blocks the main thread.
          The overlay is painted before the blocking call via a double rAF in
          importDocument.  isImporting is always cleared in a finally block,
          so a thrown import error can never leave the overlay stuck. */}
      {isImporting && <ImportingOverlay fileName={importingName} />}

      {/* Import report modal — shown after a successful COLLADA import */}
      {importReport !== null && (
        <ImportReportDialog
          report={importReport}
          onClose={() => setImportReport(null)}
        />
      )}

      {/* Recovery prompt — shown once at startup when an autosaved snapshot
          exists and nothing else was loaded yet. */}
      {recoveryPrompt !== null && (
        <RecoveryDialog
          snapshot={recoveryPrompt}
          onRecover={handleRecover}
          onDiscard={handleDiscardRecovery}
          onDismiss={handleDismissRecovery}
        />
      )}

      {/* Settings modal — web-only fallback (Tauri opens a real, separate OS
          window instead; see openSettings in App.tsx). */}
      {showSettingsModal && (
        <div
          onClick={() => setShowSettingsModal(false)}
          style={{
            position: 'fixed',
            inset: 0,
            background: 'rgba(0,0,0,0.5)',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            zIndex: 300,
          }}
        >
          <div
            onClick={(e) => e.stopPropagation()}
            style={{
              width: '420px',
              maxHeight: '80vh',
              overflowY: 'auto',
              background: '#1a1a1a',
              color: '#ddd',
              border: '1px solid #333',
              borderRadius: '6px',
              boxShadow: '0 8px 32px rgba(0,0,0,0.6)',
              padding: '16px 20px',
            }}
          >
            <div
              style={{
                display: 'flex',
                justifyContent: 'space-between',
                alignItems: 'center',
                marginBottom: '12px',
              }}
            >
              <span style={{ fontSize: '14px', fontWeight: 600, color: '#eee' }}>Settings</span>
              <button
                onClick={() => setShowSettingsModal(false)}
                style={{
                  background: 'none',
                  border: 'none',
                  color: '#888',
                  cursor: 'pointer',
                  fontSize: '16px',
                  lineHeight: 1,
                }}
              >
                ×
              </button>
            </div>
            <UnitsPane />
          </div>
        </div>
      )}
    </main>
  )
}
