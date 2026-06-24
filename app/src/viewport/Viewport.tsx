/**
 * Viewport — M2 interactive 3D viewport.
 *
 * Wires together:
 *   - THREE.js WebGL2 renderer + PerspectiveCamera
 *   - OrbitControls with SketchUp-style bindings:
 *       middle-drag  → orbit
 *       right-drag   → pan
 *       wheel        → dolly toward cursor
 *   - CueLayer (snap-point overlay, rebuilt each pointer move)
 *   - SnapService (wraps Scene.snap with ground-plane fallback)
 *   - SceneRenderer (live object + sketch geometry, refreshed after commits)
 *   - ToolController routing pointer events to the active Tool
 *   - RectangleTool + PushPullTool + SelectTool + Move/Rotate/Scale
 *   - Undo/redo keyboard shortcuts (Cmd/Ctrl+Z, Shift+Cmd/Ctrl+Z)
 *   - Ground grid
 *   - : context path navigation, group-aware picking
 */

import { useEffect, useRef } from 'react'
import * as THREE from 'three'
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js'
import type { Scene as WasmScene } from '../wasm/loader'
import { CueLayer } from './CueLayer'
import { SnapService } from './snapService'
import { SceneRenderer } from './SceneRenderer'
import { exportSceneToGlb } from '../io/exporters/gltfExport'
import { ToolController } from '../tools/ToolController'
import { RectangleTool } from '../tools/RectangleTool'
import { PushPullTool } from '../tools/PushPullTool'
import { PaintTool, MATERIAL_SENTINEL } from '../tools/PaintTool'
import { MoveTool } from '../tools/MoveTool'
import { RotateTool } from '../tools/RotateTool'
import { ScaleTool } from '../tools/ScaleTool'
import { TapeMeasureTool } from '../tools/TapeMeasureTool'
import { ProtractorTool } from '../tools/ProtractorTool'
import { SliceTool } from '../tools/SliceTool'
import { parseKernelErrorCode, kernelErrorMessage } from './geoHelpers'
import type { Ray } from './math'
import type { NodeRef } from '../panels/treeModel'
import { cursorFor } from '../tools/toolIcons'

/**
 * Centered message overlay shown over the viewport when the WebGL2 context is
 * unavailable or has been lost. WebKitGTK (the Linux/Tauri webview) drops the GL
 * context more readily than Chromium does — on suspend/resume or a GPU/driver
 * reset — and a dropped context otherwise leaves a frozen grey canvas with no
 * explanation. The node is absolutely positioned, so its container must be
 * `position: relative`.
 */
function buildViewportOverlay(title: string, detail: string): HTMLDivElement {
  const overlay = document.createElement('div')
  overlay.className = 'viewport-overlay'
  overlay.style.cssText = [
    'position:absolute', 'inset:0', 'display:flex', 'flex-direction:column',
    'align-items:center', 'justify-content:center', 'gap:8px', 'padding:24px',
    'text-align:center', 'background:#d0d0d0', 'color:#333',
    'font-family:system-ui,sans-serif', 'z-index:10', 'pointer-events:none',
  ].join(';')
  const h = document.createElement('div')
  h.textContent = title
  h.style.cssText = 'font-size:16px;font-weight:600'
  const p = document.createElement('div')
  p.textContent = detail
  p.style.cssText = 'font-size:13px;max-width:36em;line-height:1.4;opacity:0.8'
  overlay.appendChild(h)
  overlay.appendChild(p)
  return overlay
}

interface Props {
  /** WASM Scene — owns inference, sketches, objects */
  wasmScene: WasmScene
  /** Called when tool name or snap kind changes (for status bar) */
  onStatusChange?: (toolName: string, snapKind: string | null) => void
  /** Called after any scene mutation with new watertight state per object */
  onSceneChange?: (watertightMap: Map<bigint, boolean>) => void
  /** Called when an error toast should be shown */
  onToast?: (message: string, code?: string) => void
  /** Active tool name from parent (undefined = parent doesn't control) */
  activeTool?: string
  /** Active context path. Empty = top level. */
  activeContext?: NodeRef[]
  /** Selected nodes (ordered; index 0 = primary). */
  selectedIds?: NodeRef[]
  /** Lit set for isolation rendering — null = top level. */
  activeLitSet?: Set<bigint> | null
  /** Lift an in-viewport selection up to the parent. `additive` = shift-click. */
  onSelect?: (node: NodeRef | null, additive: boolean) => void
  /** Lift a construction-guide pick to the parent; `null` clears. */
  onSelectGuide?: (id: bigint | null) => void
  /** The currently selected guide, reflected into the renderer highlight. */
  selectedGuide?: bigint | null
  /** Request entering a node's editing context (double-click). */
  onEnterContext?: (node: NodeRef) => void
  /** Request popping one level off the context path (Esc). */
  onExitContext?: () => void
  /** Fired after any document change so the parent can refresh the tree. */
  onDocumentChanged?: () => void
  /** Populated by the viewport with imperative commands the parent can call. */
  apiRef?: React.MutableRefObject<ViewportApi | null>
  /** Called with the live measurement text from tools that support VCB entry. */
  onMeasurement?: (text: string) => void
  /** Currently selected material id for the Paint tool. `u64::MAX` =
   *  default / unpaint. The viewport keeps a stable ref so a paint tool
   *  instantiated inside the effect always sees the latest value. */
  currentMaterialId?: bigint
}

/** Imperative handle the viewport exposes to the parent. */
/** One of the seven SketchUp-style standard camera framings. */
export type StandardView = 'top' | 'bottom' | 'front' | 'back' | 'left' | 'right' | 'iso'

/**
 * A hair of tilt off the ±Z pole for Top/Bottom (≈0.06°, visually imperceptible).
 * Looking *exactly* straight down with world-up +Z is gimbal-degenerate (the
 * look direction is parallel to up), which both breaks the view's roll and — the
 * real problem — would force a horizontal up, so orbiting from a top view pivots
 * around the wrong axis. Nudging the eye a touch off the pole lets every view
 * keep world-up +Z, so orbit always pivots around Z (natural in a Z-up world).
 */
const POLE_TILT = 0.001

/**
 * Eye direction (target→camera) for each standard view, in the Z-up world (X
 * red, Y green, Z blue). Every view keeps world-up +Z (see {@link POLE_TILT});
 * Iso is the SketchUp front-right-top corner.
 */
const STANDARD_VIEWS: Record<StandardView, { eye: [number, number, number] }> = {
  top:    { eye: [0, -POLE_TILT, 1] },
  bottom: { eye: [0, -POLE_TILT, -1] },
  front:  { eye: [0, -1, 0] },
  back:   { eye: [0, 1, 0] },
  right:  { eye: [1, 0, 0] },
  left:   { eye: [-1, 0, 0] },
  iso:    { eye: [1, -1, 1] },
}

