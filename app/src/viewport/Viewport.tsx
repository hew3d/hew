/**
 * Viewport — M1 interactive 3D viewport.
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
 *   - RectangleTool + PushPullTool + SelectTool
 *   - Undo/redo keyboard shortcuts (Cmd/Ctrl+Z, Shift+Cmd/Ctrl+Z)
 *   - Ground grid
 */

import { useEffect, useRef } from 'react'
import * as THREE from 'three'
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js'
import type { Scene as WasmScene } from '../wasm/loader'
import { CueLayer } from './CueLayer'
import { SnapService } from './snapService'
import { SceneRenderer } from './SceneRenderer'
import { ToolController } from '../tools/ToolController'
import { RectangleTool } from '../tools/RectangleTool'
import { PushPullTool } from '../tools/PushPullTool'
import { MoveTool } from '../tools/MoveTool'
import { RotateTool } from '../tools/RotateTool'
import { ScaleTool } from '../tools/ScaleTool'
import { parseKernelErrorCode, kernelErrorMessage } from './geoHelpers'
import type { Ray } from './math'

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
  /** Entered object (editing context), or null at top level. */
  activeContext?: bigint | null
  /** Selected entities (ordered; index 0 = primary). */
  selectedIds?: bigint[]
  /** Lift an in-viewport selection up to the parent. `additive` = shift-click. */
  onSelect?: (id: bigint | null, additive: boolean) => void
  /** Request entering an object's editing context (double-click). */
  onEnterContext?: (objectId: bigint) => void
  /** Request exiting to the parent context (Esc / click outside). */
  onExitContext?: () => void
  /** Fired after any document change so the parent can refresh the tree. */
  onDocumentChanged?: () => void
  /** Populated by the viewport with imperative commands the parent can call
   *  (e.g. running a boolean, which must also refresh the viewport). */
  apiRef?: React.MutableRefObject<ViewportApi | null>
  /** Called with the live measurement text from tools that support VCB entry
   *  (e.g. MoveTool). Empty string means no measurement to show. */
  onMeasurement?: (text: string) => void
}

/** Imperative handle the viewport exposes to the parent. */
export interface ViewportApi {
  /** Combine two objects (0=union, 1=subtract a−b, 2=intersect). */
  runBoolean: (op: number, a: bigint, b: bigint) => void
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

  // Main grid: 10x10 meters, 1m divisions
  const grid = new THREE.GridHelper(10, 10, 0x888888, 0xcccccc)
  grid.rotation.x = Math.PI / 2  // GridHelper is in XZ plane; rotate to XY
  group.add(grid)

  // Axes
  const axesPts = new Float32Array([
    0, 0, 0.001,  5, 0, 0.001,   // X axis
    0, 0, 0.001,  0, 5, 0.001,   // Y axis
  ])
  const axesGeo = new THREE.BufferGeometry()
  axesGeo.setAttribute('position', new THREE.BufferAttribute(axesPts, 3))
  const axesMat = new THREE.LineBasicMaterial({ vertexColors: true })
  const colors = new Float32Array([
    1, 0.1, 0.1,  1, 0.1, 0.1,   // X: red
    0.1, 0.8, 0.1,  0.1, 0.8, 0.1,  // Y: green
  ])
  axesGeo.setAttribute('color', new THREE.BufferAttribute(colors, 3))
  group.add(new THREE.LineSegments(axesGeo, axesMat))

  return group
}

