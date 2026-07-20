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
import { DEPTH_BIAS } from './depthPolicy'
import type { Scene as WasmScene, DocChangeJs } from '../wasm/loader'
import { CueLayer } from './CueLayer'
import { DrawPlaneCueLayer } from './DrawPlaneCueLayer'
import type { DrawPlane } from '../tools/drawPlane'
import { SnapService } from './snapService'
import { SceneRenderer, type RefreshTouched } from './SceneRenderer'
import { expandByVisibleObject } from './visibleBounds'
import * as inputRecorder from '../recording/inputRecorder'
import { exportSceneToGlb } from '../io/exporters/gltfExport'
import { exportSceneToStl, type StlBuildResult } from '../io/exporters/stlExport'
import { exportSceneTo3mf, type ThreeMfBuildResult } from '../io/exporters/threeMfExport'
import { ToolController } from '../tools/ToolController'
import { RectangleTool } from '../tools/RectangleTool'
import { CircleTool } from '../tools/CircleTool'
import { PolygonTool, DEFAULT_POLYGON_SIDES } from '../tools/PolygonTool'
import { ArcTool } from '../tools/ArcTool'
import { LineTool } from '../tools/LineTool'
import { PushPullTool } from '../tools/PushPullTool'
import { FollowMeTool } from '../tools/FollowMeTool'
import { OffsetTool } from '../tools/OffsetTool'
import { PaintTool, MATERIAL_SENTINEL } from '../tools/PaintTool'
import { MoveTool } from '../tools/MoveTool'
import { RotateTool } from '../tools/RotateTool'
import { ScaleTool } from '../tools/ScaleTool'
import { TapeMeasureTool } from '../tools/TapeMeasureTool'
import { ProtractorTool } from '../tools/ProtractorTool'
import { SliceTool } from '../tools/SliceTool'
import { EditVertexTool } from '../tools/EditVertexTool'
import { makeSketchPlaneCache } from '../tools/sketchGesture'
import { parseKernelErrorCode, kernelErrorMessage, friendlyErrorText } from '../kernelErrors'
import type { Ray } from './math'
import type { Snap } from '../tools/types'
import { collectLeafIds, nodeRefFromJs, structuralSelection, type NodeRef } from '../panels/treeModel'
import { MarqueeProjector, normalizedRect, type MarqueeMode, type MarqueeRect } from './marquee'
import { dragMoveTargets, exceedsDragThreshold } from './dragMove'
import { resolveSelectableRef, type ResolveDeps, type SelectScene } from '../tools/snapSelection'
import { cursorFor } from '../tools/toolIcons'
import { getResolvedTheme, subscribe as subscribeTheme } from '../settings/theme'
import { getLengthUnit, homeFramingScale } from '../settings/units'
import { InfiniteGrid } from './InfiniteGrid'
import { SketchHoverGate } from './sketchHoverGate'
import { isRenderStatsActive, recordRender } from './renderStats'
import {
  currentGpuEnvironment,
  webglUnavailableMessage,
  detectRenderProfile,
  shouldShowSoftwareNotice,
} from './gpuCapability'

/**
 * Centered message overlay shown over the viewport when the WebGL2 context is
 * unavailable or has been lost. WebKitGTK (the Linux/Tauri webview) drops the GL
 * context more readily than Chromium does — on suspend/resume or a GPU/driver
 * reset — and a dropped context otherwise leaves a frozen grey canvas with no
 * explanation. The node is absolutely positioned, so its container must be
 * `position: relative`.
 */
function buildViewportOverlay(title: string, detail: string | readonly string[]): HTMLDivElement {
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
  overlay.appendChild(h)
  const lines = typeof detail === 'string' ? [detail] : detail
  for (const line of lines) {
    const p = document.createElement('div')
    p.textContent = line
    p.style.cssText = 'font-size:13px;max-width:36em;line-height:1.4;opacity:0.8'
    overlay.appendChild(p)
  }
  return overlay
}

/**
 * One-time, non-blocking notice pinned to the top of the viewport when the
 * session is running on a software rasterizer (see gpuCapability.ts). Unlike
 * `buildViewportOverlay` this never blocks the view — modeling continues under
 * it — and it carries a Dismiss button, so the container ignores pointer
 * events while the button alone accepts them (a stray click near the top of
 * the viewport must still reach the canvas).
 */
function buildSoftwareNotice(onDismiss: () => void): HTMLDivElement {
  const notice = document.createElement('div')
  notice.className = 'viewport-software-notice'
  notice.style.cssText = [
    // Below the camera-preset button row (top-left, ~40px tall) so the
    // notice never visually covers controls.
    'position:absolute', 'top:52px', 'left:50%', 'transform:translateX(-50%)',
    'display:flex', 'align-items:center', 'gap:12px', 'padding:8px 14px',
    'max-width:calc(100% - 48px)',
    'background:var(--surface-panel, #d0d0d0)', 'color:var(--text-primary, #333)',
    'border:1px solid var(--border-strong, rgba(0,0,0,0.2))', 'border-radius:6px',
    'box-shadow:var(--shadow-chip, 0 2px 8px rgba(0,0,0,0.25))',
    'font-family:system-ui,sans-serif', 'font-size:12.5px', 'line-height:1.4',
    'z-index:9', 'pointer-events:none',
  ].join(';')
  const text = document.createElement('span')
  text.textContent = 'Running without graphics acceleration — large models will be slow.'
  notice.appendChild(text)
  const button = document.createElement('button')
  button.type = 'button'
  button.textContent = 'Dismiss'
  button.setAttribute('aria-label', 'Dismiss the graphics-acceleration notice')
  button.style.cssText = [
    'pointer-events:auto', 'cursor:pointer', 'font:inherit', 'font-weight:600',
    'background:none', 'border:none', 'padding:0', 'color:inherit',
    'text-decoration:underline', 'white-space:nowrap',
  ].join(';')
  button.addEventListener('click', onDismiss)
  notice.appendChild(button)
  return notice
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
  /** Called when the active tool's live status-bar guidance changes.
   * `null` = the tool has no stage hint; the status bar falls back to the
   * palette's static tool description. */
  onToolHint?: (hint: string | null) => void
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
  /** Fired after a SUCCESSFUL undo/redo, from the one code path every
   * entry point shares (menu, palette, and the viewport's own Cmd+Z/Cmd+
   * Shift+Z all funnel into runUndo/runRedo). Undo/redo can change state
   * that plain document changes cannot — e.g. restore a deleted tag's
   * registry entry — so the parent reconciles view state (tag visibility)
   * here rather than per entry point. */
  onHistoryChanged?: () => void
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
 *
 * The same constant also floors FREE orbit via the OrbitControls polar-angle
 * clamp (see the controls setup): near-pole poses are ill-conditioned (basis
 * roll amplifies position jitter into whole-frame shimmer), and the safe
 * margin for the baked views and for orbiting must be one value so they
 * can't drift apart.
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
  /** Combine two nodes — plain solids or whole groups (0=union,
   * 1=subtract a−b, 2=intersect). Returns the result root (a single object,
   * or a result group when the result has disjoint pieces); null on a
   * refused op (already toasted). */
  runBoolean: (op: number, a: NodeRef, b: NodeRef) => NodeRef | null
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
   * Apply a committed palette-opacity edit to the already-built scene.
   * Palette alpha is live render state, not baked geometry (the kernel's
   * `set_material_alpha` returns an empty change for the same reason), so
   * this updates the built THREE materials in place and re-renders — no
   * re-tessellation, unlike `refreshScene`. Also fires the document-changed
   * bookkeeping (docRev, dirty marking, undo-button state) a commit needs.
   */
  syncMaterialOpacity: () => void
  /**
   * True while the active tool is capturing raw keyboard input (mid-VCB entry),
   * so the global Delete/Backspace handler must not steal the key (Backspace
   * edits the typed buffer). False for non-capturing tools (e.g. Select).
   * Pass the pending key to honor a tool's per-key capture (Tool.capturesKey)
   * — Move's armed array window owns its buffer keys but never Space.
   */
  isCapturingInput: (key?: string) => boolean
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
   * Render the scene at the current camera and read the framebuffer back
   * (RGBA8, rows bottom-up per GL convention). Drives `__hew_test`
   * frame-stability probes: consecutive captures at near-identical camera
   * poses must differ only where the scene legitimately moved — a spray of
   * high-contrast per-pixel flips is depth-test instability (the edge-shimmer
   * defect). Renders synchronously because the drawing buffer is not
   * preserved after the frame is composited, so pixels must be read in the
   * same task as the draw.
   */
  captureFrame: () => { width: number; height: number; pixels: Uint8Array }

  /**
   * The camera's current pose (position, orbit target, vertical FOV) —
   * the read complement of `setCamera`, for tests that assert framing
   * (e.g. that Zoom Extents re-targeted onto a placed instance).
   */
  getCamera: () => {
    position: [number, number, number]
    target: [number, number, number]
    fovDeg: number
  }
  /**
   * Re-pose the camera at the default home view, `scale`× the meter-scale
   * distance (the welcome screen's unit choice re-frames a blank document —
   * see settings/units.ts homeFramingScale). Callers guard that the scene is
   * empty; this never inspects geometry.
   */
  setHomeFraming: (scale: number) => void
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
   * STL buffer — millimeter scale, Z-up, cylinder walls re-faceted at
   * `segmentsPerTurn` (0 = stored facets). Resolves null when the model has
   * no solids.
   */
  exportStl: (segmentsPerTurn: number) => Promise<StlBuildResult | null>
  /**
   * Serialize the current solid geometry (objects + instances) to a 3MF
   * container — millimeter unit, Z-up, one named colored mesh per part.
   * Resolves null when the model has no solids.
   */
  export3mf: () => Promise<ThreeMfBuildResult | null>
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
    // Axes are geometrically coincident with ground-sketch lines and any
    // object edge drawn along them (a box on the origin shares its vertical
    // edge with +Z); the bias — not a world-space lift — is what resolves
    // those depth ties deterministically. See depthPolicy.ts.
    polygonOffset: true,
    polygonOffsetFactor: DEPTH_BIAS.AXES,
    polygonOffsetUnits: DEPTH_BIAS.AXES,
  })
  const line = new Line2(geo, mat)
  if (dashed) line.computeLineDistances()
  line.renderOrder = 1 // draw over the grid plane
  // Metadata for the per-frame camera-plane clamp (`clampOriginAxes`): the
  // half's nominal far endpoint (`from` is always the origin) and the last
  // written clamp params so unclamped frames cost nothing.
  line.userData.axisEnd = to
  line.userData.clampT = [0, 1]
  // The clamp rewrites endpoints in place; skip the stale computed bounds
  // rather than recomputing them per frame (the axes are essentially always
  // in view anyway — they span the whole world).
  line.frustumCulled = false
  return line
}

