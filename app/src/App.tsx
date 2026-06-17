import { useEffect, useState, useCallback, useRef, useMemo } from 'react'
import { loadKernel, type Scene } from './wasm/loader'
import Viewport, { type ViewportApi } from './viewport/Viewport'
import { DocumentTree } from './panels/DocumentTree'
import { MaterialPalette } from './panels/MaterialPalette'
import { nextSelection, canMakeComponent, canPlaceInstance, canExplodeInstance, canMakeUnique, type NodeRef } from './panels/treeModel'
import { LogPanel } from './log/LogPanel'
import * as LogStore from './log/LogStore'
import { install as installConsoleCapture, restore as restoreConsoleCapture } from './log/consoleCapture'
import { MATERIAL_SENTINEL } from './tools/PaintTool'

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

const TOOLS = ['Select', 'Rectangle', 'Push/Pull', 'Paint', 'Move', 'Rotate', 'Scale'] as const
type ToolName = (typeof TOOLS)[number]
const TOOL_KEYS: Record<ToolName, string> = {
  'Select': '1',
  'Rectangle': '2',
  'Push/Pull': '3',
  'Paint': '4',
  'Move': '5',
  'Rotate': '6',
  'Scale': '7',
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
  /** Imperative handle into the viewport (e.g. running a boolean). */
  const viewportApi = useRef<ViewportApi | null>(null)

  // Stable ref to the Scene for undo/redo button state queries
  const sceneRef = useRef<Scene | null>(null)

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
        setState({ kernelVersion, scene })
        LogStore.log.info('app', `Kernel loaded — version ${kernelVersion}`)
      })
      .catch((err: unknown) => {
        const msg = String(err)
        setError(msg)
        LogStore.log.error('app', `Kernel load failed: ${msg}`)
      })
  }, [])

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

  // Bump the tree's revision; trim stale context path entries.
  const handleDocumentChanged = useCallback(() => {
    setDocRev((r) => r + 1)
    setActiveContext((ctx) => {
      const scene = sceneRef.current
      if (scene === null) return ctx
      return trimContextPath(scene, ctx)
    })
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
        padding: '1rem',
        position: 'relative',
        display: 'flex',
        flexDirection: 'column',
        height: '100vh',
        boxSizing: 'border-box',
        overflow: 'hidden',
      }}
    >
      <h1 style={{ margin: '0 0 0.5rem' }}>Hew — M2</h1>
      <p style={{ margin: '0 0 0.5rem', fontSize: '12px', color: '#666' }}>
        Kernel {state.kernelVersion}
      </p>

      {/* Kernel panic sticky banner */}
      {kernelPanicked && (
        <div
          style={{
            background: '#8b0000',
            color: '#fff',
            padding: '10px 16px',
            marginBottom: '6px',
            borderRadius: '4px',
            display: 'flex',
            alignItems: 'center',
            gap: '12px',
            fontFamily: 'monospace',
            fontSize: '13px',
            zIndex: 200,
          }}
        >
          <span style={{ flex: 1 }}>
            ⚠ A kernel error occurred — the session is no longer usable. Reload the page to recover.
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

      {/* Toolbar */}
      <div
        style={{
          display: 'flex',
          gap: '6px',
          marginBottom: '4px',
          padding: '4px 6px',
          background: '#333',
          borderRadius: '4px',
          alignItems: 'center',
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
              background: activeTool === t ? '#5588cc' : '#555',
              color: '#eee',
              border: 'none',
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
          marginBottom: '4px',
          background: '#222',
          color: '#eee',
          fontFamily: 'monospace',
          fontSize: '12px',
          borderRadius: '3px',
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

      {/* Viewport + document tree — fills space above the log panel */}
      <div style={{ flex: 1, minHeight: 0, display: 'flex', gap: '8px' }}>
        <div style={{ flex: 1, minWidth: 0, position: 'relative' }}>
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
        />
        <MaterialPalette
          scene={state.scene}
          docRev={docRev}
          currentMaterialId={currentMaterialId}
          onSelectMaterial={setCurrentMaterialId}
          onDocumentChanged={handleDocumentChanged}
          selectedIds={selectedIds}
        />
      </div>

      {/* Log panel — docked at bottom, never covers the viewport */}
      <LogPanel panelHeight={160} />
    </main>
  )
}