export interface ViewportApi {
  /** Combine two objects (0=union, 1=subtract a−b, 2=intersect). */
  runBoolean: (op: number, a: bigint, b: bigint) => void
  /** Group the given nodes into a merge group. */
  runGroup: (nodes: NodeRef[]) => bigint | null
  /** Dissolve a group. */
  runUngroup: (groupId: bigint) => void
  /** Delete whole tree nodes (Object/Group/Instance), undoably. */
  runDelete: (nodes: NodeRef[]) => void
  /** Fold a sibling selection into a component + identity instance. Returns the instance handle. */
  runMakeComponent: (nodes: NodeRef[]) => bigint | null
  /** Place a second instance of the given instance's definition, offset slightly. */
  runPlaceInstance: (instanceId: bigint) => bigint | null
  /** Explode an instance into independent world objects. Returns their handles, or null on error. */
  runExplodeInstance: (instanceId: bigint) => bigint[] | null
  /** Detach an instance onto a private copy of its definition. Returns the new component handle. */
  runMakeUnique: (instanceId: bigint) => bigint | null
  /**
   * Call after a `scene.load()` to rebuild all viewport-side caches and
   * propagate the new watertight state / docRev to the parent.  Mirrors the
   * same path that undo/redo use (`handleSceneRefresh` + `refreshAllSketches`).
   */
  notifyLoaded: () => void
  /** Trigger scene undo (same as Cmd/Ctrl+Z keyboard shortcut). */
  runUndo: () => void
  /** Trigger scene redo (same as Shift+Cmd/Ctrl+Z keyboard shortcut). */
  runRedo: () => void
  /**
   * Frame all rendered geometry into view (View → Zoom Extents).
   * Computes the world bounding box of objectsGroup + instancesGroup,
   * re-targets the orbit camera to the box center, and dolly-zooms so
   * the box fits the vertical FOV with a 1.2× margin. No-op when the
   * scene is empty. Idempotent — safe to call multiple times.
   */
  zoomExtents: () => void
  /**
   * Reposition the orbit camera to a standard axis-aligned or isometric view
   * (Camera ▸ Standard Views), re-framing the scene each time. The current
   * (perspective) projection is retained. No model geometry changes.
   */
  setStandardView: (view: StandardView) => void
  /**
   * Update the renderer's hidden object/instance sets.  Hidden groups have
   * `.visible = false` (not raypicked by three.js tools) and are excluded from
   * the kernel pick results in the Select tool path.
   */
  setHidden: (hiddenObjectIds: bigint[], hiddenInstanceIds: bigint[]) => void
  /** Show/hide the ground grid + origin axes (View ▸ Axes). */
  setAxesVisible: (visible: boolean) => void
  /** Show/hide all construction guides (View ▸ Guides). */
  setGuidesVisible: (visible: boolean) => void
  /** Delete every construction guide (Edit ▸ Delete Guide Lines). */
  deleteAllGuides: () => void
  /** Delete a single picked construction guide. */
  runDeleteGuide: (id: bigint) => void
  /**
   * Serialize the current solid geometry (objects + instances) to a binary
   * glTF (.glb) buffer. Resolves null when the model has no solids.
   */
  exportGlb: () => Promise<Uint8Array | null>
}

/** Build a normalised world-space ray from NDC (-1..1) coords and a camera */
function makeWorldRay(
  ndcX: number,
  ndcY: number,
  camera: THREE.PerspectiveCamera,
): Ray {
  const near = new THREE.Vector3(ndcX, ndcY, -1).unproject(camera)
  const far = new THREE.Vector3(ndcX, ndcY, 1).unproject(camera)
  const dir = far.clone().sub(near).normalize()
  return {
    origin: [near.x, near.y, near.z],
    direction: [dir.x, dir.y, dir.z],
  }
}

/** Convert a DOM mouse/pointer event to NDC coordinates given the canvas element */
function pointerToNDC(
  ev: MouseEvent,
  canvas: HTMLElement,
): [number, number] {
  const rect = canvas.getBoundingClientRect()
  const x = ((ev.clientX - rect.left) / rect.width) * 2 - 1
  const y = -((ev.clientY - rect.top) / rect.height) * 2 + 1
  return [x, y]
}

/** Build a ground grid */
function buildGroundGrid(): THREE.Group {
  const group = new THREE.Group()
  group.name = 'GroundGrid'

  // Main grid: 10x10 meters, 1m divisions (person-scale: 1m squares)
  const grid = new THREE.GridHelper(10, 10, 0x888888, 0xcccccc)
  grid.rotation.x = Math.PI / 2  // GridHelper is in XZ plane; rotate to XY
  group.add(grid)

  // World origin axes: ~1 m, person-scale (red=+X, green=+Y, blue=+Z)
  const AXIS_LEN = 1.0
  const axesPts = new Float32Array([
    0, 0, 0.001,  AXIS_LEN, 0, 0.001,   // +X axis
    0, 0, 0.001,  0, AXIS_LEN, 0.001,   // +Y axis
    0, 0, 0,      0, 0, AXIS_LEN,       // +Z axis
  ])
  const axesGeo = new THREE.BufferGeometry()
  axesGeo.setAttribute('position', new THREE.BufferAttribute(axesPts, 3))
  const axesMat = new THREE.LineBasicMaterial({ vertexColors: true })
  const colors = new Float32Array([
    1, 0.1, 0.1,  1, 0.1, 0.1,           // X: red
    0.1, 0.8, 0.1,  0.1, 0.8, 0.1,       // Y: green
    0.15, 0.35, 1.0,  0.15, 0.35, 1.0,   // Z: blue
  ])
  axesGeo.setAttribute('color', new THREE.BufferAttribute(colors, 3))
  group.add(new THREE.LineSegments(axesGeo, axesMat))

  return group
}

/**
 * Walk the ancestor chain of a leaf object up to (and including) any groups,
 * and return the array [objectId, ...parentGroupIds from innermost to outermost].
 */
function buildAncestorChain(wasmScene: WasmScene, objectId: bigint): NodeRef[] {
  const chain: NodeRef[] = [{ kind: 'object', id: objectId }]
  let parentId = wasmScene.node_parent(0, objectId)
  while (parentId !== undefined) {
    chain.push({ kind: 'group', id: parentId })
    parentId = wasmScene.node_parent(1, parentId)
  }
  return chain
}

/**
 * Resolve a pick to the selectable NodeRef given the active context path.
 *
 * When the pick carries an instance id (the ray hit instanced geometry), the
 * selectable at top level is the instance node. Inside an instance's editing
 * context the pick resolves directly to the object.
 *
 * For world objects the existing group-walk logic applies.
 *
 * - Top level (ctx empty):
 *   - instance hit → select the instance
 *   - world object hit → topmost ancestor (object or group)
 * - Inside instance I (deepest ctx node is instance I):
 *   - pick must be inside I → return the picked definition-member object
 *   - pick is not inside I → null (out of scope)
 * - Inside group G: selectable = direct child of G in the ancestor chain.
 * - Inside world object O: out-of-scope picks return null.
 */
function resolvePickToSelectable(
  wasmScene: WasmScene,
  pickedObjectId: bigint,
  activeContext: NodeRef[],
  pickedInstanceId?: bigint,
): NodeRef | null {
  if (activeContext.length === 0) {
    // Top level
    if (pickedInstanceId !== undefined) {
      return { kind: 'instance', id: pickedInstanceId }
    }
    const chain = buildAncestorChain(wasmScene, pickedObjectId)
    return chain[chain.length - 1]
  }

  const deepest = activeContext[activeContext.length - 1]

  if (deepest.kind === 'instance') {
    // Inside a component's editing context: only that instance's members are in scope.
    if (pickedInstanceId === deepest.id) {
      // The pick is inside the entered instance — the selectable is the definition member.
      return { kind: 'object', id: pickedObjectId }
    }
    return null
  }

  if (deepest.kind === 'object') {
    // Inside an object's edit context: picking other objects is out of scope
    return null
  }

  // Inside group G: find the direct child of G in the world-object ancestor chain
  const chain = buildAncestorChain(wasmScene, pickedObjectId)
  for (let i = 0; i < chain.length - 1; i++) {
    if (chain[i + 1].kind === 'group' && chain[i + 1].id === deepest.id) {
      return chain[i]
    }
  }
  // G is not an ancestor of this object → click outside context
  return null
}

