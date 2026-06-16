import { useEffect, useState, useCallback, useRef } from 'react'
import { loadKernel, type Scene } from './wasm/loader'
import Viewport, { type ViewportApi } from './viewport/Viewport'
import { DocumentTree } from './panels/DocumentTree'
import { nextSelection } from './panels/treeModel'
import { LogPanel } from './log/LogPanel'
import * as LogStore from './log/LogStore'
import { install as installConsoleCapture, restore as restoreConsoleCapture } from './log/consoleCapture'

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

const TOOLS = ['Select', 'Rectangle', 'Push/Pull', 'Move', 'Rotate', 'Scale'] as const
type ToolName = (typeof TOOLS)[number]
const TOOL_KEYS: Record<ToolName, string> = {
  'Select': '1',
  'Rectangle': '2',
  'Push/Pull': '3',
  'Move': '4',
  'Rotate': '5',
  'Scale': '6',
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
  /** Selected entities (ordered; index 0 = primary). */
  const [selectedIds, setSelectedIds] = useState<bigint[]>([])
  /** Entered object (editing context), or null at top level. */
  const [activeContext, setActiveContext] = useState<bigint | null>(null)
  /** Bumped on any document change so the tree re-queries entity lists. */
  const [docRev, setDocRev] = useState(0)
  /** Imperative handle into the viewport (e.g. running a boolean). */
  const viewportApi = useRef<ViewportApi | null>(null)

  // Stable ref to the Scene for undo/redo button state queries
  const sceneRef = useRef<Scene | null>(null)

  // Install console capture on mount, restore on unmount.
  // The capture is set up before the kernel loads so the panic hook output
  // (console.error) is forwarded to the LogStore immediately.
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

  const handleSelect = useCallback((id: bigint | null, additive: boolean) => {
    setSelectedIds((cur) => nextSelection(cur, id, additive))
  }, [])

  const handleEnterContext = useCallback((objectId: bigint) => {
    setActiveContext(objectId)
    setSelectedIds([objectId])
  }, [])

  const handleExitContext = useCallback(() => {
    setActiveContext(null)
  }, [])

  // Bump the tree's revision; if the entered object vanished (e.g. its creation
  // was undone), drop back to the top-level context so nothing stays isolated
  // around a hidden object.
  const handleDocumentChanged = useCallback(() => {
    setDocRev((r) => r + 1)
    setActiveContext((ctx) => {
      if (ctx === null) return null
      const scene = sceneRef.current
      if (scene === null) return ctx
      return Array.from(scene.object_ids()).includes(ctx) ? ctx : null
    })
  }, [])

  const handleToast = useCallback((message: string, code?: string) => {
    // Determine severity: error codes that are "hard" errors, vs. info/warn
    const isError = code !== undefined &&
      ['WouldVanish', 'NonManifoldResult', 'ObjectNotSolid', 'DegenerateGeometry',
        'OperandNotSolid', 'DegenerateContact', 'EmptyResult', 'SingularTransform'].includes(code)
    const level = isError ? 'error' : 'warn'
    const logMessage = code !== undefined ? `[${code}] ${message}` : message
    LogStore.log[level]('tool', logMessage)

    // Detect kernel panic signatures in the message and trip the sticky banner
    if (isPanicError(message)) {
      setKernelPanicked(true)
    }

    const id = ++toastCounter
    setToasts((prev) => [...prev, { id, message, code }])
    // Auto-dismiss after 4 seconds
    setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id))
    }, 4000)
  }, [])

  const dismissToast = useCallback((id: number) => {
    setToasts((prev) => prev.filter((t) => t.id !== id))
  }, [])

  // Listen for console.error messages that contain panic signatures —
  // the Rust panic hook writes to console.error before the wasm trap.
  // We detect them via a LogStore subscriber so we don't need to duplicate
  // the pattern-match logic.
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

  // The boolean operands: the selection narrowed to current objects (sketches
  // can't be booleaned). Re-queried each render; docRev forces a refresh.
  const objectIdSet = new Set(Array.from(state.scene.object_ids()))
  const booleanOperands = selectedIds.filter((id) => objectIdSet.has(id))
  const canBoolean = activeContext === null && booleanOperands.length === 2
  const handleBoolean = (op: number) => {
    if (booleanOperands.length === 2) {
      viewportApi.current?.runBoolean(op, booleanOperands[0], booleanOperands[1])
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
      <h1 style={{ margin: '0 0 0.5rem' }}>Hew — M1</h1>
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
          onSelect={handleSelect}
          onEnterContext={handleEnterContext}
          onExitContext={handleExitContext}
          onDocumentChanged={handleDocumentChanged}
          apiRef={viewportApi}
          onMeasurement={handleMeasurement}
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
          canBoolean={canBoolean}
          onBoolean={handleBoolean}
        />
      </div>

      {/* Log panel — docked at bottom, never covers the viewport */}
      <LogPanel panelHeight={160} />
    </main>
  )
}