/**
 * Clip every axis half's rendered segment to a slightly enlarged view
 * frustum, in float64 — called once per rendered frame (camera changes only
 * reach the GPU through a render).
 *
 * Why: each half is a single 150 m fat-line segment, and `LineMaterial`
 * handles extreme segments badly in two distinct ways, both measured by the
 * line-stability probes (`edge-stability.spec.ts`):
 *
 *  1. Whenever a half extends behind the camera (orbit low over the ground
 *     and -Y is behind you; look down at a model and +Z is), the vertex
 *     shader trims the segment at the near plane — and that trim is
 *     catastrophically noisy in float32: the trimmed endpoint lands at
 *     w ≈ near, where the perspective division amplifies the rounding noise
 *     of the 150 m-magnitude view-space coordinates ~100×, so the whole
 *     on-screen quad wobbles by a few tenths of a pixel per repaint. During
 *     an orbit's damping tail that reads as the axis shimmering — worst
 *     where it overlays high-contrast coincident linework (the blue +Z axis
 *     sharing a cube's vertical edge). Measured: ~600 hard pixel flips per
 *     sub-pixel repaint at a 34 m pose, all tracing trimmed halves; 0 on
 *     untrimmed ones.
 *
 *  2. Even with the near-plane case handled, an endpoint that projects far
 *     outside the viewport (tens of thousands of pixels, for a half receding
 *     toward the camera plane) makes the rasterizer interpolate depth and
 *     dash-distance across a gigantic quad of which the screen shows a tiny
 *     parameter sliver — imprecisely enough that the depth-bias ladder
 *     (depthPolicy.ts, a few depth quanta) drowns: an axis over a coincident
 *     model edge stayed nondeterministic (~200 hard flips) until the bias
 *     was cranked to hundreds of quanta, which is no longer a hairline.
 *
 * Fix for both: clip each half here, in float64, to a modestly enlarged view
 * frustum (near plane at a distance-scaled margin, side planes pushed out
 * FRUSTUM_SLACK×), every rendered frame. The shader then never trims, and
 * every endpoint it sees projects within ~1.5 screens, so its float32
 * interpolation is exact to well under one depth quantum and the ladder's
 * single-digit biases resolve ties deterministically. The clipped-away
 * portions are off-screen by construction; the dash phase stays anchored at
 * the origin because the distance attributes are rewritten to the clipped
 * parameter range.
 */
const FRUSTUM_SLACK = 1.5
function clampOriginAxes(group: THREE.Group, camera: THREE.PerspectiveCamera): void {
  // View transform in float64: three stores matrix elements and camera pose
  // as JS numbers, so composing the two matrix-vector products here (rather
  // than in the f32 vertex shader) is what buys the precision. Recompute the
  // inverse from the camera's current pose — `camera.matrixWorldInverse` is
  // only refreshed by `renderer.render`, i.e. it still holds LAST frame's
  // pose here, and a stale view would misclip the very frame captured right
  // after a programmatic `setCamera` jump.
  camera.updateMatrixWorld()
  const m = _axisView.copy(camera.matrixWorld).invert().elements
  // View-space position of the world origin (the shared start of every half).
  const ax = m[12]
  const ay = m[13]
  const az = m[14]
  // Near margin: comfortably past the near plane, growing with camera
  // distance so the float noise floor (ulps of camera/axis coordinate
  // magnitudes) stays orders of magnitude below one depth/pixel quantum at
  // every scale.
  const margin = Math.max(4 * camera.near, 0.02 * camera.position.length())
  const tanV = Math.tan((camera.fov * Math.PI) / 360) * FRUSTUM_SLACK
  const tanH = tanV * camera.aspect

  for (const child of group.children) {
    if (!(child instanceof Line2)) continue
    const end = child.userData.axisEnd as [number, number, number]
    // View-space direction origin→end (rotation part only — `end` is a
    // position but the origin's translation cancels in the difference).
    const bx = m[0] * end[0] + m[4] * end[1] + m[8] * end[2]
    const by = m[1] * end[0] + m[5] * end[1] + m[9] * end[2]
    const bz = m[2] * end[0] + m[6] * end[1] + m[10] * end[2]

    // Clip the parameter range [t0, t1] of origin→end against five planes,
    // each linear in t (view space: camera at 0 looking down -z, so the
    // depth in front is -z):
    //   depth:  -z(t) ≥ margin
    //   sides:  |x(t)| ≤ tanH·(-z(t)),  |y(t)| ≤ tanV·(-z(t))
    let t0 = 0
    let t1 = 1
    // Each constraint as g(t) = c + d·t ≥ 0.
    const planes: Array<[number, number]> = [
      [-az - margin, -bz],
      [-az * tanH - ax, -bz * tanH - bx],
      [-az * tanH + ax, -bz * tanH + bx],
      [-az * tanV - ay, -bz * tanV - by],
      [-az * tanV + ay, -bz * tanV + by],
    ]
    for (const [c, d] of planes) {
      if (d === 0) {
        if (c < 0) t0 = t1 = 0 // wholly outside this plane — degenerate
      } else {
        const tc = -c / d
        if (d > 0) {
          if (tc > t0) t0 = tc
        } else if (tc < t1) {
          t1 = tc
        }
      }
    }
    if (t0 >= t1) t0 = t1 = 0 // no visible span — collapse (nothing drawn)

    const cached = child.userData.clampT as [number, number]
    if (cached[0] === t0 && cached[1] === t1) continue
    cached[0] = t0
    cached[1] = t1

    // Rewrite the single segment instance in place (instanceStart/End share
    // one interleaved buffer: [sx,sy,sz,ex,ey,ez]).
    const geo = child.geometry
    const posAttr = geo.attributes.instanceStart as THREE.InterleavedBufferAttribute
    const arr = posAttr.data.array as Float32Array
    arr[0] = end[0] * t0
    arr[1] = end[1] * t0
    arr[2] = end[2] * t0
    arr[3] = end[0] * t1
    arr[4] = end[1] * t1
    arr[5] = end[2] * t1
    posAttr.data.needsUpdate = true

    // Keep dashes world-anchored at the origin: distances are the clamped
    // parameter range scaled by the half's full length.
    const distAttr = geo.attributes.instanceDistanceStart as THREE.InterleavedBufferAttribute | undefined
    if (distAttr !== undefined) {
      const len = Math.hypot(end[0], end[1], end[2])
      const darr = distAttr.data.array as Float32Array
      darr[0] = t0 * len
      darr[1] = t1 * len
      distAttr.data.needsUpdate = true
    }
  }
}
const _axisView = new THREE.Matrix4()