export default function Viewport({
  wasmScene,
  onStatusChange,
  onSceneChange,
  onToast,
  activeTool: activeToolProp,
  activeContext = null,
  selectedIds = [],
  onSelect,
  onEnterContext,
  onExitContext,
  onDocumentChanged,
  apiRef,
  onMeasurement,
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
  // Latest editing context, readable inside the stable event closures.
  const activeContextRef = useRef<bigint | null>(activeContext)
  // Latest selected ids, readable inside the stable event closures.
  const selectedIdsRef = useRef<bigint[]>(selectedIds)
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

  useEffect(() => {
    const container = containerRef.current
    if (container === null) return
    const el: HTMLDivElement = container

    // ------------------------------------------------------------------ renderer
    const renderer = new THREE.WebGLRenderer({ antialias: true })
    renderer.setPixelRatio(window.devicePixelRatio)
    renderer.setClearColor(0xd0d0d0)
    renderer.setSize(el.clientWidth, el.clientHeight)
    el.appendChild(renderer.domElement)

    const threeScene = new THREE.Scene()

    const camera = new THREE.PerspectiveCamera(45, el.clientWidth / el.clientHeight, 0.01, 100)
    camera.position.set(2, -2, 2)
    camera.up.set(0, 0, 1)
    camera.lookAt(0, 0, 0)

    // Lights
    const ambient = new THREE.AmbientLight(0xffffff, 0.4)
    threeScene.add(ambient)
    const dirLight = new THREE.DirectionalLight(0xffffff, 0.9)
    dirLight.position.set(3, -5, 8)
    threeScene.add(dirLight)

    // Ground grid
    threeScene.add(buildGroundGrid())

    // ------------------------------------------------------------------ scene renderer
    const sceneRenderer = new SceneRenderer(threeScene, wasmScene)
    sceneRendererRef.current = sceneRenderer
    // Initial refresh (empty scene is fine — just populates nothing)
    sceneRenderer.refresh()

    // ------------------------------------------------------------------ cue layer
    const cueLayer = new CueLayer()
    threeScene.add(cueLayer.group)

    // Preview group shared by tools
    const previewGroup = new THREE.Group()
    previewGroup.name = 'Preview'
    threeScene.add(previewGroup)

    // ------------------------------------------------------------------ snap + tool
    const snapService = new SnapService(wasmScene)

    // onSelect wired from SelectTool → lifted to the parent (which owns the
    // selection list and feeds it back via the selectedIds prop). Additive
    // (shift) multi-select is only meaningful at top level. Inside an editing
    // context, a plain click on anything that isn't the entered object (incl.
    // empty space) exits the context (SketchUp behavior).
    function handleSelect(objectId: bigint | null): void {
      const additive = selectAdditiveRef.current && activeContextRef.current === null
      const ctx = activeContextRef.current
      if (!additive && ctx !== null && objectId !== ctx) {
        onExitContextRef.current?.()
      }
      onSelectRef.current?.(objectId, additive)
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

    // Imperative command surface for the parent (e.g. the boolean buttons in the
    // tree, which live outside the viewport but must refresh it). A boolean
    // mutates the Scene (consuming both operands), so it follows the same
    // refresh path as a tool commit, then selects the result.
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
      onSelectRef.current?.(result, false)
      scheduleRender()
    }
    if (apiRefRef.current !== undefined) {
      apiRefRef.current.current = { runBoolean }
    }

    // ------------------------------------------------------------------ tool factories
    function makeRectTool(): RectangleTool {
      return new RectangleTool(
        wasmScene,
        previewGroup,
        (result) => {
          sceneRenderer.refreshAllSketches(result.sketchHandle)
          onDocumentChangedRef.current?.()
          scheduleRender()
        },
        handleToast,
      )
    }

    function makePushPullTool(): PushPullTool {
      const tool = new PushPullTool(
        wasmScene,
        previewGroup,
        (_objectId) => {
          handleSceneRefresh()
          // Refresh sketch fills so consumed regions (dropped from sketch_regions
          // after extrude_region) no longer render a stray translucent fill.
          sceneRenderer.refreshAllSketches()
        },
        handleToast,
      )
      // Give it the current sketch handle if one exists
      const sketchHandle = sceneRenderer.currentSketchHandle
      if (sketchHandle !== null) {
        tool.setSketchHandle(sketchHandle)
      }
      // Scope it to the current editing context, if any.
      tool.setActiveContext(activeContextRef.current)
      return tool
    }

    function makeMoveTool(): MoveTool {
      return new MoveTool(
        wasmScene,
        previewGroup,
        sceneRenderer.objectsGroup,
        selectedIdsRef.current[0] ?? null,
        (_objectId) => {
          handleSceneRefresh()
        },
        handleToast,
        (text: string) => { onMeasurementRef.current?.(text) },
      )
    }

    function makeRotateTool(): RotateTool {
      return new RotateTool(
        wasmScene,
        previewGroup,
        sceneRenderer.objectsGroup,
        selectedIdsRef.current[0] ?? null,
        (_objectId) => {
          handleSceneRefresh()
        },
        handleToast,
      )
    }

    function makeScaleTool(): ScaleTool {
      return new ScaleTool(
        wasmScene,
        previewGroup,
        sceneRenderer.objectsGroup,
        selectedIdsRef.current[0] ?? null,
        (_objectId) => {
          handleSceneRefresh()
        },
        handleToast,
      )
    }

    // Switch tool by name
    switchToolRef.current = (toolName: string) => {
      switch (toolName) {
        case 'Rectangle':
          toolController.setTool(makeRectTool())
          break
        case 'Push/Pull':
          toolController.setTool(makePushPullTool())
          break
        case 'Move':
          toolController.setTool(makeMoveTool())
          break
        case 'Rotate':
          toolController.setTool(makeRotateTool())
          break
        case 'Scale':
          toolController.setTool(makeScaleTool())
          break
        default:
          toolController.resetToSelect()
      }
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

    // Shift key tracking kept for potential future use (currently harmless).
    function onShiftKeyDown(_ev: KeyboardEvent): void { /* no-op */ }
    function onShiftKeyUp(_ev: KeyboardEvent): void { /* no-op */ }
    window.addEventListener('keydown', onShiftKeyDown)
    window.addEventListener('keyup', onShiftKeyUp)

    // ------------------------------------------------------------------ animation loop
    let rafId = 0
    let needsRender = true

    function render(): void {
      rafId = requestAnimationFrame(render)
      const changed = controls.update()
      if (changed || needsRender) {
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

    // ------------------------------------------------------------------ pointer move (snap + cue)
    function onPointerMove(ev: PointerEvent): void {
      if (ev.buttons !== 0 && ev.button !== -1) return

      const [ndcX, ndcY] = pointerToNDC(ev, renderer.domElement)
      const ray = makeWorldRay(ndcX, ndcY, camera)
      const viewportH = el.clientHeight
      const fovY = camera.fov

      // Cache for live re-lock after key events
      lastRayRef.current = { ray, viewportH, fovY }

      const activeTool = toolController.activeTool
      const constraint = 'snapConstraint' in activeTool
        ? (activeTool as { snapConstraint(): { anchor: [number, number, number]; lockAxis?: 0 | 1 | 2 } | null }).snapConstraint()
        : null
      const { snap } = snapService.resolve(ray, viewportH, fovY, constraint?.anchor, constraint?.lockAxis)
      activeTool.onPointerMove(snap, ray)
      cueLayer.update(snap)
      scheduleRender()

      const snapKind = 'lastSnap' in activeTool && (activeTool as { lastSnap: unknown }).lastSnap !== null
        ? ((activeTool as { lastSnap: { kind: string } }).lastSnap).kind
        : (snap !== null ? snap.kind : null)
      onStatusChangeRef.current?.(toolController.activeToolName, snapKind)
    }

    // ------------------------------------------------------------------ pointer down
    function onPointerDown(ev: PointerEvent): void {
      if (ev.button !== 0) return

      const [ndcX, ndcY] = pointerToNDC(ev, renderer.domElement)
      const ray = makeWorldRay(ndcX, ndcY, camera)
      const viewportH = el.clientHeight
      const fovY = camera.fov

      // Record shift state so handleSelect (driven by the tool's onSelect) can
      // treat this click as additive multi-select.
      selectAdditiveRef.current = ev.shiftKey

      const activeTool = toolController.activeTool
      const constraint = 'snapConstraint' in activeTool
        ? (activeTool as { snapConstraint(): { anchor: [number, number, number]; lockAxis?: 0 | 1 | 2 } | null }).snapConstraint()
        : null
      const { snap } = snapService.resolve(ray, viewportH, fovY, constraint?.anchor, constraint?.lockAxis)
      activeTool.onPointerDown(snap, ray)
    }

    // Double-click an object to enter its editing context (SketchUp-style).
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
          onEnterContextRef.current?.(pick.object())
        } finally {
          pick.free()
        }
      }
    }

    // ------------------------------------------------------------------ keyboard
    function onKeyDown(ev: KeyboardEvent): void {
      const isMod = ev.metaKey || ev.ctrlKey

      // Esc exits the editing context first (before tool cancel).
      if (ev.key === 'Escape' && activeContextRef.current !== null) {
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
              ? (activeTool as { snapConstraint(): { anchor: [number, number, number]; lockAxis?: 0 | 1 | 2 } | null }).snapConstraint()
              : null
            const { snap } = snapService.resolve(cached.ray, cached.viewportH, cached.fovY, constraint?.anchor, constraint?.lockAxis)
            activeTool.onPointerMove(snap, cached.ray)
            cueLayer.update(snap)
          }
          return
        }
      }

      // Number keys / shortcuts: switch tools (SketchUp muscle memory)
      // 1 = Select, 2 = Rectangle, 3 = Push/Pull, 4 = Move, 5 = Rotate, 6 = Scale
      if (!isMod) {
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

  // Reflect the editing context into the renderer (isolation fade) and the
  // active tool (scoped editing) when the parent changes it.
  useEffect(() => {
    activeContextRef.current = activeContext
    sceneRendererRef.current?.setActiveContext(activeContext)
    const tool = toolControllerRef.current?.activeTool
    if (tool !== undefined && 'setActiveContext' in tool) {
      (tool as { setActiveContext: (id: bigint | null) => void }).setActiveContext(activeContext)
    }
    scheduleRenderRef.current()
  }, [activeContext])

  // Reflect the parent's selection into the renderer highlight (e.g. a click in
  // the tree). Sketch ids match no object group and are simply ignored, which
  // is fine pre-sketch-selection-visuals.
  useEffect(() => {
    selectedIdsRef.current = selectedIds
    sceneRendererRef.current?.setSelected(selectedIds)
    scheduleRenderRef.current()
  }, [selectedIds])

  return (
    <div
      ref={containerRef}
      style={{ width: '100%', height: '480px' }}
    />
  )
}
