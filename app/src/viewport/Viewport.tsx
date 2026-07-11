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
import { Line2 } from 'three/examples/jsm/lines/Line2.js'
import { LineMaterial } from 'three/examples/jsm/lines/LineMaterial.js'
import { LineGeometry } from 'three/examples/jsm/lines/LineGeometry.js'
import { updateFatLineResolutions } from './fatLine'
import type { Scene as WasmScene } from '../wasm/loader'
import { CueLayer } from './CueLayer'
import { SnapService } from './snapService'
import { SceneRenderer, type RefreshTouched } from './SceneRenderer'
import * as inputRecorder from '../recording/inputRecorder'
import { exportSceneToGlb } from '../io/exporters/gltfExport'
import { exportSceneToStl, type StlBuildResult } from '../io/exporters/stlExport'
import { ToolController } from '../tools/ToolController'
import { RectangleTool } from '../tools/RectangleTool'
import { CircleTool } from '../tools/CircleTool'
import { ArcTool } from '../tools/ArcTool'
import { LineTool } from '../tools/LineTool'
import { PushPullTool } from '../tools/PushPullTool'
import { PaintTool, MATERIAL_SENTINEL } from '../tools/PaintTool'
import { MoveTool } from '../tools/MoveTool'
import { RotateTool } from '../tools/RotateTool'
import { ScaleTool } from '../tools/ScaleTool'
import { TapeMeasureTool } from '../tools/TapeMeasureTool'
import { ProtractorTool } from '../tools/ProtractorTool'
import { SliceTool } from '../tools/SliceTool'
import { EditVertexTool } from '../tools/EditVertexTool'
import { makeSketchHandleCache } from '../tools/sketchGesture'
import { parseKernelErrorCode, kernelErrorMessage } from './geoHelpers'
import type { Ray } from './math'
import type { Snap } from '../tools/types'
import { collectLeafIds, nodeRefFromJs, type NodeRef } from '../panels/treeModel'
import { MarqueeProjector, normalizedRect, type MarqueeMode, type MarqueeRect } from './marquee'
import { cursorFor } from '../tools/toolIcons'
import { getResolvedTheme, subscribe as subscribeTheme } from '../settings/theme'
import { InfiniteGrid } from './InfiniteGrid'
import { SketchHoverGate } from './sketchHoverGate'
import { isRenderStatsActive, recordRender } from './renderStats'

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
    'text-align:center', 'background:var(--surface-panel, #d0d0d0)', 'color:var(--text-primary, #333)',
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

/**
 * Live inference-cursor info for the inference tooltip chip
 * (`07_inference_feedback.md`) — a DOM overlay App.tsx positions at
 * (screenX, screenY), so unlike `onStatusChange`'s plain status-bar text
 * this needs screen-space coordinates. `direction` is passed through
 * unprocessed (not pre-resolved to an axis/color) so the tooltip component
 * can call `axisColorForDirection` itself, keeping this callback a thin,
 * additive forward of data already available at the existing pointer-move
 * call site — no new geometry logic added to Viewport.tsx.
 */
export interface InferenceInfo {
  kind: string
  screenX: number
  screenY: number
  direction?: [number, number, number]
}

