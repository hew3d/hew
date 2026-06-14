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

/** Convert a DOM PointerEvent to NDC coordinates given the canvas element */
function pointerToNDC(
  ev: PointerEvent,
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
}: Props) {
  const containerRef = useRef<HTMLDivElement>(null)

  // Keep stable refs to latest callbacks
  const onStatusChangeRef = useRef(onStatusChange)
  onStatusChangeRef.current = onStatusChange
  const onSceneChangeRef = useRef(onSceneChange)
  onSceneChangeRef.current = onSceneChange
  const onToastRef = useRef(onToast)
  onToastRef.current = onToast

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

    // onSelect wired from SelectTool → SceneRenderer highlight
    function handleSelect(objectId: bigint | null): void {
      sceneRenderer.setSelected(objectId)
      scheduleRender()
    }

    const toolController = new ToolController(wasmScene, handleSelect)
    toolControllerRef.current = toolController

    // ------------------------------------------------------------------ commit callbacks
    function handleSceneRefresh(): void {
      const wtMap = sceneRenderer.refresh()
      onSceneChangeRef.current?.(wtMap)
      scheduleRender()
    }

    function handleToast(message: string, code?: string): void {
      onToastRef.current?.(message, code)
    }

    // ------------------------------------------------------------------ tool factories
    function makeRectTool(): RectangleTool {
      return new RectangleTool(
        wasmScene,
        previewGroup,
        (result) => {
          sceneRenderer.refreshSketch(result.sketchHandle)
          if (result.regionsCreated.length > 0) {
            sceneRenderer.showSketchRegion(result.sketchHandle)
          }
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
          const sketchHandle = sceneRenderer.currentSketchHandle
          if (sketchHandle !== null) {
            sceneRenderer.refreshSketch(sketchHandle)
            sceneRenderer.showSketchRegion(sketchHandle)
          }
        },
        handleToast,
      )
      // Give it the current sketch handle if one exists
      const sketchHandle = sceneRenderer.currentSketchHandle
      if (sketchHandle !== null) {
        tool.setSketchHandle(sketchHandle)
      }
      return tool
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

      const { snap } = snapService.resolve(ray, viewportH, fovY)
      toolController.activeTool.onPointerMove(snap, ray)
      cueLayer.update(snap)
      scheduleRender()

      const sc = toolController.activeTool
      const snapKind = 'lastSnap' in sc && sc.lastSnap !== null
        ? (sc.lastSnap as { kind: string }).kind
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

      const { snap } = snapService.resolve(ray, viewportH, fovY)
      toolController.activeTool.onPointerDown(snap, ray)
    }

    // ------------------------------------------------------------------ keyboard
    function onKeyDown(ev: KeyboardEvent): void {
      const isMod = ev.metaKey || ev.ctrlKey

      // Number keys / shortcuts: switch tools (SketchUp muscle memory)
      // 1 = Select, 2 = Rectangle, 3 = Push/Pull
      if (!isMod) {
        if (ev.key === '1') { switchToolRef.current?.('Select'); return }
        if (ev.key === '2') { switchToolRef.current?.('Rectangle'); return }
        if (ev.key === '3') { switchToolRef.current?.('Push/Pull'); return }
        if (ev.key === 'r' || ev.key === 'R') { switchToolRef.current?.('Rectangle'); return }
        if (ev.key === 'p' || ev.key === 'P') { switchToolRef.current?.('Push/Pull'); return }
      }

      // Undo: Cmd/Ctrl+Z — document-level, covers creations + per-object ops
      if (isMod && !ev.shiftKey && ev.key === 'z') {
        ev.preventDefault()
        if (wasmSceneRef.current.can_scene_undo()) {
          try {
            wasmSceneRef.current.scene_undo()
            handleSceneRefresh()
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
      window.removeEventListener('keydown', onKeyDown)
      resizeObserver.disconnect()
      controls.dispose()
      cueLayer.clear()
      sceneRenderer.dispose()
      toolControllerRef.current = null
      switchToolRef.current = null
      sceneRendererRef.current = null
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

  return (
    <div
      ref={containerRef}
      style={{ width: '100%', height: '480px' }}
    />
  )
}
