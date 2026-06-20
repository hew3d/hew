import { useEffect, useState, useCallback, useRef, useMemo } from 'react'
import { flushSync } from 'react-dom'
import { loadKernel, type Scene } from './wasm/loader'
import Viewport, { type ViewportApi } from './viewport/Viewport'
import { DocumentTree } from './panels/DocumentTree'
import { MaterialPalette } from './panels/MaterialPalette'
import { MenuBar } from './panels/MenuBar'
import { nextSelection, canMakeComponent, canPlaceInstance, canExplodeInstance, canMakeUnique, type NodeRef } from './panels/treeModel'
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
import { ImportReportDialog } from './panels/ImportReportDialog'
import { ImportingOverlay } from './panels/ImportingOverlay'

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

const TOOLS = ['Select', 'Rectangle', 'Push/Pull', 'Paint', 'Move', 'Rotate', 'Scale', 'Orbit', 'Pan', 'Zoom'] as const
type ToolName = (typeof TOOLS)[number]
const TOOL_KEYS: Record<ToolName, string> = {
  'Select': 'Spc',
  'Rectangle': '⌘K',
  'Push/Pull': '⌘=',
  'Paint': '4',
  'Move': '⌘0',
  'Rotate': '⌘8',
  'Scale': '⌘9',
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
  const [showMaterials, setShowMaterials] = useState(true)
  /** Import report to display (null = no dialog). */
  const [importReport, setImportReport] = useState<ImportReport | null>(null)
  /** True while import_dae is running (blocks main thread). */
  const [isImporting, setIsImporting] = useState(false)
  /** Display name of the file being imported (shown in the overlay). */
  const [importingName, setImportingName] = useState('')

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

  // Update document.title whenever session state changes.
  useEffect(() => {
    document.title = deriveTitle(docSession)
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
    setSelectedIds((cur) => nextSelection(cur, node, additive))
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

      // Step 5: import the DAE into the now-empty document.
      report = scene.import_dae(result!.daeBytes, Object.keys(result!.images).length > 0 ? result!.images : null) as ImportReport
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
    LogStore.log.info('app', `Imported DAE: ${report.objects_created} objects (${report.watertight} solid, ${report.leaky} leaky)`)
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
  useEffect(() => { newDocumentRef.current = newDocument }, [newDocument])
  useEffect(() => { openDocumentRef.current = openDocument }, [openDocument])
  useEffect(() => { importDocumentRef.current = importDocument }, [importDocument])
  useEffect(() => { saveDocumentRef.current = saveDocument }, [saveDocument])
  useEffect(() => { saveAsDocumentRef.current = saveAsDocument }, [saveAsDocument])
  useEffect(() => { handleUndoRef.current = handleUndo }, [handleUndo])
  useEffect(() => { handleRedoRef.current = handleRedo }, [handleRedo])
  useEffect(() => { handleZoomExtentsRef.current = handleZoomExtents }, [handleZoomExtents])
  useEffect(() => { openPathRef.current = openPath }, [openPath])

  // ---------------------------------------------------------------- native menu-action listener (Tauri only)
  // Registered once; reads latest callbacks via refs so no stale-closure risk.
  useEffect(() => {
    if (!isTauri) return
    let unlisten: (() => void) | undefined
    let cancelled = false
    import('@tauri-apps/api/event').then(({ listen }) => {
      return listen<string>('menu-action', (event) => {
        switch (event.payload) {
          case 'new':      newDocumentRef.current(); break
          case 'open':     openDocumentRef.current(); break
          case 'import':   importDocumentRef.current(); break
          case 'save':     saveDocumentRef.current(); break
          case 'save-as':  saveAsDocumentRef.current(); break
          case 'undo':     handleUndoRef.current(); break
          case 'redo':     handleRedoRef.current(); break
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
          case 'tool-orbit':     setActiveTool('Orbit'); break
          case 'tool-pan':       setActiveTool('Pan'); break
          case 'tool-zoom':      setActiveTool('Zoom'); break
          // Window pane toggles — must use functional updaters (StrictMode safe)
          case 'toggle-model-info': setShowModelInfo((v) => !v); break
          case 'toggle-materials':  setShowMaterials((v) => !v); break
          case 'zoom-extents':      handleZoomExtentsRef.current(); break
        }
      })
    }).then((fn) => { if (cancelled) fn(); else unlisten = fn }).catch(() => { /* ignore if not in Tauri */ })
    return () => { cancelled = true; unlisten?.() }
  }, []) // stable — all callbacks accessed via refs

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
    // Under Tauri, the native menu bar owns all keyboard shortcuts.
    // The JS keydown handler must not double-fire them.
    if (isTauri) return

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
      if (ev.key === 'o') {
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
      // Window pane toggles
      if (ev.key === 'i' && ev.shiftKey) {
        ev.preventDefault()
        setShowModelInfo((v) => !v)
        return
      }
      if (ev.key === 'c' && ev.shiftKey) {
        ev.preventDefault()
        setShowMaterials((v) => !v)
        return
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [saveDocument, saveAsDocument, openDocument, newDocument])

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
        height: '100vh',
        boxSizing: 'border-box',
        overflow: 'hidden',
        background: '#1a1a1a',
      }}
    >
      {/* App bar / menu bar.
          Under Tauri, the native OS menu bar owns File/Edit; the in-app bar
          shows only the document title + kernel version. */}
      <MenuBar
        title={deriveTitle(docSession)}
        kernelVersion={state.kernelVersion}
        nativeMenuBar={isTauri}
        onNew={newDocument}
        onOpen={openDocument}
        onSave={saveDocument}
        onSaveAs={saveAsDocument}
        onImport={importDocument}
        onUndo={handleUndo}
        onRedo={handleRedo}
        canUndo={canUndo}
        canRedo={canRedo}
        activeTool={activeTool}
        onSelectTool={(name) => setActiveTool(name as ToolName)}
        showModelInfo={showModelInfo}
        showMaterials={showMaterials}
        onToggleModelInfo={() => setShowModelInfo((v) => !v)}
        onToggleMaterials={() => setShowMaterials((v) => !v)}
        onZoomExtents={handleZoomExtents}
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
            style={{
              padding: '3px 10px',
              fontSize: '12px',
              cursor: 'pointer',
              background: activeTool === t ? '#5588cc' : '#444',
              color: '#eee',
              border: activeTool === t ? '1px solid #7aaaee' : '1px solid #555',
              borderRadius: '3px',
              fontFamily: 'monospace',
            }}
          >
            [{TOOL_KEYS[t]}] {t}
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

      {/* Status bar */}
      <div
        style={{
          padding: '4px 8px',
          background: '#222',
          color: '#eee',
          fontFamily: 'monospace',
          fontSize: '12px',
          borderBottom: '1px solid #3a3a3a',
          display: 'flex',
          gap: '1.5rem',
          flexShrink: 0,
        }}
      >
        <span>Tool: <strong>{toolName}</strong></span>
        <span>Snap: {snapKind ?? '—'}</span>
        <span>Length: {measurement !== '' ? measurement : '—'}</span>
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

      {/* Viewport + document tree — fills remaining space above the log panel */}
      <div style={{ flex: 1, minHeight: 0, display: 'flex', gap: '8px', padding: '8px 8px 0 8px' }}>
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
            onEnterContext={handleEnterContext}
            onExitContext={handleExitContext}
            onDocumentChanged={handleDocumentChanged}
            apiRef={viewportApi}
            onMeasurement={handleMeasurement}
            currentMaterialId={currentMaterialId}
          />

          {/* Toast stack — positioned inside the viewport container */}
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

        {showModelInfo && (
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
            onClose={() => setShowModelInfo(false)}
          />
        )}
        {showMaterials && (
          <MaterialPalette
            scene={state.scene}
            docRev={docRev}
            currentMaterialId={currentMaterialId}
            onSelectMaterial={setCurrentMaterialId}
            onDocumentChanged={handleDocumentChanged}
            selectedIds={selectedIds}
            onClose={() => setShowMaterials(false)}
          />
        )}
      </div>

      {/* Log panel — docked at bottom, never covers the viewport */}
      <LogPanel panelHeight={160} />

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
    </main>
  )
}