interface Props {
  /** WASM Scene — owns inference, sketches, objects */
  wasmScene: WasmScene
  /** Called when tool name or snap kind changes (for status bar) */
  onStatusChange?: (toolName: string, snapKind: string | null) => void
  /** Called on every pointer move with the live inference-cursor info,
   * or null when there's no active snap. Screen-space coordinates only —
   * `App.tsx` positions the tooltip chip; this component does no DOM overlay
   * work itself. */
  onInferenceChange?: (info: InferenceInfo | null) => void
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
  /** Lift a multi-node selection (marquee, Select All) up to the parent.
   * `additive` = shift held: merge into the current selection. */
  onSelectMany?: (nodes: NodeRef[], additive: boolean) => void
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
  /** Fired when a pointer-drag camera navigation (orbit/pan/dolly-drag via
   * OrbitControls) starts (true) / ends (false) —; App.tsx fades the
   * contextual dock out while active. Wheel dollies are deliberately NOT
   * reported: OrbitControls fires an immediate 'start'+'end' pair per wheel
   * tick, which would blink the dock on every scroll. */
  onCameraDragChange?: (active: boolean) => void
  /** Fired on a hover TRANSITION (true when the cursor is aimed at a live
   * sketch's extrudable region, false when it leaves) —, "sketches
   * are first-class interactable" contextual-dock half. Only polled while
   * selection is empty and no camera-drag/tool-drag/button-down is in
   * flight; throttled via `SketchHoverGate` so the underlying wasm ray-cast
   * runs at most once per ~100ms regardless of mousemove frequency. App.tsx
   * feeds this into `ContextualDock` so an idle cursor over a sketch
   * previews the Push/Pull verb instead of the empty-selection draw row. */
  onHoverSketchRegionChange?: (hovering: boolean) => void
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
  /**
   * Re-tessellate + re-render after a mutation made *outside* a tool (the
   * `__hew_test` harness commits kernel ops directly). Mirrors what a tool commit
   * runs internally — `handleSceneRefresh` (re-tessellate + propagate watertight
   * state + reconcile + schedule a frame) plus `refreshAllSketches`. Without it
   * harness geometry exists in the kernel but never reaches the GPU.
   */
  refreshScene: () => void
  /**
   * True while the active tool is capturing raw keyboard input (mid-VCB entry),
   * so the global Delete/Backspace handler must not steal the key (Backspace
   * edits the typed buffer). False for non-capturing tools (e.g. Select).
   */
  isCapturingInput: () => boolean
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
   * Pin the camera to an explicit pose ( `__hew_test.setCamera`): position,
   * orbit target, up, vertical FOV (deg). Deterministic framing for E2E / pixel
   * tests; mirrors the recorded `camera` input shape and  `PINNED_CAMERA`.
   */
  setCamera: (
    position: [number, number, number],
    target: [number, number, number],
    up: [number, number, number],
    fovDeg: number,
  ) => void
  /**
   * Update the renderer's hidden object/instance sets.  Hidden groups have
   * `.visible = false` (not raypicked by three.js tools) and are excluded from
   * the kernel pick results in the Select tool path.
   */
  setHidden: (hiddenObjectIds: bigint[], hiddenInstanceIds: bigint[]) => void
  /** Select every visible top-level node + free sketch (Edit ▸ Select All);
   * inside a group's editing context, its direct members. */
  selectAll: () => void
  /** Show/hide the origin axes (View ▸ Axes). */
  setAxesVisible: (visible: boolean) => void
  /** Show/hide the ground grid (View ▸ Grid). */
  setGridVisible: (visible: boolean) => void
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
  /**
   * Serialize the current solid geometry (objects + instances) to a binary
   * STL buffer — millimeter scale, Z-up. Resolves null when the
   * model has no solids.
   */
  exportStl: () => Promise<StlBuildResult | null>
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

/** Ground grid colors, dark/light.
 *
 * `ground` is the plane's own base tint (distinct from — and, per testing,
 * darker than — the sky/clear-color above, so there's a visible horizon
 * even between grid lines); `major`/`minor` are the line colors. Dark-mode
 * lines were brightened significantly per testing ("almost invisible in
 * dark mode... needs a lighter hue") — light mode's original values tested
 * well and are unchanged. */
const GROUND_GRID_COLORS: Record<'light' | 'dark', { ground: number; major: number; minor: number }> = {
  dark: { ground: 0x0c0e11, major: 0x8b95a3, minor: 0x565f6b },
  light: { ground: 0xd7dee6, major: 0xb0b8c2, minor: 0xd8dee5 },
}

const ORIGIN_AXIS_COLORS: Record<'light' | 'dark', { x: [number, number, number]; y: [number, number, number]; z: [number, number, number] }> = {
  // Normalized 0-1 RGB (vertex colors), not hex — matches DARK_AXIS_COLORS/
  // LIGHT_AXIS_COLORS in axisColors.ts (#e85a60/#5fce80/#5f96eb dark,
  // #d6454b/#28a055/#2d78e1 light) converted to float triples.
  dark: { x: [0.910, 0.353, 0.376], y: [0.373, 0.808, 0.502], z: [0.373, 0.588, 0.922] },
  light: { x: [0.839, 0.271, 0.294], y: [0.157, 0.627, 0.333], z: [0.176, 0.471, 0.882] },
}

/** World origin axis lines, colored for `theme`, long enough (150 — beyond
 * the camera's far-clip of 100) to always run off the edge of the visible
 * world in every direction, reading as "infinite" without needing a shader
 *.
 *
 * Rendered as fat lines (`Line2`/`LineMaterial`) rather than `LineBasicMaterial`
 * because WebGL ignores `linewidth` on plain lines — every line is 1px, which
 * read as dim/thin regardless of the (already vibrant) axis colors (Refinement
 * pass, issue C). Each axis is two segments: a SOLID positive half and a
 * DASHED negative half (SketchUp convention — the dashing distinguishes the
 * +/- direction of each axis). `LineMaterial.resolution` must track the canvas
 * size for correct pixel width; `updateAxisResolution` handles that on build,
 * resize, and theme rebuild. Rebuilt (not mutated) on every theme change. */
const AXIS_WIDTH_POS = 2.6 // px — solid positive halves
const AXIS_WIDTH_NEG = 1.8 // px — dashed negative halves

function buildAxisLine(
  from: [number, number, number],
  to: [number, number, number],
  color: [number, number, number],
  dashed: boolean,
): Line2 {
  const geo = new LineGeometry()
  geo.setPositions([...from, ...to])
  const mat = new LineMaterial({
    color: new THREE.Color(color[0], color[1], color[2]).getHex(),
    linewidth: dashed ? AXIS_WIDTH_NEG : AXIS_WIDTH_POS,
    dashed,
    dashSize: 0.28,
    gapSize: 0.22,
    transparent: dashed,
    opacity: dashed ? 0.75 : 1,
    depthTest: true,
  })
  const line = new Line2(geo, mat)
  if (dashed) line.computeLineDistances()
  line.renderOrder = 1 // draw over the grid plane
  return line
}

function buildOriginAxes(theme: 'light' | 'dark'): THREE.Group {
  const group = new THREE.Group()
  group.name = 'OriginAxes'

  const L = 150
  const E = 0.002 // tiny lift off Z=0 so the ground-plane axes don't z-fight the grid
  const { x: xc, y: yc, z: zc } = ORIGIN_AXIS_COLORS[theme]

  // X (red): solid +X, dashed -X
  group.add(buildAxisLine([0, 0, E], [L, 0, E], xc, false))
  group.add(buildAxisLine([0, 0, E], [-L, 0, E], xc, true))
  // Y (green): solid +Y, dashed -Y
  group.add(buildAxisLine([0, 0, E], [0, L, E], yc, false))
  group.add(buildAxisLine([0, 0, E], [0, -L, E], yc, true))
  // Z (blue): solid +Z, dashed -Z (below ground)
  group.add(buildAxisLine([0, 0, 0], [0, 0, L], zc, false))
  group.add(buildAxisLine([0, 0, 0], [0, 0, -L], zc, true))

  return group
}

/** Point every axis `LineMaterial` at the current canvas pixel size — required
 * for `Line2` to compute correct screen-space widths. */
function updateAxisResolution(group: THREE.Group, width: number, height: number): void {
  group.traverse((child) => {
    if (child instanceof Line2) {
      ;(child.material as LineMaterial).resolution.set(width, height)
    }
  })
}

/**
 * Walk the ancestor chain of a picked node up to (and including) any groups,
 * and return the array [pickedNode, ...parentGroupIds from innermost to
 * outermost]. The chain is rooted at the picked node itself: when the pick
 * carries an instance id (the ray hit instanced geometry), the chain starts
 * at that instance (kind 2) and walks group parents from there; otherwise it
 * starts at the leaf object as before. Rooting at the instance (rather than
 * the definition-member object, which has no doc-tree parent of its own) is
 * what lets a nested instance resolve up to its outermost wrapper group.
 */
export function buildAncestorChain(wasmScene: WasmScene, objectId: bigint, instanceId?: bigint): NodeRef[] {
  if (instanceId !== undefined) {
    const chain: NodeRef[] = [{ kind: 'instance', id: instanceId }]
    let parentId = wasmScene.node_parent(2, instanceId)
    while (parentId !== undefined) {
      chain.push({ kind: 'group', id: parentId })
      parentId = wasmScene.node_parent(1, parentId)
    }
    return chain
  }
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
 * Both the top-level and inside-a-group cases root the ancestor chain at the
 * picked node itself (the instance, if the ray hit instanced geometry;
 * otherwise the leaf object) via `buildAncestorChain`, so a nested instance
 * resolves the same way a nested plain object does.
 *
 * - Top level (ctx empty): selectable = outermost ancestor in the chain — a
 *   top-level instance/object resolves to itself; a nested one resolves to
 *   its outermost wrapper group.
 * - Inside instance I (deepest ctx node is instance I):
 *   - pick must be inside I → return the picked definition-member object
 *   - pick is not inside I → null (out of scope)
 * - Inside group G: selectable = direct child of G in the ancestor chain
 *   (may be a group, an instance, or a plain object).
 * - Inside world object O: out-of-scope picks return null.
 */
export function resolvePickToSelectable(
  wasmScene: WasmScene,
  pickedObjectId: bigint,
  activeContext: NodeRef[],
  pickedInstanceId?: bigint,
): NodeRef | null {
  if (activeContext.length === 0) {
    // Top level: root the chain at the picked node itself (the instance, if
    // the pick hit instanced geometry; otherwise the leaf object), walk group
    // parents up, and return the outermost ancestor. A top-level instance has
    // no group parent, so its chain is length 1 and it resolves to itself; a
    // nested instance resolves to its outermost wrapper group.
    const chain = buildAncestorChain(wasmScene, pickedObjectId, pickedInstanceId)
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

  // Inside group G: find the direct child of G in the instance-rooted ancestor
  // chain (same rooting as the top-level case) — the direct child may be a
  // group, an instance, or a plain object.
  const chain = buildAncestorChain(wasmScene, pickedObjectId, pickedInstanceId)
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
  onInferenceChange,
  onSceneChange,
  onToast,
  activeTool: activeToolProp,
  activeContext = [],
  selectedIds = [],
  activeLitSet = null,
  onSelect,
  onSelectMany,
  onSelectGuide,
  selectedGuide = null,
  onEnterContext,
  onExitContext,
  onDocumentChanged,
  apiRef,
  onMeasurement,
  onCameraDragChange,
  onHoverSketchRegionChange,
  currentMaterialId = MATERIAL_SENTINEL,
}: Props) {
  const containerRef = useRef<HTMLDivElement>(null)

  // Keep stable refs to latest callbacks
  const onStatusChangeRef = useRef(onStatusChange)
  onStatusChangeRef.current = onStatusChange
  const onInferenceChangeRef = useRef(onInferenceChange)
  onInferenceChangeRef.current = onInferenceChange
  const onSceneChangeRef = useRef(onSceneChange)
  onSceneChangeRef.current = onSceneChange
  const onToastRef = useRef(onToast)
  onToastRef.current = onToast
  const onSelectRef = useRef(onSelect)
  onSelectRef.current = onSelect
  const onSelectManyRef = useRef(onSelectMany)
  onSelectManyRef.current = onSelectMany
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
  const onCameraDragChangeRef = useRef(onCameraDragChange)
  onCameraDragChangeRef.current = onCameraDragChange
  const onHoverSketchRegionChangeRef = useRef(onHoverSketchRegionChange)
  onHoverSketchRegionChangeRef.current = onHoverSketchRegionChange
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

  // Last pointer NDC position, captured on every `onPointerMove` regardless of
  // any early-return below it (camera-nav mode, button held, ...) — unlike
  // `lastRayRef` this exists purely so a document mutation with NO subsequent
  // pointer move (undo/redo/delete/tool-commit) can still re-evaluate the
  // sketch-hover probe against "wherever the cursor actually is" instead of
  // leaving it stale until the next real move ( Follow-up:).
  // `null` until the pointer has entered the viewport at least once.
  const lastPointerNdcRef = useRef<{ ndcX: number; ndcY: number } | null>(null)

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
    // Canvas clear color — theme-aware. Matches --surface-canvas-page: dark is the exact
    // token hex; light approximates the CSS gradient with its middle stop
    // (a flat WebGL clear color can't reproduce a gradient).
    // "Sky" — lighter than the ground plane's own tint (GROUND_TINT below) in
    // both themes, so there's a visible horizon even where the grid has no
    // lines. Dark sky reuses
    // --surface-window (a shade lighter than --surface-canvas-page); light
    // sky is the lightest stop of the CSS gradient token.
    const CANVAS_CLEAR_COLOR: Record<'light' | 'dark', number> = { dark: 0x15181d, light: 0xf2f5f9 }
    renderer.setClearColor(CANVAS_CLEAR_COLOR[getResolvedTheme()])
    renderer.setSize(el.clientWidth, el.clientHeight)
    el.appendChild(renderer.domElement)

    const threeScene = new THREE.Scene()

    const camera = new THREE.PerspectiveCamera(45, el.clientWidth / el.clientHeight, 0.01, 100)
    // Person-scale default: frames a ~2–3 m region; classic SketchUp 3/4 angle.
    // Distance ≈ 4.7 m; a 1.8 m figure reads as substantial, not dwarfed.
    camera.position.set(3.5, -3.0, 2.5)
    camera.up.set(0, 0, 1)
    camera.lookAt(0, 0, 0)

    // Lights — theme-aware intensities. Dark keeps the original dim rig: its
    // low floor is what makes the dark viewport read as deliberately muted.
    // Light is tuned so a face square to the sun totals a surface multiplier
    // of ~1.0 — it renders its authored material color at full value
    // (SketchUp's look: white is 255, not grey) — with a high ambient floor
    // so no face falls into murk.
    //
    // The ×π: three r155+ interprets light intensity in physical units — the
    // Lambert BRDF divides irradiance by π — so an intensity of 1 lights a
    // white face to only 1/π (~0.32, i.e. 153 grey). Scaling by π makes each
    // value below read as the effective surface multiplier it produces;
    // verified against screen pixels with the Digital Color Meter approach
    // (a white face square to the sun measures 255, a shaded one ~221).
    // Dark's values stay raw (sub-physical) on purpose — that IS its mute.
    const MODEL_LIGHT_RIG: Record<'light' | 'dark', { ambient: number; directional: number }> = {
      dark: { ambient: 0.4, directional: 0.9 },
      light: { ambient: 0.72 * Math.PI, directional: 0.55 * Math.PI },
    }
    const initialRig = MODEL_LIGHT_RIG[getResolvedTheme()]
    const ambient = new THREE.AmbientLight(0xffffff, initialRig.ambient)
    threeScene.add(ambient)
    const dirLight = new THREE.DirectionalLight(0xffffff, initialRig.directional)
    dirLight.position.set(3, -5, 8)
    threeScene.add(dirLight)

    // Ground plane: an effectively-infinite, zoom-adaptive shader grid
    // (`InfiniteGrid.ts`) plus the world origin axes (named group so View ▸
    // Axes can toggle both,  / Follow-up:). `originAxes` is
    // `let`, not `const`: rebuilt on a theme change (static vertex-color
    // geometry, not a material .color that can just be reassigned) — the
    // grid, in contrast, only needs a cheap uniform write via `setColors()`.
    let originAxes = buildOriginAxes(getResolvedTheme())
    updateAxisResolution(originAxes, el.clientWidth, el.clientHeight)
    // Seed every other fat-line material (sketch edges, tool-preview
    // rubber-bands) at the initial canvas size too — mirrors the axes call
    // just above. Kept current on resize below; no longer walked every
    // render frame (see fatLine.ts's module doc comment).
    updateFatLineResolutions(el.clientWidth, el.clientHeight)
    threeScene.add(originAxes)
    const initialGridColors = GROUND_GRID_COLORS[getResolvedTheme()]
    const infiniteGrid = new InfiniteGrid(initialGridColors.ground, initialGridColors.minor, initialGridColors.major)
    threeScene.add(infiniteGrid.mesh)

    function disposeOriginAxes(group: THREE.Group): void {
      group.traverse((child) => {
        // Line2 (fat axes) extends Mesh, not LineSegments — match both so the
        // fat-line geometry+material are actually released.
        if (child instanceof THREE.LineSegments || child instanceof Line2) {
          child.geometry.dispose()
          if (child.material instanceof THREE.Material) child.material.dispose()
        }
      })
    }

    // Live theme reactivity: Settings > Theme can change at any time while
    // the viewport is mounted, so the clear color, light rig, and ground
    // plane need to follow it without a reload — everything else in the app
    // (CSS variables) already updates live via `data-theme`.
    const unsubscribeTheme = subscribeTheme(() => {
      const theme = getResolvedTheme()
      renderer.setClearColor(CANVAS_CLEAR_COLOR[theme])
      const rig = MODEL_LIGHT_RIG[theme]
      ambient.intensity = rig.ambient
      dirLight.intensity = rig.directional
      const gridColors = GROUND_GRID_COLORS[theme]
      infiniteGrid.setColors(gridColors.ground, gridColors.minor, gridColors.major)
      const wasVisible = originAxes.visible
      threeScene.remove(originAxes)
      disposeOriginAxes(originAxes)
      originAxes = buildOriginAxes(theme)
      updateAxisResolution(originAxes, el.clientWidth, el.clientHeight)
      originAxes.visible = wasVisible
      threeScene.add(originAxes)
      scheduleRenderRef.current()
    })

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
    function handleSelect(
      pickedObjectId: bigint | null,
      pickedInstanceId?: bigint,
      pickedSketchId?: bigint,
      pickedSketchEdgeId?: bigint,
      pickedSketchRegionId?: bigint,
    ): void {
      const additive = selectAdditiveRef.current && activeContextRef.current.length === 0
      const ctx = activeContextRef.current

      if (pickedObjectId === null) {
        // A sketch hit: free-standing sketches are top-level-only (
        // sketches have no group/instance nesting), so any active context
        // is simply out of scope for them, like a plain miss.
        if (pickedSketchId !== undefined && ctx.length === 0) {
          // An edge pick selects the drawn CURVE it belongs to (an arc's or
          // circle's facets act as one), else that single line. An interior
          // pick selects the ISLAND — the connected shape under the cursor,
          // never unrelated geometry meters away.
          if (pickedSketchEdgeId !== undefined) {
            const curve = wasmScene.sketch_edge_curve(pickedSketchId, pickedSketchEdgeId)
            if (curve !== undefined) {
              onSelectRef.current?.(
                { kind: 'sketch-curve', id: curve, sketch: pickedSketchId },
                additive,
              )
            } else {
              onSelectRef.current?.(
                { kind: 'sketch-edge', id: pickedSketchEdgeId, sketch: pickedSketchId },
                additive,
              )
            }
          } else if (pickedSketchRegionId !== undefined) {
            const island = wasmScene.sketch_region_island(pickedSketchId, pickedSketchRegionId)
            if (island !== undefined) {
              onSelectRef.current?.(
                { kind: 'sketch-island', id: island, sketch: pickedSketchId },
                additive,
              )
            } else {
              onSelectRef.current?.({ kind: 'sketch', id: pickedSketchId }, additive)
            }
          } else {
            onSelectRef.current?.({ kind: 'sketch', id: pickedSketchId }, additive)
          }
          scheduleRender()
          return
        }
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

    // ONE ground sketch shared by every draw tool (Line/Rectangle/Circle/
    // Arc), surviving tool switches, so mixed-tool profiles — an arc closed
    // by a Line chord, a rectangle meeting an arc — land in the same sketch
    // and can close regions. Cleared when a new document replaces the Scene.
    const groundSketchCache = makeSketchHandleCache()

    // ------------------------------------------------- select-all + marquee
    /**
     * Top-level selectable candidates with their visible leaf geometry ids.
     * Nodes whose every leaf is hidden (manually or via tags) are skipped —
     * neither Select All nor a marquee should sweep up invisible geometry.
     */
    /** Expand a node to its non-hidden leaf ids; null when every leaf is
     * hidden (the node has nothing on screen to select). */
    function visibleLeaves(node: NodeRef): { leafObjects: bigint[]; leafInstances: bigint[] } | null {
      const getGroupMembers = (gid: bigint): NodeRef[] =>
        wasmScene.group_members(gid).map(nodeRefFromJs)
      const { objectIds, instanceIds } = collectLeafIds(node, getGroupMembers)
      const leafObjects = objectIds.filter((id) => !hiddenObjectIdsRef.current.has(id))
      const leafInstances = instanceIds.filter((id) => !hiddenInstanceIdsRef.current.has(id))
      if (leafObjects.length === 0 && leafInstances.length === 0) return null
      return { leafObjects, leafInstances }
    }

    function visibleTopLevelCandidates(): {
      node: NodeRef
      leafObjects: bigint[]
      leafInstances: bigint[]
    }[] {
      const out: { node: NodeRef; leafObjects: bigint[]; leafInstances: bigint[] }[] = []
      for (const nj of wasmScene.top_level_nodes()) {
        const node = nodeRefFromJs(nj)
        const leaves = visibleLeaves(node)
        if (leaves === null) continue
        out.push({ node, ...leaves })
      }
      return out
    }

    /** Free-standing sketch refs (the kernel lists visible sketches only). */
    function visibleSketchRefs(): NodeRef[] {
      return Array.from(wasmScene.sketch_ids()).flatMap((id) =>
        Array.from(wasmScene.sketch_island_ids(id)).map((island) => ({
          kind: 'sketch-island' as const,
          id: island,
          sketch: id,
        })),
      )
    }

    /**
     * Select All (Edit ▸ Select All / ⌘A). At the top level: every visible
     * top-level node plus every free-standing sketch. Inside a group's
     * editing context: the group's direct members (what clicks select
     * there). Inside an instance/object context there is no multi-selectable
     * child set yet — no-op.
     */
    function selectAll(): void {
      const ctx = activeContextRef.current
      if (ctx.length > 0) {
        const top = ctx[ctx.length - 1]
        if (top.kind !== 'group') return
        // Same visibility rule as the top level: hidden members stay out.
        const members = wasmScene.group_members(top.id)
          .map(nodeRefFromJs)
          .filter((m) => visibleLeaves(m) !== null)
        if (members.length > 0) {
          onSelectManyRef.current?.(members, false)
          scheduleRender()
        }
        return
      }
      const refs = [...visibleTopLevelCandidates().map((c) => c.node), ...visibleSketchRefs()]
      if (refs.length > 0) {
        onSelectManyRef.current?.(refs, false)
        scheduleRender()
      }
    }

    /** The face meshes rendered for one node's visible leaves. */
    function candidateMeshes(cand: { leafObjects: bigint[]; leafInstances: bigint[] }): THREE.Mesh[] {
      const meshes: THREE.Mesh[] = []
      for (const id of cand.leafObjects) {
        sceneRenderer.getObjectGroup(id)?.traverse((child) => {
          if (child instanceof THREE.Mesh) meshes.push(child)
        })
      }
      for (const id of cand.leafInstances) {
        sceneRenderer.getInstanceGroup(id)?.traverse((child) => {
          if (child instanceof THREE.Mesh) meshes.push(child)
        })
      }
      return meshes
    }

    /**
     * The nodes a completed marquee selects. Window mode (L→R) requires every
     * vertex of every visible leaf mesh inside the rect; crossing mode (R→L)
     * takes any triangle/segment touching it. Construction guides keep their
     * own click-selection path and are not swept up.
     */
    function computeMarqueeSelection(rect: MarqueeRect, mode: MarqueeMode): NodeRef[] {
      // Matrices are current from the last render; a non-forced update only
      // touches nodes still flagged dirty.
      threeScene.updateMatrixWorld()
      const projector = new MarqueeProjector(camera, el.clientWidth, el.clientHeight)
      const out: NodeRef[] = []

      for (const cand of visibleTopLevelCandidates()) {
        const meshes = candidateMeshes(cand)
        if (meshes.length === 0) continue
        let hit: boolean
        if (mode === 'window') {
          hit = meshes.every((m) =>
            projector.allVerticesInRect(
              m.geometry.getAttribute('position').array, m.matrixWorld, rect,
            ),
          )
        } else {
          hit = meshes.some((m) =>
            projector.meshTouchesRect(
              m.geometry.getAttribute('position').array,
              m.geometry.index?.array ?? null,
              m.matrixWorld,
              rect,
            ),
          )
        }
        if (hit) out.push(cand.node)
      }

      const identity = new THREE.Matrix4()
      for (const s of visibleSketchRefs()) {
        // Island refs: their lines come from the island query, not the
        // whole-sketch one (s.id is an ISLAND handle).
        if (s.sketch === undefined) continue
        const lines = wasmScene.sketch_island_lines(s.sketch, s.id)
        if (lines.length === 0) continue
        const hit = mode === 'window'
          ? projector.allVerticesInRect(lines, identity, rect)
          : projector.segmentsTouchRect(lines, identity, rect)
        if (hit) out.push(s)
      }
      return out
    }

    /**
     * The Select tool's click-pick chain — a construction guide first (thin
     * deliberate targets beat the object beneath), then the tool's ray-pick
     * fallback chain. Shared by the in-context immediate press and the
     * top-level deferred (pointerup) click so the two paths stay in lockstep.
     */
    function dispatchSelectPick(ndcX: number, ndcY: number, ray: Ray): void {
      const g = pickGuide(ndcX, ndcY)
      if (g !== null) {
        onSelectGuideRef.current?.(g)
        scheduleRender()
        return
      }
      const { snap } = snapService.resolve(ray, el.clientHeight, camera.fov)
      toolController.activeTool.onPointerDown(snap, ray)
    }

    // Marquee drag state (Select tool, top-level context only). Armed on
    // pointerdown; becomes active past a small threshold. While armed, the
    // click-pick is DEFERRED to pointerup so a drag can become a marquee
    // instead of selecting whatever was under the initial press.
    const MARQUEE_DRAG_THRESHOLD_PX = 5
    interface MarqueeDrag {
      startX: number
      startY: number
      additive: boolean
      active: boolean
    }
    let marqueeDrag: MarqueeDrag | null = null

    const marqueeOverlay = document.createElement('div')
    marqueeOverlay.style.position = 'absolute'
    marqueeOverlay.style.display = 'none'
    marqueeOverlay.style.pointerEvents = 'none'
    marqueeOverlay.style.boxSizing = 'border-box'
    marqueeOverlay.style.background = 'rgba(74, 144, 226, 0.12)'
    marqueeOverlay.style.zIndex = '5'
    if (el.style.position === '') el.style.position = 'relative'
    el.appendChild(marqueeOverlay)

    function canvasPoint(ev: PointerEvent): [number, number] {
      const r = renderer.domElement.getBoundingClientRect()
      return [ev.clientX - r.left, ev.clientY - r.top]
    }

    function updateMarqueeOverlay(x: number, y: number): void {
      if (marqueeDrag === null) return
      const rect = normalizedRect(marqueeDrag.startX, marqueeDrag.startY, x, y)
      // Solid border = window (L→R), dashed = crossing (R→L) — SketchUp's cue.
      marqueeOverlay.style.border =
        x >= marqueeDrag.startX ? '1px solid #4a90e2' : '1px dashed #4a90e2'
      marqueeOverlay.style.left = `${rect.minX}px`
      marqueeOverlay.style.top = `${rect.minY}px`
      marqueeOverlay.style.width = `${rect.maxX - rect.minX}px`
      marqueeOverlay.style.height = `${rect.maxY - rect.minY}px`
      marqueeOverlay.style.display = 'block'
    }

    function clearMarquee(): void {
      marqueeDrag = null
      marqueeOverlay.style.display = 'none'
    }

    // ------------------------------------------------------------------ commit callbacks
    /**
     * Re-tessellate after a committed kernel mutation. With a `touched` hint
     * (single-object tool commits: push/pull, paint, move/rotate/scale of one
     * node) only the touched groups rebuild (`SceneRenderer.refreshTouched`);
     * without one (load/import/undo/redo/boolean/group/structural ops) the
     * full rebuild runs as before.
     */
    function handleSceneRefresh(touched?: RefreshTouched): void {
      const wtMap = touched !== undefined
        ? sceneRenderer.refreshTouched(touched)
        : sceneRenderer.refresh()
      onSceneChangeRef.current?.(wtMap)
      onDocumentChangedRef.current?.()
      scheduleRender()
      // Every kernel-mutating path funnels through here (tool commits,
      // boolean/group/delete, undo, redo, harness ops) — re-poll the sketch
      // hover probe against the cursor's last known position right away so a
      // stationary mouse across the mutation doesn't leave the contextual
      // dock showing a stale context (see `reevaluateHoverNow`'s doc comment).
      reevaluateHoverNow()
    }

    // Touched-hint for a single transformed/committed node: objects and
    // instances get a targeted refresh; groups (many leaves, possibly nested
    // instances) and sketches (also the Option-copy of either) fall back to
    // the full rebuild by returning undefined.
    /** Merged refresh hints for a tool commit — undefined (full refresh)
     * as soon as any node is a group/sketch. */
    function touchedForNodes(nodes: NodeRef[]): RefreshTouched | undefined {
      const objectIds: bigint[] = []
      const instanceIds: bigint[] = []
      for (const node of nodes) {
        if (node.kind === 'object') objectIds.push(node.id)
        else if (node.kind === 'instance') instanceIds.push(node.id)
        else return undefined
      }
      return { objectIds, instanceIds }
    }

    // Re-tessellate after a harness-driven kernel mutation (ViewportApi.refreshScene).
    function refreshScene(): void {
      handleSceneRefresh()
      sceneRenderer.refreshAllSketches()
      // Harness mutations can also add/remove construction guides
      // (addGuideLine/addGuidePoint/deleteGuide); keep their overlay faithful too.
      sceneRenderer.refreshGuides()
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
      // kind: 0=object, 1=group, 2=instance — the same 3-way mapping as
      // runDelete/runMakeComponent. Instances must not collapse to 0: the
      // object and instance slotmaps reuse bit patterns, so a mis-kinded id
      // can silently address a different live node.
      const kinds = new Uint8Array(nodes.map((n) =>
        n.kind === 'group' ? 1 : n.kind === 'instance' ? 2 : 0,
      ))
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
      // kind: 0=object, 1=group, 2=instance; 'sketch' has no NodeId — its own
      // dedicated delete_sketch call, mirroring delete_guide's shape.
      //
      // Sketch-edge deletes run FIRST, and an edge whose whole sketch is
      // also selected is skipped — the sketch delete covers it, and deleting
      // the sketch first would strand the edge's handle in a kernel error.
      const deletedSketches = new Set(
        nodes.filter((n) => n.kind === 'sketch').map((n) => n.id),
      )
      const isSub = (n: NodeRef) =>
        n.kind === 'sketch-edge' || n.kind === 'sketch-curve' || n.kind === 'sketch-island'
      const ordered = [...nodes.filter(isSub), ...nodes.filter((n) => !isSub(n))]
      // Dissolve a batch of edges as ONE gesture (one undo step); an emptied
      // sketch husk is removed afterward.
      const removeEdgeBatch = (sketch: bigint, edges: bigint[]): void => {
        if (edges.length === 0) return
        // Pre-validate: if ANY edge lies on an extruded region's boundary
        // the whole batch refuses up front — never a partial removal
        // committed as one undo step.
        for (const e of edges) {
          if (wasmScene.sketch_edge_borders_solid(sketch, e)) {
            handleToast(
              "Can't delete: part of this shape is the footprint of a solid",
              'EdgeBordersSolid',
            )
            return
          }
        }
        wasmScene.sketch_begin_gesture(sketch)
        try {
          for (const e of edges) wasmScene.sketch_remove_edge(sketch, e)
        } finally {
          wasmScene.sketch_end_gesture(sketch)
        }
        const stillListed = Array.from(wasmScene.sketch_ids()).includes(sketch)
        if (stillListed && wasmScene.sketch_lines(sketch).length === 0) {
          wasmScene.delete_sketch(sketch)
        }
      }
      for (const n of ordered) {
        try {
          if (n.kind === 'sketch-island' && n.sketch !== undefined) {
            if (deletedSketches.has(n.sketch)) continue
            removeEdgeBatch(n.sketch, Array.from(wasmScene.sketch_island_edges(n.sketch, n.id)))
            continue
          }
          if (n.kind === 'sketch-curve' && n.sketch !== undefined) {
            if (deletedSketches.has(n.sketch)) continue
            removeEdgeBatch(n.sketch, Array.from(wasmScene.sketch_curve_edges(n.sketch, n.id)))
            continue
          }
          if (n.kind === 'sketch-edge' && n.sketch !== undefined) {
            if (deletedSketches.has(n.sketch)) continue
            // Dissolve one line: regions it separated merge back together.
            // Bracketed in a sketch gesture so it lands as ONE undo step
            // (the same mechanism the draw tools commit through).
            wasmScene.sketch_begin_gesture(n.sketch)
            try {
              wasmScene.sketch_remove_edge(n.sketch, n.id)
            } finally {
              wasmScene.sketch_end_gesture(n.sketch)
            }
            // Deleting the last line leaves an invisible, unusable empty
            // sketch — remove the husk too (its own undo step). Guarded on
            // sketch_ids: a sketch with zero LIVE lines whose remaining
            // edges are consumed backs an extruded solid and has already
            // dropped out of the listing — it must not be tombstoned.
            const stillListed = Array.from(wasmScene.sketch_ids()).includes(n.sketch)
            if (stillListed && wasmScene.sketch_lines(n.sketch).length === 0) {
              wasmScene.delete_sketch(n.sketch)
            }
          } else if (n.kind === 'sketch') {
            wasmScene.delete_sketch(n.id)
          } else {
            const kind = n.kind === 'group' ? 1 : n.kind === 'instance' ? 2 : 0
            wasmScene.delete_node(kind, n.id)
          }
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
      // A new/loaded document replaced the Scene — the shared ground-sketch
      // handle and any handles the active tool cached are now stale.
      // Re-selecting the same tool doesn't recreate it, so reset explicitly.
      groundSketchCache.set(null)
      const at = toolController.activeTool
      if ('onDocumentReset' in at) {
        (at as { onDocumentReset(): void }).onDocumentReset()
      }
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

    function setCamera(
      position: [number, number, number],
      target: [number, number, number],
      up: [number, number, number],
      fovDeg: number,
    ): void {
      camera.position.set(position[0], position[1], position[2])
      controls.target.set(target[0], target[1], target[2])
      camera.up.set(up[0], up[1], up[2])
      camera.fov = fovDeg
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
      originAxes.visible = visible
      // Hidden axes must not snap or flash a cue — gate inference too.
      wasmScene.set_axes_snappable(visible)
      scheduleRender()
    }

    function setGridVisible(visible: boolean): void {
      infiniteGrid.mesh.visible = visible
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

    async function exportStl(): Promise<StlBuildResult | null> {
      return exportSceneToStl(sceneRenderer)
    }

    if (apiRefRef.current !== undefined) {
      const isCapturingInput = (): boolean => {
        const t = toolController.activeTool
        return 'capturingInput' in t && (t as { capturingInput(): boolean }).capturingInput()
      }
      apiRefRef.current.current = { runBoolean, runGroup, runUngroup, runDelete, runMakeComponent, runPlaceInstance, runExplodeInstance, runMakeUnique, notifyLoaded, refreshScene, isCapturingInput, runUndo, runRedo, zoomExtents, setStandardView, setCamera, setHidden, selectAll, setAxesVisible, setGridVisible, setGuidesVisible, deleteAllGuides, runDeleteGuide, exportGlb, exportStl }
    }

    // ------------------------------------------------------------------ tool factories
    function makeRectTool(): RectangleTool {
      const tool = new RectangleTool(
        wasmScene,
        previewGroup,
        (result) => {
          sceneRenderer.refreshAllSketches()
          sceneRenderer.refreshGuides()
          onDocumentChangedRef.current?.()
          scheduleRender()
        },
        handleToast,
        (objectId) => {
          handleSceneRefresh({ objectIds: [objectId] })
        },
        (text: string) => { onMeasurementRef.current?.(text) },
        groundSketchCache,
      )
      // Scope the tool to the current editing context, if any.
      const ctx = activeContextRef.current
      const ctxId = ctx.length > 0 && ctx[ctx.length - 1].kind === 'object'
        ? ctx[ctx.length - 1].id : null
      tool.setActiveContext(ctxId)
      return tool
    }

    function makeCircleTool(): CircleTool {
      const tool = new CircleTool(
        wasmScene,
        previewGroup,
        (result) => {
          sceneRenderer.refreshAllSketches()
          sceneRenderer.refreshGuides()
          onDocumentChangedRef.current?.()
          scheduleRender()
        },
        handleToast,
        (objectId) => {
          handleSceneRefresh({ objectIds: [objectId] })
        },
        (text: string) => { onMeasurementRef.current?.(text) },
        groundSketchCache,
      )
      // Scope the tool to the current editing context, if any.
      const ctx = activeContextRef.current
      const ctxId = ctx.length > 0 && ctx[ctx.length - 1].kind === 'object'
        ? ctx[ctx.length - 1].id : null
      tool.setActiveContext(ctxId)
      return tool
    }

    function makeArcTool(): ArcTool {
      const tool = new ArcTool(
        wasmScene,
        previewGroup,
        (result) => {
          sceneRenderer.refreshAllSketches()
          sceneRenderer.refreshGuides()
          onDocumentChangedRef.current?.()
          scheduleRender()
        },
        handleToast,
        (objectId) => {
          handleSceneRefresh({ objectIds: [objectId] })
        },
        (text: string) => { onMeasurementRef.current?.(text) },
        groundSketchCache,
      )
      // Scope the tool to the current editing context, if any.
      const ctx = activeContextRef.current
      const ctxId = ctx.length > 0 && ctx[ctx.length - 1].kind === 'object'
        ? ctx[ctx.length - 1].id : null
      tool.setActiveContext(ctxId)
      return tool
    }

    function makeLineTool(): LineTool {
      const tool = new LineTool(
        wasmScene,
        previewGroup,
        (sketchHandle) => {
          sceneRenderer.refreshAllSketches()
          sceneRenderer.refreshGuides()
          onDocumentChangedRef.current?.()
          scheduleRender()
        },
        handleToast,
        (objectId) => {
          handleSceneRefresh({ objectIds: [objectId] })
        },
        (text: string) => { onMeasurementRef.current?.(text) },
        groundSketchCache,
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
        // Targeted refresh: the committed object (a world object OR a def
        // member for push_pull_in_component — refreshTouched rebuilds every
        // placement of a touched member). A through-cut's extra result
        // objects / consumed source are caught by refreshTouched's id diff.
        (objectId) => {
          handleSceneRefresh({ objectIds: [objectId] })
          sceneRenderer.refreshAllSketches()
          sceneRenderer.refreshGuides()
        },
        handleToast,
        (text: string) => { onMeasurementRef.current?.(text) },
      )
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
        // Targeted refresh: only the painted object rebuilds. Painting a def
        // member (instanced geometry) invalidates all its placements via
        // refreshTouched's member-cache path.
        (objectId) => {
          handleSceneRefresh({ objectIds: [objectId] })
        },
        handleToast,
      )
    }

    function makeMoveTool(): MoveTool {
      return new MoveTool(
        wasmScene,
        previewGroup,
        sceneRenderer.objectsGroup,
        [...selectedIdsRef.current],
        (nodes) => {
          handleSceneRefresh(touchedForNodes(nodes))
          // A sketch move bakes new vertex positions; rebuild sketch buffers so
          // the lines follow (objects refresh via handleSceneRefresh; sketches
          // do not). Mirrors the boolean/undo refresh pairing.
          sceneRenderer.refreshAllSketches()
          // Select the committed nodes — for an Option-copy these are the
          // fresh clones, so a follow-up Alt-drag chains off the new copies.
          if (nodes.length === 1) onSelectRef.current?.(nodes[0], false)
          else onSelectManyRef.current?.(nodes, false)
        },
        handleToast,
        (text: string) => { onMeasurementRef.current?.(text) },
        (id: bigint) => sceneRenderer.getInstanceGroup(id),
      )
    }

    function makeRotateTool(): RotateTool {
      return new RotateTool(
        wasmScene,
        previewGroup,
        sceneRenderer.objectsGroup,
        [...selectedIdsRef.current],
        (nodes) => {
          handleSceneRefresh(touchedForNodes(nodes))
          // Rebuild sketch buffers so a rotated sketch's lines follow (see
          // makeMoveTool).
          sceneRenderer.refreshAllSketches()
        },
        handleToast,
        (id: bigint) => sceneRenderer.getInstanceGroup(id),
        (text: string) => { onMeasurementRef.current?.(text) },
      )
    }

    function makeScaleTool(): ScaleTool {
      return new ScaleTool(
        wasmScene,
        previewGroup,
        sceneRenderer.objectsGroup,
        [...selectedIdsRef.current],
        (nodes) => {
          handleSceneRefresh(touchedForNodes(nodes))
          // Rebuild sketch buffers so a scaled sketch's lines follow (see
          // makeMoveTool).
          sceneRenderer.refreshAllSketches()
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

    function makeEditVertexTool(): EditVertexTool {
      return new EditVertexTool(
        wasmScene,
        previewGroup,
        // A vertex drag bakes new sketch geometry: refresh objects AND rebuild
        // sketch line buffers (handleSceneRefresh alone does NOT cover sketches
        // — same pairing as makeMoveTool's sketch branch and undo/redo).
        () => {
          handleSceneRefresh()
          sceneRenderer.refreshAllSketches()
          onDocumentChangedRef.current?.()
          scheduleRender()
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
        case 'Circle':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          toolController.setTool(makeCircleTool())
          break
        case 'Arc':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          toolController.setTool(makeArcTool())
          break
        case 'Line':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          toolController.setTool(makeLineTool())
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
        case 'Edit Vertex':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          toolController.setTool(makeEditVertexTool())
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
      onInferenceChangeRef.current?.(null)
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

    // Camera-drag notifications: tell the parent while a pointer-drag
    // navigation is in flight so it can fade the contextual dock out of the
    // way. OrbitControls' 'start'/'end' also fire as an immediate pair on
    // every wheel tick, so gate 'start' on a pointer actually being down —
    // the window-level CAPTURE listeners below run before OrbitControls' own
    // element-level pointerdown handler (which dispatches 'start' from inside
    // the same event), so the flag is always current by then.
    let cameraPointerDown = false
    let cameraDragActive = false
    function onCameraPointerDown(): void { cameraPointerDown = true }
    function onCameraPointerUp(): void { cameraPointerDown = false }
    function onControlsStart(): void {
      if (!cameraPointerDown || cameraDragActive) return
      cameraDragActive = true
      onCameraDragChangeRef.current?.(true)
    }
    function onControlsEnd(): void {
      if (!cameraDragActive) return
      cameraDragActive = false
      onCameraDragChangeRef.current?.(false)
    }
    window.addEventListener('pointerdown', onCameraPointerDown, true)
    window.addEventListener('pointerup', onCameraPointerUp, true)
    window.addEventListener('pointercancel', onCameraPointerUp, true)
    controls.addEventListener('start', onControlsStart)
    controls.addEventListener('end', onControlsEnd)

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
        // Keep guide-line dashes screen-constant too (see updateGuideDashScale).
        sceneRenderer.updateGuideDashScale(controls.getDistance())
        // Protractor's plane-preview disk is a virtual construct too — keep
        // it screen-constant the same way (see ProtractorTool.updateDiskScale).
        const activeToolForScale = toolController.activeTool
        if ('updateDiskScale' in activeToolForScale) {
          ;(activeToolForScale as { updateDiskScale(c: THREE.Camera): void }).updateDiskScale(camera)
        }
        // Feed the shader grid the camera's current position so it can pick
        // the right cell-size decade per fragment.
        infiniteGrid.update(camera.position)
        // Fat-line resolutions (sketch edges, tool-preview rubber-bands) are
        // NOT refreshed here: LineMaterial's resolution uniform depends only
        // on the canvas size, so it's set at mount and on resize (see the
        // ResizeObserver below) through the fat-line material registry. The
        // old per-frame full-scene traverse walked every Object3D (thousands
        // on a large document) each orbit frame just to re-set an unchanged
        // uniform on a handful of materials.
        // Render stats (debug-log readout): only timed while the readout is
        // mounted — with it closed this is a single boolean check per
        // rendered frame. Read renderer.info right after render(), before
        // three.js auto-resets the per-frame counters on the next frame.
        const statsActive = isRenderStatsActive()
        const renderStart = statsActive ? performance.now() : 0
        renderer.render(threeScene, camera)
        if (statsActive) {
          recordRender(renderer.info, performance.now() - renderStart)
        }
        needsRender = false
      }
    }
    render()

    function scheduleRender(): void {
      needsRender = true
    }
    scheduleRenderRef.current = scheduleRender

    controls.addEventListener('change', scheduleRender)

    // Low-level capture: camera state on every orbit/pan/zoom change, and
    // keys (Shift axis-lock, Esc/Enter/Del). All no-ops unless recording.
    function recordCameraInput(): void {
      if (!inputRecorder.isActive()) return
      inputRecorder.recordCamera(
        [camera.position.x, camera.position.y, camera.position.z],
        [controls.target.x, controls.target.y, controls.target.z],
        [camera.up.x, camera.up.y, camera.up.z],
        camera.fov,
      )
    }
    function onKeyDownRecord(ev: KeyboardEvent): void {
      inputRecorder.recordKey('keydown', ev)
    }
    function onKeyUpRecord(ev: KeyboardEvent): void {
      inputRecorder.recordKey('keyup', ev)
    }
    controls.addEventListener('change', recordCameraInput)
    window.addEventListener('keydown', onKeyDownRecord)
    window.addEventListener('keyup', onKeyUpRecord)

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

    // Low-level input capture. A no-op unless a recording is active, so
    // it costs nothing in normal use; coords are canvas-relative CSS px so replay
    // can dispatch synthetic events at the same place.
    function recordPointerInput(
      kind: 'pointermove' | 'pointerdown' | 'pointerup',
      ev: PointerEvent,
    ): void {
      if (!inputRecorder.isActive()) return
      const [px, py] = canvasPoint(ev)
      inputRecorder.recordPointer(kind, px, py, ev)
    }

    // ------------------------------------------------------------------ sketch-region hover probe
    // "Sketches are first-class interactable" — an idle cursor aimed at a
    // free-standing sketch's extrudable region previews the dock's Push/Pull
    // verb (App.tsx/ContextualDock.tsx), but ONLY while nothing is selected
    // (an explicit selection's dock always wins — the check here just avoids
    // paying for the ray-cast in that case too). Throttled + edge-detected by
    // SketchHoverGate so the wasm pick runs at most once per ~100ms and the
    // callback (-> React state) fires only on an actual transition.
    const hoverGate = new SketchHoverGate()
    // Shared "should the hover probe be suppressed right now" predicate —
    // explicit selection, a camera-nav tool, or an in-flight camera drag
    // (stray middle/right-drag orbit/pan while a non-camera tool is active —
    // cameraModeRef alone wouldn't catch that). Factored out of
    // `updateSketchHover` so `reevaluateHoverNow` (post-mutation re-poll,
    // below) shares the exact same rule instead of drifting from it.
    function isHoverPaused(): boolean {
      return selectedIdsRef.current.length > 0 || cameraModeRef.current || cameraDragActive
    }
    function pickSketchHover(ray: Ray): boolean {
      const pick = wasmScene.pick_sketch_region(
        ray.origin[0], ray.origin[1], ray.origin[2],
        ray.direction[0], ray.direction[1], ray.direction[2],
      )
      const hovering = pick !== undefined
      pick?.free()
      return hovering
    }
    function updateSketchHover(ray: Ray, ev: PointerEvent): void {
      const cb = onHoverSketchRegionChangeRef.current
      if (cb === undefined) return
      // Also paused on any button held (the same "mid-gesture" signal the
      // geometry-routing early-return below uses). Forces the state back to
      // false so a stale "hovering" can't stick through a drag the cursor
      // drifted away during.
      const paused = isHoverPaused() || (ev.buttons !== 0 && ev.button !== -1)
      if (paused) {
        const next = hoverGate.pause()
        if (next !== null) cb(next)
        return
      }
      const now = typeof performance !== 'undefined' ? performance.now() : Date.now()
      if (!hoverGate.shouldPoll(now)) return
      const next = hoverGate.update(pickSketchHover(ray))
      if (next !== null) cb(next)
    }

    /**
     * Re-evaluate the sketch-hover probe right now, against the last known
     * pointer position, instead of waiting for the next `pointermove`
     *. Called from
     * `handleSceneRefresh`, the one choke point every kernel-mutating path
     * (tool commits, boolean/group/delete, undo, redo, harness ops) already
     * funnels through.
     *
     * `hoverGate.reset()` clears the throttle clock (not the emitted state)
     * so this always re-polls immediately rather than being swallowed by the
     * normal ~100ms window — the whole point is that the document just
     * changed, so the last poll's result may already be stale.
     */
    function reevaluateHoverNow(): void {
      const cb = onHoverSketchRegionChangeRef.current
      if (cb === undefined) return
      hoverGate.reset()
      if (lastPointerNdcRef.current === null) {
        // No pointer has entered the viewport yet — nothing to re-pick against.
        const next = hoverGate.update(false)
        if (next !== null) cb(next)
        return
      }
      if (isHoverPaused()) {
        const next = hoverGate.pause()
        if (next !== null) cb(next)
        return
      }
      const { ndcX, ndcY } = lastPointerNdcRef.current
      const ray = makeWorldRay(ndcX, ndcY, camera)
      const next = hoverGate.update(pickSketchHover(ray))
      if (next !== null) cb(next)
    }

    // ------------------------------------------------------------------ pointer move (snap + cue)
    function onPointerMove(ev: PointerEvent): void {
      // Capture every raw move first (before any early-return) so low-level
      // replay reproduces the whole stack, camera-nav moves included.
      recordPointerInput('pointermove', ev)

      // NDC/ray math is cheap (no wasm calls) — compute it up front so both
      // the hover probe above and the geometry routing below share one ray,
      // and the probe still runs even when the early-returns below skip the
      // rest of this function (it has its own pause conditions).
      const [ndcX, ndcY] = pointerToNDC(ev, renderer.domElement)
      const ray = makeWorldRay(ndcX, ndcY, camera)
      // Remember where the pointer is, unconditionally, so a later document
      // mutation with no further pointer move (undo/redo/delete/tool-commit)
      // can still re-evaluate the hover probe via `reevaluateHoverNow` instead
      // of leaving it stale (see that function's doc comment).
      lastPointerNdcRef.current = { ndcX, ndcY }
      updateSketchHover(ray, ev)

      // In camera-nav mode, OrbitControls owns left-drag — skip geometry routing.
      if (cameraModeRef.current) return

      // Armed marquee: past the drag threshold the rubber-band owns the
      // pointer — update the rectangle and skip hover/snap work entirely.
      if (marqueeDrag !== null) {
        if ((ev.buttons & 1) === 0) {
          // The release happened outside our listeners (focus loss) — drop it.
          clearMarquee()
        } else {
          const [px, py] = canvasPoint(ev)
          if (
            !marqueeDrag.active &&
            Math.hypot(px - marqueeDrag.startX, py - marqueeDrag.startY) >= MARQUEE_DRAG_THRESHOLD_PX
          ) {
            marqueeDrag.active = true
          }
          if (marqueeDrag.active) {
            updateMarqueeOverlay(px, py)
            return
          }
        }
      }
      if (ev.buttons !== 0 && ev.button !== -1) return

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

      // Inference tooltip chip + snap dot (Refinement B) —
      // container-relative screen coords so App.tsx can position DOM overlays
      // directly. These project the SNAP POINT's world position (not the raw
      // cursor), so the dot sits exactly on the inferred point and, crucially,
      // stays pinned there when the magnetic hysteresis in SnapService holds a
      // snap while the cursor drifts off it (that "resistance" is invisible if
      // the dot just tracks the cursor). snap.direction passes through
      // unprocessed for the tooltip to resolve its own axis/color.
      if (snap === null) {
        onInferenceChangeRef.current?.(null)
      } else {
        const p = worldToPixels(new THREE.Vector3(snap.x, snap.y, snap.z))
        onInferenceChangeRef.current?.({
          kind: snapKind ?? snap.kind,
          screenX: p.x,
          screenY: p.y,
          direction: snap.direction,
        })
      }
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
      recordPointerInput('pointerdown', ev)
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

      if (toolController.activeToolName === 'Select') {
        // Top level: arm a marquee and DEFER the pick to pointerup — a drag
        // becomes a rubber-band selection, a plain release runs the click-pick
        // at the release position. Inside an editing context the marquee is
        // out of scope; the press is an immediate click-pick.
        if (activeContextRef.current.length === 0) {
          const [px, py] = canvasPoint(ev)
          marqueeDrag = { startX: px, startY: py, additive: ev.shiftKey, active: false }
          // Track the drag even when it leaves the canvas.
          renderer.domElement.setPointerCapture(ev.pointerId)
        } else {
          dispatchSelectPick(ndcX, ndcY, ray)
        }
        return
      }

      const activeTool = toolController.activeTool

      // The second pointerdown of a double-click carries `detail >= 2` — it's
      // the phantom that precedes the 'dblclick' event. For tools that finish
      // on double-click (LineTool), skip routing it so it can't place a
      // spurious near-duplicate point; the 'dblclick' handler runs
      // onDoubleClick instead. Distinct clicks are always detail === 1, so
      // normal point-by-point drawing is unaffected at any cadence.
      if (ev.detail >= 2 && 'onDoubleClick' in activeTool) return

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

      // Let the active tool consume the double-click first (e.g. LineTool
      // ending a chain) — only fall through to the default "enter context"
      // gesture below when the tool doesn't handle it (or isn't mid-gesture).
      const activeTool = toolController.activeTool
      if ('onDoubleClick' in activeTool) {
        const viewportH = el.clientHeight
        const fovY = camera.fov
        const constraint = 'snapConstraint' in activeTool
          ? (activeTool as { snapConstraint(ray?: Ray): { anchor?: [number, number, number]; lockAxis?: 0 | 1 | 2; constraintPlane?: { point: [number, number, number]; normal: [number, number, number] } } | null }).snapConstraint(ray)
          : null
        const { snap } = snapService.resolve(ray, viewportH, fovY, constraint?.anchor, constraint?.lockAxis, constraint?.constraintPlane)
        const handled = (activeTool as { onDoubleClick(snap: Snap | null, ray: Ray): boolean }).onDoubleClick(snap, ray)
        if (handled) {
          scheduleRender()
          return
        }
      }

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

      // Esc cancels an in-flight marquee before anything else.
      if (ev.key === 'Escape' && marqueeDrag !== null) {
        clearMarquee()
        return
      }

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
        if (ev.key === 'c' || ev.key === 'C') { switchToolRef.current?.('Circle'); return }
        if (ev.key === 'a' || ev.key === 'A') { switchToolRef.current?.('Arc'); return }
        if (ev.key === 'l' || ev.key === 'L') { switchToolRef.current?.('Line'); return }
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

      // Redo: Shift+Cmd/Ctrl+Z — document-level. With Shift held, ev.key is
      // the UPPERCASE letter, so compare case-insensitively (a bare === 'z'
      // never fires on a physical keyboard — caught by the input-pipeline
      // E2E redo spec).
      if (isMod && ev.shiftKey && ev.key.toLowerCase() === 'z') {
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

      // Mod-combos never reach the tool: tools' onKey treats bare letters
      // as VCB length input, so an unhandled chord like Ctrl+K (palette) or
      // Ctrl+C would otherwise append its letter to a mid-entry buffer
      // ("5" → "5k") and wedge it. Tools only consume plain keys.
      if (!isMod) toolController.activeTool.onKey(ev)
    }

    // Pointerup completes the Select tool's deferred press: a no-drag release
    // replays the press as the usual click-pick (guide first, then the tool's
    // pick chain); a dragged release commits the marquee selection. Other
    // tools are click-based, so beyond input recording nothing else needs it.
    function onPointerUp(ev: PointerEvent): void {
      recordPointerInput('pointerup', ev)
      if (ev.button !== 0 || marqueeDrag === null) return
      const drag = marqueeDrag
      clearMarquee()

      // The tool changed mid-drag (keyboard shortcut) — drop the gesture.
      if (toolController.activeToolName !== 'Select') return

      if (!drag.active) {
        // A plain click: run the pick chain at the RELEASE position — if the
        // camera moved between press and release (scroll zoom, inertia), the
        // pick lands on what is visibly under the cursor now.
        const [ndcX, ndcY] = pointerToNDC(ev, renderer.domElement)
        dispatchSelectPick(ndcX, ndcY, makeWorldRay(ndcX, ndcY, camera))
        return
      }

      const [px, py] = canvasPoint(ev)
      const rect = normalizedRect(drag.startX, drag.startY, px, py)
      // Drag direction picks the mode: L→R window, R→L crossing (SketchUp).
      const mode: MarqueeMode = px >= drag.startX ? 'window' : 'crossing'
      const refs = computeMarqueeSelection(rect, mode)
      // An empty marquee clears a non-additive selection, like clicking air.
      onSelectManyRef.current?.(refs, drag.additive)
      scheduleRender()
    }
    function onPointerCancel(ev: PointerEvent): void {
      recordPointerInput('pointerup', ev)
      clearMarquee()
    }
    renderer.domElement.addEventListener('pointermove', onPointerMove)
    renderer.domElement.addEventListener('pointerdown', onPointerDown)
    renderer.domElement.addEventListener('pointerup', onPointerUp)
    renderer.domElement.addEventListener('pointercancel', onPointerCancel)
    renderer.domElement.addEventListener('dblclick', onDoubleClick)
    window.addEventListener('keydown', onKeyDown)

    // ------------------------------------------------------------------ resize
    const resizeObserver = new ResizeObserver(() => {
      const w = el.clientWidth
      const h = el.clientHeight
      camera.aspect = w / h
      camera.updateProjectionMatrix()
      renderer.setSize(w, h)
      updateAxisResolution(originAxes, w, h)
      // Keep every fat line (sketch edges, tool-preview rubber-bands) sized
      // to the new canvas — resize is the only time the resolution uniform
      // actually changes (the registry replaces the old per-frame traverse).
      updateFatLineResolutions(w, h)
      scheduleRender()
    })
    resizeObserver.observe(el)

    // ------------------------------------------------------------------ cleanup
    return () => {
      cancelAnimationFrame(rafId)
      controls.removeEventListener('change', scheduleRender)
      controls.removeEventListener('change', recordCameraInput)
      controls.removeEventListener('start', onControlsStart)
      controls.removeEventListener('end', onControlsEnd)
      window.removeEventListener('pointerdown', onCameraPointerDown, true)
      window.removeEventListener('pointerup', onCameraPointerUp, true)
      window.removeEventListener('pointercancel', onCameraPointerUp, true)
      // Don't leave the parent thinking a drag is still active mid-teardown.
      if (cameraDragActive) onCameraDragChangeRef.current?.(false)
      window.removeEventListener('keydown', onShiftKeyDown)
      window.removeEventListener('keyup', onShiftKeyUp)
      window.removeEventListener('keydown', onKeyDownRecord)
      window.removeEventListener('keyup', onKeyUpRecord)
      renderer.domElement.removeEventListener('webglcontextlost', onContextLost)
      renderer.domElement.removeEventListener('webglcontextrestored', onContextRestored)
      contextLostOverlay?.remove()
      renderer.domElement.removeEventListener('contextmenu', onContextMenu)
      renderer.domElement.removeEventListener('pointermove', onPointerMove)
      renderer.domElement.removeEventListener('pointerdown', onPointerDown)
      renderer.domElement.removeEventListener('pointerup', onPointerUp)
      renderer.domElement.removeEventListener('pointercancel', onPointerCancel)
      renderer.domElement.removeEventListener('dblclick', onDoubleClick)
      marqueeOverlay.remove()
      window.removeEventListener('keydown', onKeyDown)
      resizeObserver.disconnect()
      unsubscribeTheme()
      disposeOriginAxes(originAxes)
      infiniteGrid.dispose()
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
  // the tree). Object and instance refs are passed straight through; group
  // refs own no geometry themselves, so they're expanded (recursively —
  // groups nest) to their leaf objects/instances via the shared
  // `collectLeafIds` helper before being handed to setSelected/
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
    // Collect leaf object ids, instance ids, and sketch ids for highlighting.
    // Groups recurse via collectLeafIds/group_members so a group selection
    // (e.g. an imported component's outermost group) highlights every leaf
    // object and instance it contains, however deeply nested.
    const leafIds: bigint[] = []
    const instanceIds: bigint[] = []
    const sketchIds: bigint[] = []
    const sketchEdges: { sketch: bigint; edge: bigint }[] = []
    const sketchIslands: { sketch: bigint; island: bigint }[] = []
    const getGroupMembers = (groupId: bigint): NodeRef[] =>
      wasmSceneRef.current.group_members(groupId).map((m) => ({ kind: m.kind as NodeRef['kind'], id: m.id }))
    for (const node of selectedIds) {
      if (node.kind === 'sketch') {
        sketchIds.push(node.id)
        continue
      }
      if (node.kind === 'sketch-island' && node.sketch !== undefined) {
        sketchIslands.push({ sketch: node.sketch, island: node.id })
        continue
      }
      if (node.kind === 'sketch-curve' && node.sketch !== undefined) {
        // A curve highlights as its member edges.
        for (const edge of wasmSceneRef.current.sketch_curve_edges(node.sketch, node.id)) {
          sketchEdges.push({ sketch: node.sketch, edge })
        }
        continue
      }
      if (node.kind === 'sketch-edge' && node.sketch !== undefined) {
        sketchEdges.push({ sketch: node.sketch, edge: node.id })
        continue
      }
      const { objectIds, instanceIds: leafInstanceIds } = collectLeafIds(node, getGroupMembers)
      leafIds.push(...objectIds)
      instanceIds.push(...leafInstanceIds)
    }
    sceneRendererRef.current?.setSelected(leafIds)
    sceneRendererRef.current?.setSelectedInstances(instanceIds)
    sceneRendererRef.current?.setSelectedSketches(sketchIds)
    sceneRendererRef.current?.setSelectedSketchIslands(sketchIslands)
    sceneRendererRef.current?.setSelectedSketchEdges(sketchEdges)
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