export default function Viewport({
  wasmScene,
  onStatusChange,
  onSceneChange,
  onToast,
  activeTool: activeToolProp,
  activeContext = [],
  selectedIds = [],
  activeLitSet = null,
  onSelect,
  onSelectGuide,
  selectedGuide = null,
  onEnterContext,
  onExitContext,
  onDocumentChanged,
  apiRef,
  onMeasurement,
  currentMaterialId = MATERIAL_SENTINEL,
}: Props) {
  const containerRef = useRef<HTMLDivElement>(null)

  // Keep stable refs to latest callbacks
  const onStatusChangeRef = useRef(onStatusChange)
  onStatusChangeRef.current = onStatusChange
  const onSceneChangeRef = useRef(onSceneChange)
  onSceneChangeRef.current = onSceneChange
  const onToastRef = useRef(onToast)
  onToastRef.current = onToast
  const onSelectRef = useRef(onSelect)
  onSelectRef.current = onSelect
  const onSelectGuideRef = useRef(onSelectGuide)
  onSelectGuideRef.current = onSelectGuide
  const onEnterContextRef = useRef(onEnterContext)
  onEnterContextRef.current = onEnterContext
  const onExitContextRef = useRef(onExitContext)
  onExitContextRef.current = onExitContext
  const onDocumentChangedRef = useRef(onDocumentChanged)
  onDocumentChangedRef.current = onDocumentChanged
  const apiRefRef = useRef(apiRef)
  apiRefRef.current = apiRef
  const onMeasurementRef = useRef(onMeasurement)
  onMeasurementRef.current = onMeasurement
  // Latest context path, readable inside the stable event closures.
  const activeContextRef = useRef<NodeRef[]>(activeContext)
  // Latest selected ids, readable inside the stable event closures.
  const selectedIdsRef = useRef<NodeRef[]>(selectedIds)
  // Latest current material id for the Paint tool.
  const currentMaterialIdRef = useRef<bigint>(currentMaterialId)
  // Whether the in-flight click is a shift-click (additive multi-select).
  const selectAdditiveRef = useRef(false)

  // Expose tool switch and undo/redo triggers to parent via ref-based mechanism
  const activeToolPropRef = useRef(activeToolProp)
  activeToolPropRef.current = activeToolProp

  const toolControllerRef = useRef<ToolController | null>(null)
  const wasmSceneRef = useRef<WasmScene>(wasmScene)
  wasmSceneRef.current = wasmScene

  const sceneRendererRef = useRef<SceneRenderer | null>(null)
  const scheduleRenderRef = useRef<() => void>(() => { /* filled in effect */ })

  // Tool instances are created inside the effect, but we need to be able to
  // switch them from outside (via activeToolProp). Use a ref for the switch fn.
  const switchToolRef = useRef<((toolName: string) => void) | null>(null)

  // Last pointer ray + viewport params cached so key-driven re-lock can
  // immediately re-resolve snap without waiting for the next pointer move.
  const lastRayRef = useRef<{ ray: import('./math').Ray; viewportH: number; fovY: number } | null>(null)

  // True when a camera-navigation tool (Orbit/Pan/Zoom) is active.
  // Used inside the mount-effect pointer handlers to suppress geometry routing.
  const cameraModeRef = useRef(false)
  // Hidden object/instance id sets — used to filter pick results so hidden
  // objects can't be accidentally selected through a click.
  const hiddenObjectIdsRef = useRef<Set<bigint>>(new Set())
  const hiddenInstanceIdsRef = useRef<Set<bigint>>(new Set())

  useEffect(() => {
    const container = containerRef.current
    if (container === null) return
    const el: HTMLDivElement = container
    // Anchor absolutely-positioned overlays (WebGL-loss / unavailable messages).
    el.style.position = 'relative'

    // ------------------------------------------------------------------ renderer
    //
    // WebGL2 context creation can fail outright on WebKitGTK (no GPU, software
    // GL disabled, or a headless session). three throws in that case; catch it
    // and show a readable message instead of an unhandled error + blank grey
    // panel, then bail out of setup.
    let renderer: THREE.WebGLRenderer
    try {
      renderer = new THREE.WebGLRenderer({ antialias: true })
    } catch (err) {
      console.error('[viewport] WebGL2 renderer creation failed:', err)
      el.appendChild(
        buildViewportOverlay(
          'WebGL2 is unavailable',
          'Hew needs a WebGL2-capable GPU to render. On Linux, check that ' +
            'hardware acceleration is enabled and your graphics drivers are installed.',
        ),
      )
      return () => {
        el.style.position = ''
        el.replaceChildren()
      }
    }
    renderer.setPixelRatio(window.devicePixelRatio)
    renderer.setClearColor(0xd0d0d0)
    renderer.setSize(el.clientWidth, el.clientHeight)
    el.appendChild(renderer.domElement)

    const threeScene = new THREE.Scene()

    const camera = new THREE.PerspectiveCamera(45, el.clientWidth / el.clientHeight, 0.01, 100)
    // Person-scale default: frames a ~2–3 m region; classic SketchUp 3/4 angle.
    // Distance ≈ 4.7 m; a 1.8 m figure reads as substantial, not dwarfed.
    camera.position.set(3.5, -3.0, 2.5)
    camera.up.set(0, 0, 1)
    camera.lookAt(0, 0, 0)

    // Lights
    const ambient = new THREE.AmbientLight(0xffffff, 0.4)
    threeScene.add(ambient)
    const dirLight = new THREE.DirectionalLight(0xffffff, 0.9)
    dirLight.position.set(3, -5, 8)
    threeScene.add(dirLight)

    // Ground grid (named group so View ▸ Axes can toggle its visibility)
    const groundGrid = buildGroundGrid()
    threeScene.add(groundGrid)

    // ------------------------------------------------------------------ scene renderer
    const sceneRenderer = new SceneRenderer(threeScene, wasmScene)
    sceneRendererRef.current = sceneRenderer
    // Initial refresh (empty scene is fine — just populates nothing)
    sceneRenderer.refresh()
    sceneRenderer.refreshGuides()

    // ------------------------------------------------------------------ cue layer
    const cueLayer = new CueLayer()
    threeScene.add(cueLayer.group)

    // Preview group shared by tools
    const previewGroup = new THREE.Group()
    previewGroup.name = 'Preview'
    threeScene.add(previewGroup)

    // ------------------------------------------------------------------ snap + tool
    const snapService = new SnapService(wasmScene)

    // Resolve a raw pick (leaf object id + optional instance id) to a
    // context-aware NodeRef, then lift the selection to the parent.
    function handleSelect(pickedObjectId: bigint | null, pickedInstanceId?: bigint): void {
      const additive = selectAdditiveRef.current && activeContextRef.current.length === 0
      const ctx = activeContextRef.current

      if (pickedObjectId === null) {
        // Miss: if we're inside a context at top-level, exit; otherwise clear.
        if (!additive && ctx.length > 0 && ctx[ctx.length - 1].kind !== 'object') {
          // Click outside while inside a group → deselect but don't exit
          // (SketchUp style: click outside within group deselects)
        }
        onSelectRef.current?.(null, additive)
        scheduleRender()
        return
      }

      // Filter out picks against hidden objects/instances so hidden geometry
      // is non-selectable. The kernel pick_face() raycasts through all scene
      // geometry regardless of three.js visibility, so we filter here.
      if (pickedInstanceId !== undefined && hiddenInstanceIdsRef.current.has(pickedInstanceId)) {
        onSelectRef.current?.(null, additive)
        scheduleRender()
        return
      }
      if (hiddenObjectIdsRef.current.has(pickedObjectId)) {
        onSelectRef.current?.(null, additive)
        scheduleRender()
        return
      }

      const resolved = resolvePickToSelectable(wasmScene, pickedObjectId, ctx, pickedInstanceId)

      // If click was outside the current group/instance context, treat as deselect
      if (resolved === null && ctx.length > 0 && ctx[ctx.length - 1].kind !== 'object') {
        onSelectRef.current?.(null, false)
        scheduleRender()
        return
      }

      // At top level, clicking something while not in context always just selects
      onSelectRef.current?.(resolved, additive)
      scheduleRender()
    }

    const toolController = new ToolController(wasmScene, handleSelect)
    toolControllerRef.current = toolController

    // ------------------------------------------------------------------ commit callbacks
    function handleSceneRefresh(): void {
      const wtMap = sceneRenderer.refresh()
      onSceneChangeRef.current?.(wtMap)
      onDocumentChangedRef.current?.()
      scheduleRender()
    }

    function handleToast(message: string, code?: string): void {
      onToastRef.current?.(message, code)
    }

    // Imperative command surface for the parent.
    function runBoolean(op: number, a: bigint, b: bigint): void {
      let result: bigint
      try {
        result = wasmScene.boolean(op, a, b)
      } catch (err) {
        const code = parseKernelErrorCode(err)
        const rawMsg = err instanceof Error ? err.message : String(err)
        handleToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
        return
      }
      handleSceneRefresh()
      sceneRenderer.refreshAllSketches()
      sceneRenderer.refreshGuides()
      onSelectRef.current?.({ kind: 'object', id: result }, false)
      scheduleRender()
    }

    function runGroup(nodes: NodeRef[]): bigint | null {
      if (nodes.length === 0) return null
      const kinds = new Uint8Array(nodes.map((n) => n.kind === 'group' ? 1 : 0))
      const ids = new BigUint64Array(nodes.map((n) => n.id))
      try {
        const groupId = wasmScene.group_nodes(kinds, ids)
        handleSceneRefresh()
        return groupId
      } catch (err) {
        const code = parseKernelErrorCode(err)
        const rawMsg = err instanceof Error ? err.message : String(err)
        handleToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
        return null
      }
    }

    function runUngroup(groupId: bigint): void {
      try {
        wasmScene.ungroup(groupId)
        handleSceneRefresh()
      } catch (err) {
        const code = parseKernelErrorCode(err)
        const rawMsg = err instanceof Error ? err.message : String(err)
        handleToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
      }
    }

    function runDelete(nodes: NodeRef[]): void {
      if (nodes.length === 0) return
      // kind: 0=object, 1=group, 2=instance
      for (const n of nodes) {
        const kind = n.kind === 'group' ? 1 : n.kind === 'instance' ? 2 : 0
        try {
          wasmScene.delete_node(kind, n.id)
        } catch (err) {
          const code = parseKernelErrorCode(err)
          const rawMsg = err instanceof Error ? err.message : String(err)
          handleToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
        }
      }
      handleSceneRefresh()
      sceneRenderer.refreshAllSketches()
      sceneRenderer.refreshGuides()
    }

    function runMakeComponent(nodes: NodeRef[]): bigint | null {
      if (nodes.length === 0) return null
      // kind: 0=object, 1=group, 2=instance
      const kinds = new Uint8Array(nodes.map((n) =>
        n.kind === 'group' ? 1 : n.kind === 'instance' ? 2 : 0,
      ))
      const ids = new BigUint64Array(nodes.map((n) => n.id))
      try {
        const instanceId = wasmScene.make_component(kinds, ids)
        handleSceneRefresh()
        return instanceId
      } catch (err) {
        const code = parseKernelErrorCode(err)
        const rawMsg = err instanceof Error ? err.message : String(err)
        handleToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
        return null
      }
    }

    function runPlaceInstance(instanceId: bigint): bigint | null {
      const componentId = wasmScene.instance_def(instanceId)
      if (componentId === undefined) return null
      // Place at a small offset from the original — user can Move it.
      const OFFSET = 0.5
      const affine = new Float64Array([
        1, 0, 0, OFFSET,
        0, 1, 0, OFFSET,
        0, 0, 1, 0,
      ])
      try {
        const newInstanceId = wasmScene.place_instance(componentId, affine)
        handleSceneRefresh()
        return newInstanceId
      } catch (err) {
        const code = parseKernelErrorCode(err)
        const rawMsg = err instanceof Error ? err.message : String(err)
        handleToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
        return null
      }
    }

    function runExplodeInstance(instanceId: bigint): bigint[] | null {
      try {
        const objectIds = wasmScene.explode_instance(instanceId)
        handleSceneRefresh()
        return Array.from(objectIds)
      } catch (err) {
        const code = parseKernelErrorCode(err)
        const rawMsg = err instanceof Error ? err.message : String(err)
        handleToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
        return null
      }
    }

    function runMakeUnique(instanceId: bigint): bigint | null {
      try {
        const _componentId = wasmScene.make_unique(instanceId)
        handleSceneRefresh()
        return instanceId
      } catch (err) {
        const code = parseKernelErrorCode(err)
        const rawMsg = err instanceof Error ? err.message : String(err)
        handleToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
        return null
      }
    }

    function notifyLoaded(): void {
      handleSceneRefresh()
      sceneRenderer.refreshAllSketches()
      sceneRenderer.refreshGuides()
    }

    function runUndo(): void {
      if (wasmSceneRef.current.can_scene_undo()) {
        try {
          wasmSceneRef.current.scene_undo()
          handleSceneRefresh()
          sceneRenderer.refreshAllSketches()
          sceneRenderer.refreshGuides()
        } catch (err) {
          console.warn('[Viewport] scene_undo failed:', err)
        }
      }
    }

    function runRedo(): void {
      if (wasmSceneRef.current.can_scene_redo()) {
        try {
          wasmSceneRef.current.scene_redo()
          handleSceneRefresh()
          sceneRenderer.refreshAllSketches()
          sceneRenderer.refreshGuides()
        } catch (err) {
          console.warn('[Viewport] scene_redo failed:', err)
        }
      }
    }

    function zoomExtents(): void {
      // Compute the world bounding box over all rendered objects and instances.
      const box = new THREE.Box3()
      box.expandByObject(sceneRenderer.objectsGroup)
      box.expandByObject(sceneRenderer.instancesGroup)
      if (box.isEmpty()) return

      const center = new THREE.Vector3()
      box.getCenter(center)
      const size = new THREE.Vector3()
      box.getSize(size)

      // Fit the bounding sphere to the vertical FOV with a 1.2× margin.
      const halfDiag = box.getBoundingSphere(new THREE.Sphere()).radius
      const fovRad = (camera.fov * Math.PI) / 180
      const distance = (halfDiag * 1.2) / Math.tan(fovRad / 2)

      // Keep the current view direction; re-target at box center.
      const dir = new THREE.Vector3()
      dir.subVectors(camera.position, controls.target).normalize()
      controls.target.copy(center)
      camera.position.copy(center).addScaledVector(dir, distance)
      camera.updateProjectionMatrix()
      controls.update()
      scheduleRender()
    }

    function setStandardView(view: StandardView): void {
      // Eye direction (target → camera) in the Z-up world. Up is always world-up
      // +Z so orbit keeps pivoting around Z (Top/Bottom dodge the gimbal via a
      // tiny tilt baked into their eye direction — see POLE_TILT).
      const spec = STANDARD_VIEWS[view]

      // Re-frame the scene each time (like zoomExtents), falling back to the
      // current target/distance when the scene is empty.
      const box = new THREE.Box3()
      box.expandByObject(sceneRenderer.objectsGroup)
      box.expandByObject(sceneRenderer.instancesGroup)

      const center = new THREE.Vector3()
      let distance: number
      if (box.isEmpty()) {
        center.copy(controls.target)
        distance = controls.getDistance()
      } else {
        box.getCenter(center)
        const radius = box.getBoundingSphere(new THREE.Sphere()).radius
        const fovRad = (camera.fov * Math.PI) / 180
        distance = (radius * 1.2) / Math.tan(fovRad / 2)
      }

      const eye = new THREE.Vector3(spec.eye[0], spec.eye[1], spec.eye[2]).normalize()
      camera.up.set(0, 0, 1)
      controls.target.copy(center)
      camera.position.copy(center).addScaledVector(eye, distance)
      camera.updateProjectionMatrix()
      controls.update()
      scheduleRender()
    }

    function setHidden(objectIds: bigint[], instanceIds: bigint[]): void {
      hiddenObjectIdsRef.current = new Set(objectIds)
      hiddenInstanceIdsRef.current = new Set(instanceIds)
      sceneRenderer.setHidden(objectIds, instanceIds)
      scheduleRender()
    }

    function setAxesVisible(visible: boolean): void {
      groundGrid.visible = visible
      // Hidden axes must not snap or flash a cue — gate inference too.
      wasmScene.set_axes_snappable(visible)
      scheduleRender()
    }

    function setGuidesVisible(visible: boolean): void {
      sceneRenderer.setGuidesVisible(visible)
      // Hidden guides must not snap or flash a cue — gate inference too.
      wasmScene.set_guides_snappable(visible)
      scheduleRender()
    }

    function deleteAllGuides(): void {
      try {
        wasmScene.delete_all_guides()
      } catch (err) {
        handleToast(err instanceof Error ? err.message : String(err))
        return
      }
      sceneRenderer.refreshGuides()
      onDocumentChangedRef.current?.()
      scheduleRender()
    }

    function runDeleteGuide(id: bigint): void {
      try {
        wasmScene.delete_guide(id)
      } catch (err) {
        handleToast(err instanceof Error ? err.message : String(err))
        return
      }
      sceneRenderer.refreshGuides()
      onDocumentChangedRef.current?.()
      scheduleRender()
    }

    async function exportGlb(): Promise<Uint8Array | null> {
      return exportSceneToGlb(sceneRenderer)
    }

    if (apiRefRef.current !== undefined) {
      apiRefRef.current.current = { runBoolean, runGroup, runUngroup, runDelete, runMakeComponent, runPlaceInstance, runExplodeInstance, runMakeUnique, notifyLoaded, runUndo, runRedo, zoomExtents, setStandardView, setHidden, setAxesVisible, setGuidesVisible, deleteAllGuides, runDeleteGuide, exportGlb }
    }

    // ------------------------------------------------------------------ tool factories
    function makeRectTool(): RectangleTool {
      const tool = new RectangleTool(
        wasmScene,
        previewGroup,
        (result) => {
          sceneRenderer.refreshAllSketches(result.sketchHandle)
          sceneRenderer.refreshGuides()
          onDocumentChangedRef.current?.()
          scheduleRender()
        },
        handleToast,
        (_objectId) => {
          handleSceneRefresh()
        },
        (text: string) => { onMeasurementRef.current?.(text) },
      )
      // Scope the tool to the current editing context, if any.
      const ctx = activeContextRef.current
      const ctxId = ctx.length > 0 && ctx[ctx.length - 1].kind === 'object'
        ? ctx[ctx.length - 1].id : null
      tool.setActiveContext(ctxId)
      return tool
    }

    function makePushPullTool(): PushPullTool {
      const tool = new PushPullTool(
        wasmScene,
        previewGroup,
        (_objectId) => {
          handleSceneRefresh()
          sceneRenderer.refreshAllSketches()
          sceneRenderer.refreshGuides()
        },
        handleToast,
        (text: string) => { onMeasurementRef.current?.(text) },
      )
      // Give it the current sketch handle if one exists
      const sketchHandle = sceneRenderer.currentSketchHandle
      if (sketchHandle !== null) {
        tool.setSketchHandle(sketchHandle)
      }
      // Scope it to the current editing context, if any.
      const ctx = activeContextRef.current
      const deepest = ctx.length > 0 ? ctx[ctx.length - 1] : null
      // For an object context (entered world object), use the object id as-is.
      const ctxId = deepest?.kind === 'object' ? deepest.id : null
      tool.setActiveContext(ctxId)
      // For an instance context (entered component), get the component def id.
      if (deepest?.kind === 'instance') {
        const componentId = wasmScene.instance_def(deepest.id)
        tool.setComponentContext(componentId ?? null)
      } else {
        tool.setComponentContext(null)
      }
      return tool
    }

    function makePaintTool(): PaintTool {
      return new PaintTool(
        wasmScene,
        (_objectId) => {
          handleSceneRefresh()
        },
        handleToast,
      )
    }

    function makeMoveTool(): MoveTool {
      const sel = selectedIdsRef.current[0] ?? null
      return new MoveTool(
        wasmScene,
        previewGroup,
        sceneRenderer.objectsGroup,
        sel,
        (node) => {
          handleSceneRefresh()
          // Select the committed node — for an Option-copy this is the fresh
          // clone, so a follow-up Alt-drag chains off the new copy.
          onSelectRef.current?.(node, false)
        },
        handleToast,
        (text: string) => { onMeasurementRef.current?.(text) },
        (id: bigint) => sceneRenderer.getInstanceGroup(id),
      )
    }

    function makeRotateTool(): RotateTool {
      const sel = selectedIdsRef.current[0] ?? null
      return new RotateTool(
        wasmScene,
        previewGroup,
        sceneRenderer.objectsGroup,
        sel,
        (_node) => {
          handleSceneRefresh()
        },
        handleToast,
        (id: bigint) => sceneRenderer.getInstanceGroup(id),
        (text: string) => { onMeasurementRef.current?.(text) },
      )
    }

    function makeScaleTool(): ScaleTool {
      const sel = selectedIdsRef.current[0] ?? null
      return new ScaleTool(
        wasmScene,
        previewGroup,
        sceneRenderer.objectsGroup,
        sel,
        (_node) => {
          handleSceneRefresh()
        },
        handleToast,
        (id: bigint) => sceneRenderer.getInstanceGroup(id),
        (text: string) => { onMeasurementRef.current?.(text) },
      )
    }

    function makeTapeMeasureTool(): TapeMeasureTool {
      return new TapeMeasureTool(
        wasmScene,
        previewGroup,
        () => {
          sceneRenderer.refreshGuides()
          onDocumentChangedRef.current?.()
          scheduleRender()
        },
        handleToast,
        (text: string) => { onMeasurementRef.current?.(text) },
      )
    }

    function makeProtractorTool(): ProtractorTool {
      return new ProtractorTool(
        wasmScene,
        previewGroup,
        () => {
          sceneRenderer.refreshGuides()
          onDocumentChangedRef.current?.()
          scheduleRender()
        },
        handleToast,
        (text: string) => { onMeasurementRef.current?.(text) },
      )
    }

    function makeSliceTool(): SliceTool {
      return new SliceTool(
        wasmScene,
        previewGroup,
        // A slice consumes the source object and yields two new ones; refresh
        // the scene and select the returned (positive) piece so highlight lands
        // on live geometry, mirroring how runBoolean reports its result.
        (objectId: bigint) => {
          handleSceneRefresh()
          sceneRenderer.refreshGuides()
          onSelectRef.current?.({ kind: 'object', id: objectId }, false)
        },
        handleToast,
        (text: string) => { onMeasurementRef.current?.(text) },
      )
    }

    // Shift-in-Orbit temporarily swaps to Pan, mirroring SketchUp.
    // Tracked here (not in switchToolRef's closure alone) so the keydown/keyup
    // handlers and the tool switch can all see/clear the same flag.
    let shiftPanActive = false

    // Switch tool by name
    switchToolRef.current = (toolName: string) => {
      // A tool switch always wins over a stale shift-pan state: if the user
      // changes tools while Shift happens to be held, the explicit
      // mouseButtons.LEFT/cursor this switch sets below must not later be
      // clobbered by onShiftKeyUp restoring the *previous* tool's Orbit state.
      shiftPanActive = false
      switch (toolName) {
        case 'Rectangle':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          toolController.setTool(makeRectTool())
          break
        case 'Push/Pull':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          toolController.setTool(makePushPullTool())
          break
        case 'Paint': {
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          const pt = makePaintTool()
          pt.setCurrentMaterial(currentMaterialIdRef.current)
          toolController.setTool(pt)
          break
        }
        case 'Move':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          toolController.setTool(makeMoveTool())
          break
        case 'Rotate':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          toolController.setTool(makeRotateTool())
          break
        case 'Scale':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          toolController.setTool(makeScaleTool())
          break
        case 'Tape Measure':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          toolController.setTool(makeTapeMeasureTool())
          break
        case 'Protractor':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          toolController.setTool(makeProtractorTool())
          break
        case 'Slice':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          toolController.setTool(makeSliceTool())
          break
        case 'Orbit':
          cameraModeRef.current = true
          controls.mouseButtons.LEFT = THREE.MOUSE.ROTATE
          toolController.resetToSelect()
          break
        case 'Pan':
          cameraModeRef.current = true
          controls.mouseButtons.LEFT = THREE.MOUSE.PAN
          toolController.resetToSelect()
          break
        case 'Zoom':
          cameraModeRef.current = true
          controls.mouseButtons.LEFT = THREE.MOUSE.DOLLY
          toolController.resetToSelect()
          break
        default:
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          toolController.resetToSelect()
      }
      // Reflect the REQUESTED tool name immediately so the status bar doesn't
      // lag until the next pointer-move. Camera tools (Orbit/Pan/Zoom) call
      // resetToSelect() internally, so toolController.activeToolName would
      // read "Select" here — use the requested toolName instead. The snap
      // kind is reset to null since switching tools invalidates any prior snap.
      onStatusChangeRef.current?.(toolName, null)
      // Tool-aware cursor: derived from the same Material Symbols
      // icon as the toolbar button, so the active tool is readable from the
      // pointer. The canvas owns its cursor — nothing else should set
      // renderer.domElement.style.cursor outside this switch.
      renderer.domElement.style.cursor = cursorFor(toolName)
      scheduleRender()
    }

    // ------------------------------------------------------------------ OrbitControls
    const controls = new OrbitControls(camera, renderer.domElement)
    // middle-drag = orbit, right-drag = pan, wheel = dolly-to-cursor
    controls.mouseButtons = {
      LEFT: null,
      MIDDLE: THREE.MOUSE.ROTATE,
      RIGHT: THREE.MOUSE.PAN,
    }
    controls.zoomToCursor = true
    controls.enableDamping = true
    controls.dampingFactor = 0.08
    controls.screenSpacePanning = true
    controls.minDistance = 0.1
    controls.maxDistance = 50
    controls.enablePan = true

    // Prevent the browser context menu on right-drag so pan isn't interrupted.
    function onContextMenu(ev: MouseEvent): void {
      ev.preventDefault()
    }
    renderer.domElement.addEventListener('contextmenu', onContextMenu)

    // Shift-in-Orbit -> temporary Pan. OrbitControls already handles
    // this natively: with mouseButtons.LEFT === MOUSE.ROTATE, holding
    // Shift/Ctrl/Meta during onMouseDown makes it pan instead of rotate (see
    // OrbitControls.js). So we must NOT touch controls.mouseButtons.LEFT here
    // — doing so would fight that built-in inversion. These handlers only
    // swap the cursor to match. Only Orbit is affected; every other tool
    // behaves exactly as before. Guarded by shiftPanActive so keydown
    // autorepeat doesn't re-apply the same state repeatedly, and so keyup
    // only restores the Orbit cursor if we're the ones who changed it.
    function onShiftKeyDown(ev: KeyboardEvent): void {
      if (ev.key !== 'Shift') return
      // Move's Shift-held axis lock. Idempotent under keydown autorepeat.
      const at = toolController.activeTool
      if ('setShiftHeld' in at) {
        (at as { setShiftHeld(held: boolean): void }).setShiftHeld(true)
      }
      if (shiftPanActive) return
      if (activeToolPropRef.current !== 'Orbit') return
      shiftPanActive = true
      renderer.domElement.style.cursor = cursorFor('Pan')
    }
    function onShiftKeyUp(ev: KeyboardEvent): void {
      if (ev.key === 'Shift') {
        const at = toolController.activeTool
        if ('setShiftHeld' in at) {
          (at as { setShiftHeld(held: boolean): void }).setShiftHeld(false)
        }
      }
      if (!shiftPanActive) return
      if (ev.key !== 'Shift' && ev.shiftKey) return
      shiftPanActive = false
      renderer.domElement.style.cursor = cursorFor('Orbit')
    }
    window.addEventListener('keydown', onShiftKeyDown)
    window.addEventListener('keyup', onShiftKeyUp)

    // ------------------------------------------------------------------ animation loop
    let rafId = 0
    let needsRender = true

    function render(): void {
      rafId = requestAnimationFrame(render)
      const changed = controls.update()
      if (changed || needsRender) {
        // Keep the snap cursor at a constant screen size regardless of zoom.
        cueLayer.updateMarkerScale(camera)
        // Keep guide-line dashes screen-constant too (see updateGuideDashScale).
        sceneRenderer.updateGuideDashScale(controls.getDistance())
        // Protractor's plane-preview disk is a virtual construct too — keep
        // it screen-constant the same way (see ProtractorTool.updateDiskScale).
        const activeToolForScale = toolController.activeTool
        if ('updateDiskScale' in activeToolForScale) {
          ;(activeToolForScale as { updateDiskScale(c: THREE.Camera): void }).updateDiskScale(camera)
        }
        renderer.render(threeScene, camera)
        needsRender = false
      }
    }
    render()

    function scheduleRender(): void {
      needsRender = true
    }
    scheduleRenderRef.current = scheduleRender

    controls.addEventListener('change', scheduleRender)

    // ------------------------------------------------------------------ context loss
    // WebKitGTK drops the GL context more readily than Chromium (suspend/resume,
    // GPU/driver reset). Without these handlers the canvas freezes grey with no
    // recovery. preventDefault on 'lost' lets the browser fire 'restored'; on
    // restore we rebuild GPU geometry — three does not re-upload buffers dropped
    // with the old context — and resume the loop.
    let contextLostOverlay: HTMLDivElement | null = null
    function onContextLost(ev: Event): void {
      ev.preventDefault()
      cancelAnimationFrame(rafId)
      console.warn('[viewport] WebGL context lost')
      if (contextLostOverlay === null) {
        contextLostOverlay = buildViewportOverlay(
          'Rendering paused',
          'The graphics context was lost (this can follow sleep/resume or a ' +
            'driver reset on Linux). Recovering automatically when it returns…',
        )
        el.appendChild(contextLostOverlay)
      }
    }
    function onContextRestored(): void {
      console.info('[viewport] WebGL context restored')
      if (contextLostOverlay !== null) {
        contextLostOverlay.remove()
        contextLostOverlay = null
      }
      sceneRenderer.refresh()
      needsRender = true
      render()
    }
    renderer.domElement.addEventListener('webglcontextlost', onContextLost)
    renderer.domElement.addEventListener('webglcontextrestored', onContextRestored)

    // ------------------------------------------------------------------ pointer move (snap + cue)
    function onPointerMove(ev: PointerEvent): void {
      // In camera-nav mode, OrbitControls owns left-drag — skip geometry routing.
      if (cameraModeRef.current) return
      if (ev.buttons !== 0 && ev.button !== -1) return

      const [ndcX, ndcY] = pointerToNDC(ev, renderer.domElement)
      const ray = makeWorldRay(ndcX, ndcY, camera)
      const viewportH = el.clientHeight
      const fovY = camera.fov

      // Cache for live re-lock after key events
      lastRayRef.current = { ray, viewportH, fovY }

      const activeTool = toolController.activeTool
      // Option/Alt held → Move commits as a copy; live-tracked so the
      // readout and chaining follow the modifier.
      if ('setCopyMode' in activeTool) {
        (activeTool as { setCopyMode(on: boolean): void }).setCopyMode(ev.altKey)
      }
      const constraint = 'snapConstraint' in activeTool
        ? (activeTool as { snapConstraint(ray?: Ray): { anchor?: [number, number, number]; lockAxis?: 0 | 1 | 2; constraintPlane?: { point: [number, number, number]; normal: [number, number, number] } } | null }).snapConstraint(ray)
        : null
      const { snap } = snapService.resolve(ray, viewportH, fovY, constraint?.anchor, constraint?.lockAxis, constraint?.constraintPlane)
      activeTool.onPointerMove(snap, ray)
      cueLayer.update(snap)
      scheduleRender()

      const snapKind = 'lastSnap' in activeTool && (activeTool as { lastSnap: unknown }).lastSnap !== null
        ? ((activeTool as { lastSnap: { kind: string } }).lastSnap).kind
        : (snap !== null ? snap.kind : null)
      onStatusChangeRef.current?.(toolController.activeToolName, snapKind)
    }

    // ------------------------------------------------------------------ pointer down
    // --- construction-guide picking ---------------------------------
    const GUIDE_PICK_PX = 8
    // Matches SceneRenderer's GUIDE_LINE_HALF_LENGTH (the rendered extent).
    const GUIDE_LINE_SAMPLE_HALF = 50

    function worldToPixels(p: THREE.Vector3): { x: number; y: number; behind: boolean } {
      const v = p.clone().project(camera)
      const w = renderer.domElement.clientWidth
      const h = renderer.domElement.clientHeight
      return { x: (v.x * 0.5 + 0.5) * w, y: (-v.y * 0.5 + 0.5) * h, behind: v.z > 1 }
    }

    function pointSegPx(
      px: number, py: number,
      ax: number, ay: number, bx: number, by: number,
    ): number {
      const dx = bx - ax, dy = by - ay
      const len2 = dx * dx + dy * dy
      if (len2 < 1e-9) return Math.hypot(px - ax, py - ay)
      let t = ((px - ax) * dx + (py - ay) * dy) / len2
      t = Math.max(0, Math.min(1, t))
      return Math.hypot(px - (ax + t * dx), py - (ay + t * dy))
    }

    /** Nearest construction guide within GUIDE_PICK_PX of the NDC click, or null.
     * Hidden guides (View ▸ Guides off) are not pickable. */
    function pickGuide(ndcX: number, ndcY: number): bigint | null {
      if (!sceneRenderer.guidesGroup.visible) return null
      const w = renderer.domElement.clientWidth
      const h = renderer.domElement.clientHeight
      const clickX = (ndcX * 0.5 + 0.5) * w
      const clickY = (-ndcY * 0.5 + 0.5) * h
      let best: bigint | null = null
      let bestDist = GUIDE_PICK_PX
      for (const id of wasmScene.guide_ids()) {
        const kind = wasmScene.guide_kind(id)
        const geom = wasmScene.guide_geometry(id)
        if (kind === undefined || geom === undefined) continue
        let d: number
        if (kind === 'line') {
          const [ox, oy, oz, dx, dy, dz] = geom
          const len = Math.hypot(dx, dy, dz)
          if (len < 1e-9) continue
          const ux = dx / len, uy = dy / len, uz = dz / len
          // Sample along the line and measure pixel distance segment-by-segment
          // between consecutive IN-FRONT samples. (Projecting the bare ±50 m
          // endpoints breaks when one is behind the camera — its projection is
          // garbage — so a near guide could never be picked.)
          const N = 64
          d = Infinity
          let prev: { x: number; y: number; behind: boolean } | null = null
          for (let i = 0; i <= N; i++) {
            const t = -GUIDE_LINE_SAMPLE_HALF + (2 * GUIDE_LINE_SAMPLE_HALF) * (i / N)
            const p = worldToPixels(new THREE.Vector3(ox + ux * t, oy + uy * t, oz + uz * t))
            if (!p.behind) {
              const dp = Math.hypot(p.x - clickX, p.y - clickY)
              if (dp < d) d = dp
              if (prev !== null && !prev.behind) {
                const ds = pointSegPx(clickX, clickY, prev.x, prev.y, p.x, p.y)
                if (ds < d) d = ds
              }
            }
            prev = p
          }
          if (!Number.isFinite(d)) continue
        } else if (kind === 'point') {
          const [x, y, z] = geom
          const p = worldToPixels(new THREE.Vector3(x, y, z))
          if (p.behind) continue
          d = Math.hypot(p.x - clickX, p.y - clickY)
        } else {
          continue
        }
        if (d < bestDist) {
          bestDist = d
          best = id
        }
      }
      return best
    }

    function onPointerDown(ev: PointerEvent): void {
      if (ev.button !== 0) return
      // In camera-nav mode, OrbitControls owns left-drag — skip geometry routing.
      if (cameraModeRef.current) return

      const [ndcX, ndcY] = pointerToNDC(ev, renderer.domElement)
      const ray = makeWorldRay(ndcX, ndcY, camera)
      const viewportH = el.clientHeight
      const fovY = camera.fov

      // Record shift state so handleSelect (driven by the tool's onSelect) can
      // treat this click as additive multi-select.
      selectAdditiveRef.current = ev.shiftKey

      // Guide pick: when selecting, a click near a construction guide
      // selects that guide (thin deliberate targets take priority over the
      // object beneath). Other tools ignore guides.
      if (toolController.activeToolName === 'Select') {
        const g = pickGuide(ndcX, ndcY)
        if (g !== null) {
          onSelectGuideRef.current?.(g)
          scheduleRender()
          return
        }
      }

      const activeTool = toolController.activeTool

      // ⌘/Ctrl-click on the Paint tool fills the whole object (base material).
      if (activeTool instanceof PaintTool) {
        activeTool.setWholeObject(ev.metaKey || ev.ctrlKey)
      }
      // Option/Alt held at the click → Move commits as a copy.
      if ('setCopyMode' in activeTool) {
        (activeTool as { setCopyMode(on: boolean): void }).setCopyMode(ev.altKey)
      }

      const constraint = 'snapConstraint' in activeTool
        ? (activeTool as { snapConstraint(ray?: Ray): { anchor?: [number, number, number]; lockAxis?: 0 | 1 | 2; constraintPlane?: { point: [number, number, number]; normal: [number, number, number] } } | null }).snapConstraint(ray)
        : null
      const { snap } = snapService.resolve(ray, viewportH, fovY, constraint?.anchor, constraint?.lockAxis, constraint?.constraintPlane)
      activeTool.onPointerDown(snap, ray)
    }

    // Double-click a node to enter its context (SketchUp-style).
    // At top level: enters the topmost ancestor group/instance/object.
    // Inside a group: enters the direct child of that group.
    // Inside an instance: enters the instance's definition for editing.
    function onDoubleClick(ev: MouseEvent): void {
      if (ev.button !== 0) return
      const [ndcX, ndcY] = pointerToNDC(ev, renderer.domElement)
      const ray = makeWorldRay(ndcX, ndcY, camera)
      const pick = wasmScene.pick_face(
        ray.origin[0], ray.origin[1], ray.origin[2],
        ray.direction[0], ray.direction[1], ray.direction[2],
      )
      if (pick !== undefined) {
        try {
          const objectId = pick.object()
          const instanceId = pick.instance()
          // Resolve to the selectable node in the current context, then enter it
          const selectable = resolvePickToSelectable(wasmScene, objectId, activeContextRef.current, instanceId)
          if (selectable !== null) {
            onEnterContextRef.current?.(selectable)
          }
        } finally {
          pick.free()
        }
      }
    }

    // ------------------------------------------------------------------ keyboard
    function onKeyDown(ev: KeyboardEvent): void {
      const isMod = ev.metaKey || ev.ctrlKey

      // Esc pops one level off the context path before tool cancel.
      if (ev.key === 'Escape' && activeContextRef.current.length > 0) {
        onExitContextRef.current?.()
        return
      }

      // If the active tool is capturing input (e.g. MoveTool VCB), route
      // non-modifier keys to it BEFORE the tool-switch shortcuts so that digit
      // keys feed the VCB rather than switching tools. Esc is intentionally
      // allowed through so cancel always works (the tool handles it too).
      if (!isMod && ev.key !== 'Escape') {
        const activeTool = toolController.activeTool
        if (
          'capturingInput' in activeTool &&
          (activeTool as { capturingInput(): boolean }).capturingInput()
        ) {
          activeTool.onKey(ev)
          ev.preventDefault()
          scheduleRender()

          // Live re-lock: re-resolve snap with the updated constraint so the
          // lock / distance display updates immediately without waiting for the
          // next pointer move.
          const cached = lastRayRef.current
          if (cached !== null) {
            const constraint = 'snapConstraint' in activeTool
              ? (activeTool as { snapConstraint(ray?: Ray): { anchor?: [number, number, number]; lockAxis?: 0 | 1 | 2; constraintPlane?: { point: [number, number, number]; normal: [number, number, number] } } | null }).snapConstraint(cached.ray)
              : null
            const { snap } = snapService.resolve(cached.ray, cached.viewportH, cached.fovY, constraint?.anchor, constraint?.lockAxis, constraint?.constraintPlane)
            activeTool.onPointerMove(snap, cached.ray)
            cueLayer.update(snap)
          }
          return
        }
      }

      // Number keys / shortcuts: switch tools (SketchUp muscle memory)
      // Space = Select, 1 = Select, 2 = Rectangle, 3 = Push/Pull, 4 = Move, 5 = Rotate, 6 = Scale
      const target = ev.target as HTMLElement
      const isTyping = target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable
      if (!isMod && !isTyping) {
        if (ev.key === ' ') { ev.preventDefault(); switchToolRef.current?.('Select'); return }
        if (ev.key === '1') { switchToolRef.current?.('Select'); return }
        if (ev.key === '2') { switchToolRef.current?.('Rectangle'); return }
        if (ev.key === '3') { switchToolRef.current?.('Push/Pull'); return }
        if (ev.key === '4') { switchToolRef.current?.('Move'); return }
        if (ev.key === '5') { switchToolRef.current?.('Rotate'); return }
        if (ev.key === '6') { switchToolRef.current?.('Scale'); return }
        if (ev.key === 'r' || ev.key === 'R') { switchToolRef.current?.('Rectangle'); return }
        if (ev.key === 'p' || ev.key === 'P') { switchToolRef.current?.('Push/Pull'); return }
        if (ev.key === 'm' || ev.key === 'M') { switchToolRef.current?.('Move'); return }
        if (ev.key === 'q' || ev.key === 'Q') { switchToolRef.current?.('Rotate'); return }
        if (ev.key === 's' || ev.key === 'S') { switchToolRef.current?.('Scale'); return }
      }

      // Undo: Cmd/Ctrl+Z — document-level, covers creations + per-object ops
      if (isMod && !ev.shiftKey && ev.key === 'z') {
        ev.preventDefault()
        if (wasmSceneRef.current.can_scene_undo()) {
          try {
            wasmSceneRef.current.scene_undo()
            handleSceneRefresh()
            // Undoing an extrude un-consumes its region; re-show its fill.
            sceneRenderer.refreshAllSketches()
            sceneRenderer.refreshGuides()
          } catch (err) {
            console.warn('[Viewport] scene_undo failed:', err)
          }
        }
        return
      }

      // Redo: Shift+Cmd/Ctrl+Z — document-level
      if (isMod && ev.shiftKey && ev.key === 'z') {
        ev.preventDefault()
        if (wasmSceneRef.current.can_scene_redo()) {
          try {
            wasmSceneRef.current.scene_redo()
            handleSceneRefresh()
            // Redoing an extrude re-consumes its region; drop its fill.
            sceneRenderer.refreshAllSketches()
            sceneRenderer.refreshGuides()
          } catch (err) {
            console.warn('[Viewport] scene_redo failed:', err)
          }
        }
        return
      }

      toolController.activeTool.onKey(ev)
    }

    renderer.domElement.addEventListener('pointermove', onPointerMove)
    renderer.domElement.addEventListener('pointerdown', onPointerDown)
    renderer.domElement.addEventListener('dblclick', onDoubleClick)
    window.addEventListener('keydown', onKeyDown)

    // ------------------------------------------------------------------ resize
    const resizeObserver = new ResizeObserver(() => {
      const w = el.clientWidth
      const h = el.clientHeight
      camera.aspect = w / h
      camera.updateProjectionMatrix()
      renderer.setSize(w, h)
      scheduleRender()
    })
    resizeObserver.observe(el)

    // ------------------------------------------------------------------ cleanup
    return () => {
      cancelAnimationFrame(rafId)
      controls.removeEventListener('change', scheduleRender)
      window.removeEventListener('keydown', onShiftKeyDown)
      window.removeEventListener('keyup', onShiftKeyUp)
      renderer.domElement.removeEventListener('webglcontextlost', onContextLost)
      renderer.domElement.removeEventListener('webglcontextrestored', onContextRestored)
      contextLostOverlay?.remove()
      renderer.domElement.removeEventListener('contextmenu', onContextMenu)
      renderer.domElement.removeEventListener('pointermove', onPointerMove)
      renderer.domElement.removeEventListener('pointerdown', onPointerDown)
      renderer.domElement.removeEventListener('dblclick', onDoubleClick)
      window.removeEventListener('keydown', onKeyDown)
      resizeObserver.disconnect()
      controls.dispose()
      cueLayer.clear()
      sceneRenderer.dispose()
      toolControllerRef.current = null
      switchToolRef.current = null
      sceneRendererRef.current = null
      if (apiRefRef.current !== undefined) {
        apiRefRef.current.current = null
      }
      renderer.dispose()
      el.removeChild(renderer.domElement)
      el.style.position = ''
    }
  // wasmScene is stable for the lifetime of the app; no re-init on each change
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wasmScene])

  // React to activeTool changes from parent without re-mounting the effect
  useEffect(() => {
    if (activeToolProp !== undefined && switchToolRef.current !== null) {
      switchToolRef.current(activeToolProp)
    }
  }, [activeToolProp])

  // Reflect the editing context path into the renderer (isolation fade) and the
  // active tool (scoped editing) when the parent changes it.
  useEffect(() => {
    activeContextRef.current = activeContext
    // Compute a lit-instance set for isolation: when inside an instance, light
    // that instance; at top level no restriction.
    const deepestCtx = activeContext.length > 0 ? activeContext[activeContext.length - 1] : null
    const litInstances: Set<bigint> | null = null  // instances always draw when not isolated
    sceneRendererRef.current?.setActiveContext(activeLitSet ?? null, litInstances)
    const tool = toolControllerRef.current?.activeTool
    if (tool !== undefined && 'setActiveContext' in tool) {
      // For tools that need the entered object id (e.g. RectangleTool, PushPullTool)
      const ctxId = deepestCtx?.kind === 'object' ? deepestCtx.id : null
      ;(tool as { setActiveContext: (id: bigint | null) => void }).setActiveContext(ctxId)
    }
    if (tool !== undefined && 'setComponentContext' in tool) {
      // For PushPullTool inside a component context
      const scene = wasmSceneRef.current
      const componentId = deepestCtx?.kind === 'instance'
        ? scene.instance_def(deepestCtx.id) ?? null
        : null
      ;(tool as { setComponentContext: (id: bigint | null) => void }).setComponentContext(componentId)
    }
    scheduleRenderRef.current()
  }, [activeContext, activeLitSet])

  // Reflect the parent's selection into the renderer highlight (e.g. a click in
  // the tree). Object refs are passed to setSelected; group refs match nothing
  // in the object groups but the leaf objects inside them are highlighted via
  // the lit set / isolation mechanism. Instance refs are highlighted via
  // setSelectedInstances.
  // Push the latest material id into a live PaintTool without re-creating it.
  useEffect(() => {
    currentMaterialIdRef.current = currentMaterialId
    const tool = toolControllerRef.current?.activeTool
    if (tool instanceof PaintTool) {
      tool.setCurrentMaterial(currentMaterialId)
    }
  }, [currentMaterialId])

  useEffect(() => {
    selectedIdsRef.current = selectedIds
    // Collect leaf object ids and instance ids for highlighting
    const leafIds: bigint[] = []
    const instanceIds: bigint[] = []
    for (const node of selectedIds) {
      if (node.kind === 'object') {
        leafIds.push(node.id)
      } else if (node.kind === 'instance') {
        instanceIds.push(node.id)
      }
      // Groups: isolation/lit set covers visual feedback
    }
    sceneRendererRef.current?.setSelected(leafIds)
    sceneRendererRef.current?.setSelectedInstances(instanceIds)
    scheduleRenderRef.current()
  }, [selectedIds])

  // Reflect the selected construction guide into the renderer highlight.
  useEffect(() => {
    sceneRendererRef.current?.setSelectedGuide(selectedGuide)
    scheduleRenderRef.current()
  }, [selectedGuide])

  return (
    <div
      ref={containerRef}
      style={{ width: '100%', height: '100%' }}
    />
  )
}