function buildOriginAxes(theme: 'light' | 'dark'): THREE.Group {
  const group = new THREE.Group()
  group.name = 'OriginAxes'

  const L = 150
  // Exactly at Z=0 — the axes must be geometrically coplanar with the ground
  // grid and ground sketches at every zoom (a former +0.002 world-space lift
  // read as the axes floating above a cm-scale sketch). The grid is a
  // non-depth-writing backdrop, so there is nothing to z-fight; coincident
  // lines are settled by the depth-bias ladder instead (depthPolicy.ts).
  const { x: xc, y: yc, z: zc } = ORIGIN_AXIS_COLORS[theme]

  // X (red): solid +X, dashed -X
  group.add(buildAxisLine([0, 0, 0], [L, 0, 0], xc, false))
  group.add(buildAxisLine([0, 0, 0], [-L, 0, 0], xc, true))
  // Y (green): solid +Y, dashed -Y
  group.add(buildAxisLine([0, 0, 0], [0, L, 0], yc, false))
  group.add(buildAxisLine([0, 0, 0], [0, -L, 0], yc, true))
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
  onToolHint,
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
  onHistoryChanged,
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
  const onToolHintRef = useRef(onToolHint)
  onToolHintRef.current = onToolHint
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
  const onHistoryChangedRef = useRef(onHistoryChanged)
  onHistoryChangedRef.current = onHistoryChanged
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
  // Polygon's last-used side count — session-lived across tool re-selection
  // (design §1: "the last-used side count persists for the session"),
  // mirroring how currentMaterialIdRef persists Paint's material.
  const polygonSidesRef = useRef<number>(DEFAULT_POLYGON_SIDES)
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
  // The drawing-plane cue layer is created inside the setup effect's closure
  // (keyed to `wasmScene`); this ref lets the separate activeContext-change
  // effect below reach it too, to clear a stale cue on context change
  // (Blocker 3, the sketch-planes design §6 bullet 1).
  const drawPlaneCueLayerRef = useRef<DrawPlaneCueLayer | null>(null)

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
    // GPU triage first (gpuCapability.ts): one throwaway probe context decides
    // hardware vs software GL, which picks the constructor's antialias flag and
    // the pixel-ratio cap — on llvmpipe/SwiftShader every fragment is CPU-shaded,
    // so both are direct fill-rate wins.
    //
    // WebGL2 context creation can then still fail outright — WebKitGTK with no
    // GPU path, or Chrome 137+ on a machine with acceleration off (Chrome removed
    // its software fallback). three throws in that case; catch it and show
    // environment-specific guidance instead of an unhandled error + blank grey
    // panel, then bail out of setup.
    const gpuProfile = detectRenderProfile()
    let renderer: THREE.WebGLRenderer
    try {
      renderer = new THREE.WebGLRenderer({ antialias: gpuProfile.antialias })
    } catch (err) {
      console.error('[viewport] WebGL2 renderer creation failed:', err)
      const message = webglUnavailableMessage(currentGpuEnvironment())
      el.appendChild(buildViewportOverlay(message.title, message.lines))
      return () => {
        el.style.position = ''
        el.replaceChildren()
      }
    }
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, gpuProfile.maxPixelRatio))
    // Software rasterizer: say so once (ever), quietly — the degrade above is
    // already active; the notice only sets expectations for large models.
    let softwareNotice: HTMLDivElement | null = null
    if (gpuProfile.software) {
      console.info(
        `[viewport] software WebGL detected (${gpuProfile.rendererString || 'renderer string unavailable'})` +
          ' — antialias off, pixel ratio capped at 1',
      )
      if (shouldShowSoftwareNotice()) {
        softwareNotice = buildSoftwareNotice(() => {
          softwareNotice?.remove()
          softwareNotice = null
        })
        el.appendChild(softwareNotice)
      }
    }
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
    // Scaled down for small-scale display units (cm/mm/inches imply a small
    // model — see homeFramingScale); the direction is always the same 3/4 view.
    const homeScale = homeFramingScale(getLengthUnit())
    camera.position.set(3.5 * homeScale, -3.0 * homeScale, 2.5 * homeScale)
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

    // Drawing-plane cue (sketches on any plane, Phase 4 — design §6 bullet
    // 1): a subtle grid patch on a draw tool's active non-ground plane.
    // Purely visual — see DrawPlaneCueLayer.ts's module doc.
    const drawPlaneCueLayer = new DrawPlaneCueLayer()
    threeScene.add(drawPlaneCueLayer.group)
    drawPlaneCueLayerRef.current = drawPlaneCueLayer

    // Duck-typed like `snapConstraint` — only the four draw tools implement
    // this. Called after the active tool's own state update (`onPointerMove`
    // / `onKey`) so a just-changed idle-lock or hover is reflected.
    function queryDrawPlaneCue(tool: object): { plane: DrawPlane; through: [number, number, number] } | null {
      return 'activeDrawPlaneCue' in tool
        ? (tool as { activeDrawPlaneCue(): { plane: DrawPlane; through: [number, number, number] } | null }).activeDrawPlaneCue()
        : null
    }

    // Preview group shared by tools
    const previewGroup = new THREE.Group()
    previewGroup.name = 'Preview'
    threeScene.add(previewGroup)

    // ------------------------------------------------------------------ snap + tool
    const snapService = new SnapService(wasmScene)

    // Scratch vector reused for the camera-forward axis (avoids per-pick alloc).
    const cameraForwardV = new THREE.Vector3()

    /** Everything the shared selection resolver needs from this Viewport: the
     * scene pickers, the active editing context, an object→node resolver
     * (context-scoped + hidden-filtered), and the live camera forward/far for
     * the axial depth bound. Rebuilt per pick so it always reads live state. */
    function selectionDeps(): ResolveDeps {
      camera.getWorldDirection(cameraForwardV)
      return {
        scene: wasmScene as unknown as SelectScene,
        context: activeContextRef.current,
        resolveObject: (objectId, instanceId) => {
          if (instanceId !== undefined && hiddenInstanceIdsRef.current.has(instanceId)) return null
          if (hiddenObjectIdsRef.current.has(objectId)) return null
          return resolvePickToSelectable(wasmScene, objectId, activeContextRef.current, instanceId)
        },
        cameraForward: [cameraForwardV.x, cameraForwardV.y, cameraForwardV.z],
        cameraFar: camera.far,
      }
    }

    // The Select click: resolve the snap+ray to a selectable node through the
    // SAME shared resolver the drag-move arm uses (`pickTransformableUnderCursor`),
    // so click, drag, and hover agree by construction. `null` means nothing
    // selectable is under the cursor — clear (context-scoped: `additive` is
    // false inside a context, so an in-context miss deselects without exiting).
    function handleSelect(snap: Snap | null, ray: Ray): void {
      const additive = selectAdditiveRef.current && activeContextRef.current.length === 0
      const ref = resolveSelectableRef(snap, ray, selectionDeps())
      onSelectRef.current?.(ref, additive)
      scheduleRender()
    }

    const toolController = new ToolController(wasmScene, handleSelect)
    toolControllerRef.current = toolController

    // Live status-bar guidance: re-poll the active tool's stage hint after
    // every routed event (the wrapped listeners below) and on tool switches,
    // pushing CHANGES up — a string compare keeps the per-move cost trivial.
    let lastToolHint: string | null = null
    function reportToolHint(): void {
      // Camera tools (Orbit/Pan/Zoom) park the controller on Select while
      // OrbitControls owns the left button — left-clicks navigate, they
      // don't select — so Select's hint would mislabel them. Report null
      // and let the status bar fall back to the camera tool's static
      // description. cameraModeRef is set BEFORE resetToSelect() fires the
      // tool-change listener, so this reads the new mode.
      const hint = cameraModeRef.current
        ? null
        : (toolController.activeTool.statusHint?.() ?? null)
      if (hint !== lastToolHint) {
        lastToolHint = hint
        onToolHintRef.current?.(hint)
      }
    }
    toolController.onToolChange(() => reportToolHint())
    reportToolHint()

    // ONE plane-keyed sketch cache shared by every draw tool (Line/
    // Rectangle/Circle/Arc), surviving tool switches, so mixed-tool profiles
    // drawn on the SAME plane — an arc closed by a Line chord, a rectangle
    // meeting an arc — land in the same sketch and can close regions.
    // Cleared (every plane's handle) when a new document replaces the Scene.
    const sketchPlaneCache = makeSketchPlaneCache()

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

    /**
     * The transformable node under `ray` — the drag-move arm and the transform
     * tools' auto-select. Reduces to the SAME shared resolver the Select click
     * uses (`resolveSelectableRef` via `selectionDeps`), so hover, click, and
     * drag can never diverge: a region's fill drags the region (never the solid
     * behind it), a sketch edge drags the shape the click selects (the
     * transform layer lifts it to its island), a solid drags its selectable
     * ancestor, and a provenance-less/out-of-context snap resolves what is
     * actually under the ray — a far-plane-bounded, context-scoped solid.
     * Null when nothing movable is under the cursor.
     */
    function pickTransformableUnderCursor(ray: Ray): NodeRef | null {
      const { snap } = snapService.resolve(ray, el.clientHeight, camera.fov)
      return resolveSelectableRef(snap, ray, selectionDeps())
    }

    /**
     * Auto-select for the transform tools (Move/Rotate/Scale): with an empty
     * selection, their first click picks whatever is under the cursor,
     * lifts it into the app selection (highlight + dock follow), and returns
     * it so the gesture proceeds on it immediately — selecting and moving is
     * one fluid motion, not a two-step Select-then-Move.
     */
    function acquireTransformTargets(ray: Ray): NodeRef[] | null {
      const node = pickTransformableUnderCursor(ray)
      if (node === null) return null
      onSelectRef.current?.(node, false)
      scheduleRender()
      return [node]
    }

    /**
     * "Plain objects are immediately editable": may a draw tool
     * (Line/Rectangle/Circle/Polygon/Arc) draw directly on this picked face?
     * Injected into the draw tools (which only know an entered-object id)
     * because the answer depends on the full context path:
     *
     * - Inside an entered object: only that object's faces.
     * - Inside an entered component instance: that instance's member faces.
     * - Top level / inside a group: yes iff the pick RESOLVES to the plain
     *   object itself — i.e. the object is not wrapped by a group or
     *   instance at this level. Groups and Components keep their explicit
     *   double-click editing step.
     */
    function faceDrawEligible(objectId: bigint, instanceId: bigint | undefined): boolean {
      const ctx = activeContextRef.current
      const deepest = ctx.length > 0 ? ctx[ctx.length - 1] : null
      if (deepest?.kind === 'object') {
        return instanceId === undefined && objectId === deepest.id
      }
      if (deepest?.kind === 'instance') {
        return instanceId === deepest.id
      }
      const resolved = resolvePickToSelectable(wasmScene, objectId, ctx, instanceId)
      return resolved !== null && resolved.kind === 'object' && resolved.id === objectId
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

    // Drag-to-move (Select tool): a press on a movable node arms this instead
    // of the marquee. Past the drag threshold the gesture is handed to a
    // one-shot Move tool (`beginDragMove`); a sub-threshold release is still
    // a plain click (top level defers it to pointerup exactly like the
    // marquee path; in-context the press already click-picked). The tool
    // SPRINGS BACK to Select on release — matching OS drag muscle memory —
    // so the tool rail never leaves Select.
    interface DragMove {
      startX: number
      startY: number
      /** The ray of the original press — the Move base point on activation. */
      pressRay: Ray
      /** What the drag moves: the whole selection when the pressed node was
       * already part of it, else just the pressed node (OS convention). */
      nodes: NodeRef[]
      active: boolean
      /** Top-level presses defer their click-pick to pointerup (mirrors the
       * marquee); in-context presses already dispatched it. */
      deferClick: boolean
    }
    let dragMove: DragMove | null = null

    /** Abandon an armed/active drag-move (Esc, focus loss, pointercancel). */
    function abortDragMove(): void {
      if (dragMove === null) return
      if (dragMove.active) {
        toolController.activeTool.cancel()
        switchToolRef.current?.('Select')
      }
      dragMove = null
    }

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

    // Apply a committed palette-opacity edit in place (ViewportApi.
    // syncMaterialOpacity) — no re-tessellation; alpha lives on the
    // already-built THREE materials.
    function syncMaterialOpacity(): void {
      sceneRenderer.syncPaletteOpacity()
      onDocumentChangedRef.current?.()
      scheduleRender()
    }

    function handleToast(message: string, code?: string): void {
      onToastRef.current?.(message, code)
    }

    // Imperative command surface for the parent.
    function runBoolean(op: number, a: NodeRef, b: NodeRef): NodeRef | null {
      // Operands are plain solids or whole groups; the kernel composes group
      // operands and owns every eligibility rule (boolean_nodes,
      // the group-ops design). kind: 0=object, 1=group — the same
      // mapping as runGroup; instances are refused typed by the kernel.
      const kindNum = (n: NodeRef) => (n.kind === 'group' ? 1 : n.kind === 'instance' ? 2 : 0)
      let result: NodeRef
      try {
        result = nodeRefFromJs(
          wasmScene.boolean_nodes(op, kindNum(a), a.id, kindNum(b), b.id),
        )
      } catch (err) {
        const code = parseKernelErrorCode(err)
        const rawMsg = err instanceof Error ? err.message : String(err)
        handleToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
        return null
      }
      handleSceneRefresh()
      sceneRenderer.refreshAllSketches()
      sceneRenderer.refreshGuides()
      onSelectRef.current?.(result, false)
      scheduleRender()
      return result
    }

    function runGroup(nodes: NodeRef[]): bigint | null {
      if (nodes.length === 0) return null
      // Id-space boundary: only nodes with a kernel NodeId may cross into
      // group_nodes' kind/id arrays. A sketch-scoped ref refuses here with a
      // typed toast — its id lives in a different slotmap, and slotmaps reuse
      // bit patterns, so collapsing it to kind 0 could silently mutate an
      // unrelated live object (see structuralSelection in treeModel.ts).
      const sel = structuralSelection(nodes)
      if (sel === null) {
        handleToast(kernelErrorMessage('InvalidSelection', ''), 'InvalidSelection')
        return null
      }
      try {
        const groupId = wasmScene.group_nodes(sel.kinds, sel.ids)
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

    /**
     * Quietly close Move's armed ×N / /N window, if any. Explicit document
     * commands — delete, undo, redo — are deliberate and must execute, but
     * they end the refinement: without this, the window's keyboard capture
     * outlives it (Delete silently no-ops, bare-letter tool shortcuts feed
     * a stale VCB buffer until Esc). Only the ambiguous bare
     * Delete/Backspace KEYSTROKE stays guarded upstream by capturingInput —
     * see MoveTool.capturingInput / disarmArray.
     */
    function disarmActiveArrayWindow(): void {
      const activeTool = toolController.activeTool
      if ('disarmArray' in activeTool) {
        (activeTool as { disarmArray(): void }).disarmArray()
      }
    }

    function runDelete(nodes: NodeRef[]): void {
      if (nodes.length === 0) return
      // An explicit delete is deliberate and executes — but first disarm the
      // array window so no hot state points at the deleted copies.
      disarmActiveArrayWindow()
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
            removeEdgeBatch(n.sketch, Array.from(wasmScene.sketch_curve_chain(n.sketch, n.id)))
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
            // sketch_ids: a sketch already removed (e.g. wholly consumed by
            // an extrusion) must not be tombstoned twice.
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
      // Same id-space boundary as runGroup: a sketch-scoped ref must never
      // collapse into make_component's node-id arrays (typed refusal, never a
      // kind-0 fallback that could alias an unrelated live object).
      const sel = structuralSelection(nodes)
      if (sel === null) {
        handleToast(kernelErrorMessage('InvalidSelection', ''), 'InvalidSelection')
        return null
      }
      try {
        const instanceId = wasmScene.make_component(sel.kinds, sel.ids)
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
      // A new/loaded document replaced the Scene — every plane's cached
      // sketch handle, and any handle the active tool cached itself, is now
      // stale. Re-selecting the same tool doesn't recreate it, so reset
      // explicitly.
      sketchPlaneCache.clear()
      const at = toolController.activeTool
      if ('onDocumentReset' in at) {
        (at as { onDocumentReset(): void }).onDocumentReset()
      }
      // The reset silently rewound the tool to idle (and cleared any idle
      // plane lock) — the drawing-plane cue, if any was showing, no longer
      // applies (design §6 bullet 1).
      drawPlaneCueLayer.clear()
      // The reset silently rewound the tool to idle — without a re-poll the
      // status bar would keep the mid-gesture hint until the next mouse move.
      reportToolHint()
      handleSceneRefresh()
      sceneRenderer.refreshAllSketches()
      sceneRenderer.refreshGuides()
    }

    /**
     * Refresh policy for an undo/redo step: the kernel's DocChange names
     * exactly what the step touched, so rebuild only those scene nodes.
     * A touched group can restructure arbitrarily many leaves (visibility
     * cascades, membership), so any group falls back to the full rebuild.
     * Sketch overlays refresh when a sketch OR an object changed — consumed
     * regions derive from live object footprints, so an object-only change
     * can still reshape a sketch's extrudable regions. Palette opacity is
     * live render state the kernel deliberately reports as an empty change
     * (never baked into geometry), so re-sync it unconditionally; it is a
     * cheap walk over already-built materials.
     */
    function applyHistoryChange(change: DocChangeJs): void {
      try {
        if (change.groups_touched().length > 0) {
          handleSceneRefresh()
        } else {
          handleSceneRefresh({
            objectIds: Array.from(change.objects_touched()),
            instanceIds: Array.from(change.instances_touched()),
            componentIds: Array.from(change.components_touched()),
          })
        }
        if (change.sketches_touched().length > 0 || change.objects_touched().length > 0) {
          sceneRenderer.refreshAllSketches()
        }
        if (change.guides_touched().length > 0) {
          sceneRenderer.refreshGuides()
        }
      } finally {
        change.free()
      }
      sceneRenderer.syncPaletteOpacity()
    }

    // The shared undo/redo choke point: the Edit menu and command palette
    // (via App.handleUndo/handleRedo → ViewportApi) and this component's own
    // Cmd+Z / Cmd+Shift+Z keydown all land here, so post-history
    // reconciliation (onHistoryChanged) fires for EVERY entry point instead
    // of being duplicated per caller.
    function runUndo(): void {
      if (wasmSceneRef.current.can_scene_undo()) {
        // As explicit as menu delete: the undo executes AND ends the armed
        // array window (the generation guard already prevented any
        // wrong-action harm; this releases the window's keyboard capture).
        disarmActiveArrayWindow()
        try {
          applyHistoryChange(wasmSceneRef.current.scene_undo())
          onHistoryChangedRef.current?.()
        } catch (err) {
          console.warn('[Viewport] scene_undo failed:', err)
        }
      }
    }

    function runRedo(): void {
      if (wasmSceneRef.current.can_scene_redo()) {
        // Mirror runUndo — see disarmActiveArrayWindow.
        disarmActiveArrayWindow()
        try {
          applyHistoryChange(wasmSceneRef.current.scene_redo())
          onHistoryChangedRef.current?.()
        } catch (err) {
          console.warn('[Viewport] scene_redo failed:', err)
        }
      }
    }

    function zoomExtents(): void {
      // Compute the world bounding box over all rendered model geometry:
      // objects, instances, AND sketches — a document that is only a drawn
      // rectangle must frame correctly too. Guides are deliberately
      // excluded: they are reference geometry, and a long construction line
      // would blow the framing out past the model it references. Hidden
      // geometry (eye/tag hides flip wrapper-group `.visible`, which
      // Box3.expandByObject ignores) is excluded too — Zoom Extents frames
      // every VISIBLE thing (learn/viewing.md), not invisible solids.
      const box = new THREE.Box3()
      expandByVisibleObject(box, sceneRenderer.objectsGroup)
      expandByVisibleObject(box, sceneRenderer.instancesGroup)
      expandByVisibleObject(box, sceneRenderer.sketchGroup)
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

      // Re-frame the scene each time (like zoomExtents, same group set —
      // sketches count as model geometry, guides stay excluded, hidden
      // geometry is skipped), falling back to the current target/distance
      // when nothing is visible.
      const box = new THREE.Box3()
      expandByVisibleObject(box, sceneRenderer.objectsGroup)
      expandByVisibleObject(box, sceneRenderer.instancesGroup)
      expandByVisibleObject(box, sceneRenderer.sketchGroup)

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

    function captureFrame(): { width: number; height: number; pixels: Uint8Array } {
      // Mirror the per-frame camera-dependent updates of the animation loop
      // (this renders out-of-band, without going through it) so a captured
      // frame is exactly what the loop would put on screen for this pose.
      infiniteGrid.update(camera.position)
      clampOriginAxes(originAxes, camera)
      renderer.render(threeScene, camera)
      const gl = renderer.getContext()
      const width = gl.drawingBufferWidth
      const height = gl.drawingBufferHeight
      const pixels = new Uint8Array(width * height * 4)
      gl.readPixels(0, 0, width, height, gl.RGBA, gl.UNSIGNED_BYTE, pixels)
      return { width, height, pixels }
    }

    function getCamera(): {
      position: [number, number, number]
      target: [number, number, number]
      fovDeg: number
    } {
      return {
        position: [camera.position.x, camera.position.y, camera.position.z],
        target: [controls.target.x, controls.target.y, controls.target.z],
        fovDeg: camera.fov,
      }
    }

    function setHomeFraming(scale: number): void {
      // Re-pose the camera at the default 3/4 home view, `scale`× the
      // meter-scale distance (welcome-screen unit choice on a blank
      // document). Same direction and target as the mount-time default.
      camera.position.set(3.5 * scale, -3.0 * scale, 2.5 * scale)
      controls.target.set(0, 0, 0)
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
        handleToast(friendlyErrorText(err))
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
        handleToast(friendlyErrorText(err))
        return
      }
      sceneRenderer.refreshGuides()
      onDocumentChangedRef.current?.()
      scheduleRender()
    }

    async function exportGlb(): Promise<Uint8Array | null> {
      return exportSceneToGlb(sceneRenderer)
    }

    async function exportStl(segmentsPerTurn: number): Promise<StlBuildResult | null> {
      // Kernel-sourced: the wasm scene serves export tessellation directly
      // (re-faceted true curves); the three.js scene is not involved.
      return exportSceneToStl(wasmScene, segmentsPerTurn)
    }

    async function export3mf(): Promise<ThreeMfBuildResult | null> {
      return exportSceneTo3mf(sceneRenderer, wasmSceneRef.current)
    }

    if (apiRefRef.current !== undefined) {
      const isCapturingInput = (key?: string): boolean => {
        const t = toolController.activeTool
        // With a key, honor a tool's per-key capture (Tool.capturesKey) so
        // App-level shortcut gates (Space→Select, Delete/Backspace) agree
        // with the Viewport's own routing about which keys the tool owns.
        if (key !== undefined && 'capturesKey' in t) {
          return (t as { capturesKey(key: string): boolean }).capturesKey(key)
        }
        return 'capturingInput' in t && (t as { capturingInput(): boolean }).capturingInput()
      }
      apiRefRef.current.current = { runBoolean, runGroup, runUngroup, runDelete, runMakeComponent, runPlaceInstance, runExplodeInstance, runMakeUnique, notifyLoaded, refreshScene, syncMaterialOpacity, isCapturingInput, runUndo, runRedo, zoomExtents, setStandardView, setCamera, captureFrame, getCamera, setHomeFraming, setHidden, selectAll, setAxesVisible, setGridVisible, setGuidesVisible, deleteAllGuides, runDeleteGuide, exportGlb, exportStl, export3mf }
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
        sketchPlaneCache,
      )
      // Scope the tool to the current editing context, if any.
      const ctx = activeContextRef.current
      const ctxId = ctx.length > 0 && ctx[ctx.length - 1].kind === 'object'
        ? ctx[ctx.length - 1].id : null
      tool.setActiveContext(ctxId)
      // Plain objects are directly drawable — context-path-aware eligibility.
      tool.setFaceEligibility(faceDrawEligible)
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
        sketchPlaneCache,
      )
      // Scope the tool to the current editing context, if any.
      const ctx = activeContextRef.current
      const ctxId = ctx.length > 0 && ctx[ctx.length - 1].kind === 'object'
        ? ctx[ctx.length - 1].id : null
      tool.setActiveContext(ctxId)
      // Plain objects are directly drawable — context-path-aware eligibility.
      tool.setFaceEligibility(faceDrawEligible)
      return tool
    }

    function makePolygonTool(): PolygonTool {
      const tool = new PolygonTool(
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
        sketchPlaneCache,
        // Side count persists across tool re-selection for the session
        // (design §1), the same way Paint's current material does.
        (sides) => { polygonSidesRef.current = sides },
      )
      tool.setSideCount(polygonSidesRef.current)
      // Scope the tool to the current editing context, if any.
      const ctx = activeContextRef.current
      const ctxId = ctx.length > 0 && ctx[ctx.length - 1].kind === 'object'
        ? ctx[ctx.length - 1].id : null
      tool.setActiveContext(ctxId)
      // Plain objects are directly drawable — context-path-aware eligibility.
      tool.setFaceEligibility(faceDrawEligible)
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
        sketchPlaneCache,
      )
      // Scope the tool to the current editing context, if any.
      const ctx = activeContextRef.current
      const ctxId = ctx.length > 0 && ctx[ctx.length - 1].kind === 'object'
        ? ctx[ctx.length - 1].id : null
      tool.setActiveContext(ctxId)
      // Plain objects are directly drawable — context-path-aware eligibility.
      tool.setFaceEligibility(faceDrawEligible)
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
        sketchPlaneCache,
      )
      // Scope the tool to the current editing context, if any.
      const ctx = activeContextRef.current
      const ctxId = ctx.length > 0 && ctx[ctx.length - 1].kind === 'object'
        ? ctx[ctx.length - 1].id : null
      tool.setActiveContext(ctxId)
      // Plain objects are directly drawable — context-path-aware eligibility.
      tool.setFaceEligibility(faceDrawEligible)
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
      // Same context-path-aware eligibility as the draw tools: plain objects
      // are directly push/pullable; group/instance members only from inside
      // their container's editing context.
      tool.setFaceEligibility(faceDrawEligible)
      // The two id channels above only carry object/instance contexts; a
      // GROUP context leaves both null, so tell the tool it is scoped
      // explicitly (drives the refusal hint's wording only).
      tool.setContextScoped(ctx.length > 0)
      return tool
    }

    function makeFollowMeTool(): FollowMeTool {
      const tool = new FollowMeTool(
        wasmScene,
        previewGroup,
        // A sweep births one new object and consumes its profile sketch's
        // outline; refresh the object plus all sketch line buffers, then
        // select the result so the highlight lands on the new solid.
        (objectId) => {
          handleSceneRefresh({ objectIds: [objectId] })
          sceneRenderer.refreshAllSketches()
          onSelectRef.current?.({ kind: 'object', id: objectId }, false)
        },
        handleToast,
        // The path may be preselected (SketchUp's primary Follow Me idiom).
        [...selectedIdsRef.current],
      )
      // Put Follow Me's FACE path on the same face-eligibility system as every
      // other face tool: `face_boundary`/`follow_me_around_face` are
      // coordinate-correct only for a plain, top-level, non-instanced object,
      // and there is no `follow_me_in_component` surface, so no component
      // context is wired (an instanced/in-context face is refused, not swept).
      const ctx = activeContextRef.current
      const ctxId = ctx.length > 0 && ctx[ctx.length - 1].kind === 'object'
        ? ctx[ctx.length - 1].id : null
      tool.setActiveContext(ctxId)
      tool.setFaceEligibility(faceDrawEligible)
      tool.setContextScoped(ctx.length > 0)
      return tool
    }

    function makeOffsetTool(): OffsetTool {
      const tool = new OffsetTool(
        wasmScene,
        previewGroup,
        // Region offset: new sketch geometry — rebuild sketch lines/fills.
        () => {
          sceneRenderer.refreshAllSketches()
          sceneRenderer.refreshGuides()
          onDocumentChangedRef.current?.()
          scheduleRender()
        },
        handleToast,
        // Face offset: an imprint on one object — targeted refresh.
        (objectId) => {
          handleSceneRefresh({ objectIds: [objectId] })
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

    function makeMoveTool(selection?: NodeRef[]): MoveTool {
      const tool = new MoveTool(
        wasmScene,
        previewGroup,
        sceneRenderer.objectsGroup,
        selection ?? [...selectedIdsRef.current],
        (nodes) => {
          handleSceneRefresh(touchedForNodes(nodes))
          // A sketch move bakes new vertex positions; rebuild sketch buffers so
          // the lines follow (objects refresh via handleSceneRefresh; sketches
          // do not). Mirrors the boolean/undo refresh pairing.
          sceneRenderer.refreshAllSketches()
          // Select the committed nodes — for a copy these are the fresh
          // clones, so a follow-up move chains off the new copies.
          if (nodes.length === 1) onSelectRef.current?.(nodes[0], false)
          else onSelectManyRef.current?.(nodes, false)
        },
        handleToast,
        (text: string) => { onMeasurementRef.current?.(text) },
        (id: bigint) => sceneRenderer.getInstanceGroup(id),
        // Durable copy toggle → badge the Move cursor with a `+` (the same
        // cursorFor pipeline the tool-switch cursor uses).
        (on: boolean) => {
          renderer.domElement.style.cursor = cursorFor('Move', on)
        },
        // ×N / /N array re-resolve: the previous copies were scene-undone
        // before the new set landed, so a targeted refresh isn't enough —
        // rebuild fully so the retracted meshes vanish too.
        (nodes) => {
          handleSceneRefresh()
          sceneRenderer.refreshAllSketches()
          if (nodes.length === 1) onSelectRef.current?.(nodes[0], false)
          else onSelectManyRef.current?.(nodes, false)
        },
      )
      tool.setSelectionAcquirer(acquireTransformTargets)
      return tool
    }

    /**
     * Threshold crossed on a Select-tool drag that started on a movable node:
     * hand the rest of the gesture to a one-shot Move tool. The press point
     * becomes the Move base point (snapped through the same resolve a Move
     * click gets), so the drag continues seamlessly with full inference,
     * axis locks, Alt-copy, and VCB entry; pointerup commits (see
     * onPointerUp) and the tool springs back to Select.
     */
    function beginDragMove(dm: DragMove): void {
      // Select what's about to move so the highlight + dock follow the drag.
      if (dm.nodes.length === 1) onSelectRef.current?.(dm.nodes[0], false)
      else onSelectManyRef.current?.(dm.nodes, false)
      const tool = makeMoveTool(dm.nodes)
      toolController.setTool(tool)
      renderer.domElement.style.cursor = cursorFor('Move')
      const { snap } = snapService.resolve(dm.pressRay, el.clientHeight, camera.fov)
      tool.onPointerDown(snap, dm.pressRay)
      scheduleRender()
    }

    function makeRotateTool(): RotateTool {
      const tool = new RotateTool(
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
      tool.setSelectionAcquirer(acquireTransformTargets)
      return tool
    }

    function makeScaleTool(): ScaleTool {
      const tool = new ScaleTool(
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
      tool.setSelectionAcquirer(acquireTransformTargets)
      return tool
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
        case 'Polygon':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          toolController.setTool(makePolygonTool())
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
        case 'Follow Me':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          toolController.setTool(makeFollowMeTool())
          break
        case 'Offset':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          toolController.setTool(makeOffsetTool())
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
      // pointer. The canvas owns its cursor — the only writers besides this
      // switch are the shift-pan swap and MoveTool's copy-toggle badge
      // (makeMoveTool), both routed through the same cursorFor pipeline.
      renderer.domElement.style.cursor = cursorFor(toolName)
      // The outgoing tool's drawing-plane cue (if any) no longer applies —
      // don't wait for the next pointer move to hide it (design §6 bullet 1).
      drawPlaneCueLayer.clear()
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
    // Free orbit must not reach the ±Z poles. Exactly at a pole the view
    // basis is gimbal-degenerate (look ∥ up), and even NEAR one it is
    // ill-conditioned: with world-up +Z, screen roll tracks the azimuth of
    // the camera's tiny lateral offset, so at a polar angle of ~1e-6 rad
    // (OrbitControls' own makeSafe floor) sub-µm position jitter re-rolls
    // the whole frame on every damping-tail repaint — severe whole-viewport
    // shimmer. The Top/Bottom standard views already embody the safe margin
    // (their baked eyes sit POLE_TILT off the pole — see STANDARD_VIEWS);
    // clamp free orbit to the polar angle of that very pose, atan(POLE_TILT),
    // so the two margins share one constant and cannot drift apart (and so
    // controls.update() leaves the Top/Bottom framing itself untouched).
    // ≈0.057° — imperceptible.
    controls.minPolarAngle = Math.atan(POLE_TILT)
    controls.maxPolarAngle = Math.PI - Math.atan(POLE_TILT)

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
        // Re-clip the axis halves to the enlarged frustum (float64 — see
        // clampOriginAxes; the fat-line shader's own handling of extreme
        // segments is what shimmered).
        clampOriginAxes(originAxes, camera)
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

      // Armed drag-to-move: past the threshold, hand the gesture to a
      // one-shot Move tool; while it runs, FALL THROUGH to normal routing so
      // the Move tool gets live snapped pointer moves (inference, axis
      // locks, Alt-copy all work exactly like a two-click Move).
      if (dragMove !== null) {
        if ((ev.buttons & 1) === 0) {
          // The release happened outside our listeners (focus loss) — drop it.
          abortDragMove()
        } else if (!dragMove.active) {
          const [px, py] = canvasPoint(ev)
          if (exceedsDragThreshold(dragMove.startX, dragMove.startY, px, py)) {
            dragMove.active = true
            beginDragMove(dragMove)
          }
        }
      }
      if (ev.buttons !== 0 && ev.button !== -1 && dragMove?.active !== true) return

      const viewportH = el.clientHeight
      const fovY = camera.fov

      // Cache for live re-lock after key events
      lastRayRef.current = { ray, viewportH, fovY }

      const activeTool = toolController.activeTool
      const constraint = 'snapConstraint' in activeTool
        ? (activeTool as { snapConstraint(ray?: Ray): { anchor?: [number, number, number]; lockAxis?: 0 | 1 | 2; constraintPlane?: { point: [number, number, number]; normal: [number, number, number] } } | null }).snapConstraint(ray)
        : null
      const { snap } = snapService.resolve(ray, viewportH, fovY, constraint?.anchor, constraint?.lockAxis, constraint?.constraintPlane)
      activeTool.onPointerMove(snap, ray)
      cueLayer.update(snap)
      drawPlaneCueLayer.update(queryDrawPlaneCue(activeTool))
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
        const [px, py] = canvasPoint(ev)
        const topLevel = activeContextRef.current.length === 0

        // A press on a movable node arms DRAG-TO-MOVE: dragging past the
        // threshold hands the gesture to a one-shot Move (see beginDragMove),
        // a plain release still just selects (click ≠ drag). Shift presses
        // keep the additive-click/marquee path, and a press near a
        // construction guide keeps the guide's click priority.
        const pressedNode = ev.shiftKey || pickGuide(ndcX, ndcY) !== null
          ? null
          : pickTransformableUnderCursor(ray)
        if (pressedNode !== null) {
          // In-context presses click-pick immediately (as they always have);
          // top level defers to pointerup like the marquee path.
          if (!topLevel) dispatchSelectPick(ndcX, ndcY, ray)
          dragMove = {
            startX: px,
            startY: py,
            pressRay: ray,
            nodes: dragMoveTargets(pressedNode, selectedIdsRef.current),
            active: false,
            deferClick: topLevel,
          }
          // Track the drag even when it leaves the canvas.
          renderer.domElement.setPointerCapture(ev.pointerId)
          return
        }

        // Top level: arm a marquee and DEFER the pick to pointerup — a drag
        // becomes a rubber-band selection, a plain release runs the click-pick
        // at the release position. Inside an editing context the marquee is
        // out of scope; the press is an immediate click-pick.
        if (topLevel) {
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
      // (Move's copy mode is a durable Alt TOGGLE handled in MoveTool.onKey —
      // no live Alt-modifier tracking here.)

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

      // Esc cancels an in-flight drag-to-move (before the context pop below,
      // so escaping a drag inside a group doesn't ALSO exit the group).
      if (ev.key === 'Escape' && dragMove !== null) {
        abortDragMove()
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
      // A tool with per-key capture (Tool.capturesKey) narrows this to the
      // keys its buffer actually needs — Move's armed ×N / /N window must
      // never eat Space (always reset-to-Select) or the letter shortcuts.
      if (!isMod && ev.key !== 'Escape') {
        const activeTool = toolController.activeTool
        const captures = 'capturesKey' in activeTool
          ? (activeTool as { capturesKey(key: string): boolean }).capturesKey(ev.key)
          : 'capturingInput' in activeTool &&
            (activeTool as { capturingInput(): boolean }).capturingInput()
        if (captures) {
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
            drawPlaneCueLayer.update(queryDrawPlaneCue(activeTool))
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
        if (ev.key === 'f' || ev.key === 'F') { switchToolRef.current?.('Offset'); return }
        if (ev.key === 'm' || ev.key === 'M') { switchToolRef.current?.('Move'); return }
        if (ev.key === 'q' || ev.key === 'Q') { switchToolRef.current?.('Rotate'); return }
        if (ev.key === 's' || ev.key === 'S') { switchToolRef.current?.('Scale'); return }
      }

      // Undo: Cmd/Ctrl+Z — document-level, covers creations + per-object ops
      if (isMod && !ev.shiftKey && ev.key === 'z') {
        ev.preventDefault()
        runUndo()
        return
      }

      // Redo: Shift+Cmd/Ctrl+Z — document-level. With Shift held, ev.key is
      // the UPPERCASE letter, so compare case-insensitively (a bare === 'z'
      // never fires on a physical keyboard — caught by the input-pipeline
      // E2E redo spec).
      if (isMod && ev.shiftKey && ev.key.toLowerCase() === 'z') {
        ev.preventDefault()
        runRedo()
        return
      }

      // Mod-combos never reach the tool: tools' onKey treats bare letters
      // as VCB length input, so an unhandled chord like Ctrl+K (palette) or
      // Ctrl+C would otherwise append its letter to a mid-entry buffer
      // ("5" → "5k") and wedge it. Tools only consume plain keys.
      // Arrow keys while a text field has focus are caret/list navigation
      // (command palette results, rename inputs), not tool input — with the
      // idle plane lock they would otherwise silently re-aim the next draw
      // gesture from inside an unrelated text box.
      if (isTyping && ev.key.startsWith('Arrow')) return
      if (!isMod) {
        const activeTool = toolController.activeTool
        activeTool.onKey(ev)
        // The idle arrow-key plane lock (design §5.2/§6) toggles here — the
        // capturing branch above only runs once a gesture has anchored, so
        // this is where a lock's ON/OFF/switch needs its own cue re-poll.
        drawPlaneCueLayer.update(queryDrawPlaneCue(activeTool))
        // A tool may have restyled its gizmo in response (e.g. Rotate /
        // Protractor / Slice locking the axis with Shift or an arrow during
        // the idle/hover phase, before capturingInput() is true). The
        // capturing branch above schedules its own render; this fallthrough
        // must too, or that change wouldn't repaint until the next pointer
        // move. The render loop is on-demand, so a spurious schedule on an
        // ignored key just skips one idle frame — cheap.
        scheduleRender()
      }
    }

    // Pointerup completes the Select tool's deferred press: a no-drag release
    // replays the press as the usual click-pick (guide first, then the tool's
    // pick chain); a dragged release commits the marquee selection. Other
    // tools are click-based, so beyond input recording nothing else needs it.
    function onPointerUp(ev: PointerEvent): void {
      recordPointerInput('pointerup', ev)
      if (ev.button !== 0) return

      // Drag-to-move release: an ACTIVE drag commits the Move at the release
      // position (the same second click a two-click Move would get, honoring
      // any live axis lock), then springs back to Select. A sub-threshold
      // release is a plain click — top level runs the deferred click-pick;
      // in-context the press already picked.
      if (dragMove !== null) {
        const dm = dragMove
        dragMove = null
        if (dm.active) {
          const tool = toolController.activeTool
          if (
            tool.name === 'Move' &&
            'capturingInput' in tool &&
            (tool as { capturingInput(): boolean }).capturingInput()
          ) {
            const [ndcX, ndcY] = pointerToNDC(ev, renderer.domElement)
            const ray = makeWorldRay(ndcX, ndcY, camera)
            const constraint = 'snapConstraint' in tool
              ? (tool as { snapConstraint(ray?: Ray): { anchor?: [number, number, number]; lockAxis?: 0 | 1 | 2; constraintPlane?: { point: [number, number, number]; normal: [number, number, number] } } | null }).snapConstraint(ray)
              : null
            const { snap } = snapService.resolve(ray, el.clientHeight, camera.fov, constraint?.anchor, constraint?.lockAxis, constraint?.constraintPlane)
            tool.onPointerDown(snap, ray)
          }
          // (If the tool is no longer mid-gesture — a VCB Enter or Esc ended
          // the move mid-drag — there is nothing to commit here.)
          switchToolRef.current?.('Select')
          return
        }
        if (dm.deferClick && toolController.activeToolName === 'Select') {
          const [ndcX, ndcY] = pointerToNDC(ev, renderer.domElement)
          dispatchSelectPick(ndcX, ndcY, makeWorldRay(ndcX, ndcY, camera))
        }
        return
      }

      if (marqueeDrag === null) return
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
      abortDragMove()
    }
    // Each routed event may advance (or cancel) the active tool's gesture —
    // wrap the handlers so the status-bar hint is re-polled after every one,
    // whichever early-return path the handler took.
    const onPointerMoveTracked = (ev: PointerEvent) => { onPointerMove(ev); reportToolHint() }
    const onPointerDownTracked = (ev: PointerEvent) => { onPointerDown(ev); reportToolHint() }
    const onPointerUpTracked = (ev: PointerEvent) => { onPointerUp(ev); reportToolHint() }
    const onDoubleClickTracked = (ev: MouseEvent) => { onDoubleClick(ev); reportToolHint() }
    const onKeyDownTracked = (ev: KeyboardEvent) => { onKeyDown(ev); reportToolHint() }
    renderer.domElement.addEventListener('pointermove', onPointerMoveTracked)
    renderer.domElement.addEventListener('pointerdown', onPointerDownTracked)
    renderer.domElement.addEventListener('pointerup', onPointerUpTracked)
    renderer.domElement.addEventListener('pointercancel', onPointerCancel)
    renderer.domElement.addEventListener('dblclick', onDoubleClickTracked)
    window.addEventListener('keydown', onKeyDownTracked)

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
      softwareNotice?.remove()
      renderer.domElement.removeEventListener('contextmenu', onContextMenu)
      renderer.domElement.removeEventListener('pointermove', onPointerMoveTracked)
      renderer.domElement.removeEventListener('pointerdown', onPointerDownTracked)
      renderer.domElement.removeEventListener('pointerup', onPointerUpTracked)
      renderer.domElement.removeEventListener('pointercancel', onPointerCancel)
      renderer.domElement.removeEventListener('dblclick', onDoubleClickTracked)
      marqueeOverlay.remove()
      window.removeEventListener('keydown', onKeyDownTracked)
      resizeObserver.disconnect()
      unsubscribeTheme()
      disposeOriginAxes(originAxes)
      infiniteGrid.dispose()
      controls.dispose()
      cueLayer.clear()
      drawPlaneCueLayer.clear()
      sceneRenderer.dispose()
      toolControllerRef.current = null
      switchToolRef.current = null
      sceneRendererRef.current = null
      drawPlaneCueLayerRef.current = null
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
    if (tool !== undefined && 'setContextScoped' in tool) {
      // The id channels above only carry object/instance contexts; a GROUP
      // context leaves both null — signal "scoped" explicitly (hint wording
      // only; eligibility comes from the injected predicate).
      ;(tool as { setContextScoped: (scoped: boolean) => void }).setContextScoped(activeContext.length > 0)
    }
    // A drawing-plane cue rendered for the OUTGOING context no longer
    // applies (e.g. a tool's non-ground plane cue from before entering/
    // exiting an object) — don't wait for the next pointer move to hide it
    // (Blocker 3, mirrors the tool-switch clear above).
    drawPlaneCueLayerRef.current?.clear()
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
    // Push the live selection into the active tool (Tool.setSelection):
    // tools that snapshot the selection at creation (Move/Rotate/Scale)
    // must not keep committing against handles an undo/redo has since
    // killed — the app-level prune flows through here like any other
    // selection change.
    const activeToolForSelection = toolControllerRef.current?.activeTool
    if (activeToolForSelection !== undefined && 'setSelection' in activeToolForSelection) {
      (activeToolForSelection as { setSelection(nodes: NodeRef[]): void }).setSelection([...selectedIds])
    }
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
        // A curve run highlights as its member edges (resolved from the
        // canonical representative edge).
        for (const edge of wasmSceneRef.current.sketch_curve_chain(node.sketch, node.id)) {
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
