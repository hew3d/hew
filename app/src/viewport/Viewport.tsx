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

import { useEffect, useLayoutEffect, useRef } from 'react'
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
import { SectionManager, rescaleSectionPlane } from './sectionManager'
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
import { PositionTextureTool } from '../tools/PositionTextureTool'
import { MoveTool } from '../tools/MoveTool'
import { RotateTool } from '../tools/RotateTool'
import { ScaleTool } from '../tools/ScaleTool'
import { TapeMeasureTool, type RescaleConfirmInfo, type SessionScopeIds } from '../tools/TapeMeasureTool'
export type { RescaleConfirmInfo } from '../tools/TapeMeasureTool'
import { ProtractorTool } from '../tools/ProtractorTool'
import { SliceTool } from '../tools/SliceTool'
import { SectionPlaneTool } from '../tools/SectionPlaneTool'
import { EditVertexTool } from '../tools/EditVertexTool'
import { TextPlaceTool, type TextPlacement } from '../tools/TextPlaceTool'
import { PositionCameraTool } from '../tools/PositionCameraTool'
import { LookAroundTool } from '../tools/LookAroundTool'
import { WalkTool } from '../tools/WalkTool'
import { DEFAULT_EYE_HEIGHT_M, type V3 as WalkV3 } from '../tools/cameraWalkMath'
import { AxesTool } from '../tools/AxesTool'
import { getDrawingAxes, type DrawingAxes } from '../tools/drawingAxes'
import { DimensionTool } from '../tools/DimensionTool'
import { TextTool, type PlacedLeader } from '../tools/TextTool'
import { makeSketchPlaneCache } from '../tools/sketchGesture'
import { parseKernelErrorCode, kernelErrorMessage, friendlyErrorText } from '../kernelErrors'
import type { Ray, ApertureBasis } from './math'
import {
  axisDashGapWorld,
  axisDashGapWorldFromWorldPerPixel,
  orthoZoomBounds,
  tanHalfFovRad,
  scaleCameraAboutOrigin,
  scaleViewLimits,
  zoomExtentsViewLimits,
  MOUNT_LIMITS,
  HOME_EYE_OFFSET,
} from './math'
import { CameraRig, type Projection, isBehindCamera } from './cameraRig'
import { fovReadoutText, activeCameraToolForName } from './fovReadout'
import { parseFovEntry } from './fovUnits'
import {
  beginFovDrag,
  decideFovDragMode,
  fovAfterWheel,
  fovDragValue,
  type FovDragState,
} from './fovDrag'
import { shouldSkipToolSwitch } from './toolSwitchGuard'
import type { Snap, SnapConstraint, Tool, EditContext } from '../tools/types'
import { toolHasArmedGesture } from '../tools/types'
import { rayPlaneIntersect, subV3, addV3, perpComponentV3, type V3 } from './geoHelpers'
import { readAnnotation, commitAnnotationText, initialEditorText, type AnnotationSnapshot } from './annotationEdit'
import { collectLeafIds, nodeEq, nodeKey, nodeRefFromJs, resolveLabel, structuralSelection, type NodeRef } from '../panels/treeModel'
import { MarqueeProjector, normalizedRect, type MarqueeMode, type MarqueeRect } from './marquee'
import { dragMoveTargets, exceedsDragThreshold } from './dragMove'
import {
  beginZoomWindowDrag as beginZoomWindowDragState,
  updateZoomWindowDrag as updateZoomWindowDragState,
  finishZoomWindowDrag as finishZoomWindowDragRect,
  type ZoomWindowDragState,
} from './zoomWindowDrag'
import { CleanModifierTap } from './cleanModifierTap'
import { MultiClickTracker } from './multiClick'
import { resolveSelectableRef, type ResolveDeps, type SelectScene } from '../tools/snapSelection'
import { cursorFor } from '../tools/toolIcons'
import { getResolvedTheme, subscribe as subscribeTheme, type ResolvedTheme } from '../settings/theme'
import { readAppliedTheme } from '../theme/applyTheme'
import { getLengthUnit, homeFramingScale, subscribe as subscribeLengthUnit } from '../settings/units'
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
 *
 * `frame` is the drawing-axes frame `direction` should be tested against
 * (tool-parity §4 — movable drawing axes): `publishSnapCues` reads it fresh
 * via `getDrawingAxes(wasmScene)` alongside `direction`, so the tooltip/dot
 * color a moved frame's own axis correctly instead of dropping to neutral.
 * Undefined only if a caller builds `InferenceInfo` without it (treated as
 * world identity downstream, in `inferenceColor.ts`).
 */
export interface InferenceInfo {
  kind: string
  screenX: number
  screenY: number
  direction?: [number, number, number]
  frame?: DrawingAxes
  /** The snap is a REFERENCE, not the committed point: the active tool
   *  projected it onto its drawing plane (see `Tool.snapProjected`). The chip
   *  qualifies the label so it stops claiming a snap that was not honoured. */
  projected?: boolean
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
  /** Called when precision snapping (the Ctrl/Cmd+Alt chord held) turns on or
   * off, so the status bar can surface it the way it surfaces other modal
   * state. */
  onPrecisionChange?: (active: boolean) => void
  /** Active tool name from parent (undefined = parent doesn't control) */
  activeTool?: string
  /** Bumped by the parent on every EXPLICIT camera-tool selection so a
   * re-choice of an already-`activeTool`-valued entry (stale because the
   * real tool drifted out from under it — an internal auto-handoff or
   * Escape, see App.tsx's `activateTool`) still re-applies: included in
   * the tool-switch effect's dependency array below purely to force it to
   * re-run even when `activeTool` itself hasn't changed. */
  activeToolSeq?: number
  /**
   * Fired at the end of switchToolRef's single entry point on EVERY
   * invocation — not just ones the parent itself requested — with the tool
   * that just actually became active. This is what makes `activeTool`
   * truthful for the internal transitions `activeToolSeq` above works
   * around otherwise: Position Camera's auto-handoff to Look Around, any
   * camera tool's Escape-to-Select, and every other switchToolRef caller.
   * The parent's handler should skip the update when the name already
   * matches its current `activeTool` — both because that's a harmless no-op
   * (React already bails on an unchanged value) and because this callback
   * must never fight `activeToolSeq`'s forced-reapply purpose: it only
   * FOLLOWS what Viewport just did, it never itself requests a switch.
   */
  onInternalToolChange?: (toolName: string) => void
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
  /** Lift an annotation (dimension/leader-text) pick to the parent; `null`
   * clears (docs/design/dimensions-text.md). */
  onSelectAnnotation?: (id: bigint | null) => void
  /** The currently selected annotation, reflected into the renderer highlight. */
  selectedAnnotation?: bigint | null
  /**
   * Requests the parent open the in-viewport text editor (`AnnotationEditor`,
   * the `InferenceTooltip` DOM-positioning pattern): a double-click on an
   * existing annotation (`id` set — editing its text/override), or the Text
   * tool's second click (`id` null — a brand-new leader, placed but not yet
   * worded). The parent renders the editor at `screenX`/`screenY`, prefilled
   * with `initialText`, and on commit/cancel calls back through
   * `ViewportApi.commitAnnotationEditorText`/`cancelAnnotationEditor` — this
   * component tracks WHICH annotation (or pending new leader) the edit
   * applies to internally, so the parent only ever passes text through.
   */
  onOpenAnnotationEditor?: (info: {
    id: bigint | null
    screenX: number
    screenY: number
    initialText: string
    placeholder: string
  }) => void
  /** Request entering a node's editing context (double-click). */
  onEnterContext?: (node: NodeRef) => void
  /** Request popping one level off the context path (Esc). */
  onExitContext?: () => void
  /** Request clearing the WHOLE context path back to the top level — the
   *  undo/redo reconcile uses this when the kernel reports an explode
   *  session re-opened by a history step while the app still stands inside
   *  a K1/K2 context (the session/context mutual-exclusion invariant,
   *  enforced across history boundaries too). */
  onExitAllContexts?: () => void
  /**
   * Fired whenever the open session STACK changes: a frame opened, closed,
   * or the whole stack resynced across an undo/redo boundary
   * (docs/design/group-session.md) — outermost frame first, empty when
   * nothing is open. Each frame carries the NodeRef the user entered (a
   * group, or a component instance — always innermost) plus its display
   * label, resolved by the viewport since a session hides its own node (its
   * name is unreadable through the ordinary `group_name`/`instance_name`
   * queries once hidden) — captured before hiding, or resolved through the
   * kernel's own hidden-safe escape hatch for a component frame. The parent
   * uses this for menu gating (Make Group/Make Component/Place Copy/Import/
   * 3D Text refuse only while the INNERMOST frame is a component; whole-
   * model rescale refuses while ANY frame is open) and for the breadcrumb/
   * "editing" chips; the viewport itself owns scoping/dimming, since those
   * need no cross-render state the parent has to hold.
   */
  onSessionChange?: (frames: { node: NodeRef; label: string }[]) => void
  /** Fired after any document change so the parent can refresh the tree. */
  onDocumentChanged?: () => void
  /** Fired whenever the section plane's existence/active state actually
   * changes (place, offset-commit, toggle, delete, or a fresh document
   * clearing it) — NOT on every pointer move of a live offset-drag preview,
   * which never changes existence/active. The parent re-derives its
   * View ▸ Section Plane check state from `getSectionState()` in response,
   * rather than tracking a shadow boolean that could drift from the section
   * manager's real state (D3, section-plane-polish). */
  onSectionChanged?: () => void
  /** Fired after a SUCCESSFUL undo/redo, from the one code path every
   * entry point shares (menu, palette, and the viewport's own Cmd+Z/Cmd+
   * Shift+Z all funnel into runUndo/runRedo). Undo/redo can change state
   * that plain document changes cannot — e.g. restore a deleted tag's
   * registry entry — so the parent reconciles view state (tag visibility)
   * here rather than per entry point. */
  onHistoryChanged?: () => void
  /** Populated by the viewport with imperative commands the parent can call. */
  apiRef?: React.MutableRefObject<ViewportApi | null>
  /** Called with the live measurement text from tools that support VCB entry.
   *  `frozen` (Tape Measure only, tape-measure-rework part 1) is true when
   *  `text` is a finished reading kept on screen for reference rather than a
   *  live typed buffer — every other tool's callback never passes it. */
  onMeasurement?: (text: string, frozen?: boolean) => void
  /** Fired when the Tape Measure tool arms a "resize the model?" confirmation
   *  (design tool-parity §3): the parent renders the confirmation modal and
   *  resolves it via `ViewportApi.confirmPendingRescale` /
   *  `cancelPendingRescale`. */
  onRescaleArmed?: (info: RescaleConfirmInfo) => void
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
  /** Fired whenever the active projection changes (mount, and every
   * `toggleProjection` call) — the parent's Camera ▸ Parallel Projection
   * checkbox state derives from this rather than polling `getProjection`
   * (docs/design/camera.md §1). */
  onProjectionChange?: (projection: Projection) => void
  /**
   * Fired when a Viewport-internal ONE-SHOT gesture finishes and the tool
   * should spring back to Select without the parent having initiated the
   * change — currently just Zoom Window (design §3: "springs back to
   * Select", the drag-to-move one-shot's precedent, but that one never
   * actually LEAVES Select at the app-tool-state level the way Zoom Window
   * does, so this callback is what closes that gap). The parent should treat
   * this exactly like a user clicking Select in the rail/menu.
   */
  onToolReverted?: () => void
  /** Fired when Paint's Alt-click eyedropper samples a face — the parent
   *  makes the sampled id current (`setCurrentMaterialId`), so the palette
   *  selection follows exactly like picking the swatch directly. */
  onSampleMaterial?: (id: bigint) => void
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
   * or a result group when the result has disjoint pieces); `null` on a
   * refused op that left the document **completely** untouched (already
   * toasted) — this is only ever true when nothing at all committed yet.
   * `'mutated-failed'` is a THIRD outcome, distinct from both: a component
   * instance operand needed auto-exploding first (`explodeInstanceOperand`
   * via `runBooleanCore` — make_unique/explode_instance/group_nodes), and
   * SOME step of that committed a real document mutation before something
   * downstream failed — whether that's a later auto-explode sub-step (the
   * other operand, or this same operand's own explode/group step), or the
   * retried boolean itself. The document is left holding whatever exists at
   * the point of failure (exploded pieces for an operand that finished,
   * or the still-present-but-now-unique instance for one that didn't) — the
   * caller must treat this like a mutation (bump its own revision counter)
   * even though there is no single result node; the surviving pieces' new
   * selection is already pushed via `onSelectMany` and an honest recovery
   * toast (naming what committed, what failed, and exactly how many undo
   * steps restore the pre-explode state) is already shown before this
   * returns. */
  runBoolean: (op: number, a: NodeRef, b: NodeRef) => NodeRef | null | 'mutated-failed'
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
   * Open an explode session on `instanceId` — bakes its definition's
   * members into world-owned geometry so the ordinary, unmodified tool set
   * can edit them. Returns whether it succeeded (a toast already reported a
   * typed refusal). The ordinary double-click gesture goes through its own
   * open+fallback path instead (silently falling back to the K1/K2 instance
   * context on `ExplodeSessionPoseUnsupported`); this is for direct callers
   * (the test harness) that want the raw open/refuse outcome.
   */
  runOpenExplodeSession: (instanceId: bigint) => boolean
  /**
   * Open an explode session on `instanceId`, silently falling back to a
   * K1/K2 `onEnterContext(fallback)` push on the two refusals the design
   * treats as "this instance doesn't get a session" rather than an error
   * (`ExplodeSessionPoseUnsupported`/`ExplodeSessionGroupedInstance`) — the
   * same fallback the ordinary viewport double-click gesture uses,
   * exposed for the entry-convergence helper (`App.tsx`) so the Outliner/
   * dock entry paths get identical fallback behavior. Any other refusal
   * toasts and enters nothing.
   */
  runOpenExplodeSessionOrFallback: (instanceId: bigint, fallback: NodeRef) => void
  /** Close the open explode session, folding edits back into the
   *  definition. A no-op (returns `false`) if none is open. */
  runCloseExplodeSession: () => boolean
  /** The instance an open explode session was entered through,
   *  or `null` if none is open. */
  explodeSessionInstance: () => bigint | null
  /**
   * Open a GROUP session on `groupId` — the ungroup posture
   * (docs/design/group-session.md): direct members surface to the world
   * top level so the ordinary tool set edits them unmodified. Returns
   * whether it succeeded (a toast already reported a typed refusal, e.g.
   * `ExplodeSessionNestedGroup`/`ExplodeSessionOpen`). Unlike an instance
   * entry, a group entry has no silent fallback — every refusal toasts.
   */
  runOpenGroupSession: (groupId: bigint) => boolean
  /** Close the open GROUP session (the innermost frame must be one),
   *  re-homing survivors and folding in mid-session creations. A no-op
   *  (returns `false`) if the innermost frame isn't a group session. */
  runCloseGroupSession: () => boolean
  /** Close the INNERMOST open session frame, whichever kind it is — the
   *  Escape / double-click-outside gesture, and the entry-convergence
   *  helper's own "close frames below the divergence" step. A no-op
   *  (returns `false`) if the stack is empty. */
  runCloseInnermostSession: () => boolean
  /** The current session stack, outermost first — mirrors the kernel's own
   *  `session_stack()`. Empty when nothing is open. The entry-convergence
   *  helper diffs against this to find the common prefix it can leave
   *  open. */
  sessionStack: () => NodeRef[]
  /** Whether the active tool has an armed, in-progress gesture that Escape's
   *  own refusal posture already protects (`toolHasArmedGesture` — a
   *  Move/Rotate/Scale/PushPull/Offset drag, a Follow Me sweep, a draw
   *  tool's anchored chain, Slice's plane lock, Tape Measure's pending
   *  rescale confirmation, …). App-side session-close paths that don't run
   *  through the viewport's own Escape/double-click-outside handling
   *  (breadcrumb crumb clicks, the Model crumb, `enterNode`'s entry-
   *  convergence close loop — finding 3, adversarial review) consult this
   *  FIRST and no-op while it's true, matching Escape exactly: re-homing
   *  session state out from under an armed gesture would either fold
   *  geometry back while the gesture still holds stale handles, or silently
   *  retarget the gesture's eventual commit via the shallower context this
   *  would push. */
  hasArmedGesture: () => boolean
  /**
   * The innermost open session frame's current direct-member scope (any
   * node kind — a group frame's members can be objects, nested groups, or
   * instances; a component frame's are always plain objects), or `null`
   * when no session is open. The Outliner's synthetic session-header row
   * nests these underneath it, generalizing the single-instance "explode
   * session member list" it rendered before groups existed.
   */
  sessionMembers: () => NodeRef[] | null
  /** Resolve a pending Tape Measure "resize the model?"/"resize in
   *  context?" confirmation (design tool-parity §3; group-session.md's
   *  scoped-rescale phase): apply the armed `rescale_document` (`scoped`
   *  false) or `rescale_session` (`scoped` true, anchored at the arm's
   *  measured point) call and refresh the scene. `scoped`/`scopeLabel` are
   *  App's `handleRescaleArmed` decision, threaded straight through so this
   *  call can never disagree with what the dialog told the user it would
   *  do. A no-op if the active tool isn't (still) the Tape Measure tool with
   *  a pending confirmation. */
  confirmPendingRescale: (scoped: boolean, scopeLabel: string | null) => void
  /** Decline a pending rescale confirmation: falls through to the normal
   *  guide-point commit the typed distance would have produced without the
   *  arm. A no-op under the same conditions as `confirmPendingRescale`. */
  cancelPendingRescale: () => void
  /**
   * Arms a one-shot 3D Text placement tool (docs/design/3d-text.md) with
   * the dialog's already-resolved glyph geometry — the ghost follows the
   * cursor on whatever face/ground/axis-locked plane the draw-tool rules
   * resolve; a click commits the whole placement (gesture, extrusion, and
   * component fold) as one undo step.
   */
  armTextPlacement: (placement: TextPlacement) => void
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
   * Project a world point to canvas-relative CSS pixels (origin top-left) at
   * the current camera. `behind` is true when the point is behind the camera
   * (the x/y are then meaningless). Test-only: lets a pointer-driven E2E
   * (e.g. the Scale gizmo's grip-grab) target a grip's exact screen position
   * robustly, instead of hard-coding pixels read off a screenshot.
   */
  worldToScreen: (world: [number, number, number]) => { x: number; y: number; behind: boolean }

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
   * The camera's full working view (projection + fov + eye/target/up),
   * for document-save persistence (docs/design/camera.md §5) — see
   * `getCameraState`'s doc comment for how this differs from `getCamera`.
   */
  getCameraState: () => {
    projection: Projection
    fovDeg: number
    eye: [number, number, number]
    target: [number, number, number]
    up: [number, number, number]
  }
  /**
   * Restores a full camera view saved by `getCameraState` — the document-
   * load complement (design §5). Handles a projection change either
   * direction.
   */
  applyCameraState: (state: {
    projection: Projection
    fovDeg: number
    eye: [number, number, number]
    target: [number, number, number]
    up: [number, number, number]
  }) => void
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
  /** Reset the movable drawing axes (tool-parity §4) to world identity
   *  (View ▸ Reset Axes) — same commit path as the Axes tool's own
   *  `set_axes`, one undo step. */
  resetAxes: () => void
  /** Delete a single picked construction guide. */
  runDeleteGuide: (id: bigint) => void
  /** Delete a single picked annotation (dimension or leader text) —
   * docs/design/dimensions-text.md. */
  runDeleteAnnotation: (id: bigint) => void
  /**
   * Commit whatever the in-viewport annotation editor currently has pending
   * (`AnnotationEditor`'s Enter/blur) — either an edit to an existing
   * annotation's text (a dimension's `text_override`, or a leader's
   * content) or a brand-new leader text placed by the Text tool. No-op if
   * nothing is pending (e.g. a stray call after the editor already closed).
   */
  commitAnnotationEditorText: (text: string) => void
  /** Discard whatever the in-viewport annotation editor currently has
   * pending (Esc) — no kernel call, unlike `commitAnnotationEditorText`. */
  cancelAnnotationEditor: () => void
  /**
   * Test/E2E: the currently rendered label text for annotation `id` (the
   * app-computed measurement, or its `text_override`), or `null` if not
   * live/rendered — proves a unit-setting change actually re-labels a
   * dimension without reading the rasterized canvas texture directly.
   */
  getAnnotationLabel: (id: bigint) => string | null
  /**
   * Test/E2E: the current world-space position of annotation `id`'s text
   * billboard (the same point the double-click-to-edit gesture projects to
   * place the in-viewport editor), or `null` if `id` isn't live/rendered.
   * Lets a pixel-precision E2E check that a dimension's LABEL actually
   * rendered sample the real render-time anchor — correct for both a linear
   * dimension's 'broken' (label centered on the line) and 'outside' (label
   * pushed past an end) gap-layout modes — rather than guessing at the
   * line's midpoint.
   */
  getAnnotationTextWorldPosition: (id: bigint) => [number, number, number] | null
  /**
   * Toggle the placed section plane's active (clipping) flag — "Toggle
   * Section Plane Active" (menu + palette), SketchUp's "Active Cut". A
   * no-op when no section is placed. Session-only view state, exactly like
   * `setAxesVisible`/`setGuidesVisible` above (DESIGN §2).
   */
  toggleSectionActive: () => void
  /**
   * Test/E2E: the current session section (`{ origin, normal, active }`) or
   * null. Reads the same SectionManager the tool/menu mutate — no document
   * state. Observe-only, like `getCamera`.
   */
  getSectionState: () => { origin: [number, number, number]; normal: [number, number, number]; active: boolean } | null
  /**
   * Test/E2E: section-plane render diagnostics — whether the widget overlay
   * is built, the widget's own clip count (must be 0 — overlays never clip),
   * the clip-plane count on a specific rendered object/instance face material
   * (-1 if that node isn't rendered), and the active clip plane's world
   * normal + constant (null when inactive) so a spec can assert the clip
   * SIDE. Lets a spec assert the real-wiring clip state unit tests can't reach.
   */
  getSectionRenderInfo: (
    kind: 'object' | 'instance',
    id: bigint,
  ) => {
    widget: boolean
    widgetClipCount: number
    nodeClipCount: number
    clipPlane: { normal: [number, number, number]; constant: number } | null
  }
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
  /**
   * Camera ▸ Parallel Projection (docs/design/camera.md §1): toggles between
   * perspective and parallel projection, visually stable at the orbit
   * target. `onProjectionChange` (a Props callback) reports the result so
   * the parent's checkbox state never needs a separate `getProjection` poll.
   */
  toggleProjection: () => void
  /** The active projection right now — for callers that need it
   * synchronously (e.g. gating a fov-dependent affordance under parallel
   * projection, which has no lens) rather than waiting on the next
   * `onProjectionChange` callback. */
  getProjection: () => Projection
  /**
   * Set the perspective vertical fov directly (degrees, clamped to
   * `[MIN_FOV_DEG, MAX_FOV_DEG]` — `cameraRig.ts`). The normal path is typing
   * into the Zoom tool's VCB (design §2); this is the equivalent direct/test
   * entry point. No-op on the orthographic frustum — fov is a
   * perspective-only property that persists across a projection toggle.
   */
  setFov: (deg: number) => void
}

/** Build a normalised world-space ray from NDC (-1..1) coords and a camera.
 * Projection-agnostic: `Vector3.unproject` works identically for a
 * perspective or orthographic camera (docs/design/camera.md §1) — picking
 * needed no change at all when Parallel Projection landed, only this type
 * widening. */
function makeWorldRay(
  ndcX: number,
  ndcY: number,
  camera: THREE.Camera,
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

/** Each axis half's nominal world length — beyond the camera's DEFAULT
 * far-clip of 100 (see `buildOriginAxes`) — before any Tape Measure rescale
 * has scaled it. Exported as a named constant (not just a literal `150` at
 * the one call site) because `Viewport.tsx`'s mount effect also needs it as
 * the SEED value for `axesHalfLength`, the running length a rescale scales
 * in lockstep with the camera's far-clip (see `applyRescaleToView`) so the
 * "always beyond far" invariant below keeps holding after the model's size
 * changes. */
export const AXIS_HALF_LENGTH_DEFAULT = 150

/** World origin axis lines, colored for `theme`, `halfLength` long (beyond
 * the camera's far-clip at whatever scale the document is currently at) to
 * always run off the edge of the visible world in every direction, reading
 * as "infinite" without needing a shader.
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

/** Desired on-screen dash/gap length (px) for the negative axis halves'
 * dash pattern, kept SCREEN-constant rather than world-constant —
 * `LineMaterial.dashSize`/`gapSize` are world-space lengths, so a flat
 * world constant (the former 0.28/0.22 m) reads solid at cm-scale work (one
 * dash+gap period dwarfs the whole visible model) and only reads clearly
 * dashed around 10 m scale. Recomputed every rendered frame in
 * `clampOriginAxes` via `screenConstantWorldHalf`, using distance from the
 * camera to the world origin as the reference distance (the same distance
 * the near-margin calc just below already uses). Ratio kept close to the
 * original world-constant values (0.28/0.22 ≈ 56/44) — only the sizing
 * model changes, not the look. Set once here too, as a reasonable initial
 * value before the first frame's `clampOriginAxes` call overwrites it. */
const AXIS_DASH_SCREEN_PX = 9
const AXIS_GAP_SCREEN_PX = 7
// Floor so dash/gap never collapse to (near) zero world size when the
// camera sits right on top of the origin — mirrors ScaleTool's
// MIN_GRIP_WORLD_HALF pattern (viewport/math.ts's screenConstantWorldHalf).
const AXIS_DASH_MIN_WORLD = 1e-5

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
    // Placeholder pre-first-frame values — `clampOriginAxes` overwrites both
    // to a screen-constant size before the first `renderer.render()` call
    // ever runs (it's invoked synchronously at the end of mount, not only
    // from the rAF loop). Only meaningful if this material were ever read
    // before that, which never happens in practice.
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
 *
 * Also recomputes the negative (dashed) halves' `dashSize`/`gapSize` every
 * frame here, piggybacking on this function's existing per-frame call sites
 * rather than adding a new one: a screen-constant dash pattern needs the
 * same camera-distance-to-origin figure this function already derives for
 * `margin` (see `screenConstantWorldHalf`'s doc comment in `math.ts` for the
 * general technique — same one `ScaleTool.updateGripScale` uses per-grip).
 * `dashSize`/`gapSize` are plain `LineMaterial` uniforms (not a `dashed`
 * flag flip), so writing them every frame recompiles nothing.
 *
 * Projection-agnostic (docs/design/camera.md §1): under perspective the
 * side-plane bound GROWS with view-space depth (a true cone,
 * `bound(-z) = tanH·(-z)`); under parallel projection it's a CONSTANT
 * half-width/height independent of depth (a box, not a cone — apparent
 * size doesn't track distance under ortho at all). Folding both into
 * `hBoundBase + hBoundSlope·(-bz)` (ortho: `hBoundSlope = 0`, so the
 * depth-dependent term drops out entirely) lets the per-child plane math
 * below stay a single formula either way — no `instanceof PerspectiveCamera`
 * branch in the clip loop itself.
 */
const FRUSTUM_SLACK = 1.5
function clampOriginAxes(group: THREE.Group, rig: CameraRig, viewportHeightPx: number): void {
  // View transform in float64: three stores matrix elements and camera pose
  // as JS numbers, so composing the two matrix-vector products here (rather
  // than in the f32 vertex shader) is what buys the precision. Recompute the
  // inverse from the camera's current pose — `camera.matrixWorldInverse` is
  // only refreshed by `renderer.render`, i.e. it still holds LAST frame's
  // pose here, and a stale view would misclip the very frame captured right
  // after a programmatic `setCamera` jump.
  const camera = rig.active
  camera.updateMatrixWorld()
  const m = _axisView.copy(camera.matrixWorld).invert().elements
  // View-space position of the world origin (the shared start of every half).
  const ax = m[12]
  const ay = m[13]
  const az = m[14]
  // Near margin: comfortably past the near plane, growing with camera
  // distance so the float noise floor (ulps of camera/axis coordinate
  // magnitudes) stays orders of magnitude below one depth/pixel quantum at
  // every scale. `rig.perspective.near` is always current: CameraRig keeps
  // both cameras' near/far synced through every toggle.
  const margin = Math.max(4 * rig.perspective.near, 0.02 * camera.position.length())

  let hBoundBase: number, hBoundSlope: number, vBoundBase: number, vBoundSlope: number
  if (rig.projection === 'perspective') {
    const tanV = Math.tan((rig.perspective.fov * Math.PI) / 360) * FRUSTUM_SLACK
    const tanH = tanV * rig.perspective.aspect
    hBoundBase = tanH * -az
    hBoundSlope = tanH
    vBoundBase = tanV * -az
    vBoundSlope = tanV
  } else {
    const o = rig.orthographic
    hBoundBase = ((o.right - o.left) / (2 * o.zoom)) * FRUSTUM_SLACK
    vBoundBase = ((o.top - o.bottom) / (2 * o.zoom)) * FRUSTUM_SLACK
    hBoundSlope = 0
    vBoundSlope = 0
  }

  // Screen-constant dash/gap sizing for the negative halves, via the active
  // projection's worldPerPixel (docs/design/camera.md §1) — same
  // camera-distance-to-origin reference the near `margin` above uses for
  // perspective; ortho's worldPerPixel ignores the distance argument
  // entirely (distance-independent by definition).
  const { dashSize: dashWorld, gapSize: gapWorld } = axisDashGapWorldFromWorldPerPixel(
    AXIS_DASH_SCREEN_PX,
    AXIS_GAP_SCREEN_PX,
    rig.worldPerPixel(camera.position.length(), viewportHeightPx),
    AXIS_DASH_MIN_WORLD,
  )

  for (const child of group.children) {
    if (!(child instanceof Line2)) continue
    // Unconditional every frame (unlike the clip below, which skips once
    // converged) — the clamp's [t0,t1] can stay unchanged across a pure
    // dolly that never crosses a clip plane, but the dash pattern must still
    // rescale with distance.
    const mat = child.material as LineMaterial
    if (mat.dashed) {
      mat.dashSize = dashWorld
      mat.gapSize = gapWorld
    }
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
    //   sides:  |x(t)| ≤ hBound(-z(t)),  |y(t)| ≤ vBound(-z(t))
    let t0 = 0
    let t1 = 1
    // Each constraint as g(t) = c + d·t ≥ 0.
    const planes: Array<[number, number]> = [
      [-az - margin, -bz],
      [hBoundBase - ax, hBoundSlope * -bz - bx],
      [hBoundBase + ax, hBoundSlope * -bz + bx],
      [vBoundBase - ay, vBoundSlope * -bz - by],
      [vBoundBase + ay, vBoundSlope * -bz + by],
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

function buildOriginAxes(theme: 'light' | 'dark', halfLength: number = AXIS_HALF_LENGTH_DEFAULT): THREE.Group {
  const group = new THREE.Group()
  group.name = 'OriginAxes'

  const L = halfLength
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

// Scratch objects for `updateOriginAxesFrame`, reused across calls (every
// render frame) to avoid an allocation per frame — same pattern as
// `_axisView` above.
const _axesBasis = new THREE.Matrix4()
const _axesQuat = new THREE.Quaternion()

/**
 * Orient the `OriginAxes` group to the document's current movable drawing
 * axes (tool-parity §4): the group's position becomes the frame's origin
 * and its rotation the basis spanned by the frame's red/green/blue
 * directions, so the gizmo built by `buildOriginAxes` (always in LOCAL
 * space, at the local origin along local X/Y/Z) renders at wherever the
 * frame currently is — while `buildOriginAxes`'s own per-segment coloring
 * (X=red/Y=green/Z=blue, tied to axis INDEX) and `clampOriginAxes`'s
 * frustum-clip math stay untouched, exactly as before a frame move.
 *
 * Cheap (a `Scene::axes()` call plus one quaternion-from-basis conversion)
 * so this runs unconditionally every frame — no cache/dirty-check needed.
 * At world identity this sets position (0,0,0) and an identity quaternion,
 * so an untouched document's gizmo renders pixel-identical to before this
 * existed.
 */
function updateOriginAxesFrame(group: THREE.Group, wasmScene: WasmScene): void {
  const frame = getDrawingAxes(wasmScene)
  _axesBasis.makeBasis(
    new THREE.Vector3(...frame.x),
    new THREE.Vector3(...frame.y),
    new THREE.Vector3(...frame.z),
  )
  _axesQuat.setFromRotationMatrix(_axesBasis)
  group.position.set(frame.origin[0], frame.origin[1], frame.origin[2])
  group.quaternion.copy(_axesQuat)
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
 *
 * `sessionScope`, when non-null, is the nodeKey set (`treeModel.nodeKey`) of
 * every node currently "inside" the open, innermost session frame
 * (docs/design/group-session.md) — a session keeps `activeContext` empty
 * while it alone is open (component-edit-parity's "explode session" phase —
 * it edits at the top level, not through a pushed context; an object
 * context CAN sit on top of a session now, in which case the resolver's own
 * `activeContext`-nonzero branches take over and this scope check never
 * runs), so scoping is layered on top of the ordinary top-level resolution
 * here rather than threaded through a new `EditContext` kind. A component
 * session's scope is always plain objects (grouping/component/instance
 * creation are kernel-refused while a component frame is open); a group
 * session's scope can be any node kind (its direct members surface to the
 * top level verbatim — objects, nested groups, or instances), so the check
 * is a plain nodeKey membership test rather than the object-id-only,
 * kind-narrowed check this used before groups existed. `undefined`/omitted
 * preserves the pre-session behavior exactly, so every existing caller (and
 * test) that doesn't pass it is unaffected.
 */
export function resolvePickToSelectable(
  wasmScene: WasmScene,
  pickedObjectId: bigint,
  activeContext: NodeRef[],
  pickedInstanceId?: bigint,
  sessionScope?: Set<string> | null,
): NodeRef | null {
  const resolved = resolvePickToSelectableUnscoped(
    wasmScene, pickedObjectId, activeContext, pickedInstanceId,
  )
  if (sessionScope == null) return resolved
  if (resolved === null || !sessionScope.has(nodeKey(resolved))) return null
  return resolved
}

function resolvePickToSelectableUnscoped(
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

/**
 * The single editing-context channel (component-edit-parity.md phase A1):
 * reduce the app's `activeContext: NodeRef[]` breadcrumb path to the one
 * `EditContext` value every tool consults uniformly, replacing the four
 * ad-hoc duck-typed channels (`setActiveContext`/`setComponentContext`/
 * `setContextScoped`/`setActiveGroup`) that used to each derive their own
 * slice of the same path independently. An instance context resolves its
 * definition (`instance_def`) here once, so tools never have to.
 *
 * A stale/hidden deepest instance (definition lookup misses) degrades to
 * `'top'` rather than throwing — the same defensive posture
 * `activeLitSet`'s memo already takes for the identical lookup.
 */
export function computeEditContext(wasmScene: WasmScene, activeContext: NodeRef[]): EditContext {
  if (activeContext.length === 0) return { kind: 'top' }
  const deepest = activeContext[activeContext.length - 1]
  if (deepest.kind === 'object') return { kind: 'object', id: deepest.id }
  if (deepest.kind === 'group') return { kind: 'group', id: deepest.id }
  if (deepest.kind === 'instance') {
    const component = wasmScene.instance_def(deepest.id)
    if (component === undefined) return { kind: 'top' }
    return { kind: 'instance', id: deepest.id, component }
  }
  return { kind: 'top' }
}

/** Push `ctx` to `tool` if it implements the optional `setEditContext` hook
 *  (component-edit-parity.md phase A1) — a no-op for tools that don't need
 *  editing-context awareness at all. */
export function applyEditContext(tool: Tool, ctx: EditContext): void {
  if ('setEditContext' in tool) {
    ;(tool as { setEditContext(ctx: EditContext): void }).setEditContext(ctx)
  }
}

/**
 * Outcome of exploding ONE component-instance boolean operand into plain
 * world geometry — the auto-explode helper `runBooleanCore` calls per
 * instance operand (playtest finding 4: the engrave/emboss use case needs
 * booleans against 3D text, which is always a component instance, and the
 * kernel rightly refuses an instance operand outright —
 * `BooleanOperandHasInstance`, since a boolean consumes its operand and an
 * instance's geometry is shared).
 *
 * No new kernel surface: `make_unique` (give the instance its own
 * definition — the kernel error's own suggested fallback), then
 * `explode_instance` (bake its pose into plain world solids), then — for a
 * definition with more than one disjoint solid (exactly the 3D Text case,
 * one Object per extruded glyph region) — `group_nodes` folds the pieces
 * into one group so the boolean still sees a single operand (mirroring the
 * existing multi-solid boolean flow: `canBoolean` in treeModel.ts accepts
 * exactly 2 top-level operands, so combining >2 solids always means
 * grouping first).
 *
 * Each of these is its own committed undo step (no sanctioned
 * single-compound mechanism covers this sequence — `DocAction::Compound` is
 * scoped to `place_text` alone), so `steps` on EVERY outcome — success or
 * failure — reports exactly how many landed. Delta-review finding 1: a
 * mirrored/reflected-pose instance (legal via import/load) passes
 * `make_unique` and only then fails `explode_instance`
 * (`CannotExplodeReflected`) — and `group_nodes` can likewise fail after
 * `explode_instance` already committed. Both leave `steps > 0` with the
 * operand never replaced; a caller that reads a bare failure as "nothing
 * happened" loses track of a real, undoable document mutation.
 */
export interface ExplodeInstanceSuccess {
  ok: true
  /** The plain object (single solid) or group (multiple) now standing in for the instance. */
  node: NodeRef
  /** Undo steps committed to reach this: 2 (make_unique + explode) or 3 (+ group). */
  steps: number
}

export interface ExplodeInstanceFailure {
  ok: false
  /** Undo steps already committed before the failure. 0 means the document
   *  is untouched — `make_unique` itself refused. */
  steps: number
  /** Whatever now exists in the document in the instance's place, for
   *  selection/recovery: the still-present (now-uniquely-defined) instance
   *  if only `make_unique` landed, the ungrouped exploded objects if
   *  `group_nodes` refused, or empty when nothing committed. */
  pieces: NodeRef[]
  code?: string
  message: string
}

export type ExplodeInstanceOutcome = ExplodeInstanceSuccess | ExplodeInstanceFailure

export function explodeInstanceOperand(wasmScene: WasmScene, instance: NodeRef): ExplodeInstanceOutcome {
  try {
    wasmScene.make_unique(instance.id)
  } catch (err) {
    const code = parseKernelErrorCode(err)
    const rawMsg = err instanceof Error ? err.message : String(err)
    return {
      ok: false,
      steps: 0,
      pieces: [],
      code: code ?? undefined,
      message: kernelErrorMessage(code ?? 'Unknown', rawMsg),
    }
  }

  let created: bigint[]
  try {
    created = Array.from(wasmScene.explode_instance(instance.id))
  } catch (err) {
    const code = parseKernelErrorCode(err)
    const rawMsg = err instanceof Error ? err.message : String(err)
    // make_unique committed — the instance (same handle, now on a private
    // definition) is still what's rendered in its place.
    return {
      ok: false,
      steps: 1,
      pieces: [instance],
      code: code ?? undefined,
      message: kernelErrorMessage(code ?? 'Unknown', rawMsg),
    }
  }

  if (created.length === 0) {
    return {
      ok: false,
      steps: 2,
      pieces: [],
      message: kernelErrorMessage('Unknown', 'exploding the component produced no solids'),
    }
  }

  const pieces = created.map((id): NodeRef => ({ kind: 'object', id }))
  if (created.length === 1) {
    return { ok: true, node: pieces[0], steps: 2 }
  }

  const sel = structuralSelection(pieces)
  if (sel === null) {
    // Defensive: every piece here is a freshly-created plain object, always
    // a valid kernel node id, so this should be unreachable — but the
    // exploded pieces are still real, ungrouped document state if it fires.
    return {
      ok: false,
      steps: 2,
      pieces,
      message: kernelErrorMessage('Unknown', 'could not fold the exploded pieces into a group'),
    }
  }
  try {
    const groupId = wasmScene.group_nodes(sel.kinds, sel.ids)
    return { ok: true, node: { kind: 'group', id: groupId }, steps: 3 }
  } catch (err) {
    const code = parseKernelErrorCode(err)
    const rawMsg = err instanceof Error ? err.message : String(err)
    return {
      ok: false,
      steps: 2,
      pieces,
      code: code ?? undefined,
      message: kernelErrorMessage(code ?? 'Unknown', rawMsg),
    }
  }
}

/**
 * The instance ids that must stay individually lit (materialized + full
 * opacity) for the current `activeContext` breadcrumb — passed to
 * `SceneRenderer.setActiveContext` as `litInstances`.
 *
 * Derived from the WHOLE chain, not just the deepest node (delta-review
 * Finding 2 / Finding 3 on component-edit-parity.md's original Finding 2
 * fix):
 * - Every 'instance' node ANYWHERE in the chain stays lit. The original fix
 *   only checked the deepest node, so double-clicking a definition member
 *   while already inside an instance context — `[..., instance, object]`,
 *   deepest is 'object' — lost the enclosing instance's highlight and the
 *   edited member dimmed right along with everything else.
 * - When the deepest node is a 'group', the group's own member INSTANCES
 *   light up too (recursively, through nested groups) — previously only
 *   leaf OBJECTS were lit for a group context (`activeLitSet`), so a
 *   component instance placed directly in the entered group rendered
 *   dimmed as if it were outside the context. Their def-sketch groups
 *   follow for free: `_applySketchIsolation` keys off this same set.
 *
 * Returns `null` (not an empty Set) when no instance qualifies, matching
 * the "no restriction" convention `SceneRenderer.setActiveContext` expects.
 */
export function computeLitInstances(wasmScene: WasmScene, activeContext: NodeRef[]): Set<bigint> | null {
  if (activeContext.length === 0) return null
  const lit = new Set<bigint>()
  for (const node of activeContext) {
    if (node.kind === 'instance') lit.add(node.id)
  }
  const deepest = activeContext[activeContext.length - 1]
  if (deepest.kind === 'group') {
    const { instanceIds } = collectLeafIds(deepest, (groupId) =>
      wasmScene.group_members(groupId).map(nodeRefFromJs),
    )
    for (const id of instanceIds) lit.add(id)
  }
  return lit.size > 0 ? lit : null
}

/**
 * Flattens a session's direct-member list (any node kind — a group frame's
 * members can be objects, nested groups, or instances; a component frame's
 * are always plain objects) into the leaf object/instance ids the renderer's
 * isolation fade and marquee/Select-All actually need, recursing into any
 * nested member group via `getGroupMembers` (`collectLeafIds`).
 *
 * `directMembers` itself must come from the kernel's own `session_members()`
 * (`Scene.session_members` — adversarial-review finding 2, group-session.md):
 * an earlier version derived a GROUP frame's scope app-side from a
 * `top_level_nodes()` snapshot taken right before `open_group_session`,
 * diffed against the live `top_level_nodes()` on every refresh. That
 * baseline was captured once, per group id, and never invalidated — so
 * undoing back into an EARLIER bracket of the same group's session (the
 * group closes and reopens, or history rewinds across the open boundary)
 * left the stale baseline in place, misattributing unrelated top-level
 * nodes that happened to appear after the ORIGINAL open into the reopened
 * session's scope. `session_members()` has no such staleness: it is
 * re-derived from the CURRENT document on every call, correct across any
 * undo/redo re-entry into any session bracket, for either frame kind (the
 * kernel dispatches by frame type internally — see `Document::session_direct_members`).
 */
export function flattenSessionScope(
  wasmScene: WasmScene,
  directMembers: NodeRef[],
): { objectIds: Set<bigint>; instanceIds: Set<bigint> } {
  const getGroupMembers = (gid: bigint): NodeRef[] =>
    Array.from(wasmScene.group_members(gid) ?? []).map(nodeRefFromJs)
  const objectIds = new Set<bigint>()
  const instanceIds = new Set<bigint>()
  for (const m of directMembers) {
    const { objectIds: os, instanceIds: is_ } = collectLeafIds(m, getGroupMembers)
    for (const id of os) objectIds.add(id)
    for (const id of is_) instanceIds.add(id)
  }
  return { objectIds, instanceIds }
}

/**
 * Pure decision core for `ViewportApi.runBoolean`: every wasm call already
 * made, every committed undo step already counted, so the UI shell (the
 * `runBoolean` closure inside the `Viewport` component) only has to turn
 * this into scene refresh / selection / toast side effects. Split out so
 * the accounting itself — exactly what delta-review finding 1 found
 * wrong — is exercised directly against a real compiled `Scene` in tests
 * without mounting the (WebGL-backed) `Viewport` component; see
 * `pickResolution.test.ts` for the same pattern with
 * `buildAncestorChain`/`resolvePickToSelectable`.
 */
export type BooleanCoreOutcome =
  | { kind: 'ok'; node: NodeRef; autoExploded: boolean }
  | { kind: 'refused'; code?: string; message: string }
  | {
      kind: 'mutated-failed'
      /** Every node now sitting in the document in place of a pre-explode
       *  instance operand — for `onSelectMany`. */
      settledNodes: NodeRef[]
      /** Total undo steps committed across both operands' auto-explode
       *  before the failure — exactly how many undos restore the original
       *  document (delta-review finding 2). */
      committedSteps: number
      /** True when both operands fully resolved and it was the RETRIED
       *  boolean itself that then failed; false when an operand's own
       *  auto-explode (make_unique/explode_instance/group_nodes) failed
       *  partway — drives which toast lead-in is accurate. */
      retryFailed: boolean
      /** Only meaningful when `retryFailed` is false: whether the OTHER
       *  operand had already been fully exploded before this one failed —
       *  the worst case (both operands were instances, one fully exploded,
       *  the other then failed), so the toast can honestly say one
       *  committed and is left sitting exploded while the other refused. */
      otherOperandExploded: boolean
      code?: string
      message: string
    }

export function runBooleanCore(
  wasmScene: WasmScene,
  op: number,
  a: NodeRef,
  b: NodeRef,
): BooleanCoreOutcome {
  // Operands are plain solids, whole groups, or (transparently exploded
  // below) component instances; the kernel composes group operands and owns
  // every eligibility rule (boolean_nodes, the group-ops design). kind:
  // 0=object, 1=group, 2=instance.
  const kindNum = (n: NodeRef) => (n.kind === 'group' ? 1 : n.kind === 'instance' ? 2 : 0)
  const attempt = (opA: NodeRef, opB: NodeRef): NodeRef =>
    nodeRefFromJs(wasmScene.boolean_nodes(op, kindNum(opA), opA.id, kindNum(opB), opB.id))

  let opA = a
  let opB = b
  try {
    const node = attempt(opA, opB)
    return { kind: 'ok', node, autoExploded: false }
  } catch (err) {
    const code = parseKernelErrorCode(err)
    if (code !== 'BooleanOperandHasInstance') {
      const rawMsg = err instanceof Error ? err.message : String(err)
      return { kind: 'refused', code: code ?? undefined, message: kernelErrorMessage(code ?? 'Unknown', rawMsg) }
    }

    // Scoped to an operand that IS an instance, not one nested inside a
    // group operand's subtree (the other way this refusal can fire) —
    // finding 4's auto-explode targets the direct engrave/emboss case.
    // `committedSteps` tracks "anything committed yet" across BOTH
    // operands (delta-review finding 1c: opA can fully explode — up to 3
    // committed steps — before opB's own explode then fails; that is
    // still a real, partial mutation, never a bare no-op).
    const settledNodes: NodeRef[] = []
    let committedSteps = 0
    let aExploded = false

    if (opA.kind === 'instance') {
      const outcome = explodeInstanceOperand(wasmScene, opA)
      committedSteps += outcome.steps
      if (!outcome.ok) {
        settledNodes.push(...outcome.pieces)
        if (committedSteps === 0) {
          return { kind: 'refused', code: outcome.code, message: outcome.message }
        }
        return {
          kind: 'mutated-failed',
          settledNodes,
          committedSteps,
          retryFailed: false,
          otherOperandExploded: false,
          code: outcome.code,
          message: outcome.message,
        }
      }
      opA = outcome.node
      settledNodes.push(outcome.node)
      aExploded = true
    }

    if (opB.kind === 'instance') {
      const outcome = explodeInstanceOperand(wasmScene, opB)
      committedSteps += outcome.steps
      if (!outcome.ok) {
        settledNodes.push(...outcome.pieces)
        if (committedSteps === 0) {
          return { kind: 'refused', code: outcome.code, message: outcome.message }
        }
        return {
          kind: 'mutated-failed',
          settledNodes,
          committedSteps,
          retryFailed: false,
          otherOperandExploded: aExploded,
          code: outcome.code,
          message: outcome.message,
        }
      }
      opB = outcome.node
      settledNodes.push(outcome.node)
    }

    if (committedSteps === 0) {
      // Neither operand was a direct instance — the refusal fired for a
      // reason this auto-explode doesn't cover (e.g. an instance nested
      // inside a group operand's subtree). Nothing committed, clean refusal.
      const rawMsg = err instanceof Error ? err.message : String(err)
      return { kind: 'refused', code, message: kernelErrorMessage(code, rawMsg) }
    }

    try {
      const node = attempt(opA, opB)
      return { kind: 'ok', node, autoExploded: true }
    } catch (err2) {
      // Unlike every refusal above, the explode(s) already committed real
      // mutations — the document now holds the exploded pieces, not the
      // pre-explode instance(s).
      const code2 = parseKernelErrorCode(err2)
      const rawMsg2 = err2 instanceof Error ? err2.message : String(err2)
      return {
        kind: 'mutated-failed',
        settledNodes,
        committedSteps,
        retryFailed: true,
        otherOperandExploded: false,
        code: code2 ?? undefined,
        message: kernelErrorMessage(code2 ?? 'Unknown', rawMsg2),
      }
    }
  }
}

export default function Viewport({
  wasmScene,
  onStatusChange,
  onInferenceChange,
  onSceneChange,
  onToast,
  onToolHint,
  onPrecisionChange,
  activeTool: activeToolProp,
  activeToolSeq: activeToolSeqProp,
  onInternalToolChange,
  activeContext = [],
  selectedIds = [],
  activeLitSet = null,
  onSelect,
  onSelectMany,
  onSelectGuide,
  selectedGuide = null,
  onSelectAnnotation,
  selectedAnnotation = null,
  onOpenAnnotationEditor,
  onEnterContext,
  onExitContext,
  onExitAllContexts,
  onSessionChange,
  onDocumentChanged,
  onSectionChanged,
  onHistoryChanged,
  apiRef,
  onMeasurement,
  onRescaleArmed,
  onCameraDragChange,
  onHoverSketchRegionChange,
  currentMaterialId = MATERIAL_SENTINEL,
  onProjectionChange,
  onToolReverted,
  onSampleMaterial,
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
  const onPrecisionChangeRef = useRef(onPrecisionChange)
  onPrecisionChangeRef.current = onPrecisionChange
  const onSelectRef = useRef(onSelect)
  onSelectRef.current = onSelect
  const onSelectManyRef = useRef(onSelectMany)
  onSelectManyRef.current = onSelectMany
  const onSelectGuideRef = useRef(onSelectGuide)
  onSelectGuideRef.current = onSelectGuide
  const onSelectAnnotationRef = useRef(onSelectAnnotation)
  onSelectAnnotationRef.current = onSelectAnnotation
  const onOpenAnnotationEditorRef = useRef(onOpenAnnotationEditor)
  onOpenAnnotationEditorRef.current = onOpenAnnotationEditor
  const onEnterContextRef = useRef(onEnterContext)
  onEnterContextRef.current = onEnterContext
  const onExitContextRef = useRef(onExitContext)
  onExitContextRef.current = onExitContext
  const onExitAllContextsRef = useRef(onExitAllContexts)
  onExitAllContextsRef.current = onExitAllContexts
  const onSessionChangeRef = useRef(onSessionChange)
  onSessionChangeRef.current = onSessionChange
  const onDocumentChangedRef = useRef(onDocumentChanged)
  onDocumentChangedRef.current = onDocumentChanged
  const onSectionChangedRef = useRef(onSectionChanged)
  onSectionChangedRef.current = onSectionChanged
  const onHistoryChangedRef = useRef(onHistoryChanged)
  onHistoryChangedRef.current = onHistoryChanged
  const apiRefRef = useRef(apiRef)
  apiRefRef.current = apiRef
  const onMeasurementRef = useRef(onMeasurement)
  onMeasurementRef.current = onMeasurement
  const onRescaleArmedRef = useRef(onRescaleArmed)
  onRescaleArmedRef.current = onRescaleArmed
  const onCameraDragChangeRef = useRef(onCameraDragChange)
  onCameraDragChangeRef.current = onCameraDragChange
  const onHoverSketchRegionChangeRef = useRef(onHoverSketchRegionChange)
  onHoverSketchRegionChangeRef.current = onHoverSketchRegionChange
  const onProjectionChangeRef = useRef(onProjectionChange)
  onProjectionChangeRef.current = onProjectionChange
  const onToolRevertedRef = useRef(onToolReverted)
  onToolRevertedRef.current = onToolReverted
  const onInternalToolChangeRef = useRef(onInternalToolChange)
  onInternalToolChangeRef.current = onInternalToolChange
  const onSampleMaterialRef = useRef(onSampleMaterial)
  onSampleMaterialRef.current = onSampleMaterial
  // Latest context path, readable inside the stable event closures.
  const activeContextRef = useRef<NodeRef[]>(activeContext)
  // This component's own belief about the open COMPONENT session
  // specifically — `null` unless the innermost session frame is one (a
  // component frame is always innermost, kernel-enforced) — resynced from
  // the kernel's own answer inside `applyHistoryChange` (the undo/redo
  // choke point) and `refreshSessionScope` (every open/close/commit) so a
  // session boundary crossed by undo/redo is reconciled rather than
  // assumed. Kept distinct from the generalized stack below because
  // `ViewportApi.explodeSessionInstance`/the "Editing <name>" status text
  // still mean exactly this one thing, unchanged by group sessions.
  const explodeSessionInstanceRef = useRef<bigint | null>(null)
  // The full open session stack, outermost first — mirrors the kernel's own
  // `session_stack()` (docs/design/group-session.md); resynced by
  // `refreshSessionScope`, the generalization of the old single-instance
  // resync. Empty when nothing is open.
  const sessionStackRef = useRef<NodeRef[]>([])
  // The innermost frame's current direct-member scope (any node kind),
  // nodeKey-keyed — the generalization of the old object-id-only session
  // scope now that a group frame's members can be groups/instances too.
  // Picking/selection/marquee/draw-eligibility all scope to exactly this
  // set while a session is open (ARCHITECTURE.md 2.4: inference/snapping is
  // a read-only query and stays unscoped; only EDIT targets are scoped
  // here). `null` when no session is open (no filtering). Exposed via
  // `ViewportApi.sessionMembers` as the flat NodeRef list (not just keys)
  // for the Outliner's session-header row.
  const sessionScopeKeysRef = useRef<Set<string> | null>(null)
  const sessionDirectMembersRef = useRef<NodeRef[] | null>(null)
  // The innermost frame's live scope flattened to LEAF object/instance ids
  // (recursing through any nested member group) — what the renderer's
  // isolation fade (`SceneRenderer.setActiveContext`) and marquee/Select-All
  // actually need to light/include. `explodeSessionObjectIdsRef` predates
  // groups (component members are always plain objects, so it used to be
  // exactly the session's own object scope); it now doubles as the group
  // case's flattened object-leaf set, and `sessionInstanceIdsRef` is new (a
  // component session never has instance members, so it holds an empty Set
  // for that kind — `refreshSessionScope` always writes a real Set for both,
  // never `null`, while a session is open; `_applyIsolation` treats an empty
  // set and `null` identically once `explodeSessionObjectIdsRef` is
  // non-null, so this is not a behavior change from the pre-unification
  // component-only `null`).
  const explodeSessionObjectIdsRef = useRef<Set<bigint> | null>(null)
  const sessionInstanceIdsRef = useRef<Set<bigint> | null>(null)
  // Per-group-id cache of what `runOpenGroupSession` captured right before
  // hiding the group: its display label, unreadable through `group_name`
  // once hidden (finding 2, group-session.md: this used to also cache a
  // top-level-node-key baseline for `computeGroupSessionScope`'s diff — the
  // baseline is gone, now that scope comes from the kernel's own live
  // `session_members()` instead). Entries are never evicted (closing a
  // session leaves a harmless stale entry; document-scoped, not a real leak)
  // — a fresh document clears the whole map (`notifyLoaded`).
  const groupSessionInfoRef = useRef<Map<bigint, { label: string }>>(new Map())
  // The last session stack pushed to the parent — dedupes the
  // onSessionChange callback now that the scope re-derives on every commit
  // while a session is open, not only at boundaries.
  const lastPushedSessionRef = useRef<{ node: NodeRef; label: string }[]>([])
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

  // The tool-switch effect's last-applied guard (see `shouldSkipToolSwitch`'s
  // doc comment): the `activeToolSeqProp` value as of the last run that
  // actually invoked `switchToolRef`, so a later run can tell an explicit
  // forced reapply (the seq bumped) apart from an echoed prop update that
  // merely re-reports a transition the tool controller already made.
  const lastAppliedToolSeqRef = useRef<number | undefined>(undefined)
  // The `toolName` argument `switchToolRef` was last invoked with — set on
  // EVERY invocation (prop-driven switch, internal handoff, Escape revert,
  // one-shot), inside `switchToolRef.current` itself below. This, NOT
  // `toolController.activeToolName`, is what the guard compares the
  // requested name against: Orbit/Pan/Zoom/Zoom Window/default all call
  // `toolController.resetToSelect()` (see the switch body), so the
  // controller reads 'Select' while one of those camera tools is active —
  // comparing against the controller's name made the guard fire for a
  // completely unrelated later switch TO 'Select' (e.g. the rail's Select
  // button while Orbit is active), skipping the entire switch body and
  // leaving the viewport wedged in camera mode while the rail showed
  // Select.
  const lastAppliedToolNameRef = useRef<string | undefined>(undefined)

  // Last pointer ray + viewport params cached so key-driven re-lock can
  // immediately re-resolve snap without waiting for the next pointer move.
  const lastRayRef = useRef<{ ray: import('./math').Ray; viewportH: number; basis: ApertureBasis } | null>(null)

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
    // The single resolved-theme read for this mount: `getResolvedTheme()`
    // queries `matchMedia` when the setting is 'auto', so it is read ONCE
    // here (not re-read at every one-time init call below) and kept current
    // by `subscribeTheme` below for the clear color/light rig/grid/axes,
    // which only need to react to an EXPLICIT Settings > Theme change (that
    // subscription's own event). The per-frame annotation billboards read
    // `readAppliedTheme()` directly instead (see the `updateAnnotationBillboards`
    // call below) — `subscribeTheme` alone would miss an OS-level
    // `prefers-color-scheme` flip while 'auto' is in effect, since that flip
    // never fires the settings-change subscription.
    let currentTheme: ResolvedTheme = getResolvedTheme()
    renderer.setClearColor(CANVAS_CLEAR_COLOR[currentTheme])
    renderer.setSize(el.clientWidth, el.clientHeight)
    el.appendChild(renderer.domElement)

    const threeScene = new THREE.Scene()

    // CameraRig owns BOTH a perspective and an orthographic camera and keeps
    // them pose-synchronized (docs/design/camera.md §1); `camera` is always
    // whichever is currently live (`rig.active`), reassigned by
    // `toggleProjection` below. Kept as its own binding (rather than every
    // call site reading `rig.active` directly) so the ~100 existing
    // raycasting/render/pose call sites throughout this effect needed NO
    // change beyond this declaration and the projection-specific ones this
    // effort converts explicitly.
    const rig = new CameraRig(el.clientWidth / el.clientHeight, MOUNT_LIMITS.near, MOUNT_LIMITS.far)
    let camera: THREE.PerspectiveCamera | THREE.OrthographicCamera = rig.active
    // Person-scale default: frames a ~2–3 m region; classic SketchUp 3/4 angle.
    // Distance ≈ 4.7 m; a 1.8 m figure reads as substantial, not dwarfed.
    // Scaled down for small-scale display units (cm/mm/inches imply a small
    // model — see homeFramingScale); the direction is always the same 3/4 view.
    // `HOME_EYE_OFFSET` (math.ts) is the single source `MOUNT_FIT_DISTANCE`
    // is also derived from — see its doc comment (delta-review Finding 3).
    const homeScale = homeFramingScale(getLengthUnit())
    rig.perspective.position.set(
      HOME_EYE_OFFSET[0] * homeScale,
      HOME_EYE_OFFSET[1] * homeScale,
      HOME_EYE_OFFSET[2] * homeScale,
    )
    rig.perspective.up.set(0, 0, 1)
    rig.perspective.lookAt(0, 0, 0)

    /**
     * The `ApertureBasis` (math.ts) every `snapService.resolve` call site
     * below builds from, instead of a raw `camera.fov` — projection-aware
     * (docs/design/camera.md §1, "Snap aperture"): a cone under perspective,
     * a constant-world-radius cylinder under parallel projection (no
     * distance parameter needed — `CameraRig.apertureBasis`'s own doc).
     */
    function apertureBasis(): ApertureBasis {
      return rig.apertureBasis(el.clientHeight)
    }

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
    const initialRig = MODEL_LIGHT_RIG[currentTheme]
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
    // `axesHalfLength` tracks the CURRENT (possibly rescaled — see
    // `applyRescaleToView`) half-length passed to every `buildOriginAxes`
    // rebuild, theme swaps included, so a rescale's lengthening survives a
    // later theme change instead of silently resetting to the default.
    let axesHalfLength = AXIS_HALF_LENGTH_DEFAULT
    let originAxes = buildOriginAxes(getResolvedTheme(), axesHalfLength)
    updateAxisResolution(originAxes, el.clientWidth, el.clientHeight)
    // Seed every other fat-line material (sketch edges, tool-preview
    // rubber-bands) at the initial canvas size too — mirrors the axes call
    // just above. Kept current on resize below; no longer walked every
    // render frame (see fatLine.ts's module doc comment).
    updateFatLineResolutions(el.clientWidth, el.clientHeight)
    threeScene.add(originAxes)
    const initialGridColors = GROUND_GRID_COLORS[currentTheme]
    // `originAxes.visible` (default true — nothing sets it false before
    // here) seeds the grid's own through-origin-line suppression so the two
    // start in sync; `setAxesVisible` below keeps them in sync from then on.
    const infiniteGrid = new InfiniteGrid(initialGridColors.ground, initialGridColors.minor, initialGridColors.major, originAxes.visible)
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

    // Shared dispose+rebuild used both by live theme reactivity (below) and
    // by `applyRescaleToView`: rebuilds `originAxes` at `theme` and whatever
    // `axesHalfLength` currently is (a rescale updates that variable BEFORE
    // calling this), preserving visibility across the rebuild either way.
    function rebuildOriginAxes(theme: 'light' | 'dark'): void {
      const wasVisible = originAxes.visible
      threeScene.remove(originAxes)
      disposeOriginAxes(originAxes)
      originAxes = buildOriginAxes(theme, axesHalfLength)
      updateAxisResolution(originAxes, el.clientWidth, el.clientHeight)
      originAxes.visible = wasVisible
      threeScene.add(originAxes)
    }

    // Live theme reactivity: Settings > Theme can change at any time while
    // the viewport is mounted, so the clear color, light rig, and ground
    // plane need to follow it without a reload — everything else in the app
    // (CSS variables) already updates live via `data-theme`.
    const unsubscribeTheme = subscribeTheme(() => {
      const theme = getResolvedTheme()
      currentTheme = theme
      renderer.setClearColor(CANVAS_CLEAR_COLOR[theme])
      const rig = MODEL_LIGHT_RIG[theme]
      ambient.intensity = rig.ambient
      dirLight.intensity = rig.directional
      const gridColors = GROUND_GRID_COLORS[theme]
      infiniteGrid.setColors(gridColors.ground, gridColors.minor, gridColors.major)
      rebuildOriginAxes(theme)
      // The edit-context dim opacity is theme-tuned too (component-edit-
      // parity.md Finding 2) — re-apply it so a context already active when
      // the toggle fires reads correctly right away.
      sceneRendererRef.current?.refreshThemeDependentIsolation()
      scheduleRenderRef.current()
    })

    // ------------------------------------------------------------------ scene renderer
    const sceneRenderer = new SceneRenderer(threeScene, wasmScene)
    sceneRendererRef.current = sceneRenderer
    // Initial refresh (empty scene is fine — just populates nothing)
    sceneRenderer.refresh()
    sceneRenderer.refreshGuides()
    sceneRenderer.refreshAnnotations()

    // Live unit reactivity: Settings > Units can change at any time while
    // the viewport is mounted, and every dimension's DISPLAYED measurement
    // is computed app-side from the current unit (docs/design/
    // dimensions-text.md — "changing units re-labels every dimension, like
    // SketchUp") — nothing else about the document changed, so nothing else
    // would otherwise trigger a re-render of the annotation text.
    const unsubscribeLengthUnit = subscribeLengthUnit(() => {
      sceneRenderer.refreshAnnotations()
      scheduleRenderRef.current()
    })

    // Non-destructive section plane (DESIGN §2) — a
    // three.js clip, not a kernel/document entity. Session-only for the
    // lifetime of this mounted viewport; never serialized, never undo-wired.
    // Requires local clipping on the renderer even before a section is ever
    // placed — the flag itself has no cost when no material sets
    // `clippingPlanes` (see WebGLRenderer.render's cheap early-out).
    renderer.localClippingEnabled = true
    const sectionManager = new SectionManager()

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

    /**
     * The session-scope nodeKey filter `resolvePickToSelectable` should
     * apply for a pick resolved against `ctx` — only at the TOP-LEVEL
     * resolution (`ctx` empty). Once an object/K1-K2-instance context is
     * already pushed on top of a session, `resolvePickToSelectableUnscoped`
     * resolves picks through ITS OWN in-context branches (definition
     * members, or "nothing" for an object context) — those nodes are never
     * top-level session members themselves, so applying the outer session's
     * top-level scope there would wrongly reject every in-context pick
     * whenever a session happens to sit beneath it (mutual exclusion no
     * longer holds now that an object context can coexist with a session).
     */
    function sessionScopeFor(ctx: NodeRef[]): Set<string> | null {
      return ctx.length === 0 ? sessionScopeKeysRef.current : null
    }

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
          return resolvePickToSelectable(
            wasmScene, objectId, activeContextRef.current, instanceId, sessionScopeFor(activeContextRef.current),
          )
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
      // Selection is scoped by `resolveSelectableRef`, so Shift-additive is
      // safe inside an editing context too. Component booleans require two
      // sibling definition members; disabling additive selection here made
      // that valid kernel operation unreachable from the real UI.
      const additive = selectAdditiveRef.current
      const ref = resolveSelectableRef(snap, ray, selectionDeps())
      onSelectRef.current?.(ref, additive)
      scheduleRender()
    }

    const toolController = new ToolController(wasmScene, handleSelect)
    toolControllerRef.current = toolController

    // Orbit/Pan/Zoom/Zoom Window are just OrbitControls `mouseButtons` remaps
    // (see the tool-switch below), not real Tool instances — so unlike every
    // geometry tool, none of them has an object to host a VCB buffer or a
    // meaningful `statusHint()`. `activeCameraTool` is the minimal state that
    // lets `reportToolHint` (just below) and Field of View typed entry (see
    // the "camera-mode tracking + FOV entry" section further down) work
    // anyway. Declared here (rather than down in that section) because
    // `reportToolHint` reads it synchronously on the very next line — this is
    // a `let`, not a hoisted `function`, so the read needs the declaration to
    // have already RUN, not just be hoisted.
    let activeCameraTool: 'Orbit' | 'Pan' | 'Zoom' | 'ZoomWindow' | null = null

    /** Eye height (meters) for Position Camera / Look Around / Walk
     * (docs/design/camera.md §4) — SESSION-shared across all three, VCB-
     * editable from any of them. A plain closure `let`, not per-tool state:
     * a fresh tool instance is constructed on every activation (the
     * `makeXTool()` convention this file already uses throughout), so the
     * value has to live above any single instance to actually persist. */
    let eyeHeightM = DEFAULT_EYE_HEIGHT_M

    // Live status-bar guidance: re-poll the active tool's stage hint after
    // every routed event (the wrapped listeners below) and on tool switches,
    // pushing CHANGES up — a string compare keeps the per-move cost trivial.
    let lastToolHint: string | null = null
    function reportToolHint(): void {
      // Camera tools (Orbit/Pan/Zoom/Zoom Window) park the controller on
      // Select — for Orbit/Pan/Zoom because OrbitControls owns the left
      // button (left-clicks navigate, they don't select); for Zoom Window
      // because it owns left-drag itself (see the Zoom Window section) —
      // either way Select's own hint would mislabel them. Report null and
      // let the status bar fall back to the camera tool's static
      // description. `activeCameraTool` (like `cameraModeRef`) is set
      // BEFORE resetToSelect() fires the tool-change listener, so this
      // reads the new mode.
      const hint = activeCameraTool !== null
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
      // While a session is open, Select All / marquee scope to exactly the
      // innermost frame's own scope — outside geometry must not be
      // marquee-selectable. Session-scoped nodes are always genuinely
      // top-level (a component session's members are baked to world
      // objects; a group session's are surfaced by the ungroup posture), so
      // this is a flat nodeKey-membership check against `top_level_nodes()`,
      // not a tree walk — and, unlike the old object-only check, now admits
      // group/instance direct members too (a group session's members can be
      // any node kind).
      const sessionScope = sessionScopeKeysRef.current
      const out: { node: NodeRef; leafObjects: bigint[]; leafInstances: bigint[] }[] = []
      for (const nj of wasmScene.top_level_nodes()) {
        const node = nodeRefFromJs(nj)
        if (sessionScope != null && !sessionScope.has(nodeKey(node))) continue
        const leaves = visibleLeaves(node)
        if (leaves === null) continue
        out.push({ node, ...leaves })
      }
      return out
    }

    /** Free-standing sketch refs (the kernel lists visible sketches only).
     *
     * Known scoping gap: unlike `visibleTopLevelCandidates`, this is NOT
     * filtered to an open COMPONENT session's own geometry via
     * `explode_session_sketches()` — the sketch analog of the object
     * scoping above, so a pre-existing, unrelated top-level sketch is not
     * selectable while a component session is open (a sketch drawn during
     * the session IS in the set: its creation attributes to the session,
     * and a fold-in extrude is meant to reach it). A GROUP session
     * deliberately does NOT restrict sketches at all — world sketches are
     * global (docs/design/group-session.md) — which this gets for free:
     * `explode_session_sketches()` answers `undefined` (no scoping) unless
     * the INNERMOST frame is specifically a component session. */
    function visibleSketchRefs(): NodeRef[] {
      const sessionSketches = wasmScene.explode_session_sketches()
      const inScope =
        sessionSketches === undefined ? null : new Set(Array.from(sessionSketches))
      return Array.from(wasmScene.sketch_ids())
        .filter((id) => inScope === null || inScope.has(id))
        .flatMap((id) =>
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
      const { snap } = snapService.resolve(ray, el.clientHeight, apertureBasis())
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
      const { snap } = snapService.resolve(ray, el.clientHeight, apertureBasis())
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
     *
     * An open session with no object context pushed on top keeps
     * `activeContext` empty (it edits at the top level), so the top-level
     * branch below is the one an in-session draw usually takes; scoping it
     * to the session's own scope keeps a draw tool from starting a fresh
     * sketch on a face OUTSIDE the session (the kernel would refuse the
     * eventual extrude/fold as `ExplodeSessionScope` — this keeps the draw
     * from ever starting).
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
      const resolved = resolvePickToSelectable(
        wasmScene, objectId, ctx, instanceId, sessionScopeFor(ctx),
      )
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

    // Annotation offset/leader drag (docs/design/dimensions-text.md "Tools
    // & UX": "drag moves the offset/text placement"). A press directly on
    // the picked annotation (see `pickAnnotation` below) arms this; past
    // `exceedsDragThreshold` (the same threshold `dragMove` uses) it goes
    // `active` and live-previews the new placement, committed in ONE kernel
    // call on release (never per-move — that would flood undo with one step
    // per mouse-move frame). A sub-threshold release is a plain click — the
    // annotation is already selected (done at arm time), nothing to commit.
    interface AnnotationDrag {
      id: bigint
      snapshot: AnnotationSnapshot
      startX: number
      startY: number
      active: boolean
    }
    let annotationDrag: AnnotationDrag | null = null
    let annotationDragPreview: THREE.LineSegments | null = null

    // The in-viewport text editor's pending target (docs/design/
    // dimensions-text.md): either an existing annotation's text being
    // edited (double-click), or a brand-new leader the Text tool just
    // placed but hasn't been worded yet. `ViewportApi.commitAnnotationEditorText`/
    // `cancelAnnotationEditor` resolve against this — the parent (App.tsx)
    // only ever passes typed TEXT through, never annotation internals.
    type PendingAnnotationEdit =
      | { kind: 'edit'; id: bigint; snapshot: AnnotationSnapshot }
      | { kind: 'new-leader'; leader: PlacedLeader }
    let pendingAnnotationEdit: PendingAnnotationEdit | null = null

    function _clearAnnotationDragPreview(): void {
      if (annotationDragPreview === null) return
      annotationDragPreview.geometry.dispose()
      if (annotationDragPreview.material instanceof THREE.Material) annotationDragPreview.material.dispose()
      previewGroup.remove(annotationDragPreview)
      annotationDragPreview = null
    }

    /** Resolve the world point the annotation's offset/leader should follow
     * for `ray`, projected onto the geometrically relevant plane: the
     * dimension's own plane (linear), the captured circle's plane (radial),
     * or a camera-facing plane through the anchor (leader — it has no
     * natural plane of its own, so it drags freely relative to the view,
     * matching a billboarded label). `null` only for a degenerate ray
     * (parallel to the plane — an edge-on view), which simply leaves the
     * preview at its last position. */
    function _resolveAnnotationDragPoint(snapshot: AnnotationSnapshot, ray: Ray): V3 | null {
      if (snapshot.kind === 'linear') {
        return rayPlaneIntersect(ray.origin, ray.direction, snapshot.a.point, [
          snapshot.plane[3], snapshot.plane[4], snapshot.plane[5],
        ])
      }
      if (snapshot.kind === 'radial') {
        return rayPlaneIntersect(ray.origin, ray.direction, snapshot.curveCenter, [
          snapshot.curvePlane[3], snapshot.curvePlane[4], snapshot.curvePlane[5],
        ])
      }
      camera.getWorldDirection(cameraForwardV)
      return rayPlaneIntersect(ray.origin, ray.direction, snapshot.anchor.point, [
        cameraForwardV.x, cameraForwardV.y, cameraForwardV.z,
      ])
    }

    /** Rebuild the drag preview line from `snapshot` with its offset/leader
     * replaced by the drag-derived one — a plain (non-fat) rubber-band line
     * in the shared preview group, matching every other tool's live-drag
     * preview (never the committed annotationsGroup rendering, which stays
     * on the pre-drag geometry until release). */
    function _updateAnnotationDragPreview(snapshot: AnnotationSnapshot, cursorWorld: V3): void {
      _clearAnnotationDragPreview()
      let positions: number[]
      if (snapshot.kind === 'linear') {
        const dir = subV3(snapshot.b.point, snapshot.a.point)
        const len = Math.hypot(dir[0], dir[1], dir[2])
        const offset = len > 1e-9
          ? perpComponentV3(subV3(cursorWorld, snapshot.a.point), [dir[0] / len, dir[1] / len, dir[2] / len])
          : snapshot.offset
        const a1 = addV3(snapshot.a.point, offset)
        const b1 = addV3(snapshot.b.point, offset)
        positions = [...snapshot.a.point, ...a1, ...snapshot.b.point, ...b1, ...a1, ...b1]
      } else if (snapshot.kind === 'radial') {
        const normal: V3 = [snapshot.curvePlane[3], snapshot.curvePlane[4], snapshot.curvePlane[5]]
        const leaderDir = perpComponentV3(subV3(cursorWorld, snapshot.anchor.point), normal)
        positions = [...snapshot.anchor.point, ...addV3(snapshot.anchor.point, leaderDir)]
      } else {
        const offset = subV3(cursorWorld, snapshot.anchor.point)
        positions = [...snapshot.anchor.point, ...addV3(snapshot.anchor.point, offset)]
      }
      const geo = new THREE.BufferGeometry()
      geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(positions), 3))
      const mat = new THREE.LineBasicMaterial({ color: 0x4d90ff, depthTest: false })
      annotationDragPreview = new THREE.LineSegments(geo, mat)
      previewGroup.add(annotationDragPreview)
    }

    /** Commit the drag's final placement (one kernel call, one undo step)
     * and clear the preview + drag state. */
    function _commitAnnotationDrag(id: bigint, snapshot: AnnotationSnapshot, cursorWorld: V3 | null): void {
      _clearAnnotationDragPreview()
      if (cursorWorld === null) return
      if (snapshot.kind === 'linear') {
        const dir = subV3(snapshot.b.point, snapshot.a.point)
        const len = Math.hypot(dir[0], dir[1], dir[2])
        if (len < 1e-9) return
        const offset = perpComponentV3(subV3(cursorWorld, snapshot.a.point), [dir[0] / len, dir[1] / len, dir[2] / len])
        commitAnnotationText(wasmScene, id, { ...snapshot, offset }, initialEditorText(snapshot))
      } else if (snapshot.kind === 'radial') {
        const normal: V3 = [snapshot.curvePlane[3], snapshot.curvePlane[4], snapshot.curvePlane[5]]
        const leaderDir = perpComponentV3(subV3(cursorWorld, snapshot.anchor.point), normal)
        commitAnnotationText(wasmScene, id, { ...snapshot, leaderDir }, initialEditorText(snapshot))
      } else {
        const offset = subV3(cursorWorld, snapshot.anchor.point)
        commitAnnotationText(wasmScene, id, { ...snapshot, offset }, snapshot.text)
      }
      sceneRenderer.refreshAnnotations()
      onDocumentChangedRef.current?.()
    }

    function _cancelAnnotationDrag(): void {
      _clearAnnotationDragPreview()
      annotationDrag = null
    }

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

    // ------------------------------------------------------------------ camera-mode tracking + FOV entry (camera.md §2, camera-playtest2.md §1-2)
    // Typed FOV entry (degrees or millimetres, fovUnits.ts) works WHENEVER
    // the Zoom camera mode is active — Camera ▸ Zoom / the Z shortcut / the
    // command palette all enter the same interactive Zoom tool, exactly
    // like SketchUp's own Zoom tool. There is no separate Field of View
    // menu entry (camera-playtest2.md §2 — Kurt's playtest call: FOV is
    // reachable only through Zoom, via typed entry or Shift-drag/wheel,
    // §3 below).
    let fovEntryBuffer: string | null = null

    /**
     * The VCB's resting (not-currently-typing) display while the Zoom
     * camera mode is active: the CURRENT fovDeg (both units — fovUnits.ts),
     * persistently — not only once typing starts (playtest finding 4b:
     * cursor FOV drags and a fresh Zoom activation left the VCB blank until
     * the user typed a digit). Blank whenever Zoom isn't active, or under
     * parallel projection (no lens to report). Never stomps a buffer that
     * IS actively being typed — every caller below clears `fovEntryBuffer`
     * first.
     */
    function refreshFovReadout(): void {
      if (fovEntryBuffer !== null) return
      onMeasurementRef.current?.(fovReadoutText(activeCameraTool, rig.projection, rig.perspective.fov))
    }

    /** Parses the typed buffer via `parseFovEntry` (fovUnits.ts — degrees:
     * "45"/"45deg"/"45°"; focal length: "50mm", converted through the
     * 18mm-half-frame law) and applies it via `rig.setFov`; anything
     * `parseFovEntry` can't parse is silently discarded (Enter on garbage
     * input just closes the VCB, like SketchUp). `rig.setFov` clamps to
     * [MIN_FOV_DEG, MAX_FOV_DEG] — `refreshFovReadout` below always shows
     * the CLAMPED value, not the raw typed number, so e.g. a typed "5mm"
     * visibly lands on 120°/10.4mm rather than silently doing nothing. */
    function commitFovEntry(): void {
      if (fovEntryBuffer === null) return
      const parsed = parseFovEntry(fovEntryBuffer)
      fovEntryBuffer = null
      if (parsed !== null) {
        rig.setFov(parsed.fovDeg)
        controls.update()
        scheduleRender()
      }
      refreshFovReadout()
    }

    function cancelFovEntry(): void {
      if (fovEntryBuffer === null) return
      fovEntryBuffer = null
      refreshFovReadout()
    }

    /**
     * Routes a keydown to the FOV typed-entry buffer while the Zoom camera
     * mode is active. Returns true when the key was consumed (the caller
     * should treat it like a tool's own `capturesKey`/`onKey` handling —
     * `scheduleRender()` and stop further processing) — the shape a real
     * Tool would use, but Zoom has no Tool instance of its own to host it
     * (see the module doc above).
     */
    function handleFovEntryKey(ev: KeyboardEvent): boolean {
      if (activeCameraTool !== 'Zoom') return false
      // Field of View is a no-op under parallel projection (design §2) — a
      // parallel camera has no fov to set. The Zoom tool itself stays
      // reachable under parallel (Z / Camera ▸ Zoom is still a valid dolly
      // gesture there), so this must ALSO refuse to open a typed-digit
      // buffer here — otherwise typing while parallel silently overwrites
      // the persisted perspective fov with no visible feedback (parallel
      // ignores it), surfacing later as an unexplained camera jump the next
      // time the user toggles back to perspective.
      if (rig.projection === 'parallel') return false
      if (ev.key === 'Enter') {
        if (fovEntryBuffer === null) return false
        commitFovEntry()
        ev.preventDefault()
        return true
      }
      if (ev.key === 'Escape') {
        if (fovEntryBuffer === null) return false
        cancelFovEntry()
        ev.preventDefault()
        return true
      }
      if (ev.key === 'Backspace') {
        if (fovEntryBuffer === null) return false
        fovEntryBuffer = fovEntryBuffer.slice(0, -1)
        onMeasurementRef.current?.(fovEntryBuffer)
        ev.preventDefault()
        return true
      }
      // A leading digit or '.' opens the buffer; once open, letters (deg/
      // mm), the degree glyph, and an internal space are also accepted —
      // mirrors SketchUp's typed-degree VCB entry, extended for
      // camera-playtest2.md §1's "45deg or 50mm" spellings. A bare
      // letter/space/° can never START a buffer (`fovEntryBuffer === null`
      // gate on the second branch) — only digits/'.' can.
      if (
        /^[0-9.]$/.test(ev.key) ||
        (fovEntryBuffer !== null && /^[a-z°]$/i.test(ev.key)) ||
        (fovEntryBuffer !== null && ev.key === ' ')
      ) {
        fovEntryBuffer = (fovEntryBuffer ?? '') + ev.key
        onMeasurementRef.current?.(fovEntryBuffer)
        ev.preventDefault()
        return true
      }
      return false
    }

    // ------------------------------------------------------------------ Zoom Window (camera.md §3)
    // A one-shot rectangle-drag camera mode (Camera ▸ Zoom Window / command
    // palette): reuses the marquee's rubber-band visuals, but reframes the
    // camera on release instead of selecting, then always springs back to
    // Select (`onToolRevertedRef`) — the drag-to-move one-shot's precedent,
    // except that gesture never actually LEAVES Select at the app-tool-state
    // level the way this one does (see `onToolReverted`'s doc comment).
    // `zoomWindowActive` is set true only while this specific camera mode is
    // engaged (the tool-switch below) and checked at the very top of
    // onPointerDown/Move/Up so it pre-empts both OrbitControls' native
    // handling (mouseButtons.LEFT stays null for this mode) and the normal
    // Select/geometry routing.
    let zoomWindowActive = false
    // Arm/drag decision logic lives in zoomWindowDrag.ts (pure, unit-tested —
    // see its doc comment); this closure only owns the DOM/pointer-capture
    // side effects and the actual camera reframe below.
    let zoomWindowDrag: ZoomWindowDragState | null = null

    function beginZoomWindowDrag(ev: PointerEvent): void {
      const [px, py] = canvasPoint(ev)
      zoomWindowDrag = beginZoomWindowDragState(px, py)
      // Track the drag even when it leaves the canvas.
      renderer.domElement.setPointerCapture(ev.pointerId)
    }

    /** Abandon an in-flight Zoom Window drag (Esc, focus loss) — reverts the
     * rectangle, no reframe. One-shot semantics preserved: this only cancels
     * the DRAG, not the surrounding camera mode — `zoomWindowActive` stays
     * true, so the mode remains armed for another attempt, mirroring
     * `clearMarquee` (aborting a marquee doesn't leave Select either); only a
     * COMPLETED drag or an explicit tool switch leaves Zoom Window. */
    function abortZoomWindowDrag(): void {
      zoomWindowDrag = null
      marqueeOverlay.style.display = 'none'
    }

    function updateZoomWindowDrag(ev: PointerEvent): void {
      if (zoomWindowDrag === null) return
      const [px, py] = canvasPoint(ev)
      const { state, rect } = updateZoomWindowDragState(zoomWindowDrag, px, py, (ev.buttons & 1) !== 0)
      if (state === null) {
        // The release happened outside our listeners (focus loss) — drop it,
        // no reframe (mirrors clearMarquee's equivalent path).
        abortZoomWindowDrag()
        return
      }
      zoomWindowDrag = state
      if (rect !== null) {
        // Always the "window" (solid) rubber-band styling — Zoom Window has
        // no crossing-selection analogue, so drag direction carries no
        // separate meaning the way it does for the Select marquee.
        marqueeOverlay.style.border = '1px solid #4a90e2'
        marqueeOverlay.style.left = `${rect.minX}px`
        marqueeOverlay.style.top = `${rect.minY}px`
        marqueeOverlay.style.width = `${rect.maxX - rect.minX}px`
        marqueeOverlay.style.height = `${rect.maxY - rect.minY}px`
        marqueeOverlay.style.display = 'block'
      }
    }

    function finishZoomWindowDrag(ev: PointerEvent): void {
      if (zoomWindowDrag === null) return
      const drag = zoomWindowDrag
      zoomWindowDrag = null
      marqueeOverlay.style.display = 'none'
      const [px, py] = canvasPoint(ev)
      const rect = finishZoomWindowDragRect(drag, px, py)
      if (rect !== null) applyZoomWindow(rect)
      // One-shot: always springs back to Select, whether the release
      // resolved to a real drag or was just a plain (sub-threshold) click.
      onToolRevertedRef.current?.()
    }

    /**
     * Reframe the camera onto `rect` (canvas CSS pixels) — Camera ▸ Zoom
     * Window's commit (design §3):
     *   - New `controls.target`: the existing snap/pick chain through the
     *     rect center, falling back to the CURRENT target's depth plane
     *     along the new ray when nothing is under it (a miss over open space
     *     still re-centers sensibly instead of aborting the zoom).
     *   - Perspective scales distance by `max(rectW/vpW, rectH/vpH)`;
     *     parallel scales the orthographic frustum by the same factor
     *     (`CameraRig.scaleOrthoFrustum`) — the eye direction is unchanged
     *     either way.
     */
    function applyZoomWindow(rect: MarqueeRect): void {
      const vpW = el.clientWidth
      const vpH = el.clientHeight
      if (vpW <= 0 || vpH <= 0) return
      const cx = (rect.minX + rect.maxX) / 2
      const cy = (rect.minY + rect.maxY) / 2
      const ndcX = (cx / vpW) * 2 - 1
      const ndcY = -(cy / vpH) * 2 + 1
      const ray = makeWorldRay(ndcX, ndcY, camera)

      const { snap } = snapService.resolve(ray, vpH, apertureBasis())
      const rayOrigin = new THREE.Vector3(ray.origin[0], ray.origin[1], ray.origin[2])
      const rayDir = new THREE.Vector3(ray.direction[0], ray.direction[1], ray.direction[2])
      let newTarget: THREE.Vector3
      if (snap !== null) {
        newTarget = new THREE.Vector3(snap.x, snap.y, snap.z)
      } else {
        const viewDir = new THREE.Vector3()
        camera.getWorldDirection(viewDir)
        const denom = viewDir.dot(rayDir)
        const t = Math.abs(denom) > 1e-9
          ? viewDir.dot(controls.target.clone().sub(rayOrigin)) / denom
          : controls.getDistance()
        newTarget = rayOrigin.clone().addScaledVector(rayDir, t)
      }

      const scale = Math.max((rect.maxX - rect.minX) / vpW, (rect.maxY - rect.minY) / vpH)
      if (!(scale > 0)) return

      const oldDistance = controls.getDistance()
      const dir = new THREE.Vector3().subVectors(camera.position, controls.target).normalize()
      controls.target.copy(newTarget)

      if (rig.projection === 'perspective') {
        const newDist = Math.max(oldDistance * scale, controls.minDistance)
        camera.position.copy(newTarget).addScaledVector(dir, newDist)
      } else {
        camera.position.copy(newTarget).addScaledVector(dir, oldDistance)
        rig.scaleOrthoFrustum(scale)
      }
      camera.updateProjectionMatrix()
      controls.update()
      scheduleRender()
    }

    // ------------------------------------------------------------------ Shift+Zoom fov drag/wheel (camera-playtest2.md §3)
    // While the Zoom camera mode is active in perspective, a left-drag
    // begun with Shift held adjusts fov instead of dollying (eye fixed);
    // Shift+wheel does the same in smaller steps. The multiplicative law
    // and the fixed-at-press mode decision are pure/unit-tested in
    // fovDrag.ts — this section owns only the DOM side effects: pre-empting
    // OrbitControls, pointer capture, the cursor swap, and restoring
    // `MOUSE.DOLLY` on every exit path (release, Escape, focus loss,
    // pointer-capture loss, a tool switch away from Zoom).
    let fovDragState: FovDragState | null = null
    // The pointerId that armed the current `fovDragState` (finding 4,
    // camera-playtest2 review) — `updateFovDrag` only accepts events from
    // THIS pointer. Without it, a second concurrent pointer (a stray touch,
    // a second mouse-like device) moving while the drag is armed would drive
    // `rig.setFov` off ITS position instead of the arming pointer's, even
    // though it never went through the press-time mode decision at all.
    let fovDragPointerId: number | null = null

    /** Restore the Zoom tool's normal dolly binding AND cursor. Every exit
     * path from a Shift-fov gesture calls this (directly or via
     * `abortFovDrag`/`finishFovDrag`) — a leaked `mouseButtons.LEFT ===
     * null` here would silently stop Zoom's left-drag from dollying at
     * all, the same class of abort-path mode leak this branch has already
     * shipped once (see fovDrag.ts's module doc). No-op when Zoom isn't
     * the active camera tool — some callers (a tool switch, a projection
     * change) run this defensively after Zoom may already have been left,
     * and the switch's own per-case logic is what sets `mouseButtons.LEFT`
     * (and the cursor) in that case. `touches.ONE` (finding 2) shares the
     * exact same lifecycle and is restored to OrbitControls' own default
     * (`TOUCH.ROTATE` — the only value this file ever sets it to) right
     * alongside it. */
    function restoreZoomDollyBinding(): void {
      if (activeCameraTool !== 'Zoom') return
      controls.mouseButtons.LEFT = THREE.MOUSE.DOLLY
      controls.touches.ONE = THREE.TOUCH.ROTATE
      // Shift may still be held when the gesture ends (e.g. a plain
      // release with Shift still down) — show its cursor, not the plain
      // Zoom one, so the user isn't told Shift stopped doing anything.
      renderer.domElement.style.cursor = cursorFor(shiftFovCursorActive ? 'Zoom Window' : 'Zoom')
    }

    /** Abandon an in-flight Shift-fov drag with no commit — Escape, focus
     * loss, or pointer-capture loss (see the listeners wired below). The
     * fov stays wherever the drag last left it (matching Zoom Window's own
     * "abort cancels the GESTURE, not any already-applied change"
     * precedent — there is no un-set to roll back to, unlike Zoom Window's
     * rectangle).
     *
     * `pointerId`, when given, scopes the abort to the pointer that ARMED
     * the drag (findings 1+2, camera-playtest2 DELTA review): a
     * pointer-specific exit — release, cancel, or pointer-capture loss —
     * belongs to ONE DOM pointer, but `onPointerUp`/`onPointerCancel`/the
     * `lostpointercapture` listener are plain listeners on the canvas that
     * fire for EVERY pointer (capture only redirects the CAPTURING
     * pointer's OWN move/up events elsewhere; it does not stop other
     * pointers' events from reaching the same listener). Without this,
     * some OTHER pointer's own release/cancel/capture-loss — touch and pen
     * report `button === 0` too — would silently end a gesture it never
     * armed, and worse than ending it cleanly: `updateFovDrag` was already
     * scoped to the arming pointer, so the arming pointer's FURTHER
     * movement after the wrong end would be silently dropped (no further
     * fov change) while OrbitControls can't pick the gesture back up
     * either, since `mouseButtons.LEFT`/`touches.ONE` were never restored
     * mid-hold — the drag freezes rather than ending or handing off.
     * Callers with no pointer of their own to scope to — window blur, a
     * tool switch, a projection rebind, Escape — omit the argument and get
     * the old unconditional abort, which IS correct there: those are
     * program-level exits (not one pointer's own event) that must end the
     * gesture regardless of which pointer armed it. */
    function abortFovDrag(pointerId?: number): void {
      if (fovDragState === null) return
      if (pointerId !== undefined && pointerId !== fovDragPointerId) return
      fovDragState = null
      fovDragPointerId = null
      restoreZoomDollyBinding()
    }

    /** A completed drag/release — same cleanup as abort, kept as a
     * separately-named entry point so call sites read as what they mean
     * (release vs. abort), even though today they do the same thing. */
    function finishFovDrag(pointerId?: number): void {
      abortFovDrag(pointerId)
    }

    function updateFovDrag(ev: PointerEvent): void {
      if (fovDragState === null) return
      // Only the pointer that armed this drag may drive it (finding 4,
      // camera-playtest2 review) — a second concurrent pointer's moves are
      // simply ignored, exactly as if the gesture weren't watching them at
      // all; they neither advance nor abort the armed drag.
      if (ev.pointerId !== fovDragPointerId) return
      if ((ev.buttons & 1) === 0) {
        // The release happened outside our listeners (focus loss /
        // pointer-capture loss) — drop it, mirroring every other drag
        // state in this file (clearMarquee, abortDragMove,
        // updateZoomWindowDrag's own buttonsDown check).
        abortFovDrag()
        return
      }
      const [, py] = canvasPoint(ev)
      rig.setFov(fovDragValue(fovDragState, py))
      controls.update()
      // Readout tracks every tick (design §3) — refreshFovReadout already
      // refuses to stomp an in-progress typed buffer.
      refreshFovReadout()
      scheduleRender()
    }

    /**
     * Decides + arms a Shift-fov drag BEFORE OrbitControls' own pointerdown
     * handler runs — this is NOT the same as the `zoomWindowActive` check
     * at the top of `onPointerDown` below, and can't be: that check works
     * because `mouseButtons.LEFT` is already `null` from a PRIOR tool
     * switch by the time any press happens. Here the fov-vs-dolly decision
     * itself must be made from `ev.shiftKey` on THIS press, and
     * OrbitControls registers its own pointerdown listener directly on
     * `renderer.domElement` at construction time (`three.js`'s `connect()`)
     * — earlier than this component's own `onPointerDown` (wired up near
     * the end of this effect). Per the DOM spec, listeners on the SAME
     * target fire in REGISTRATION order regardless of the capture flag, so
     * a capture-phase listener added later on `renderer.domElement` itself
     * would NOT run first. Registering on `el` (the canvas's parent)
     * instead runs this during the CAPTURE phase, strictly before the
     * event ever reaches the canvas target — the same trick
     * `onCameraPointerDown` already relies on (window-level capture,
     * camera.md §1's `cameraDragActive` plumbing) — so by the time
     * OrbitControls reads `mouseButtons.LEFT`, it already sees `null` and
     * takes its own no-mapped-action path (no state change, no capture
     * conflict: it still calls `setPointerCapture` on the same element,
     * which is a harmless no-op once we already hold it).
     */
    function onFovDragPointerDownCapture(ev: PointerEvent): void {
      if (ev.button !== 0) return
      if (fovDragState !== null || zoomWindowActive) return
      if (activeCameraTool !== 'Zoom' || rig.projection !== 'perspective') return
      // The fixed-at-press mode decision (fovDrag.ts) — read from `ev.shiftKey`
      // exactly once, right here, and never re-evaluated for the rest of this
      // gesture (see decideFovDragMode's doc and the module doc's "MODE FIXED
      // AT PRESS" section for why).
      if (decideFovDragMode(ev.shiftKey) !== 'fov') return
      // A new fov drag supersedes any uncommitted typed VCB entry (finding 1,
      // camera-playtest2 review): `refreshFovReadout` refuses to overwrite an
      // open `fovEntryBuffer`, so without this, the readout would show stale
      // typed digits for the entire drag (a real camera change happening
      // underneath a frozen display) and a later Enter would silently discard
      // the whole drag by re-applying that stale typed value on top of it.
      // `cancelFovEntry` is the Escape-like discard (not a commit) — exactly
      // right here: the drag is a NEW gesture, not a completion of the typed
      // one. The invariant this establishes: the readout never shows a value
      // the camera does not actually have.
      cancelFovEntry()
      const [, py] = canvasPoint(ev)
      fovDragState = beginFovDrag(py, rig.perspective.fov)
      fovDragPointerId = ev.pointerId
      controls.mouseButtons.LEFT = null
      // Touch routes through `controls.touches` (never `mouseButtons`) —
      // three.js's OrbitControls dispatches touchstart/touchmove through a
      // wholly separate internal state machine keyed off `touches.ONE`/`TWO`
      // (see OrbitControls.js's `onTouchStart`), so nulling `mouseButtons.LEFT`
      // alone leaves a Shift-held single-finger drag free to ALSO rotate the
      // camera natively — the eye moving is exactly what this feature promises
      // it won't do (finding 2, camera-playtest2 review). Pre-empt it the same
      // way, on the same capture-phase listener, restored alongside
      // `mouseButtons.LEFT` in `restoreZoomDollyBinding`. (A pen contact needs
      // no separate handling: OrbitControls routes any non-'touch' pointerType,
      // pen included, through the SAME `mouseButtons`-driven onMouseDown path
      // as the mouse — see onPointerDown's `event.pointerType === 'touch'`
      // branch — so it is already covered by the null above.)
      controls.touches.ONE = null
      renderer.domElement.setPointerCapture(ev.pointerId)
      renderer.domElement.style.cursor = cursorFor('Zoom Window')
    }

    /** Shift+wheel (design §3): same law, smaller steps, no press involved
     * — pre-empts OrbitControls' own wheel-dolly listener the same way as
     * the pointerdown capture above (registered on `el`, capture phase,
     * ahead of OrbitControls' bubble-phase listener on the canvas). */
    function onFovWheelCapture(ev: WheelEvent): void {
      if (fovDragState !== null || zoomWindowActive) return
      if (activeCameraTool !== 'Zoom' || rig.projection !== 'perspective' || !ev.shiftKey) return
      ev.preventDefault()
      ev.stopPropagation()
      // Same finding-1 fix as the drag's own pointerdown: a wheel tick is
      // just as much a new gesture as a drag press, and shares the exact
      // same stale-readout/silently-discarded-change bug shape if a typed
      // buffer is left open underneath it.
      cancelFovEntry()
      rig.setFov(fovAfterWheel(rig.perspective.fov, ev.deltaY))
      controls.update()
      refreshFovReadout()
      scheduleRender()
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
      // Every kernel-mutating path funnels through here (tool commits,
      // boolean/group/delete, undo, redo, harness ops) — a transform/
      // boolean/delete on a node can geometrically re-anchor or DETACH a
      // live annotation (docs/design/dimensions-text.md), so this is the
      // one place that guarantees the annotation render always follows,
      // rather than pairing `refreshAnnotations()` at every call site
      // individually (guides' `refreshGuides()` precedent — annotations
      // need the SAME breadth of coverage guides never did, since guides
      // are never re-anchored).
      sceneRenderer.refreshAnnotations()
      // While an explode session is open, every commit can grow the
      // session's scope (drawing a new solid mid-session folds in at close,
      // so it is session geometry from birth) — re-derive the pick-scope
      // set and dimming here at the shared commit choke point, not only at
      // the open/close/undo boundaries (delta-review finding: a solid
      // drawn mid-session was selectable in the Outliner but invisible to
      // viewport click/marquee/Select-All until the next boundary).
      if (sessionStackRef.current.length > 0) {
        refreshSessionScope()
      }
      scheduleRender()
      // Re-poll the sketch hover probe against the cursor's last known
      // position right away so a stationary mouse across the mutation
      // doesn't leave the contextual dock showing a stale context (see
      // `reevaluateHoverNow`'s doc comment).
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
      // (addGuideLine/addGuidePoint/deleteGuide); keep their overlay faithful
      // too (annotations are covered inside handleSceneRefresh itself).
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

    // Imperative command surface for the parent. Editing INSIDE a component
    // instance's own definition (component-edit-parity.md phase A2) routes
    // through `boolean_in_component` instead of the world `boolean_nodes` —
    // both operands are definition members, replaced in place within that
    // SAME definition. A definition member is never itself a component
    // instance, so the auto-explode retry below does not apply on this path.
    // At the world level (`editCtx.kind !== 'instance'`), all the wasm calls
    // and undo-step accounting for the auto-explode-instance-operand retry
    // (playtest finding 4) live in the pure, module-level `runBooleanCore`
    // (above `Viewport` in this file) — this wrapper only turns its outcome
    // into the side effects real usage needs (scene refresh, selection,
    // toast), which is exactly the part testing the accounting itself
    // doesn't need to exercise.
    function runBoolean(op: number, a: NodeRef, b: NodeRef): NodeRef | null | 'mutated-failed' {
      const editCtx = computeEditContext(wasmScene, activeContextRef.current)
      if (editCtx.kind === 'instance') {
        let result: NodeRef
        try {
          result = { kind: 'object', id: wasmScene.boolean_in_component(editCtx.component, op, a.id, b.id) }
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

      const outcome = runBooleanCore(wasmScene, op, a, b)

      if (outcome.kind === 'refused') {
        handleToast(outcome.message, outcome.code)
        return null
      }

      if (outcome.kind === 'mutated-failed') {
        // The auto-explode already committed real mutations — the document
        // now holds whatever survived (exploded pieces for whichever
        // operand(s) finished, or the still-present-but-now-unique instance
        // for one that didn't — delta-review finding 1). Refresh everything
        // the success path below refreshes, push the settled nodes up as
        // the new selection (so the outliner and viewport match the
        // document instead of any stale reference), and toast an honest
        // recovery hint naming what committed, what failed, and exactly how
        // many undos restore the pre-explode state (finding 2). The caller
        // (App's handleBoolean) must treat this like a committed mutation —
        // bump its revision counter — even though there is no single result
        // node to select.
        handleSceneRefresh()
        sceneRenderer.refreshAllSketches()
        sceneRenderer.refreshGuides()
        onSelectManyRef.current?.(outcome.settledNodes, false)
        scheduleRender()
        const undoHint =
          outcome.committedSteps === 1
            ? 'undo once to restore it'
            : `undo ${outcome.committedSteps} times to restore it`
        const lead = outcome.retryFailed
          ? 'Component exploded to solids, but the boolean then failed'
          : outcome.otherOperandExploded
            ? 'One component could not be fully exploded for the boolean (the other was, and is left exploded)'
            : 'Component could not be fully exploded for the boolean'
        handleToast(`${lead}: ${outcome.message} — ${undoHint}.`, outcome.code)
        return 'mutated-failed'
      }

      handleSceneRefresh()
      sceneRenderer.refreshAllSketches()
      sceneRenderer.refreshGuides()
      onSelectRef.current?.(outcome.node, false)
      scheduleRender()
      if (outcome.autoExploded) {
        handleToast('Component exploded to solids for the boolean.')
      }
      return outcome.node
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
     * Quietly close the active tool's post-commit windows, if any: Move's/
     * Rotate's armed ×N / /N array-copy window, and Tape Measure's idle
     * retroactive-rescale recall (tape-measure-rework part 2). Explicit
     * document commands — delete, undo, redo — are deliberate and must
     * execute, but they end either kind of refinement: without this, the
     * array window's keyboard capture outlives it (Delete silently no-ops,
     * bare-letter tool shortcuts feed a stale VCB buffer until Esc), and a
     * post-mutation recall would keep offering to resize the model against
     * world points that may no longer describe anything real. Only the
     * ambiguous bare Delete/Backspace KEYSTROKE stays guarded upstream by
     * capturingInput — see MoveTool/RotateTool's capturingInput / disarmArray.
     */
    function disarmActivePostCommitWindow(): void {
      const activeTool = toolController.activeTool
      if ('disarmArray' in activeTool) {
        (activeTool as { disarmArray(): void }).disarmArray()
      }
      if ('forgetRecall' in activeTool) {
        (activeTool as { forgetRecall(): void }).forgetRecall()
      }
    }

    function runDelete(nodes: NodeRef[]): void {
      if (nodes.length === 0) return
      // An explicit delete is deliberate and executes — but first disarm the
      // array window so no hot state points at the deleted copies.
      disarmActivePostCommitWindow()
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
      // Editing INSIDE a component instance's own definition (component-
      // edit-parity.md phase A2): every 'object' node in this delete routes
      // through `delete_def_member` instead — see the loop below.
      const editCtx = computeEditContext(wasmScene, activeContextRef.current)
      const activeComponent = editCtx.kind === 'instance' ? editCtx.component : null
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
        const stillListed =
          Array.from(wasmScene.sketch_ids()).includes(sketch) ||
          (
            activeComponent !== null &&
            Array.from(wasmScene.component_member_sketches(activeComponent)).includes(sketch)
          )
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
            const stillListed =
              Array.from(wasmScene.sketch_ids()).includes(n.sketch) ||
              (
                activeComponent !== null &&
                Array.from(wasmScene.component_member_sketches(activeComponent)).includes(n.sketch)
              )
            if (stillListed && wasmScene.sketch_lines(n.sketch).length === 0) {
              wasmScene.delete_sketch(n.sketch)
            }
          } else if (n.kind === 'sketch') {
            wasmScene.delete_sketch(n.id)
          } else if (n.kind === 'object' && activeComponent !== null) {
            // Editing INSIDE a component instance's own definition (component-
            // edit-parity.md phase A2): an 'object' node here is a definition
            // member — `delete_def_member` splices it out of the definition
            // (undo restores it) instead of the world `delete_node`, and
            // refuses typed (`LastDefinitionMember`) rather than delete a
            // definition's only member out from under every instance.
            wasmScene.delete_def_member(activeComponent, n.id)
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

    // ------------------------------------------------------- session stack
    // Open/close a session frame: a COMPONENT frame bakes an instance's
    // definition members into world-owned geometry (same ObjectIds — a
    // move, not a copy); a GROUP frame applies the ungroup posture (direct
    // members surface to the world top level, the group hides) — either way
    // the ordinary, unmodified tool set edits the exposed geometry; closing
    // folds everything back (docs/design/group-session.md). The kernel
    // enforces a component frame is always innermost, so nesting is
    // group→group→…→optionally one component at the very end.
    // `sessionStackRef`/`sessionScopeKeysRef`/`explodeSessionObjectIdsRef`/
    // `sessionInstanceIdsRef` are this component's own belief about session
    // state, resynced from the kernel's own answer by `refreshSessionScope`
    // — called from every point session state can change (open, close,
    // every commit while a session is open, and `applyHistoryChange`, the
    // undo/redo choke point) — so a session boundary crossed by undo/redo
    // is reconciled rather than assumed.

    /** Display label for a COMPONENT frame ("Editing <name>" / breadcrumb):
     *  instance name, falling back to its definition's name, falling back to
     *  a positional placeholder — the same preference `resolveLabel` (the
     *  Outliner's own label resolver) applies, at index 0 since a transient
     *  status chip has no outliner position to match. */
    function componentSessionLabel(instanceId: bigint): string {
      // Resolve the definition through the SESSION, not the instance: the
      // entered instance is hidden for the session's duration, so
      // `instance_def(instanceId)` answers undefined and the label fell
      // back to the positional "Component 1" even for a named definition
      // (playtest finding).
      const component = wasmScene.explode_session_component()
      const defName = component !== undefined ? wasmScene.component_name(component) : undefined
      return resolveLabel(wasmScene.instance_name(instanceId), defName, 'instance', 0)
    }

    /** Whether two labeled session-stack arrays are the same (identity +
     *  label at every position) — dedupes the `onSessionChange` push the
     *  same way the old single-session `lastPushedSessionRef` compare did. */
    function sameSessionStack(
      a: { node: NodeRef; label: string }[],
      b: { node: NodeRef; label: string }[],
    ): boolean {
      if (a.length !== b.length) return false
      return a.every((f, i) => nodeEq(f.node, b[i].node) && f.label === b[i].label)
    }

    /** Whether two RAW session stacks (no labels) name the SAME sequence of
     *  frames — `refreshSessionScope`'s rescale-recall invalidation
     *  (finding 6) fires whenever this is false: a genuine open/close/
     *  undo-redo boundary crossed, as opposed to a same-identity refresh
     *  (a mid-session fold-in growing the innermost frame's scope). */
    function sameSessionStackNodes(a: NodeRef[], b: NodeRef[]): boolean {
      if (a.length !== b.length) return false
      return a.every((n, i) => nodeEq(n, b[i]))
    }

    /** Re-derive the whole session-stack belief from the kernel's own answer
     *  and push both the isolation dimming and the parent-facing stack
     *  callback — the ONE place that keeps all of it in lockstep, called
     *  from every point session state can change (open, close, undo/redo
     *  resync, and every commit while a session is open — a mid-session
     *  fold-in grows the innermost group frame's scope without any
     *  open/close boundary). Isolation dimming reuses the exact mechanism
     *  the K1/K2 editing contexts already use
     *  (`SceneRenderer.setActiveContext`); an object context can now sit ON
     *  TOP of an open session (design: object pushes are allowed while a
     *  session is open), so this only writes to the renderer when
     *  `activeContext` is currently empty — otherwise the
     *  `[activeContext, activeLitSet]` effect below is the sole writer, and
     *  stomping its narrower (object-scoped) isolation here would undo the
     *  drill-down. */
    /** The live session-scope object/instance/sketch id sets Tape Measure's
     *  scoped-rescale arm gate needs (finding 6, group-session.md) — `null`
     *  when no session is open. Sketch scope is only tracked for a COMPONENT
     *  frame (`explode_session_sketches()`, kernel truth); a GROUP frame has
     *  no app-side "sketches created since open" tracking (the design
     *  itself defers this for the rescale's own sketch-scaling step), so a
     *  sketch-edge measurement inside an open group session always reads as
     *  out of scope — a documented, conservative gap: it can produce a
     *  false decline, never the wrong-anchor rescale the gate exists to
     *  prevent. */
    function sessionScopeForTapeMeasure(): SessionScopeIds | null {
      const innermost = sessionStackRef.current[sessionStackRef.current.length - 1]
      if (innermost === undefined) return null
      const sketchIds = innermost.kind === 'instance'
        ? new Set(Array.from(wasmScene.explode_session_sketches() ?? []))
        : new Set<bigint>()
      return {
        objectIds: explodeSessionObjectIdsRef.current ?? new Set<bigint>(),
        instanceIds: sessionInstanceIdsRef.current ?? new Set<bigint>(),
        sketchIds,
      }
    }

    function refreshSessionScope(): void {
      const prevStack = sessionStackRef.current
      const stack = Array.from(wasmScene.session_stack()).map(nodeRefFromJs)
      sessionStackRef.current = stack
      const innermost = stack.length > 0 ? stack[stack.length - 1] : null
      explodeSessionInstanceRef.current = innermost?.kind === 'instance' ? innermost.id : null

      if (innermost === null) {
        sessionScopeKeysRef.current = null
        sessionDirectMembersRef.current = null
        explodeSessionObjectIdsRef.current = null
        sessionInstanceIdsRef.current = null
      } else {
        // Kernel truth for the innermost frame's live direct members, either
        // kind (finding 2, group-session.md) — `Scene.session_members()`
        // dispatches by frame type internally and is re-derived from the
        // CURRENT document on every call, correct across any undo/redo
        // re-entry into any session bracket (replaces the old GROUP-only
        // `computeGroupSessionScope` open-time-baseline diff, which went
        // stale across exactly that kind of re-entry).
        const membersJs = wasmScene.session_members()
        const directMembers = membersJs !== undefined ? Array.from(membersJs).map(nodeRefFromJs) : []
        const { objectIds, instanceIds } = flattenSessionScope(wasmScene, directMembers)
        sessionDirectMembersRef.current = directMembers
        sessionScopeKeysRef.current = new Set(directMembers.map(nodeKey))
        explodeSessionObjectIdsRef.current = objectIds
        sessionInstanceIdsRef.current = instanceIds
      }

      if (activeContextRef.current.length === 0) {
        sceneRenderer.setActiveContext(explodeSessionObjectIdsRef.current, sessionInstanceIdsRef.current)
      }

      // Tape Measure scoped-rescale gating (finding 6): push the fresh
      // scope to the active tool on every refresh, if it's Tape Measure — a
      // mid-session fold-in (new geometry drawn/booleaned/grouped into the
      // open session) must read as in-scope immediately, not just at
      // open/close. A genuine stack IDENTITY change (open, close, or an
      // undo/redo boundary crossed — not a same-frame member-set change)
      // additionally drops the tool's rescale recall memory: a recalled
      // measurement's saved world points can otherwise arm a scoped resize
      // whose anchor belonged to a session that is no longer (or not yet)
      // the one now open.
      if (toolController.activeTool instanceof TapeMeasureTool) {
        const tapeMeasure = toolController.activeTool
        tapeMeasure.setSessionScope(sessionScopeForTapeMeasure())
        if (!sameSessionStackNodes(prevStack, stack)) tapeMeasure.forgetRecall()
      }

      const labeled = stack.map((node) => ({
        node,
        label: node.kind === 'instance'
          ? componentSessionLabel(node.id)
          : groupSessionInfoRef.current.get(node.id)?.label ?? resolveLabel(undefined, undefined, 'group', 0),
      }))
      if (!sameSessionStack(lastPushedSessionRef.current, labeled)) {
        lastPushedSessionRef.current = labeled
        onSessionChangeRef.current?.(labeled)
      }
    }

    /** Open a COMPONENT session on `instanceId`. Returns whether it
     *  succeeded (a toast already reported a typed refusal). Exposed on
     *  `ViewportApi` for direct callers (the test harness); the dblclick
     *  gesture goes through `openExplodeSessionOrFallback` below instead,
     *  since IT needs to distinguish `ExplodeSessionPoseUnsupported` from
     *  every other refusal. */
    function runOpenExplodeSession(instanceId: bigint): boolean {
      try {
        wasmScene.open_explode_session(instanceId)
      } catch (err) {
        const code = parseKernelErrorCode(err)
        const rawMsg = err instanceof Error ? err.message : String(err)
        handleToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
        return false
      }
      handleSceneRefresh()
      refreshSessionScope()
      return true
    }

    /**
     * The double-click gesture's own open attempt: a plain double-click on a
     * component instance opens an explode session; if the kernel gates it —
     * `ExplodeSessionPoseUnsupported` (non-uniform scale or a mirror) or
     * `ExplodeSessionGroupedInstance` (a placement of the definition is
     * nested in a group, which a session's hide-every-placement bake cannot
     * honor) — fall back SILENTLY to the ordinary "enter this instance's
     * edit context" gesture (K1/K2), which remains the editing model for
     * exactly those instances. Any other refusal surfaces as the usual
     * kernel-error toast and enters neither. Exposed on `ViewportApi` too
     * (`runOpenExplodeSessionOrFallback`) so the entry-convergence helper
     * gets the identical fallback behavior reaching a component frame
     * through the Outliner/dock instead of a direct viewport double-click.
     */
    function openExplodeSessionOrFallback(instanceId: bigint, fallback: NodeRef): void {
      try {
        wasmScene.open_explode_session(instanceId)
      } catch (err) {
        const code = parseKernelErrorCode(err)
        if (code === 'ExplodeSessionPoseUnsupported' || code === 'ExplodeSessionGroupedInstance') {
          onEnterContextRef.current?.(fallback)
          return
        }
        const rawMsg = err instanceof Error ? err.message : String(err)
        handleToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
        return
      }
      handleSceneRefresh()
      refreshSessionScope()
    }

    function runCloseExplodeSession(): boolean {
      if (explodeSessionInstanceRef.current === null) return false
      try {
        wasmScene.close_explode_session()
      } catch (err) {
        const code = parseKernelErrorCode(err)
        const rawMsg = err instanceof Error ? err.message : String(err)
        handleToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
        return false
      }
      handleSceneRefresh()
      refreshSessionScope()
      return true
    }

    /** Open a GROUP session on `groupId` — captures its display label BEFORE
     *  the kernel hides the group, since `group_name` isn't readable once it
     *  does (scope itself now comes from the kernel's own live
     *  `session_members()` inside `refreshSessionScope`, not an app-side
     *  snapshot — finding 2, group-session.md). */
    function runOpenGroupSession(groupId: bigint): boolean {
      const label = resolveLabel(wasmScene.group_name(groupId), undefined, 'group', 0)
      try {
        wasmScene.open_group_session(groupId)
      } catch (err) {
        const code = parseKernelErrorCode(err)
        const rawMsg = err instanceof Error ? err.message : String(err)
        handleToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
        return false
      }
      groupSessionInfoRef.current.set(groupId, { label })
      handleSceneRefresh()
      refreshSessionScope()
      return true
    }

    function runCloseGroupSession(): boolean {
      const innermost = sessionStackRef.current[sessionStackRef.current.length - 1]
      if (innermost === undefined || innermost.kind !== 'group') return false
      try {
        wasmScene.close_group_session()
      } catch (err) {
        const code = parseKernelErrorCode(err)
        const rawMsg = err instanceof Error ? err.message : String(err)
        handleToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
        return false
      }
      handleSceneRefresh()
      refreshSessionScope()
      return true
    }

    /** Close the INNERMOST open session frame regardless of kind — the
     *  Escape / double-click-outside gesture (one frame per gesture) and
     *  the entry-convergence helper's "close below the divergence" step.
     *  Single kernel call: `close_innermost_session` dispatches to whichever
     *  specific close applies, so the recording stays replay-exact without
     *  the app re-deriving which kind is innermost itself. */
    function runCloseInnermostSession(): boolean {
      if (sessionStackRef.current.length === 0) return false
      try {
        wasmScene.close_innermost_session()
      } catch (err) {
        const code = parseKernelErrorCode(err)
        const rawMsg = err instanceof Error ? err.message : String(err)
        handleToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
        return false
      }
      handleSceneRefresh()
      refreshSessionScope()
      return true
    }

    // The confirmation modal's Confirm/Cancel resolve into whichever
    // TapeMeasureTool instance is currently active — the overlay blocks
    // further viewport interaction while it's up, so the active tool cannot
    // change out from under it, but re-resolving through `toolController`
    // (rather than caching the instance at arm time) costs nothing and
    // degrades safely (a silent no-op) if it somehow did.
    function confirmPendingRescale(scoped: boolean, scopeLabel: string | null): void {
      const at = toolController.activeTool
      if (at instanceof TapeMeasureTool) at.confirmRescale(scoped, scopeLabel)
    }
    function cancelPendingRescale(): void {
      const at = toolController.activeTool
      if (at instanceof TapeMeasureTool) at.cancelRescale()
    }

    function notifyLoaded(): void {
      // A new/loaded document replaced the Scene — any explode session the
      // PREVIOUS scene had open is meaningless against this one (`Scene.save()`
      // always serializes as-if-closed, so a freshly loaded document never
      // legitimately has one open; "New Document" discarding an open session
      // in the old one is the other way this can be reached). Same "new
      // document, clean view state" rationale as the section-plane reset
      // below — reset the belief directly rather than routing through
      // `runCloseInnermostSession` (which would try to fold state back into
      // a Scene that is already gone).
      explodeSessionInstanceRef.current = null
      sessionStackRef.current = []
      sessionScopeKeysRef.current = null
      sessionDirectMembersRef.current = null
      explodeSessionObjectIdsRef.current = null
      sessionInstanceIdsRef.current = null
      groupSessionInfoRef.current.clear()
      lastPushedSessionRef.current = []
      onSessionChangeRef.current?.([])
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
      // A section is session view state derived from the PREVIOUS model's
      // geometry (raw coordinates, not a kernel handle) — carrying it over
      // to an unrelated freshly-loaded document would leave a confusing,
      // arbitrarily-placed cut. Not a DESIGN requirement, just the
      // obvious "new document, clean view state" behavior every other
      // session overlay already gets implicitly (guides/hidden sets key off
      // ids that simply don't exist in the new document).
      sectionManager.delete()
      sceneRenderer.setSectionPlane(null)
      onSectionChangedRef.current?.()
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
      // Notify the active tool its own cached descriptions of committed
      // kernel geometry may now be stale (`onHistoryChanged`, `types.ts`) —
      // this function is the single choke point both `runUndo` and
      // `runRedo` route through, so every undo/redo entry point reaches it.
      const historyChangedTool = toolController.activeTool
      if ('onHistoryChanged' in historyChangedTool) {
        (historyChangedTool as { onHistoryChanged(): void }).onHistoryChanged()
      }
      // Undo/redo can cross a session's open/close boundary — push/pop BOTH
      // frame kinds now (docs/design/group-session.md) — so resync the
      // WHOLE stack from the kernel's own answer rather than assuming it
      // tracks the last runOpen*/runClose* call, then push the scoping/
      // dimming/breadcrumb update the same way an explicit open/close does:
      // undoing past a close visually re-enters that frame, undoing past an
      // open exits it.
      //
      // An object context can legitimately sit on top of a session now (an
      // undo/redo that leaves the stack ITSELF unchanged must not disturb
      // it — the common case, e.g. undoing an ordinary push/pull while both
      // a group session and an object context are open). Only clear it when
      // the STACK'S OWN identity actually changed underneath it (adversarial-
      // review finding, generalized): an undo/redo that opens/closes a
      // frame invalidates whatever the object context assumed was
      // "logically inside the innermost frame" — the same "kernel wins"
      // doctrine the resync itself follows.
      const prevStack = sessionStackRef.current
      const stackNow = Array.from(wasmSceneRef.current.session_stack()).map(nodeRefFromJs)
      const stackChanged = prevStack.length !== stackNow.length ||
        prevStack.some((n, i) => !nodeEq(n, stackNow[i]))
      if (stackChanged && activeContextRef.current.length > 0) {
        onExitAllContextsRef.current?.()
      }
      refreshSessionScope()
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
        disarmActivePostCommitWindow()
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
        // Mirror runUndo — see disarmActivePostCommitWindow.
        disarmActivePostCommitWindow()
        try {
          applyHistoryChange(wasmSceneRef.current.scene_redo())
          onHistoryChangedRef.current?.()
        } catch (err) {
          console.warn('[Viewport] scene_redo failed:', err)
        }
      }
    }

    /**
     * Rescales EVERY world-length view quantity that must stay in lockstep
     * with the camera's far-clip — `near`/`far`/`minDistance`/`maxDistance`
     * (`scaleViewLimits`), the ground grid's plane footprint
     * (`InfiniteGrid.scaleAboutOrigin`), and the origin axes' half-length
     * (`axesHalfLength` + `rebuildOriginAxes`) — by the SAME `ratio`
     * (new-far / current-far, or an equivalent already-scaled-consistently
     * ratio; see the two callers below for how each derives it).
     *
     * This is the ONE place all three call sites that change `far` route
     * through (`applyRescaleToView`, `zoomExtents`, `setStandardView`), so
     * the "grid/axes stay comfortably beyond the frustum's far clip"
     * invariant holds BY CONSTRUCTION on every path that changes `far`,
     * instead of needing to be re-established ad hoc at each one
     * (delta-review Finding 1: `zoomExtents`' resync moved `far` to a fresh,
     * ground-truth value without this, permanently desyncing the grid/axes
     * from it — visibly, on any never-rescaled scene with model radius
     * greater than roughly 2.7 m, since `far` there overshoots the fixed
     * 150 m axis half-length).
     *
     * Deliberately does NOT touch the section plane — see
     * `applyRescaleToView`, the one caller that also needs to move it, for
     * why scaling it is conditional on THAT caller alone.
     *
     * A ratio of 1 is a no-op fast path (also avoids a spurious axes
     * rebuild when nothing actually changed).
     */
    function syncWorldLengthViewState(ratio: number): void {
      // A degenerate frame (a point-like visible bounding box gives fit
      // distance 0, hence ratio 0; a previously zeroed far gives Infinity)
      // must never multiply into the view state: unlike the absolute
      // assignment this ratio form replaced, a single 0 or NaN would
      // poison every later multiplicative sync unrecoverably.
      if (ratio === 1 || !Number.isFinite(ratio) || ratio <= 0) return
      const limits = scaleViewLimits(
        { near: camera.near, far: camera.far, minDistance: controls.minDistance, maxDistance: controls.maxDistance },
        ratio,
      )
      camera.near = limits.near
      camera.far = limits.far
      controls.minDistance = limits.minDistance
      controls.maxDistance = limits.maxDistance
      camera.updateProjectionMatrix()

      infiniteGrid.scaleAboutOrigin(ratio)
      axesHalfLength *= ratio
      rebuildOriginAxes(getResolvedTheme())
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
      // `perspectiveFramingDistance` uses the rig's (persisted-across-toggle)
      // perspective fov regardless of the ACTIVE projection — both
      // projections place the camera at the same distance from the target;
      // only parallel projection additionally needs its frustum sized
      // (design §1, "Framing"), since ortho's apparent size doesn't track
      // distance at all.
      const halfDiag = box.getBoundingSphere(new THREE.Sphere()).radius
      const distance = rig.perspectiveFramingDistance(halfDiag, 1.2)

      // Re-derive the view limits from THIS fit distance (delta-review
      // Finding 1) — the scene bounding box just measured is the one piece
      // of ground truth that is never stale, unlike `minDistance`/
      // `maxDistance`/`near`/`far`, which a rescale scales but an UNDO of
      // that rescale never restores (camera/view state is intentionally
      // outside undo — see `applyRescaleToView`'s doc comment). Left at
      // their old scaled values, they can permanently floor how far Zoom
      // Extents can dolly back in, compounding across repeated rescales.
      // Assigning fresh limits here — proportional to the SAME fit distance
      // the re-pose below uses — makes Zoom Extents the universal recovery
      // action at any model scale, and must happen BEFORE `controls.update()`
      // clamps the eye→target distance, exactly like `applyRescaleToView`.
      //
      // Routed through the shared `syncWorldLengthViewState` (delta-review
      // Finding 1) rather than assigning `limits` directly: the fresh
      // `far` this resync computes must carry the grid footprint and axes
      // half-length along with it in the same lockstep, or a big enough
      // fit distance pushes `far` past the fixed axes/grid extent and their
      // ends become visible mid-scene. The ratio is `far`'s own before/after
      // — `scaleViewLimits` applied to the CURRENT (possibly already
      // rescaled) limits at that ratio reproduces the exact same absolute
      // `limits` `zoomExtentsViewLimits` derives from ground truth, because
      // every path that has touched these limits already scaled all four
      // fields by one consistent factor from `MOUNT_LIMITS` — so recovering
      // via a ratio is exactly as self-healing here as the absolute
      // recompute it replaces, while also being the form the grid/axes need.
      const limits = zoomExtentsViewLimits(distance)
      const ratio = limits.far / camera.far
      syncWorldLengthViewState(ratio)

      // Keep the current view direction; re-target at box center.
      const dir = new THREE.Vector3()
      dir.subVectors(camera.position, controls.target).normalize()
      controls.target.copy(center)
      camera.position.copy(center).addScaledVector(dir, distance)
      if (rig.projection === 'parallel') {
        rig.frameOrthoToRadius(halfDiag, 1.2, el.clientWidth / el.clientHeight)
      }
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
      let radius = 0
      if (box.isEmpty()) {
        // Nothing visible to derive a fit distance from — preserve the
        // current framing verbatim, same as the pre-fix behavior. There is
        // no ground-truth fit distance here to resync the limits against,
        // so (like `zoomExtents`' own early return on an empty box) this
        // branch skips `syncWorldLengthViewState` entirely rather than
        // resyncing against a distance that doesn't describe the model.
        center.copy(controls.target)
        distance = controls.getDistance()
      } else {
        box.getCenter(center)
        radius = box.getBoundingSphere(new THREE.Sphere()).radius
        distance = rig.perspectiveFramingDistance(radius, 1.2)

        // Resync the view limits from this same fit distance, exactly like
        // `zoomExtents` (delta-review Finding 2) — Top/Front/Iso duplicated
        // the framing pipeline but not the resync, so after a rescale+undo
        // the standard views kept clamping at the stale limits even though
        // `zoomExtents` had already recovered. Same shared helper, same
        // ratio derivation.
        const limits = zoomExtentsViewLimits(distance)
        const ratio = limits.far / camera.far
        syncWorldLengthViewState(ratio)
      }

      const eye = new THREE.Vector3(spec.eye[0], spec.eye[1], spec.eye[2]).normalize()
      camera.up.set(0, 0, 1)
      controls.target.copy(center)
      camera.position.copy(center).addScaledVector(eye, distance)
      // Parallel path keeps the eye direction (above) and sizes the frustum
      // from the box radius instead of distance (design §1) — only when
      // there IS a box to frame; an empty scene keeps the current frustum
      // size, matching the perspective branch's "keep current distance".
      if (rig.projection === 'parallel' && !box.isEmpty()) {
        rig.frameOrthoToRadius(radius, 1.2, el.clientWidth / el.clientHeight)
      }
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
      // Test-pinning helper (__hew_test.setCamera): always forces perspective
      // — deterministic framing for E2E/pixel tests shouldn't depend on
      // whatever projection a PRIOR test left the rig in. Routed through the
      // SAME rebindControlsForProjectionChange helper the interactive
      // Camera ▸ Parallel Projection toggle uses (below) instead of
      // reassigning `camera` inline — forcing the rig's projection without
      // also rebuilding OrbitControls left `controls` bound to the
      // now-inactive camera, permanently freezing input (OrbitControls
      // binds to one camera for its whole lifetime; a projection change
      // can't just mutate `.object`).
      if (rig.projection !== 'perspective') {
        rig.toggleProjection(controls.target)
        rebindControlsForProjectionChange()
      }
      rig.perspective.position.set(position[0], position[1], position[2])
      controls.target.set(target[0], target[1], target[2])
      rig.perspective.up.set(up[0], up[1], up[2])
      rig.setFov(fovDeg)
      controls.update()
      scheduleRender()
    }

    function captureFrame(): { width: number; height: number; pixels: Uint8Array } {
      // Mirror the per-frame camera-dependent updates of the animation loop
      // (this renders out-of-band, without going through it) so a captured
      // frame is exactly what the loop would put on screen for this pose.
      const effDist = rig.projection === 'parallel' ? rig.effectiveDistance(controls.getDistance()) : null
      infiniteGrid.update(camera.position, effDist)
      updateOriginAxesFrame(originAxes, wasmScene)
      clampOriginAxes(originAxes, rig, el.clientHeight)
      renderer.render(threeScene, camera)
      const gl = renderer.getContext()
      const width = gl.drawingBufferWidth
      const height = gl.drawingBufferHeight
      const pixels = new Uint8Array(width * height * 4)
      gl.readPixels(0, 0, width, height, gl.RGBA, gl.UNSIGNED_BYTE, pixels)
      return { width, height, pixels }
    }

    /**
     * Tape Measure's "resize the model?" (design tool-parity §3): after
     * `rescale_document(factor)` commits, the kernel has scaled every
     * object/sketch/guide about the WORLD ORIGIN, but every view-side
     * world-length quantity stayed at its OLD (pre-rescale) value — left
     * staring at the OLD framing, with view limits sized for the OLD model.
     * Scaling the eye/target about that SAME origin by the SAME factor
     * (`scaleCameraAboutOrigin`) keeps the apparent view IDENTICAL: the
     * model just "is" the new size, at the same apparent angular size,
     * because camera-to-target distance grows/shrinks with it. But that
     * alone isn't sufficient — every OTHER world-length view quantity must
     * move in the same lockstep, or the rescale re-introduces exactly the
     * kind of "old framing" mismatch this function exists to prevent, just
     * one level down:
     *
     *  - `controls.minDistance`/`maxDistance` (`scaleViewLimits`) — left
     *    fixed, a big enough factor pins the eye→target distance at the old
     *    `maxDistance` instead of the intended (scaled) distance, and Zoom
     *    Extents can't recover either (same clamp).
     *  - `camera.near`/`far` (`scaleViewLimits`, then
     *    `updateProjectionMatrix()`) — left fixed, `far` far-clips most of
     *    the now-much-bigger model; on a SHRINK factor, `near` must shrink
     *    too or it near-clips a model that now sits entirely closer than
     *    the old fixed `near`. This also protects an UNDO of the rescale:
     *    camera moves are outside undo by design (see below), so undoing
     *    restores the pre-rescale model but leaves the camera limits at
     *    their scaled values — which is exactly what keeps the restored
     *    model inside the frustum instead of the old (comparatively tiny)
     *    `far` far-clipping it into a blank viewport.
     *  - the ground grid's plane footprint (`InfiniteGrid.scaleAboutOrigin`)
     *    and the origin axes' half-length (`axesHalfLength` +
     *    `rebuildOriginAxes`) — both fixed world lengths sized to stay
     *    comfortably beyond the camera's far-clip at construction; left
     *    unscaled, a big enough factor leaves the grid's edge or the axes'
     *    endpoints visible inside the frustum instead of running off it.
     *    (Shared with `zoomExtents`/`setStandardView` via
     *    `syncWorldLengthViewState` — see its doc comment.)
     *  - the active section plane's `origin` (`rescaleSectionPlane`) — a
     *    session-only, app-side position the kernel rescale never touches;
     *    left unscaled, an active cut lands at the wrong position in the
     *    now-rescaled model. `normal` is a direction, not a position, and is
     *    left unchanged. This is deliberately handled HERE ONLY, not inside
     *    `syncWorldLengthViewState`: the plane is anchored to the MODEL, and
     *    this is the one caller where the model itself actually changed
     *    size. `zoomExtents`/`setStandardView` call the same shared helper
     *    to recover from stale grid/axes/limits on a mere REFRAME — the
     *    model never moved in either, so scaling the plane there would slide
     *    an active cut away from the (unchanged) geometry it's slicing,
     *    corrupting it for a reason that has nothing to do with a reframe.
     *
     * All of this is a view-side adjustment only — none of it has an undo
     * of its own (camera moves, the grid, the axes, and the section plane
     * are all outside undo by design in this app), so it is applied on the
     * forward commit only; undoing the rescale does not restore any of
     * these to their pre-rescale values. There is no generic hook to do so:
     * `runUndo`/`runRedo` resolve through the kernel's `DocChange`, which
     * carries no op-kind signal that would let them recognize "this undo
     * was specifically a rescale" without new kernel plumbing this fix
     * doesn't add.
     */
    function applyRescaleToView(factor: number): void {
      const { eye, target } = scaleCameraAboutOrigin(
        [camera.position.x, camera.position.y, camera.position.z],
        [controls.target.x, controls.target.y, controls.target.z],
        factor,
      )
      // Limits/grid/axes BEFORE the pose: `controls.update()` below clamps
      // the eye→target distance to [minDistance, maxDistance] — scale those
      // first via the shared sync helper, or the very re-pose this function
      // exists to do gets clamped right back to the old (unscaled) bound.
      // A document rescale's own factor already IS the far ratio this
      // helper wants (unlike `zoomExtents`/`setStandardView`, which derive
      // theirs from a freshly-measured fit distance).
      syncWorldLengthViewState(factor)
      camera.position.set(eye[0], eye[1], eye[2])
      controls.target.set(target[0], target[1], target[2])
      controls.update()

      // The section plane is model-anchored — scale it here, and ONLY
      // here (see this function's doc comment for the asymmetry vs
      // `zoomExtents`/`setStandardView`, which never touch it).
      const plane = sectionManager.current
      if (plane !== null) {
        sectionManager.setPlane(rescaleSectionPlane(plane, factor))
        sceneRenderer.setSectionPlane(sectionManager.current)
        onSectionChangedRef.current?.()
      }
    }

    function getCamera(): {
      position: [number, number, number]
      target: [number, number, number]
      fovDeg: number
    } {
      return {
        position: [camera.position.x, camera.position.y, camera.position.z],
        target: [controls.target.x, controls.target.y, controls.target.z],
        fovDeg: rig.perspective.fov,
      }
    }

    /**
     * The camera's full working view, for document-save persistence
     * (docs/design/camera.md §5) — unlike the test-only `getCamera`
     * (always perspective-shaped, reads `controls.target` directly), this
     * covers BOTH projections and derives `target` fresh from the live
     * camera pose rather than trusting `controls.target`, which goes stale
     * while a walkthrough tool (Position Camera/Look Around/Walk, design
     * §4) has OrbitControls disabled and never touches it mid-gesture.
     *
     * The synthesized `target`'s DISTANCE from `eye` is deliberately NOT
     * `CameraRig.effectiveDistance()` — that normalizes against a fixed
     * REFERENCE fov (45°) for a different purpose entirely (keeping guide-
     * dash/grid LOD continuous across a projection toggle) and is the
     * WRONG quantity to persist: `applyCameraState`'s restore re-derives
     * the ortho frustum via `CameraRig.toggleProjection`, whose own
     * `matchOrthoToPerspective` sizes it as `distance · tan(halfFov)` using
     * the camera's ACTUAL fov, not the reference one. Feeding an
     * effectiveDistance-scaled target back through that math would size the
     * restored ortho frustum off by `tan(actualFov/2)/tan(45°/2)` — this
     * computes the matching inverse directly instead: under parallel, the
     * exact distance `matchOrthoToPerspective` needs to reproduce the
     * CURRENT ortho frustum height when this state is later restored and
     * re-toggled; under perspective, the plain orbit distance (which
     * IS what a perspective→parallel toggle already uses verbatim).
     */
    function getCameraState(): {
      projection: Projection
      fovDeg: number
      eye: [number, number, number]
      target: [number, number, number]
      up: [number, number, number]
    } {
      const forward = currentForward()
      const eye = currentEye()
      const dist =
        rig.projection === 'parallel'
          ? (rig.orthographic.top - rig.orthographic.bottom) / (2 * rig.orthographic.zoom) / tanHalfFovRad(rig.perspective.fov)
          : controls.getDistance()
      return {
        projection: rig.projection,
        fovDeg: rig.perspective.fov,
        eye,
        target: [eye[0] + forward[0] * dist, eye[1] + forward[1] * dist, eye[2] + forward[2] * dist],
        up: [camera.up.x, camera.up.y, camera.up.z],
      }
    }

    /**
     * Restores a full camera view (docs/design/camera.md §5) — the load
     * complement of `getCameraState`. Normalizes to perspective FIRST
     * regardless of the target projection: `CameraRig.toggleProjection`'s
     * math always derives from the perspective camera's CURRENT pose (its
     * own doc comment), so that pose has to be the freshly-restored one
     * before any toggle runs, in EITHER direction.
     *
     * `controls.target` is set to the RESTORED target FIRST, before either
     * toggle: `rebindControlsForProjectionChange` rebuilds `controls` and
     * calls `controls.update()` on it, which — like the walkthrough-exit
     * reseed above — unconditionally re-aims the camera at WHATEVER
     * `controls.target` currently is. Toggling with the OLD (pre-restore)
     * target still in place would silently re-aim the freshly-restored
     * camera at the wrong point the instant the ortho rebind runs.
     */
    function applyCameraState(state: {
      projection: Projection
      fovDeg: number
      eye: [number, number, number]
      target: [number, number, number]
      up: [number, number, number]
    }): void {
      const targetVec = new THREE.Vector3(state.target[0], state.target[1], state.target[2])
      controls.target.copy(targetVec)
      if (rig.projection === 'parallel') {
        rig.toggleProjection(targetVec)
        rebindControlsForProjectionChange()
      }
      rig.perspective.position.set(state.eye[0], state.eye[1], state.eye[2])
      rig.perspective.up.set(state.up[0], state.up[1], state.up[2])
      rig.perspective.lookAt(targetVec)
      rig.setFov(state.fovDeg)
      rig.perspective.updateProjectionMatrix()
      if (state.projection === 'parallel') {
        rig.toggleProjection(targetVec)
        rebindControlsForProjectionChange()
      }
      controls.target.copy(targetVec)
      rig.syncInactiveCamera(targetVec)
      controls.update()
      scheduleRender()
    }

    // Test-only world→screen projection (ViewportApi.worldToScreen). Delegates
    // to the same `worldToPixels` the inference-dot overlay uses (hoisted —
    // declared later in this effect). Canvas-relative CSS pixels, top-left
    // origin.
    function worldToScreenPx(world: [number, number, number]): {
      x: number
      y: number
      behind: boolean
    } {
      return worldToPixels(new THREE.Vector3(world[0], world[1], world[2]))
    }

    function setHomeFraming(scale: number): void {
      // Re-pose the camera at the default 3/4 home view, `scale`× the
      // meter-scale distance (welcome-screen unit choice on a blank
      // document). Same direction and target as the mount-time default —
      // `HOME_EYE_OFFSET` (math.ts), the same constant used there.
      camera.position.set(HOME_EYE_OFFSET[0] * scale, HOME_EYE_OFFSET[1] * scale, HOME_EYE_OFFSET[2] * scale)
      controls.target.set(0, 0, 0)
      camera.updateProjectionMatrix()
      controls.update()
      // Keep the INACTIVE rig camera in sync too — not just whichever is
      // active — so a projection toggle right afterward (a freshly-opened
      // blank document is still perspective by default, and toggling
      // straight to parallel is a real path) lands framed from THIS pose
      // instead of whatever stale placeholder/leftover-zoom pose the
      // long-inactive camera was last left at.
      rig.syncInactiveCamera(controls.target)
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
      // Keep the grid's own through-origin lines in sync: shown only when
      // the axes are NOT (InfiniteGrid.ts) — never both stacked.
      infiniteGrid.setAxesVisible(visible)
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
      // Explicit guide deletion is as deliberate as an object/group/instance
      // delete (`runDelete`) — a `_recall` can legitimately be anchored to an
      // on-guide point (`snapOnGeometry` treats it as real geometry), so this
      // must disarm the tool's idle recall/rescale-prompt the same way, or a
      // stale reading/affordance survives referencing a point that no longer
      // exists. See `disarmActivePostCommitWindow`.
      disarmActivePostCommitWindow()
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
      // Same reasoning as `deleteAllGuides` above — a single deleted guide
      // can be the exact point an idle recall is anchored to.
      disarmActivePostCommitWindow()
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

    /**
     * Reset the movable drawing axes (tool-parity §4) to world identity —
     * same commit path as the Axes tool's own `set_axes` gesture (one undo
     * step, fully recorded/replayable). The origin-axes gizmo and inference
     * both read the frame fresh every frame/query, so nothing else needs
     * refreshing beyond the usual document-changed bookkeeping + a render.
     */
    function resetAxes(): void {
      try {
        wasmScene.set_axes(0, 0, 0, 1, 0, 0, 0, 1, 0)
      } catch (err) {
        handleToast(friendlyErrorText(err))
        return
      }
      onDocumentChangedRef.current?.()
      scheduleRender()
    }

    function runDeleteAnnotation(id: bigint): void {
      try {
        wasmScene.delete_annotation(id)
      } catch (err) {
        handleToast(friendlyErrorText(err))
        return
      }
      sceneRenderer.refreshAnnotations()
      onDocumentChangedRef.current?.()
      scheduleRender()
    }

    function commitAnnotationEditorText(text: string): void {
      const pending = pendingAnnotationEdit
      pendingAnnotationEdit = null
      if (pending === null) return

      if (pending.kind === 'edit') {
        // Blanking out an existing leader's text is a no-op refusal rather
        // than committing an empty label (there is no "restore computed
        // value" concept for a leader the way there is for a dimension's
        // text_override — an empty leader is just a leader with nothing to
        // say, so leave the prior content alone instead).
        if (pending.snapshot.kind === 'leader' && text.trim() === '') return
        try {
          commitAnnotationText(wasmScene, pending.id, pending.snapshot, text)
        } catch (err) {
          handleToast(friendlyErrorText(err))
          return
        }
        sceneRenderer.refreshAnnotations()
        onDocumentChangedRef.current?.()
        scheduleRender()
        return
      }

      // 'new-leader': an empty commit creates nothing — the user typed
      // nothing worth annotating, so the placed-but-unworded leader is
      // simply dropped (no partial/blank annotation left behind).
      const trimmed = text.trim()
      if (trimmed === '') return
      const { anchorNode, anchorPoint, offset } = pending.leader
      try {
        wasmScene.add_leader_text(
          anchorNode?.kind ?? -1,
          anchorNode?.id ?? 0n,
          new Float64Array(anchorPoint),
          new Float64Array(offset),
          trimmed,
        )
      } catch (err) {
        handleToast(friendlyErrorText(err))
        return
      }
      sceneRenderer.refreshAnnotations()
      onDocumentChangedRef.current?.()
      scheduleRender()
    }

    function cancelAnnotationEditor(): void {
      pendingAnnotationEdit = null
    }

    function getAnnotationLabel(id: bigint): string | null {
      return sceneRenderer.annotationLabelText(id)
    }

    function getAnnotationTextWorldPosition(id: bigint): [number, number, number] | null {
      return sceneRenderer.annotationTextWorldPosition(id)
    }

    function toggleSectionActive(): void {
      sectionManager.toggleActive()
      sceneRenderer.setSectionPlane(sectionManager.current)
      onSectionChangedRef.current?.()
      scheduleRender()
    }

    function getSectionState(): { origin: [number, number, number]; normal: [number, number, number]; active: boolean } | null {
      const p = sectionManager.current
      return p === null ? null : { origin: [...p.origin], normal: [...p.normal], active: p.active }
    }

    function getSectionRenderInfo(
      kind: 'object' | 'instance',
      id: bigint,
    ): {
      widget: boolean
      widgetClipCount: number
      nodeClipCount: number
      clipPlane: { normal: [number, number, number]; constant: number } | null
    } {
      return {
        widget: sceneRenderer.hasSectionWidget(),
        widgetClipCount: sceneRenderer.debugSectionWidgetClipPlaneCount(),
        nodeClipCount: sceneRenderer.debugNodeClipPlaneCount(kind, id),
        clipPlane: sceneRenderer.debugSectionClipPlane(),
      }
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
        // Field of View typed entry (design §2) has no Tool instance of its
        // own (see the module doc above `activeCameraTool`) but must still
        // own its keys the same way a tool mid-VCB-entry does — the App-level
        // Delete/Backspace handler must not steal Backspace while a FOV
        // digit buffer is open.
        if (fovEntryBuffer !== null) return true
        const t = toolController.activeTool
        // With a key, honor a tool's per-key capture (Tool.capturesKey) so
        // App-level shortcut gates (Space→Select, Delete/Backspace) agree
        // with the Viewport's own routing about which keys the tool owns.
        if (key !== undefined && 'capturesKey' in t) {
          return (t as { capturesKey(key: string): boolean }).capturesKey(key)
        }
        return 'capturingInput' in t && (t as { capturingInput(): boolean }).capturingInput()
      }
      // 3D Text (docs/design/3d-text.md): the dialog resolves parameters and
      // lays out the glyph run app-side, then hands the result here to arm
      // a one-shot TextPlaceTool — mirrors `makeFollowMeTool`'s group/face
      // context wiring, but reached imperatively (from App.tsx's dialog
      // handoff) rather than through the named-tool `switchToolRef` switch,
      // since 3D Text has no rail slot (design doc: Draw menu + palette
      // only).
      function armTextPlacement(placement: TextPlacement): void {
        const tool = new TextPlaceTool(
          wasmScene,
          previewGroup,
          placement,
          (instanceId) => {
            handleSceneRefresh()
            sceneRenderer.refreshAllSketches()
            onSelectRef.current?.({ kind: 'instance', id: instanceId }, false)
          },
          handleToast,
        )
        const ctx = activeContextRef.current
        const deepest = ctx.length > 0 ? ctx[ctx.length - 1] : null
        tool.setActiveContext(deepest?.kind === 'object' ? deepest.id : null)
        tool.setFaceEligibility(faceDrawEligible)
        tool.setComponentContext(
          deepest?.kind === 'instance' ? (wasmScene.instance_def(deepest.id) ?? null) : null,
        )
        tool.setActiveGroup(deepest?.kind === 'group' ? deepest.id : null)
        toolController.setTool(tool)
      }

      apiRefRef.current.current = { runBoolean, runGroup, runUngroup, runDelete, runMakeComponent, runPlaceInstance, runExplodeInstance, runMakeUnique, runOpenExplodeSession, runOpenExplodeSessionOrFallback: openExplodeSessionOrFallback, runCloseExplodeSession, explodeSessionInstance: () => explodeSessionInstanceRef.current, runOpenGroupSession, runCloseGroupSession, runCloseInnermostSession, sessionStack: () => [...sessionStackRef.current], sessionMembers: () => (sessionDirectMembersRef.current === null ? null : [...sessionDirectMembersRef.current]), hasArmedGesture: () => toolHasArmedGesture(toolController.activeTool), confirmPendingRescale, cancelPendingRescale, notifyLoaded, refreshScene, syncMaterialOpacity, isCapturingInput, runUndo, runRedo, zoomExtents, setStandardView, setCamera, captureFrame, worldToScreen: worldToScreenPx, getCamera, getCameraState, applyCameraState, setHomeFraming, setHidden, selectAll, setAxesVisible, setGridVisible, setGuidesVisible, deleteAllGuides, resetAxes, runDeleteGuide, runDeleteAnnotation, commitAnnotationEditorText, cancelAnnotationEditor, getAnnotationLabel, getAnnotationTextWorldPosition, toggleSectionActive, getSectionState, getSectionRenderInfo, exportGlb, exportStl, export3mf, toggleProjection, getProjection: () => rig.projection, setFov, armTextPlacement }
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
      // Scope the tool to the current editing context (component-edit-
      // parity.md phase A1 — the single channel every tool consults).
      applyEditContext(tool, computeEditContext(wasmScene, activeContextRef.current))
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
      // Scope the tool to the current editing context (component-edit-
      // parity.md phase A1 — the single channel every tool consults).
      applyEditContext(tool, computeEditContext(wasmScene, activeContextRef.current))
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
      // Scope the tool to the current editing context (component-edit-
      // parity.md phase A1 — the single channel every tool consults).
      applyEditContext(tool, computeEditContext(wasmScene, activeContextRef.current))
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
      // Scope the tool to the current editing context (component-edit-
      // parity.md phase A1 — the single channel every tool consults).
      applyEditContext(tool, computeEditContext(wasmScene, activeContextRef.current))
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
      // Scope the tool to the current editing context (component-edit-
      // parity.md phase A1 — the single channel every tool consults).
      applyEditContext(tool, computeEditContext(wasmScene, activeContextRef.current))
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
        // Durable extrude-as-new toggle → badge the Push/Pull cursor with a
        // `+` (the same cursorFor pipeline as Move's copy toggle).
        (on: boolean) => {
          renderer.domElement.style.cursor = cursorFor('Push/Pull', on)
        },
      )
      // Scope it to the current editing context (component-edit-parity.md
      // phase A1 — the single channel every tool consults; internally
      // derives the object/instance/component-scoped id checks the old
      // four-channel wiring used to compute here by hand).
      applyEditContext(tool, computeEditContext(wasmScene, activeContextRef.current))
      // Same context-path-aware eligibility as the draw tools: plain objects
      // are directly push/pullable; group/instance members only from inside
      // their container's editing context.
      tool.setFaceEligibility(faceDrawEligible)
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
        (text: string) => { onMeasurementRef.current?.(text) },
        // The path may be preselected (SketchUp's primary Follow Me idiom).
        [...selectedIdsRef.current],
      )
      // Put Follow Me's FACE path/profile on the same face-eligibility system
      // as every other face tool (`faceDrawEligible` already understands the
      // full group/instance context path — see the tool's FACE FRAME GUARD
      // doc). The single editing-context channel (component-edit-parity.md
      // phase A1) also carries Follow-Me-specific behavior: an INSTANCE
      // context now routes through the `_in_instance` follow-me family
      // instead of refusing wholesale (phase A2); a GROUP context births the
      // sweep inside the group being edited instead of at top level.
      applyEditContext(tool, computeEditContext(wasmScene, activeContextRef.current))
      tool.setFaceEligibility(faceDrawEligible)
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
      // Scope the tool to the current editing context (component-edit-
      // parity.md phase A1).
      applyEditContext(tool, computeEditContext(wasmScene, activeContextRef.current))
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
        // Alt-click sample: lift the sampled id to the parent, which makes it
        // current — the palette selection follows, same as picking a swatch.
        (id) => {
          onSampleMaterialRef.current?.(id)
        },
        // Shift/Ctrl+Shift replace-everywhere commit: an object-scoped
        // replace is confined to the clicked object (targeted refresh, same
        // reach as a single paint); a document-wide one can touch an
        // unbounded set of objects, so it falls back to the full rebuild
        // (the same posture structural ops without a clean touched-set use).
        (scope, objectId) => {
          if (scope === 'object') {
            handleSceneRefresh({ objectIds: [objectId] })
          } else {
            handleSceneRefresh()
          }
        },
      )
    }

    function makePositionTextureTool(): PositionTextureTool {
      const tool = new PositionTextureTool(
        wasmScene,
        previewGroup,
        // Live preview only — patches the already-rendered `uv` buffer in
        // place (paint-tool design §3); no document mutation, no refresh.
        (object, face, frame) => {
          sceneRenderer.previewFaceUv(object, face, frame)
        },
        // Commit: one kernel call already landed by the time this fires —
        // targeted refresh re-tessellates just the positioned object, same
        // reach as a single paint_face.
        (objectId) => {
          handleSceneRefresh({ objectIds: [objectId] })
        },
        handleToast,
        // Typed-precision readout while a pin is grabbed (paint-tool design
        // §3 addendum; paint-playtest2 §1) — same status-bar measurement
        // callback every other typed-VCB tool feeds.
        (text: string) => { onMeasurementRef.current?.(text) },
      )
      // Scope the tool to the current editing context (paint-playtest2 §2):
      // positioning an instanced face requires being INSIDE the component,
      // since `set_face_uv_frame` on a member changes every placement of
      // that definition. Same channel every other face tool uses — Paint
      // itself is deliberately NOT wired this way in this round (its own
      // out-of-context scoping is a separate design question).
      applyEditContext(tool, computeEditContext(wasmScene, activeContextRef.current))
      tool.setFaceEligibility(faceDrawEligible)
      return tool
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
      applyEditContext(tool, computeEditContext(wasmScene, activeContextRef.current))
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
      // A tool switch that bypasses switchToolRef (this one) still needs the
      // same mid-hold-tap reset — see switchToolRef's comment above.
      ctrlTap.reset()
      pushPullModifierTap.reset()
      multiClick.reset()
      const tool = makeMoveTool(dm.nodes)
      toolController.setTool(tool)
      renderer.domElement.style.cursor = cursorFor('Move')
      const { snap } = snapService.resolve(dm.pressRay, el.clientHeight, apertureBasis())
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
          // Select the committed nodes — for a copy these are the fresh
          // clones, so a follow-up rotation chains off the new copies (see
          // makeMoveTool).
          if (nodes.length === 1) onSelectRef.current?.(nodes[0], false)
          else onSelectManyRef.current?.(nodes, false)
        },
        handleToast,
        (id: bigint) => sceneRenderer.getInstanceGroup(id),
        (text: string) => { onMeasurementRef.current?.(text) },
        // Durable copy toggle → badge the Rotate cursor with a `+` (see
        // makeMoveTool).
        (on: boolean) => {
          renderer.domElement.style.cursor = cursorFor('Rotate', on)
        },
        // ×N / /N array re-resolve: full scene refresh, same reasoning as
        // makeMoveTool's onArrayCommit.
        (nodes) => {
          handleSceneRefresh()
          sceneRenderer.refreshAllSketches()
          if (nodes.length === 1) onSelectRef.current?.(nodes[0], false)
          else onSelectManyRef.current?.(nodes, false)
        },
      )
      tool.setSelectionAcquirer(acquireTransformTargets)
      applyEditContext(tool, computeEditContext(wasmScene, activeContextRef.current))
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
      applyEditContext(tool, computeEditContext(wasmScene, activeContextRef.current))
      return tool
    }

    function makeTapeMeasureTool(): TapeMeasureTool {
      const tool = new TapeMeasureTool(
        wasmScene,
        previewGroup,
        () => {
          sceneRenderer.refreshGuides()
          onDocumentChangedRef.current?.()
          scheduleRender()
        },
        handleToast,
        // Tape Measure is the ONLY tool whose readout persists after a
        // commit (tape-measure-rework part 1) — forward the `frozen` flag
        // its own `OnMeasurement` type now carries; every other tool below
        // still passes just `text`, which leaves `frozen` `undefined` (and
        // so falsy) on their calls, unchanged.
        (text: string, frozen?: boolean) => { onMeasurementRef.current?.(text, frozen) },
        // Resize-the-model arm (design tool-parity §3): bubble up to the
        // parent, which renders the confirmation modal and resolves it via
        // ViewportApi.confirmPendingRescale/cancelPendingRescale below.
        (info) => { onRescaleArmedRef.current?.(info) },
        // A confirmed WHOLE-MODEL rescale bakes into every object, sketch,
        // guide, and instance pose — a full refresh, unlike the guide-only
        // commit above — AND leaves every view-side world-length quantity
        // framing the OLD scale; re-scale all of it by the same factor about
        // the same world-origin pivot so the view reads as unchanged (see
        // `applyRescaleToView`'s doc comment). A SCOPED (in-session)
        // rescale only touches geometry inside the open session — the world
        // outside it, camera included, is unchanged in size, so
        // `applyRescaleToView` must NOT run (group-session.md's "Tape
        // Measure scoped rescale"); the scene still needs a full refresh,
        // since the session's contents did change.
        (factor: number, scoped: boolean) => {
          if (!scoped) applyRescaleToView(factor)
          handleSceneRefresh()
          sceneRenderer.refreshAllSketches()
          sceneRenderer.refreshGuides()
        },
      )
      applyEditContext(tool, computeEditContext(wasmScene, activeContextRef.current))
      // Finding 6 (group-session.md): `refreshSessionScope` pushes fresh
      // scope only while ITS OWN triggers fire (open/close/commit/undo); a
      // tool switch INTO Tape Measure while a session is already open is
      // not one of those, so push the current scope here too — mirrors
      // `applyEditContext` just above.
      tool.setSessionScope(sessionScopeForTapeMeasure())
      return tool
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

    function makeDimensionTool(): DimensionTool {
      return new DimensionTool(
        wasmScene,
        previewGroup,
        () => {
          sceneRenderer.refreshAnnotations()
          onDocumentChangedRef.current?.()
          scheduleRender()
        },
        handleToast,
        (text: string) => { onMeasurementRef.current?.(text) },
      )
    }

    function makeTextTool(): TextTool {
      return new TextTool(previewGroup, (leader: PlacedLeader) => {
        pendingAnnotationEdit = { kind: 'new-leader', leader }
        const end: V3 = [
          leader.anchorPoint[0] + leader.offset[0],
          leader.anchorPoint[1] + leader.offset[1],
          leader.anchorPoint[2] + leader.offset[2],
        ]
        const p = worldToPixels(new THREE.Vector3(end[0], end[1], end[2]))
        onOpenAnnotationEditorRef.current?.({
          id: null,
          screenX: p.x,
          screenY: p.y,
          initialText: '',
          placeholder: 'Leader text…',
        })
      })
    }

    function makeSliceTool(): SliceTool {
      const tool = new SliceTool(
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
      applyEditContext(tool, computeEditContext(wasmScene, activeContextRef.current))
      tool.setFaceEligibility(faceDrawEligible)
      return tool
    }

    function makeSectionPlaneTool(): SectionPlaneTool {
      return new SectionPlaneTool(
        wasmScene,
        previewGroup,
        () => sectionManager.current,
        // A live read of the CACHED coverage rectangle — cheap (a field
        // read, no scene-graph walk) and always in sync with whatever the
        // widget last resized to, unlike a construction-time snapshot (see
        // SectionPlaneTool's doc comment on `getWidgetCoverage`).
        () => sceneRenderer.currentSectionWidgetCoverage(),
        // Place (or replace) — becomes the active section; the tool STAYS
        // active (no spring-back to Select) so the user can immediately
        // sweep, toggle, or delete the section they just placed. Springing
        // back to Select here would desync App's `activeTool` state (the
        // internal switchToolRef updates the viewport toolController + status
        // bar but not the React tool state), leaving the rail showing Section
        // Plane while Select is really active — which routed Delete to a
        // destructive kernel delete of the document selection. Keeping the
        // tool active is also the better inspect-tool UX (place, then sweep).
        (origin, normal) => {
          sectionManager.place(origin, normal)
          sceneRenderer.setSectionPlane(sectionManager.current)
          onSectionChangedRef.current?.()
          scheduleRender()
        },
        // Live widget-drag preview — cheap, in-place, no material rebuild.
        // Never notifies onSectionChanged: existence/active never change
        // during a pure offset sweep, only origin (see
        // SceneRenderer.updateSectionPlaneOffset's doc comment).
        (plane) => {
          sceneRenderer.updateSectionPlaneOffset(plane)
          scheduleRender()
        },
        // Offset committed (second click or typed Enter) — persist it and
        // stay in this tool (sweeping is "the primary inspection gesture",
        // meant to be repeated — DESIGN §1).
        (plane) => {
          sectionManager.setPlane(plane)
          sceneRenderer.setSectionPlane(sectionManager.current)
          onSectionChangedRef.current?.()
          scheduleRender()
        },
        // Plain click on the widget — toggle active (SketchUp's "Active Cut").
        () => {
          sectionManager.toggleActive()
          sceneRenderer.setSectionPlane(sectionManager.current)
          onSectionChangedRef.current?.()
          scheduleRender()
        },
        // Delete — the model returns to whole.
        () => {
          sectionManager.delete()
          sceneRenderer.setSectionPlane(null)
          onSectionChangedRef.current?.()
          scheduleRender()
        },
        // Esc / tool-switch mid-drag — revert the live preview to whatever
        // is still committed in sectionManager (the drag never wrote there).
        // No onSectionChanged notify: this reverts to the ALREADY-committed
        // plane, so existence/active can't have changed from what the menu
        // check state last reflected.
        () => {
          sceneRenderer.setSectionPlane(sectionManager.current)
          scheduleRender()
        },
        handleToast,
        (text: string) => { onMeasurementRef.current?.(text) },
      )
    }

    // ------------------------------------------------------------------ walkthrough camera tools (camera.md §4)
    // The first REAL camera Tool classes (Orbit/Pan/Zoom stay OrbitControls
    // `mouseButtons` remaps, above). All three mutate `camera` (whichever of
    // `rig.perspective`/`rig.orthographic` is active) directly while
    // OrbitControls sits fully DISABLED (see the switch-case wiring below) —
    // `applyWalkthroughPose` is their single shared write path, syncing the
    // currently-INACTIVE rig camera too so a projection toggle mid-walkthrough
    // (or right after exiting) starts from this pose, not a stale one.
    const walkthroughForwardV = new THREE.Vector3()
    function currentEye(): WalkV3 {
      return [camera.position.x, camera.position.y, camera.position.z]
    }
    function currentForward(): WalkV3 {
      camera.getWorldDirection(walkthroughForwardV)
      return [walkthroughForwardV.x, walkthroughForwardV.y, walkthroughForwardV.z]
    }
    function applyWalkthroughPose(eye: WalkV3, forward: WalkV3): void {
      camera.position.set(eye[0], eye[1], eye[2])
      camera.up.set(0, 0, 1)
      const target = new THREE.Vector3(eye[0] + forward[0], eye[1] + forward[1], eye[2] + forward[2])
      camera.lookAt(target)
      camera.updateProjectionMatrix()
      rig.syncInactiveCamera(target)
      scheduleRender()
    }

    function makePositionCameraTool(): PositionCameraTool {
      return new PositionCameraTool(
        currentForward,
        () => eyeHeightM,
        (h) => { eyeHeightM = h },
        applyWalkthroughPose,
        // Auto-switch to Look Around (SketchUp behavior, design §4) — routes
        // through the same switchToolRef the menu/palette/rail all use, so
        // App's React tool state (rail highlight, status bar) stays in sync;
        // a raw `toolController.setTool(...)` here would desync it exactly
        // like the SectionPlaneTool doc note above warns against.
        () => switchToolRef.current?.('Look Around'),
        // Esc returns to Select (design §4, user guide "viewing.md" — "Esc
        // returns to the Select tool from any of the three"), same as
        // Look Around/Walk below.
        () => switchToolRef.current?.('Select'),
        (text: string) => { onMeasurementRef.current?.(text) },
      )
    }

    function makeLookAroundTool(): LookAroundTool {
      return new LookAroundTool(
        currentEye,
        currentForward,
        () => eyeHeightM,
        (h) => { eyeHeightM = h },
        applyWalkthroughPose,
        () => switchToolRef.current?.('Select'),
        (text: string) => { onMeasurementRef.current?.(text) },
      )
    }

    function makeWalkTool(): WalkTool {
      return new WalkTool(
        currentEye,
        currentForward,
        () => eyeHeightM,
        (h) => { eyeHeightM = h },
        applyWalkthroughPose,
        () => switchToolRef.current?.('Select'),
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

    function makeAxesTool(): AxesTool {
      return new AxesTool(
        wasmScene,
        previewGroup,
        // A moved frame is an undoable document edit (one `set_axes` step)
        // but touches no geometry — no re-tessellation/refresh beyond the
        // usual document-changed bookkeeping + a render. The origin-axes
        // gizmo and inference both read the frame fresh every frame/query
        // (see the render-loop position/quaternion update, and
        // `getDrawingAxes`), so nothing else needs poking.
        () => {
          onDocumentChangedRef.current?.()
          scheduleRender()
        },
        handleToast,
      )
    }

    // Shift-in-Orbit temporarily swaps to Pan, mirroring SketchUp.
    // Tracked here (not in switchToolRef's closure alone) so the keydown/keyup
    // handlers and the tool switch can all see/clear the same flag.
    let shiftPanActive = false

    // Shift-in-Zoom (camera-playtest2.md §3): cursor-only tracking of
    // "Shift is currently held while a fov drag/wheel is reachable" — mirrors
    // shiftPanActive's own autorepeat-guard shape, kept as a SEPARATE flag
    // (rather than reusing shiftPanActive) since Orbit and Zoom are
    // different tools with independent cursor-swap lifecycles. This never
    // touches `controls.mouseButtons.LEFT` itself — the actual fov-vs-dolly
    // decision for a drag is made once, at pointerdown, by
    // `onFovDragPointerDownCapture`.
    let shiftFovCursorActive = false

    // Position Camera / Look Around / Walk (camera.md §4): unlike every
    // other tool switch, entering one of these fully DISABLES OrbitControls
    // (not just remaps its left button — the walkthrough tools own left-drag
    // outright), and exiting must re-enable it AND re-seed `controls.target`
    // to where the eye ends up looking, so a subsequent orbit orbits around
    // what's actually on screen instead of wherever the target was frozen at
    // before the walkthrough began.
    const WALKTHROUGH_TOOL_NAMES = new Set(['Position Camera', 'Look Around', 'Walk'])

    // Switch tool by name
    switchToolRef.current = (toolName: string) => {
      // Record the requested name on EVERY invocation, before anything else
      // runs — this is the tool-switch guard's comparison source (see
      // `lastAppliedToolNameRef`'s doc comment), not
      // `toolController.activeToolName`, which several branches below
      // deliberately overwrite with 'Select'.
      lastAppliedToolNameRef.current = toolName
      // A tool switch always wins over a stale shift-pan state: if the user
      // changes tools while Shift happens to be held, the explicit
      // mouseButtons.LEFT/cursor this switch sets below must not later be
      // clobbered by onShiftKeyUp restoring the *previous* tool's Orbit state.
      shiftPanActive = false
      shiftFovCursorActive = false
      // Any tool switch (even re-selecting the same camera tool) abandons an
      // in-progress Zoom Window drag, an in-progress Shift-fov drag, and a
      // typed-but-uncommitted FOV entry — all per-activation state that must
      // not survive past it. `abortFovDrag` reads the OLD `activeCameraTool`
      // (still 'Zoom' if that's what's being left) to restore MOUSE.DOLLY;
      // the switch below then sets `mouseButtons.LEFT` again per-case
      // regardless, so this is what stops the drag's OWN state (and
      // pointer capture) from lingering, not what decides the new binding.
      abortFovDrag()
      if (zoomWindowDrag !== null) {
        zoomWindowDrag = null
        marqueeOverlay.style.display = 'none'
      }
      zoomWindowActive = false
      cancelFovEntry()
      // Exiting a walkthrough tool (design §4) — see the const's doc above.
      // `dist` MUST be read before `controls.target` is overwritten below:
      // `controls.getDistance()` measures FROM the (still stale, pre-reseed)
      // target, and chaining `.copy(camera.position).addScaledVector(dir,
      // rig.effectiveDistance(controls.getDistance()))` evaluates that inner
      // call only after `.copy()` has already run — by then `controls.target
      // === camera.position` and `getDistance()` collapses to (numerically)
      // zero, silently discarding the reseed (a real bug this fixes, not
      // just a test artifact: EVERY walkthrough exit hit it).
      if (WALKTHROUGH_TOOL_NAMES.has(toolController.activeToolName)) {
        controls.enabled = true
        camera.getWorldDirection(walkthroughForwardV)
        const dist = rig.effectiveDistance(controls.getDistance())
        controls.target.copy(camera.position).addScaledVector(walkthroughForwardV, dist)
        controls.update()
      }
      // ONE unconditional assignment covering every case below, including
      // ones (Rectangle, Move, Position Camera, …) that never mention
      // `activeCameraTool` themselves — see `activeCameraToolForName`'s doc
      // (finding C, camera-fov-fixes): previously this was assigned only
      // inside the Orbit/Pan/Zoom/Zoom Window/default cases, so a switch
      // straight from Zoom to a NAMED tool fell through that tool's own
      // case without ever clearing it, leaving the FOV readout stuck
      // showing the stale Zoom reading.
      activeCameraTool = activeCameraToolForName(toolName)
      // A mid-hold Ctrl/Meta tap must not survive a tool switch: without
      // this, holding Ctrl on Scale then clicking over to Push/Pull before
      // releasing would leave a tap armed against the OLD tool, and the
      // eventual keyup would resolve against whichever tool is active by
      // then. `CleanModifierTap` also records the tool instance each tap
      // armed against and re-checks it at keyup — this reset is the first
      // line of defense for switches that go through switchToolRef; the
      // instance check covers switches that don't (e.g. beginDragMove's
      // direct toolController.setTool).
      ctrlTap.reset()
      pushPullModifierTap.reset()
      multiClick.reset()
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
        case 'Position Texture':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          toolController.setTool(makePositionTextureTool())
          break
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
        case 'Dimension':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          toolController.setTool(makeDimensionTool())
          break
        case 'Text':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          toolController.setTool(makeTextTool())
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
        case 'Section Plane':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          toolController.setTool(makeSectionPlaneTool())
          break
        case 'Edit Vertex':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          toolController.setTool(makeEditVertexTool())
          break
        case 'Axes':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          toolController.setTool(makeAxesTool())
          break
        case 'Position Camera':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          controls.enabled = false
          toolController.setTool(makePositionCameraTool())
          break
        case 'Look Around':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          controls.enabled = false
          toolController.setTool(makeLookAroundTool())
          break
        case 'Walk':
          cameraModeRef.current = false
          controls.mouseButtons.LEFT = null
          controls.enabled = false
          toolController.setTool(makeWalkTool())
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
        case 'Zoom Window':
          // NOT cameraModeRef — this mode owns left-drag itself (a one-shot
          // rectangle, see the Zoom Window section above), not OrbitControls'
          // native orbit/pan/dolly, so geometry routing's camera-nav
          // early-return must NOT swallow it.
          cameraModeRef.current = false
          zoomWindowActive = true
          controls.mouseButtons.LEFT = null
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
      // Report the REAL active tool to the parent — same requested-name
      // reasoning as onStatusChange just above applies here too. This is
      // switchToolRef's single entry point, so every invocation reaches
      // here regardless of why it fired: an explicit prop-driven switch, an
      // auto-handoff (Position Camera → Look Around), an Escape-to-Select,
      // or a one-shot revert — making `activeTool` (App.tsx state) truthful
      // for all of them instead of only the explicit ones.
      onInternalToolChangeRef.current?.(toolName)
      onInferenceChangeRef.current?.(null)
      // Show (or hide) the persistent FOV readout for the tool just entered
      // — AFTER activeCameraTool above has settled on its new value (design
      // §2 / finding 4b: entering Zoom in perspective must show the current
      // fov immediately, not wait for the first typed digit).
      refreshFovReadout()
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
    // middle-drag = orbit, right-drag = pan, wheel = dolly-to-cursor. Extracted
    // (docs/design/camera.md §1) so `rebindControlsForProjectionChange` below
    // can dispose and recreate an OrbitControls bound to whichever camera
    // just became active — OrbitControls binds to one camera for its whole
    // lifetime, so a projection change can't just mutate `.object`.
    function configureControls(c: OrbitControls): void {
      c.mouseButtons = {
        LEFT: null,
        MIDDLE: THREE.MOUSE.ROTATE,
        RIGHT: THREE.MOUSE.PAN,
      }
      c.zoomToCursor = true
      c.enableDamping = true
      c.dampingFactor = 0.08
      c.screenSpacePanning = true
      c.minDistance = 0.1
      c.maxDistance = 50
      c.enablePan = true
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
      c.minPolarAngle = Math.atan(POLE_TILT)
      c.maxPolarAngle = Math.PI - Math.atan(POLE_TILT)
      // Ortho zoom clamp (docs/design/camera.md §1): OrbitControls treats
      // wheel-zoom under an OrthographicCamera as scaling `.zoom` instead of
      // dollying distance, and defaults to NO clamp at all (`minZoom: 0`,
      // `maxZoom: Infinity`) — zooming out under parallel projection could
      // drive `zoom` toward 0, sending `CameraRig.effectiveDistance` toward
      // Infinity and silently rendering guide dashes solid. Mirror the same
      // reachable visual range `minDistance`/`maxDistance` give perspective
      // (see `orthoZoomBounds`'s doc for the derivation).
      const { minZoom, maxZoom } = orthoZoomBounds(c.minDistance, c.maxDistance)
      c.minZoom = minZoom
      c.maxZoom = maxZoom
    }
    // `let`, not `const`: `rebindControlsForProjectionChange` disposes and
    // replaces this with a new instance bound to the newly-active camera
    // (OrbitControls supports OrthographicCamera natively — dolly becomes
    // zoom, zoomToCursor works for both — docs/design/camera.md §1).
    let controls = new OrbitControls(camera, renderer.domElement)
    configureControls(controls)

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
    // Bundles every listener a live `controls` instance needs — called once
    // at creation above (via the initial call below) and again after
    // `rebindControlsForProjectionChange` recreates the instance. `scheduleRender`/
    // `recordCameraInput` are declared later in this effect but, like
    // `apertureBasis` above, this is a `function` declaration (hoisted) only
    // ever CALLED after both exist.
    function attachControlsListeners(c: OrbitControls): void {
      c.addEventListener('start', onControlsStart)
      c.addEventListener('end', onControlsEnd)
      c.addEventListener('change', scheduleRender)
      c.addEventListener('change', recordCameraInput)
    }
    attachControlsListeners(controls)

    // Prevent the browser context menu on right-drag so pan isn't interrupted.
    function onContextMenu(ev: MouseEvent): void {
      ev.preventDefault()
    }
    renderer.domElement.addEventListener('contextmenu', onContextMenu)

    /**
     * Rebind OrbitControls to whichever camera is now `rig.active`, right
     * after a `rig.toggleProjection()` call — extracted so the interactive
     * Camera ▸ Parallel Projection toggle (`toggleProjection` below) and the
     * test-pinning `setCamera` helper (which force-toggles to perspective)
     * share the SAME dispose/recreate/configure/attach dance rather than one
     * of them reimplementing half of it inline. `setCamera` used to do
     * exactly that — force `rig.toggleProjection` but leave `controls` bound
     * to the now-inactive camera, permanently freezing input, since
     * OrbitControls binds to one camera for its whole lifetime and a
     * projection change can't just mutate `.object`.
     *
     * Also clears any in-flight typed FOV entry (`cancelFovEntry` —
     * Escape-like discard, not commit; matches the VCB idiom, camera.md
     * §2): a value typed under perspective and left uncommitted across a
     * toggle would otherwise silently apply the STALE typed number once the
     * round trip lands back on perspective.
     *
     * `refreshFovReadout` runs UNCONDITIONALLY after that, regardless of
     * whether a buffer was typing (finding D, camera-fov-fixes):
     * `cancelFovEntry` early-returns — by design — when nothing was typed,
     * so it alone never refreshes the readout's TEXT, only its typed-entry
     * state. But the readout's text depends on `rig.projection` too (parallel
     * has no fov to show), and that just changed, so a toggle with no
     * in-flight typing left the readout showing whatever it said under the
     * PREVIOUS projection until some unrelated later event happened to
     * refresh it.
     */
    function rebindControlsForProjectionChange(): void {
      // Defensive: abort any in-flight Shift-fov drag BEFORE reading
      // `prevLeft` below, so a (practically unreachable — see
      // onFovDragPointerDownCapture's perspective-only gate) mid-drag
      // projection change doesn't carry a leaked `mouseButtons.LEFT ===
      // null` onto the freshly rebuilt `controls`.
      abortFovDrag()
      const prevLeft = controls.mouseButtons.LEFT
      const prevTarget = controls.target.clone()
      // Preserve `.enabled` across the rebuild — a fresh OrbitControls
      // defaults to enabled, which would silently re-enable orbiting if a
      // Parallel Projection toggle (independent of the active tool) fires
      // WHILE a walkthrough tool (Position Camera/Look Around/Walk,
      // camera.md §4) has it deliberately disabled.
      const prevEnabled = controls.enabled
      camera = rig.active
      controls.dispose()
      controls = new OrbitControls(camera, renderer.domElement)
      configureControls(controls)
      controls.mouseButtons.LEFT = prevLeft
      controls.target.copy(prevTarget)
      controls.enabled = prevEnabled
      controls.update()
      attachControlsListeners(controls)
      cancelFovEntry()
      refreshFovReadout()
      onProjectionChangeRef.current?.(rig.projection)
    }

    /**
     * Camera ▸ Parallel Projection (checkbox) — toggles between perspective
     * and parallel projection, visually stable at the orbit target
     * (docs/design/camera.md §1: `CameraRig.toggleProjection`).
     */
    function toggleProjection(): void {
      rig.toggleProjection(controls.target)
      rebindControlsForProjectionChange()
      scheduleRender()
    }
    // Push the initial projection once, mount-time — the parent's checkbox
    // otherwise has no way to learn the starting state without assuming it
    // matches CameraRig's default.
    onProjectionChangeRef.current?.(rig.projection)

    /** Direct fov setter (design §2): typing into the Zoom tool's VCB (see
     * `commitFovEntry` above) is the normal interactive path; this is the
     * equivalent direct/test entry point ViewportApi exposes alongside it. */
    function setFov(deg: number): void {
      rig.setFov(deg)
      controls.update()
      refreshFovReadout()
      scheduleRender()
    }

    // Shift-in-Orbit -> temporary Pan. OrbitControls already handles
    // this natively: with mouseButtons.LEFT === MOUSE.ROTATE, holding
    // Shift/Ctrl/Meta during onMouseDown makes it pan instead of rotate (see
    // OrbitControls.js). So we must NOT touch controls.mouseButtons.LEFT here
    // — doing so would fight that built-in inversion. These handlers only
    // swap the cursor to match. Only Orbit is affected; every other tool
    // behaves exactly as before. Guarded by shiftPanActive so keydown
    // autorepeat doesn't re-apply the same state repeatedly, and so keyup
    // only restores the Orbit cursor if we're the ones who changed it.
    //
    // Shift-in-Zoom (camera-playtest2.md §3) is handled in the SAME pair of
    // handlers (per that design's explicit direction — extend rather than
    // add a second Shift listener), independently guarded by
    // `shiftFovCursorActive` so neither swap's autorepeat/restore logic
    // interferes with the other's. Unlike Shift-in-Orbit, this DOES have a
    // real functional effect on a drag — but that decision is made once, at
    // pointerdown, by `onFovDragPointerDownCapture`; these handlers are
    // cursor-only, exactly like the Orbit/Pan swap above.
    function onShiftKeyDown(ev: KeyboardEvent): void {
      if (ev.key !== 'Shift') return
      // Move's Shift-held axis lock. Idempotent under keydown autorepeat.
      const at = toolController.activeTool
      if ('setShiftHeld' in at) {
        (at as { setShiftHeld(held: boolean): void }).setShiftHeld(true)
      }
      if (!shiftPanActive && activeToolPropRef.current === 'Orbit') {
        shiftPanActive = true
        renderer.domElement.style.cursor = cursorFor('Pan')
      }
      // `!cameraDragActive` (finding 3, camera-playtest2 review): the
      // fov-vs-dolly mode is fixed at the Zoom tool's own pointerdown and
      // never re-evaluated mid-gesture (fovDrag.ts's module doc — the same
      // "MODE FIXED AT PRESS" rule this cursor swap must respect). Once
      // OrbitControls has already started a plain dolly (`cameraDragActive`
      // is the same "a real drag is live" flag `onControlsStart`/`onControlsEnd`
      // maintain from OrbitControls' own 'start'/'end' events — it stays
      // false throughout an armed fov drag, since that drag pre-empts
      // OrbitControls entirely and it never dispatches 'start'), swapping the
      // cursor to the fov one here would claim a mode change that will not
      // happen: the drag stays a dolly no matter how long Shift is held.
      if (
        !shiftFovCursorActive &&
        !cameraDragActive &&
        activeCameraTool === 'Zoom' &&
        rig.projection === 'perspective'
      ) {
        shiftFovCursorActive = true
        renderer.domElement.style.cursor = cursorFor('Zoom Window')
      }
    }
    function onShiftKeyUp(ev: KeyboardEvent): void {
      if (ev.key === 'Shift') {
        const at = toolController.activeTool
        if ('setShiftHeld' in at) {
          (at as { setShiftHeld(held: boolean): void }).setShiftHeld(false)
        }
        // Same `reportToolHint()` gap `onAltKeyUp` fixes for its own
        // modifier (see that handler's comment below): a plain Shift
        // release has no keydown-driven re-poll of its own, so a tool's
        // Shift-provenance status hint (e.g. TapeMeasureTool's axis-lock
        // wording, tape-measure-rework WP-7) would otherwise stay stale
        // until an unrelated event happened to re-poll it.
        reportToolHint()
      }
      const releasedCleanly = ev.key === 'Shift' || !ev.shiftKey
      if (shiftFovCursorActive && releasedCleanly) {
        shiftFovCursorActive = false
        // An active drag/wheel-adjust owns the cursor until its OWN
        // gesture ends (see restoreZoomDollyBinding) — don't stomp it back
        // to the plain Zoom cursor mid-drag.
        if (fovDragState === null) renderer.domElement.style.cursor = cursorFor('Zoom')
      }
      if (shiftPanActive && releasedCleanly) {
        shiftPanActive = false
        renderer.domElement.style.cursor = cursorFor('Orbit')
      }
    }
    window.addEventListener('keydown', onShiftKeyDown)
    window.addEventListener('keyup', onShiftKeyUp)

    // A blur (Cmd-Tab, devtools, another window) swallows the keyup that
    // would otherwise release Shift — without this, a tool's Shift-
    // provenance axis lock (e.g. TapeMeasureTool's `_shiftAxisLock`,
    // tape-measure-rework WP-7) stays stranded true after the window loses
    // focus, even though Shift is no longer physically held. Same posture
    // as `onWindowBlurClearsEyedropper`/`onWindowBlurClearsPrecision` below.
    function onWindowBlurClearsShiftLock(): void {
      const at = toolController.activeTool
      if ('setShiftHeld' in at) {
        (at as { setShiftHeld(held: boolean): void }).setShiftHeld(false)
      }
      reportToolHint()
    }
    window.addEventListener('blur', onWindowBlurClearsShiftLock)

    // Ctrl toggles the active tool's durable center-anchor (Scale's
    // `toggleCenterAnchor`). Like Shift above, a BARE Control keydown reports
    // ctrlKey:true, so the generic key path in onKeyDown (gated on `!isMod`)
    // never carries it — hence a dedicated listener. Fire on a CLEAN TAP only:
    // Control pressed and released with NO other key in between. Toggling on
    // the leading keydown would also flip the anchor as a side effect of every
    // Ctrl chord (Ctrl+Z undo, Ctrl+A select-all, …), which the clean-tap
    // guard prevents.
    //
    // This listener and the Push/Pull one just below both live at window
    // scope and both watch the SAME bare Ctrl/Meta keydown, regardless of
    // which tool is active — so a tap that starts while Scale is active but
    // ends (keyup) after the user has switched to Push/Pull would otherwise
    // fire Push/Pull's toggle (and the reverse fires Scale's). `CleanModifierTap`
    // (see cleanModifierTap.ts) fixes this structurally: it records which
    // tool instance was active AT KEYDOWN and only reports a fired tap if
    // that same instance is STILL active at keyup, so a mid-hold switch
    // silently drops the tap instead of misdirecting it. `.reset()` is also
    // called on every tool switch (see switchToolRef/beginDragMove above) as
    // a belt-and-suspenders clear.
    // Click-count for pointerdown, reconstructed because Chromium reports
    // `detail === 0` on pointer events (see multiClick.ts). Reset alongside
    // `ctrlTap` on tool switches and on blur: two clicks either side of an
    // unrelated event are not a double-click however close together they land.
    const multiClick = new MultiClickTracker()
    const ctrlTap = new CleanModifierTap<Tool>((key) => key === 'Control')
    function onCtrlKeyDown(ev: KeyboardEvent): void {
      ctrlTap.onKeyDown(ev, toolController.activeTool)
    }
    function onCtrlKeyUp(ev: KeyboardEvent): void {
      const armedTool = ctrlTap.onKeyUp(ev, toolController.activeTool)
      if (armedTool === null) return
      if ('toggleCenterAnchor' in armedTool) {
        (armedTool as { toggleCenterAnchor(): void }).toggleCenterAnchor()
        scheduleRender()
      }
    }
    // Capture phase: dialogs/menus/the palette stopPropagation() Escape's
    // keydown at the bubble phase (see dialogs.test.tsx's
    // expectEscapeStopsPropagationToWindow) to keep it from ALSO firing the
    // Viewport's own Escape handling underneath them. That stopPropagation
    // runs before the event would otherwise reach this bubble-phase listener,
    // so a Ctrl-tap armed just before an Escape-dismissed dialog would never
    // see the Escape and never disarm — leaving `ctrlTapClean` true for the
    // keyup that follows, and firing `toggleCenterAnchor` as a side effect of
    // dismissing an unrelated overlay. Capture fires window → document →
    // target, strictly before any bubble-phase stopPropagation downstream, so
    // this listener sees every keydown regardless of what swallows it later.
    // Pure bookkeeping (no preventDefault, no dependency on handler order),
    // so moving it to capture changes nothing else about it.
    window.addEventListener('keydown', onCtrlKeyDown, true)
    window.addEventListener('keyup', onCtrlKeyUp)

    // Ctrl/Cmd toggles Push/Pull's durable extrude-as-new-object mode
    // (design tool-parity §2 — SketchUp's "leave original face", the
    // Move-copy idiom applied to Push/Pull). Same reasoning as the Ctrl
    // listener just above (a bare Control/Meta keydown reports
    // ctrlKey/metaKey: true on itself, so the generic `!isMod` key path
    // never carries it) — a dedicated listener, watching EITHER key so Ctrl
    // (Windows/Linux) and Cmd (macOS) both work. Fires on a CLEAN TAP of
    // either, with no other key in between, for the same chord-safety
    // reason as Scale's tap guard (Ctrl+Z / Ctrl+A / etc. must not toggle
    // it as a side effect). Also tool-scoped at keydown/keyup for the same
    // mid-hold-switch reason documented on `ctrlTap` above.
    const pushPullModifierTap = new CleanModifierTap<Tool>((key) => key === 'Control' || key === 'Meta')
    function onPushPullModifierKeyDown(ev: KeyboardEvent): void {
      pushPullModifierTap.onKeyDown(ev, toolController.activeTool)
    }
    function onPushPullModifierKeyUp(ev: KeyboardEvent): void {
      const armedTool = pushPullModifierTap.onKeyUp(ev, toolController.activeTool)
      if (armedTool === null) return
      if ('toggleExtrudeAsNew' in armedTool) {
        (armedTool as { toggleExtrudeAsNew(): void }).toggleExtrudeAsNew()
        scheduleRender()
      }
    }
    window.addEventListener('keydown', onPushPullModifierKeyDown)
    window.addEventListener('keyup', onPushPullModifierKeyUp)

    // Ctrl/Cmd HELD during a Tape Measure gesture = measure WITHOUT
    // creating a guide (SketchUp parity, tape-measure-rework WP-7 item 1) —
    // a LIVE held-state dispatch, same posture as `onShiftKeyDown`/
    // `onShiftKeyUp` above, NOT the CLEAN-TAP dispatch `onCtrlKeyDown`/
    // `onPushPullModifierKeyDown` use just above for Scale's/Push-Pull's own
    // Ctrl/Cmd bindings — those duck-type a DIFFERENT method
    // (`toggleCenterAnchor`/`toggleExtrudeAsNew`) that TapeMeasureTool
    // implements neither of, so a live hold here never reaches them, and
    // conversely a clean tap while TapeMeasureTool is active never reaches
    // `setGuideCreationSuppressed` (nothing here is TAP-scoped). Control and
    // Meta are treated as one logical modifier, same as
    // `onPushPullModifierKeyDown` just above — Ctrl (Windows/Linux) and Cmd
    // (macOS) both work, per this codebase's `metaKey || ctrlKey` convention
    // (see platform.ts).
    function onGuideSuppressKeyDown(ev: KeyboardEvent): void {
      if (ev.key !== 'Control' && ev.key !== 'Meta') return
      const at = toolController.activeTool
      if ('setGuideCreationSuppressed' in at) {
        (at as { setGuideCreationSuppressed(held: boolean): void }).setGuideCreationSuppressed(true)
      }
    }
    function onGuideSuppressKeyUp(ev: KeyboardEvent): void {
      if (ev.key !== 'Control' && ev.key !== 'Meta') return
      // Still logically held if the OTHER modifier key remains down (a rare
      // Ctrl+Cmd chord) — the live event's own flags are the authority.
      if (ev.ctrlKey || ev.metaKey) return
      const at = toolController.activeTool
      if ('setGuideCreationSuppressed' in at) {
        (at as { setGuideCreationSuppressed(held: boolean): void }).setGuideCreationSuppressed(false)
      }
      reportToolHint()
    }
    window.addEventListener('keydown', onGuideSuppressKeyDown)
    window.addEventListener('keyup', onGuideSuppressKeyUp)

    // Same blur-swallows-the-keyup gap as `onWindowBlurClearsShiftLock`
    // above — a Cmd-Tab mid-hold must not strand the suppression on.
    function onWindowBlurClearsGuideSuppress(): void {
      const at = toolController.activeTool
      if ('setGuideCreationSuppressed' in at) {
        (at as { setGuideCreationSuppressed(held: boolean): void }).setGuideCreationSuppressed(false)
      }
      reportToolHint()
    }
    window.addEventListener('blur', onWindowBlurClearsGuideSuppress)

    // Alt held while Paint is active = EYEDROPPER (paint-tool design §1).
    // Unlike Move's bare-Alt durable copy TOGGLE (no live "held right now"
    // signal to reuse — see the note below), the cursor needs the actual
    // live state, so this is its own dedicated keydown/keyup pair, same
    // posture as `onShiftKeyDown`/`onShiftKeyUp` above. `paintEyedropperActive`
    // (not just `ev.altKey`) gates the keyup restore so it only fires when
    // THIS handler is the one that changed the cursor.
    let paintEyedropperActive = false
    function onAltKeyDown(ev: KeyboardEvent): void {
      if (ev.key !== 'Alt') return
      const at = toolController.activeTool
      if (!(at instanceof PaintTool)) return
      at.setEyedropper(true)
      if (paintEyedropperActive) return
      paintEyedropperActive = true
      renderer.domElement.style.cursor = cursorFor('Paint', false, true)
    }
    function onAltKeyUp(ev: KeyboardEvent): void {
      if (ev.key !== 'Alt') return
      const at = toolController.activeTool
      if (at instanceof PaintTool) at.setEyedropper(false)
      if (!paintEyedropperActive) return
      paintEyedropperActive = false
      if (at instanceof PaintTool) {
        renderer.domElement.style.cursor = cursorFor('Paint')
      }
      // `onKeyDownTracked` (below) re-polls the status hint after every
      // 'keydown' — including THIS modifier's own down-press — but there is
      // no 'keyup' equivalent, so a plain Alt release must poll it itself.
      // Without this the cursor and click behavior both correctly stop
      // treating the next click as a sample (they read `PaintTool.eyedropper`
      // live), but the DISPLAYED status text stays stuck on "Alt-click a
      // face to sample" until some unrelated event happens to re-poll it.
      reportToolHint()
    }
    window.addEventListener('keydown', onAltKeyDown)
    window.addEventListener('keyup', onAltKeyUp)

    // A blur (Cmd-Tab, devtools, another window) swallows the keyup that
    // would otherwise release Alt — without this, `paintEyedropperActive`
    // and `PaintTool.eyedropper` stay stuck true: the cursor and status line
    // keep promising "sample" while a click would actually PAINT, since
    // `pointerdown` re-reads `ev.altKey` live as the authority (paint-tool
    // design §1). Same posture as `onWindowBlurClearsPrecision` below — and
    // the same `reportToolHint()` gap `onAltKeyUp` just fixed above applies
    // here too, for the same reason.
    function onWindowBlurClearsEyedropper(): void {
      const at = toolController.activeTool
      if (at instanceof PaintTool) at.setEyedropper(false)
      if (!paintEyedropperActive) return
      paintEyedropperActive = false
      if (at instanceof PaintTool) {
        renderer.domElement.style.cursor = cursorFor('Paint')
      }
      reportToolHint()
    }
    window.addEventListener('blur', onWindowBlurClearsEyedropper)

    // Ctrl+Alt (⌘+⌥ on macOS) held = PRECISION SNAPPING. The kernel's default
    // snap gravity makes a circle's center and quadrant points out-pull the
    // facet endpoints crowding them; holding the chord flattens every weight
    // so the nearest candidate wins again and a facet point is reachable.
    //
    // Why a CHORD and not a bare modifier: all four bare modifiers are taken,
    // and a bare Alt would be actively wrong. Shift is the axis lock across
    // Move/Rotate/Scale/Line (`onShiftKeyDown` above) plus OrbitControls' pan
    // inversion; a bare Control/Meta keydown arms Scale's center-anchor tap
    // (`onCtrlKeyDown` above) or, while Push/Pull is active, its
    // extrude-as-new toggle (`onPushPullModifierKeyDown` above); a bare Alt
    // keydown is MoveTool's and RotateTool's durable copy toggle
    // (`MoveTool.onKey` / `RotateTool.onKey`) and ArcTool's completion-mode
    // cycle (`ArcTool.onKey`), all reached through onKeyDown's unconditional
    // `if (!isMod) activeTool.onKey(ev)` fallback; and the arrow keys are
    // the draw-plane / axis locks.
    //
    // Adding Ctrl/Cmd is exactly what makes the chord safe: `isMod` is
    // `metaKey || ctrlKey`, so with it held onKeyDown never forwards the key
    // to a tool at all, and the Control keydown's own clean-tap arming is
    // disarmed by the Alt that follows it — so neither Move's copy mode nor
    // Scale's anchor is touched. (One residual: pressing Alt *before*
    // Ctrl/Cmd does land a bare Alt on the active tool first. That is the
    // tool's own existing binding firing on its own key, it is visible in the
    // tool's readout, and it is undone by tapping Alt again. Typing the chord
    // in its written order — Ctrl/Cmd first — avoids it entirely.)
    //
    // The state is derived from the modifier flags every key event carries
    // rather than tracked per key, so press/release order never matters and no
    // swallowed keyup can wedge the mode on.
    function precisionHeld(ev: KeyboardEvent): boolean {
      // Never while a text field owns the keyboard: the rename box and the
      // command palette do not stop propagation, and a chord typed in one of
      // them must not flip a viewport mode.
      const target = ev.target as HTMLElement | null
      if (
        target !== null &&
        (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable)
      ) {
        return false
      }
      return ev.altKey && (ev.ctrlKey || ev.metaKey)
    }

    // `setPrecision` is idempotent and reports whether anything changed, so
    // keydown autorepeat costs nothing. On a real change the snap is
    // re-resolved at the LAST ray immediately: the mode has to visibly take
    // effect while the cursor is standing still, exactly like the live re-lock
    // after an axis-lock key. The re-resolve is skipped in the states where
    // the pointer path itself skips snapping and leaves `lastRayRef` stale —
    // camera navigation and an in-flight marquee/drag — so the toggle can
    // never repaint a cue at a ray the user has since moved away from.
    function applyPrecision(on: boolean): void {
      if (!snapService.setPrecision(on)) return
      onPrecisionChangeRef.current?.(on)
      const cached = lastRayRef.current
      const snapPathLive = !cameraModeRef.current && marqueeDrag === null && dragMove === null
      if (cached !== null && snapPathLive) {
        const activeTool = toolController.activeTool
        const constraint = 'snapConstraint' in activeTool
          ? (activeTool as { snapConstraint(ray?: Ray): SnapConstraint | null }).snapConstraint(cached.ray)
          : null
        const { snap } = snapService.resolve(cached.ray, cached.viewportH, cached.basis, constraint?.anchor, constraint?.lockAxis, constraint?.constraintPlane, constraint?.offPlanePoints)
        activeTool.onPointerMove(snap, cached.ray)
        cueLayer.update(snap)
        drawPlaneCueLayer.update(queryDrawPlaneCue(activeTool), getDrawingAxes(wasmScene))
        publishSnapCues(snap, activeTool)
      }
      scheduleRender()
    }
    function onPrecisionKey(ev: KeyboardEvent): void {
      applyPrecision(precisionHeld(ev))
    }
    function onWindowBlurClearsPrecision(): void {
      // A blur (Cmd-Tab, devtools, another window) swallows the keyup that
      // would otherwise release the chord.
      applyPrecision(false)
      // The same blur also swallows the pointerup that would have closed an
      // in-flight press, and time keeps running while the window is away. Two
      // clicks either side of a trip to another application are not a
      // double-click however close together their timestamps land.
      multiClick.reset()
    }
    window.addEventListener('keydown', onPrecisionKey)
    window.addEventListener('keyup', onPrecisionKey)
    window.addEventListener('blur', onWindowBlurClearsPrecision)

    // ------------------------------------------------------------------ animation loop
    let rafId = 0
    let needsRender = true

    function render(): void {
      rafId = requestAnimationFrame(render)
      // `controls.enabled` only gates OrbitControls' OWN input listeners —
      // `update()` itself runs its full position/orientation recomputation
      // regardless, including an unconditional `camera.lookAt(controls.
      // target)` every call. A walkthrough tool (Position Camera/Look
      // Around/Walk, camera.md §4) disables `controls` specifically so it
      // can own the camera outright; calling `update()` anyway would silently
      // re-aim the camera at the stale `controls.target` every frame,
      // discarding the tool's own orientation on the very next repaint. Only
      // run it while controls actually own the camera.
      const changed = controls.enabled && controls.update()
      if (changed || needsRender) {
        // effectiveDistance (not the raw controls distance) so this reacts to
        // an ortho zoom exactly like a perspective dolly would
        // (docs/design/camera.md §1 — CameraRig.effectiveDistance).
        const effDist = rig.effectiveDistance(controls.getDistance())
        // `worldPerPixel` callback is projection-agnostic — no
        // `instanceof PerspectiveCamera` guard needed on either side anymore.
        // Defined here (rather than just below, by updateDiskScale/
        // updateGripScale) so the guide-dash update can share it too.
        const worldPerPixelAt = (dist: number) => rig.worldPerPixel(dist, el.clientHeight)
        // Keep guide-line dashes screen-constant too, the same
        // screen-constant-pixel way the origin axes' negative halves are
        // (see SceneRenderer.updateGuideDashScale and clampOriginAxes above).
        sceneRenderer.updateGuideDashScale(worldPerPixelAt(effDist))
        // Billboard every annotation text quad + keep annotation colors
        // current for the resolved theme (docs/design/dimensions-text.md).
        // Reads `readAppliedTheme()` — a `dataset.theme` DOM read, NOT the
        // `currentTheme` cache above and NOT `getResolvedTheme()` — so it
        // stays correct across BOTH an explicit Settings > Theme change and
        // an OS-level `prefers-color-scheme` flip under 'auto' (the cache is
        // only refreshed by `subscribeTheme`'s explicit-change event and
        // would silently go stale on an OS flip; `getResolvedTheme()` would
        // stay correct but re-queries `matchMedia` every call, which is pure
        // per-frame waste — the same no-per-frame-waste rule as the fat-line
        // resolution cache below (updateFatLineResolutions's doc comment)).
        // `readAppliedTheme()` is just a DOM attribute read, so it pays
        // neither cost.
        sceneRenderer.updateAnnotationBillboards(camera, el.clientWidth, el.clientHeight, readAppliedTheme())
        // Rotate/Protractor/Slice/SectionPlane's single-position preview
        // disks are virtual constructs too — keep them screen-constant the
        // same way (see RotateTool/ProtractorTool/SliceTool/
        // SectionPlaneTool.updateDiskScale, all built on
        // viewport/math.ts's screenConstantWorldHalfFromWorldPerPixel).
        // `worldPerPixelAt` is defined above, alongside the guide-dash update.
        const activeToolForScale = toolController.activeTool
        // Optional CALL (`?.()`), not an `in` check plus a cast. Both hooks are
        // declared optional on `Tool`, so this is fully type-checked against
        // that one declaration: a tool whose signature drifts is a compile
        // error, at the implementor AND here. The former shape — `'x' in tool`
        // narrowing plus `as {...}` — could not be checked at all, because the
        // asserted type was a hand-written duplicate of the interface's
        // signature that nothing kept in sync; that is precisely how
        // PositionTextureTool came to take a viewport height where the
        // callback arrives, sizing every pin to NaN. (`in` narrowing alone
        // does not satisfy the compiler here: it does not strip `undefined`
        // from an optional call signature, so it fails with TS2722 — hence the
        // optional call rather than a plain guarded invocation.)
        activeToolForScale.updateDiskScale?.(camera, worldPerPixelAt)
        // ScaleTool's grip markers are screen-constant size too, but each
        // grip needs its OWN world-space size (they sit at different
        // distances from the camera, unlike a single-position disk) — see
        // ScaleTool.updateGripScale. Called here, before renderer.render(),
        // so the scale takes effect on THIS frame's updateMatrixWorld pass.
        activeToolForScale.updateGripScale?.(camera, worldPerPixelAt)
        // DimensionTool/TextTool need the camera's actual view direction
        // (DimensionTool for `axisDimensionPlane`'s face-on candidate
        // scoring, annotationLayout.ts §3) — `onPointerMove`/`onPointerDown`
        // only carry a per-pixel cursor ray, never the camera itself, so
        // this is their one live feed for it, mirroring
        // updateDiskScale/updateGripScale's own pattern.
        activeToolForScale.updateCamera?.(camera)
        // Feed the shader grid the camera's current position so it can pick
        // the right cell-size decade per fragment; under parallel projection
        // every fragment must use the SAME (depth-independent) LOD instead
        // (see InfiniteGrid's uLodDistanceOverride doc comment).
        infiniteGrid.update(camera.position, rig.projection === 'parallel' ? effDist : null)
        // Orient the origin-axes gizmo to the document's current movable
        // drawing axes (tool-parity §4) before clamping — cheap, so this
        // runs unconditionally every frame rather than only on a change.
        updateOriginAxesFrame(originAxes, wasmScene)
        // Re-clip the axis halves to the enlarged frustum (float64 — see
        // clampOriginAxes; the fat-line shader's own handling of extreme
        // segments is what shimmered).
        clampOriginAxes(originAxes, rig, el.clientHeight)
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

    // Low-level capture: camera state on every orbit/pan/zoom change, and
    // keys (Shift axis-lock, Esc/Enter/Del). All no-ops unless recording.
    // (Both this and `scheduleRender` are wired to `controls`' 'change' event
    // via `attachControlsListeners`, called once at controls-creation time
    // above and again by `toggleProjection` after it recreates `controls`.)
    function recordCameraInput(): void {
      if (!inputRecorder.isActive()) return
      // The ortho frustum height/zoom are additive and present only under
      // parallel projection (docs/design/camera.md §5) — the minimal pair
      // needed to reconstruct the orthographic frustum on replay; absent
      // under perspective, where no ortho frustum is in play.
      inputRecorder.recordCamera(
        [camera.position.x, camera.position.y, camera.position.z],
        [controls.target.x, controls.target.y, controls.target.z],
        [camera.up.x, camera.up.y, camera.up.z],
        rig.perspective.fov,
        rig.projection,
        rig.projection === 'parallel' ? rig.orthographic.top - rig.orthographic.bottom : undefined,
        rig.projection === 'parallel' ? rig.orthographic.zoom : undefined,
      )
    }
    function onKeyDownRecord(ev: KeyboardEvent): void {
      inputRecorder.recordKey('keydown', ev)
    }
    function onKeyUpRecord(ev: KeyboardEvent): void {
      inputRecorder.recordKey('keyup', ev)
    }
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

      // Shift-fov drag owns the pointer for its whole gesture — armed at
      // pointerdown by onFovDragPointerDownCapture (camera-playtest2.md §3).
      if (fovDragState !== null) {
        updateFovDrag(ev)
        return
      }

      // Zoom Window owns the pointer for its whole gesture — see onPointerDown.
      if (zoomWindowActive) {
        updateZoomWindowDrag(ev)
        return
      }

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

      // Armed annotation offset/leader drag: past the threshold, live-preview
      // the new placement and own the pointer entirely (no tool routing) —
      // committed as ONE kernel call on release (onPointerUp).
      if (annotationDrag !== null) {
        if ((ev.buttons & 1) === 0) {
          // The release happened outside our listeners (focus loss) — drop it.
          _cancelAnnotationDrag()
        } else {
          if (!annotationDrag.active) {
            const [px, py] = canvasPoint(ev)
            if (exceedsDragThreshold(annotationDrag.startX, annotationDrag.startY, px, py)) {
              annotationDrag.active = true
            }
          }
          if (annotationDrag.active) {
            const cursorWorld = _resolveAnnotationDragPoint(annotationDrag.snapshot, ray)
            if (cursorWorld !== null) _updateAnnotationDragPreview(annotationDrag.snapshot, cursorWorld)
            scheduleRender()
            return
          }
        }
      }

      const viewportH = el.clientHeight
      const basis = apertureBasis()

      // Cache for live re-lock after key events
      lastRayRef.current = { ray, viewportH, basis }

      const activeTool = toolController.activeTool
      // A screen-space drag tool (Look Around/Walk, camera.md §4 — see
      // Tool.onPointerRawMove's doc) has no world pick of its own: skip the
      // snap resolve entirely (a wasted wasm call every frame) rather than
      // resolving one nothing will read, which would otherwise leave a
      // stale/misleading inference cue floating over the scene while walking.
      const isRawDragTool = 'onPointerRawMove' in activeTool
      const constraint = !isRawDragTool && 'snapConstraint' in activeTool
        ? (activeTool as { snapConstraint(ray?: Ray): SnapConstraint | null }).snapConstraint(ray)
        : null
      const { snap } = isRawDragTool
        ? { snap: null }
        : snapService.resolve(ray, viewportH, basis, constraint?.anchor, constraint?.lockAxis, constraint?.constraintPlane, constraint?.offPlanePoints)
      activeTool.onPointerMove(snap, ray)
      if (isRawDragTool) {
        const [rawX, rawY] = canvasPoint(ev)
        ;(activeTool as {
          onPointerRawMove(xPx: number, yPx: number, buttons: number, mods: { shift: boolean }): void
        }).onPointerRawMove(rawX, rawY, ev.buttons, { shift: ev.shiftKey })
      }
      cueLayer.update(snap)
      drawPlaneCueLayer.update(queryDrawPlaneCue(activeTool), getDrawingAxes(wasmScene))
      scheduleRender()

      publishSnapCues(snap, activeTool)
    }

    /** Status-bar text + the cursor-anchored inference chip/dot for a freshly
     * resolved snap. Shared by the pointer-move path and by anything that
     * re-resolves at the cached ray without a pointer move (the precision-mode
     * toggle) — those must refresh the readouts too, or the chip goes stale
     * and reports a snap that is no longer the winner. */
    function publishSnapCues(snap: Snap | null, activeTool: Tool): void {
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
        // Read AFTER the tool's own onPointerMove (both callers order it that
        // way), so this reflects what the tool just did with this very snap.
        onInferenceChangeRef.current?.({
          kind: snapKind ?? snap.kind,
          screenX: p.x,
          screenY: p.y,
          direction: snap.direction,
          frame: getDrawingAxes(wasmScene),
          projected: activeTool.snapProjected?.() ?? false,
        })
      }
    }

    // ------------------------------------------------------------------ pointer down
    // --- construction-guide picking ---------------------------------
    const GUIDE_PICK_PX = 8
    const ANNOTATION_PICK_PX = 8
    // Matches SceneRenderer's GUIDE_LINE_HALF_LENGTH (the rendered extent).
    const GUIDE_LINE_SAMPLE_HALF = 50

    function worldToPixels(p: THREE.Vector3): { x: number; y: number; behind: boolean } {
      const v = p.clone().project(camera)
      const w = renderer.domElement.clientWidth
      const h = renderer.domElement.clientHeight
      // `isBehindCamera` (cameraRig.ts) is projection-agnostic via
      // camera-space z — the old NDC `v.z > 1` check was a perspective-only
      // heuristic that inverts under orthographic projection (see its doc).
      return { x: (v.x * 0.5 + 0.5) * w, y: (-v.y * 0.5 + 0.5) * h, behind: isBehindCamera(p, camera) }
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

    /** Nearest annotation within ANNOTATION_PICK_PX of the NDC click, or null
     * (docs/design/dimensions-text.md — "annotations are pickable via
     * screen-space distance"). Tests the segment a real user would actually
     * click: the dimension line for a linear dimension, the leader for a
     * radial dimension or leader text — the same segment-sampling technique
     * `pickGuide` uses above, just against the annotation's own two key
     * points instead of a sampled infinite line. */
    function pickAnnotation(ndcX: number, ndcY: number): bigint | null {
      const w = renderer.domElement.clientWidth
      const h = renderer.domElement.clientHeight
      const clickX = (ndcX * 0.5 + 0.5) * w
      const clickY = (-ndcY * 0.5 + 0.5) * h
      let best: bigint | null = null
      let bestDist = ANNOTATION_PICK_PX
      for (const id of wasmScene.annotation_ids()) {
        const kind = wasmScene.annotation_kind(id)
        if (kind === undefined) continue
        let p0: V3 | undefined
        let p1: V3 | undefined
        if (kind === 'linear') {
          const a = wasmScene.annotation_anchor_point(id, 0)
          const b = wasmScene.annotation_anchor_point(id, 1)
          const offset = wasmScene.annotation_offset(id)
          if (a === undefined || b === undefined || offset === undefined) continue
          p0 = [a[0] + offset[0], a[1] + offset[1], a[2] + offset[2]]
          p1 = [b[0] + offset[0], b[1] + offset[1], b[2] + offset[2]]
        } else if (kind === 'radial') {
          const anchor = wasmScene.annotation_anchor_point(id, 0)
          const leaderDir = wasmScene.annotation_leader_dir(id)
          if (anchor === undefined || leaderDir === undefined) continue
          p0 = [anchor[0], anchor[1], anchor[2]]
          p1 = [anchor[0] + leaderDir[0], anchor[1] + leaderDir[1], anchor[2] + leaderDir[2]]
        } else {
          const anchor = wasmScene.annotation_anchor_point(id, 0)
          const offset = wasmScene.annotation_offset(id)
          if (anchor === undefined || offset === undefined) continue
          p0 = [anchor[0], anchor[1], anchor[2]]
          p1 = [anchor[0] + offset[0], anchor[1] + offset[1], anchor[2] + offset[2]]
        }
        const a2 = worldToPixels(new THREE.Vector3(p0[0], p0[1], p0[2]))
        const b2 = worldToPixels(new THREE.Vector3(p1[0], p1[1], p1[2]))
        let d = Infinity
        if (!a2.behind) d = Math.min(d, Math.hypot(a2.x - clickX, a2.y - clickY))
        if (!b2.behind) d = Math.min(d, Math.hypot(b2.x - clickX, b2.y - clickY))
        if (!a2.behind && !b2.behind) d = Math.min(d, pointSegPx(clickX, clickY, a2.x, a2.y, b2.x, b2.y))
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
      // Counted here, before any of the early returns below, so the sequence
      // reflects every primary press the canvas actually received rather than
      // only the ones that reach the tool-routing guard further down.
      const clickCount = Math.max(ev.detail, multiClick.press(ev))
      // A Shift-fov drag is already armed by now — onFovDragPointerDownCapture
      // (an `el`-level capture-phase listener, wired further down) runs
      // BEFORE this bubble-phase handler and before OrbitControls' own
      // pointerdown handler for the very same event (see its doc). Nothing
      // left to do here but decline to fall through to camera-nav/geometry
      // routing below.
      if (fovDragState !== null) return
      // Zoom Window owns left-drag itself (a one-shot rectangle, not
      // OrbitControls' native orbit/pan/dolly) — checked before the
      // camera-nav early-return below so it isn't swallowed by it.
      if (zoomWindowActive) {
        beginZoomWindowDrag(ev)
        return
      }
      // In camera-nav mode, OrbitControls owns left-drag — skip geometry routing.
      if (cameraModeRef.current) return

      const [ndcX, ndcY] = pointerToNDC(ev, renderer.domElement)
      const ray = makeWorldRay(ndcX, ndcY, camera)
      const viewportH = el.clientHeight
      const basis = apertureBasis()

      // Record shift state so handleSelect (driven by the tool's onSelect) can
      // treat this click as additive multi-select.
      selectAdditiveRef.current = ev.shiftKey

      if (toolController.activeToolName === 'Select') {
        const [px, py] = canvasPoint(ev)
        const topLevel = activeContextRef.current.length === 0

        // A press directly on an annotation selects it and arms its own
        // offset/leader drag (docs/design/dimensions-text.md) — a thin
        // deliberate target, like a guide, but (unlike a guide) draggable in
        // place rather than only click-pickable.
        if (!ev.shiftKey) {
          const annotationId = pickAnnotation(ndcX, ndcY)
          if (annotationId !== null) {
            onSelectAnnotationRef.current?.(annotationId)
            const snapshot = readAnnotation(wasmScene, annotationId)
            if (snapshot !== null) {
              annotationDrag = { id: annotationId, snapshot, startX: px, startY: py, active: false }
              renderer.domElement.setPointerCapture(ev.pointerId)
            }
            scheduleRender()
            return
          }
        }

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

      // The second pointerdown of a double-click is the phantom that precedes
      // the 'dblclick' event. For tools that finish on double-click (LineTool
      // ending a chain, PushPullTool repeating its last distance), skip
      // routing it so it can't place a spurious near-duplicate point or
      // free-commit at ~0 distance; the 'dblclick' handler runs onDoubleClick
      // instead. Distinct clicks always count 1, so normal point-by-point
      // drawing is unaffected at any cadence.
      //
      // The count is `max(ev.detail, tracker)` rather than `ev.detail` alone
      // because Chromium leaves `detail` at 0 on POINTER events — only its
      // MouseEvent-derived click/dblclick carry real counts — so this guard
      // never fired there, on the web app in Chrome/Edge and in the Windows
      // desktop shell's Chromium-based WebView2 alike. See `multiClick.ts` for
      // the measurement and for why the tracker is deliberately stricter than
      // the browsers' own double-click window. A browser that DOES report a
      // count still decides for itself, since its value wins the max.
      if (clickCount >= 2 && 'onDoubleClick' in activeTool) return

      // Paint tool modifiers (paint-tool design §1–2), all read live off this
      // pointerdown — the same idiom as the whole-object fill below. Alt is
      // read here too even though the cursor already tracks it continuously
      // (onAltKeyDown/onAltKeyUp below): belt-and-suspenders, and it keeps
      // every modifier read from one place. Shift wins over plain ⌘/Ctrl (Alt
      // wins over both, inside the tool itself) — Ctrl/Cmd+Shift replaces
      // within the object, ⌘/Ctrl alone still fills the whole object.
      if (activeTool instanceof PaintTool) {
        activeTool.setEyedropper(ev.altKey)
        if (ev.shiftKey) {
          activeTool.setReplaceScope(ev.metaKey || ev.ctrlKey ? 'object' : 'document')
          activeTool.setWholeObject(false)
        } else {
          activeTool.setReplaceScope(null)
          activeTool.setWholeObject(ev.metaKey || ev.ctrlKey)
        }
      }
      // ⌘/Ctrl-click on Follow Me's profile commits the MERGED sweep (design
      // §3b) — a live read of the real event, same precedent as Paint's
      // whole-object fill above; Move's own copy modifier is a durable TAP
      // with no live "held right now" signal to reuse (see FollowMeTool's
      // MERGE GESTURE doc note). Also read at the drag RELEASE (`onPointerUp`
      // below) so a press-drag-release commit sees the modifier at commit
      // time, not whatever was held at the arming press.
      if (activeTool instanceof FollowMeTool) {
        activeTool.setMergeModifier(ev.metaKey || ev.ctrlKey)
      }
      // (Move's copy mode is a durable Alt TOGGLE handled in MoveTool.onKey —
      // no live Alt-modifier tracking here.)

      const constraint = 'snapConstraint' in activeTool
        ? (activeTool as { snapConstraint(ray?: Ray): SnapConstraint | null }).snapConstraint(ray)
        : null
      const { snap } = snapService.resolve(ray, viewportH, basis, constraint?.anchor, constraint?.lockAxis, constraint?.constraintPlane, constraint?.offPlanePoints)
      activeTool.onPointerDown(snap, ray)
      // Walk's press-relative drag (camera.md §4) — see Tool.onPointerRawDown's doc.
      if ('onPointerRawDown' in activeTool) {
        const [rawX, rawY] = canvasPoint(ev)
        ;(activeTool as { onPointerRawDown(xPx: number, yPx: number): void }).onPointerRawDown(rawX, rawY)
        // Track the drag even when it leaves the canvas (same as
        // dragMove/marquee/Zoom Window above) — Look Around/Walk drive their
        // whole gesture off raw canvas-pixel deltas, so losing pointermove
        // the instant the cursor crosses the canvas edge would freeze the
        // camera mid-drag instead of merely clamping its input.
        renderer.domElement.setPointerCapture(ev.pointerId)
      }
    }

    // Double-click a node to enter its context (SketchUp-style).
    // At top level: enters the topmost ancestor group/instance/object.
    // Inside a group: enters the direct child of that group.
    // Inside an instance: enters the instance's definition for editing.
    /** Open the in-viewport text editor for a live annotation `id`
     * (double-click-to-edit, docs/design/dimensions-text.md): reads its full
     * current geometry (`readAnnotation`), stashes it as the pending edit
     * target, and asks the parent to render the editor at the label's
     * projected screen position, prefilled per `initialEditorText`. */
    function _openAnnotationEditorFor(id: bigint): void {
      const snapshot = readAnnotation(wasmScene, id)
      if (snapshot === null) return
      pendingAnnotationEdit = { kind: 'edit', id, snapshot }
      const worldPos = sceneRenderer.annotationTextWorldPosition(id)
      const p = worldPos !== null
        ? worldToPixels(new THREE.Vector3(worldPos[0], worldPos[1], worldPos[2]))
        : { x: el.clientWidth / 2, y: el.clientHeight / 2, behind: false }
      onOpenAnnotationEditorRef.current?.({
        id,
        screenX: p.x,
        screenY: p.y,
        initialText: initialEditorText(snapshot),
        placeholder: snapshot.kind === 'leader' ? 'Leader text…' : 'Override (blank = computed)',
      })
    }

    function onDoubleClick(ev: MouseEvent): void {
      if (ev.button !== 0) return
      const [ndcX, ndcY] = pointerToNDC(ev, renderer.domElement)
      const ray = makeWorldRay(ndcX, ndcY, camera)

      // A double-click directly on an annotation opens its text editor
      // (Select tool only — mirrors the annotation-drag priority check in
      // onPointerDown) instead of falling through to the tool/enter-context
      // gestures below.
      if (toolController.activeToolName === 'Select') {
        const annotationId = pickAnnotation(ndcX, ndcY)
        if (annotationId !== null) {
          _openAnnotationEditorFor(annotationId)
          return
        }
      }

      // Let the active tool consume the double-click first (e.g. LineTool
      // ending a chain) — only fall through to the default "enter context"
      // gesture below when the tool doesn't handle it (or isn't mid-gesture).
      const activeTool = toolController.activeTool
      if ('onDoubleClick' in activeTool) {
        const viewportH = el.clientHeight
        const basis = apertureBasis()
        const constraint = 'snapConstraint' in activeTool
          ? (activeTool as { snapConstraint(ray?: Ray): SnapConstraint | null }).snapConstraint(ray)
          : null
        const { snap } = snapService.resolve(ray, viewportH, basis, constraint?.anchor, constraint?.lockAxis, constraint?.constraintPlane, constraint?.offPlanePoints)
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
          // Resolve to the selectable node in the current context (scoped
          // to the innermost session's own members, if one is open — a
          // double-click on a dimmed sibling outside the session must not
          // enter it), then enter it.
          const selectable = resolvePickToSelectable(
            wasmScene, objectId, activeContextRef.current, instanceId, sessionScopeFor(activeContextRef.current),
          )
          if (selectable !== null) {
            if (activeContextRef.current.length > 0) {
              // Already drilled into an object/K1-K2-instance context on top
              // of whatever session (if any) sits beneath it — the resolved
              // selectable can only be a plain object at this depth
              // (`resolvePickToSelectableUnscoped`'s in-context branches),
              // so this is always a further object-context push, same as
              // before groups/nested sessions existed.
              onEnterContextRef.current?.(selectable)
            } else if (selectable.kind === 'group') {
              // A group at the current depth opens a GROUP session
              // (docs/design/group-session.md) instead of a K1/K2 context
              // push — reachable even while another session is ALREADY
              // open (nesting: the kernel's LIFO stack composes group→
              // group→…→optionally one component at the very end), unlike
              // the old single-explode-session model where a session made
              // any further entry unreachable.
              runOpenGroupSession(selectable.id)
            } else if (selectable.kind === 'instance') {
              // A component instance opens an explode session — silently
              // falling back to the ordinary "enter this instance's edit
              // context" gesture (K1/K2) when the instance's pose can't
              // support one (non-uniform scale or a mirror), or a sibling
              // placement sits inside a group. Works with a group session
              // already open beneath it now too — only ANOTHER component
              // frame already open (always innermost) refuses.
              openExplodeSessionOrFallback(selectable.id, selectable)
            } else {
              // A plain object at the current depth: an object-context push
              // is now allowed even while a session is open (design: it
              // sits logically inside the innermost frame).
              onEnterContextRef.current?.(selectable)
            }
          }
        } finally {
          pick.free()
        }
      } else if (!toolHasArmedGesture(toolController.activeTool)) {
        // Double-click on empty space: pop the object context first if one
        // is open, else close the innermost session frame — one layer per
        // gesture (docs/design/group-session.md), mirroring the Escape
        // gesture's own layering below. Like Escape, this must NOT yank
        // anything out from under an armed tool gesture (adversarial-review
        // finding, the explode-session-only precedent this generalizes):
        // the double-click's own first click already went to the tool, so
        // closing here mid-gesture would fold state back between a
        // gesture's arm and its commit. An armed tool keeps the
        // double-click; the user cancels or commits first, then closes.
        if (activeContextRef.current.length > 0) {
          onExitContextRef.current?.()
        } else if (sessionStackRef.current.length > 0) {
          runCloseInnermostSession()
        }
      }
    }

    // ------------------------------------------------------------------ keyboard
    function onKeyDown(ev: KeyboardEvent): void {
      // Field of View typed entry (design §2) takes priority over everything
      // below while the Zoom camera mode is active and a digit has started
      // the buffer — mirrors how a real tool's capturesKey pre-empts the
      // bare-letter shortcuts further down.
      if (handleFovEntryKey(ev)) {
        scheduleRender()
        return
      }

      const isMod = ev.metaKey || ev.ctrlKey

      // Esc aborts an in-flight Shift-fov drag before anything else
      // (camera-playtest2.md §3) — restores MOUSE.DOLLY, no fov rollback
      // (see abortFovDrag's doc).
      if (ev.key === 'Escape' && fovDragState !== null) {
        abortFovDrag()
        return
      }

      // Esc cancels an in-flight Zoom Window drag before anything else — a
      // DIFFERENT drag state than the Select tool's marquee/drag-move below
      // (Zoom Window owns left-drag itself; see its mode-switch comment).
      if (ev.key === 'Escape' && zoomWindowDrag !== null) {
        abortZoomWindowDrag()
        return
      }

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

      // Esc cancels an in-flight annotation offset/leader drag, discarding
      // the preview without committing anything.
      if (ev.key === 'Escape' && annotationDrag !== null) {
        _cancelAnnotationDrag()
        return
      }

      // Esc: an armed gesture (Move/Rotate/Scale/PushPull/FollowMe/Offset/
      // Slice mid-drag, or a draw tool's anchored chain — anything reporting
      // `capturingInput() === true`) consumes the key itself FIRST, before
      // the context pop below ever runs. Without this check, popping the
      // context first would silently retarget an armed gesture: the Viewport
      // pushes the new (shallower) EditContext into the still-armed tool via
      // `applyEditContext`/`setEditContext` (which does NOT re-check here —
      // see `editContextEq` in tools/types.ts), so completing the gesture
      // would commit under the wrong context instead of being cleanly
      // cancelled by the Escape the user actually pressed. Only when the
      // tool has nothing armed does Escape fall through to its traditional
      // meaning of popping one level off the context path.
      if (ev.key === 'Escape' && activeContextRef.current.length > 0) {
        const activeTool = toolController.activeTool
        if (toolHasArmedGesture(activeTool)) {
          activeTool.onKey(ev)
          drawPlaneCueLayer.update(queryDrawPlaneCue(activeTool))
          scheduleRender()
          return
        }
        onExitContextRef.current?.()
        return
      }

      // Escape closes the INNERMOST open session frame — reached only once
      // the branch above found no object/K1-K2-instance context to pop
      // first (docs/design/group-session.md: pop the object context, else
      // close the innermost frame — one layer per Escape), with the same
      // armed-gesture priority.
      if (ev.key === 'Escape' && sessionStackRef.current.length > 0) {
        const activeTool = toolController.activeTool
        if (toolHasArmedGesture(activeTool)) {
          activeTool.onKey(ev)
          drawPlaneCueLayer.update(queryDrawPlaneCue(activeTool))
          scheduleRender()
          return
        }
        runCloseInnermostSession()
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
              ? (activeTool as { snapConstraint(ray?: Ray): SnapConstraint | null }).snapConstraint(cached.ray)
              : null
            const { snap } = snapService.resolve(cached.ray, cached.viewportH, cached.basis, constraint?.anchor, constraint?.lockAxis, constraint?.constraintPlane, constraint?.offPlanePoints)
            activeTool.onPointerMove(snap, cached.ray)
            cueLayer.update(snap)
            drawPlaneCueLayer.update(queryDrawPlaneCue(activeTool), getDrawingAxes(wasmScene))
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
        drawPlaneCueLayer.update(queryDrawPlaneCue(activeTool), getDrawingAxes(wasmScene))
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
    // pick chain); a dragged release commits the marquee selection. Every
    // OTHER tool is click-based (a press arms, a SECOND press commits) and
    // needs nothing here beyond input recording — EXCEPT a tool that opts in
    // via the optional `Tool.onPointerUp` hook (Follow Me's drag-to-partial-
    // sweep, E4 — see the interface doc for why click-move-click alone can't
    // express that gesture), dispatched first and independently of the
    // Select-only `dragMove`/`marqueeDrag` state below.
    function onPointerUp(ev: PointerEvent): void {
      recordPointerInput('pointerup', ev)
      if (ev.button !== 0) return
      // Closes the press counted in onPointerDown. Recorded before any of the
      // early returns below for the same reason the press is: a release
      // consumed by a camera or FOV drag still ends that press, and a press
      // that travelled must break the click run rather than leave a stale
      // anchor for the next one to pair with.
      multiClick.release(ev)
      // Shift-fov drag owns the pointer for its whole gesture — see
      // onPointerDown. Scoped to the arming pointer (findings 1+2,
      // camera-playtest2 DELTA review): an unrelated pointer's OWN release
      // still falls into this branch (a drag stays "owned" until its own
      // pointer's exit) but must not commit someone else's gesture —
      // `finishFovDrag` no-ops for the wrong pointerId.
      if (fovDragState !== null) {
        finishFovDrag(ev.pointerId)
        return
      }
      // Zoom Window owns the pointer for its whole gesture — see onPointerDown.
      if (zoomWindowActive) {
        finishZoomWindowDrag(ev)
        return
      }
      if (!cameraModeRef.current) {
        const activeTool = toolController.activeTool
        // A live read at the RELEASE too (see the onPointerDown wiring's doc)
        // — a press-drag-release's merge decision is whatever is held at the
        // commit, not the arming press.
        if (activeTool instanceof FollowMeTool) {
          activeTool.setMergeModifier(ev.metaKey || ev.ctrlKey)
        }
        if ('onPointerUp' in activeTool) {
          const [ndcX, ndcY] = pointerToNDC(ev, renderer.domElement)
          const ray = makeWorldRay(ndcX, ndcY, camera)
          const constraint = 'snapConstraint' in activeTool
            ? (activeTool as { snapConstraint(ray?: Ray): SnapConstraint | null }).snapConstraint(ray)
            : null
          const { snap } = snapService.resolve(ray, el.clientHeight, apertureBasis(), constraint?.anchor, constraint?.lockAxis, constraint?.constraintPlane, constraint?.offPlanePoints)
          ;(activeTool as { onPointerUp(snap: Snap | null, ray: Ray): void }).onPointerUp(snap, ray)
          scheduleRender()
        }
      }

      // Annotation drag release: an ACTIVE drag commits the new offset/
      // leader placement in ONE kernel call at the release position; a
      // sub-threshold release is a plain click — the annotation was already
      // selected at the arming press, nothing further to do.
      if (annotationDrag !== null) {
        const ad = annotationDrag
        annotationDrag = null
        if (ad.active) {
          const [ndcX, ndcY] = pointerToNDC(ev, renderer.domElement)
          const ray = makeWorldRay(ndcX, ndcY, camera)
          const cursorWorld = _resolveAnnotationDragPoint(ad.snapshot, ray)
          _commitAnnotationDrag(ad.id, ad.snapshot, cursorWorld)
          scheduleRender()
        } else {
          _clearAnnotationDragPreview()
        }
        return
      }

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
              ? (tool as { snapConstraint(ray?: Ray): SnapConstraint | null }).snapConstraint(ray)
              : null
            const { snap } = snapService.resolve(ray, el.clientHeight, apertureBasis(), constraint?.anchor, constraint?.lockAxis, constraint?.constraintPlane, constraint?.offPlanePoints)
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
      // A real pointer-capture loss (OS gesture interruption, etc.) — abort
      // the fov drag exactly like Escape/focus-loss (camera-playtest2.md
      // §3), scoped to the arming pointer (findings 1+2, camera-playtest2
      // DELTA review): this listener fires for EVERY pointer's own cancel,
      // so an unrelated pointer's cancellation must not end someone else's
      // still-armed drag.
      abortFovDrag(ev.pointerId)
      clearMarquee()
      abortDragMove()
      if (zoomWindowDrag !== null) {
        // Mirrors abortDragMove's own precedent: an ACTIVE (past-threshold)
        // gesture cancelled mid-flight still springs back to Select — the
        // one-shot is consumed by starting the drag, not only by finishing
        // it — while a bare armed-but-not-yet-dragging press quietly drops
        // (the user never left the idle Zoom Window state in any visible way).
        const wasActive = zoomWindowDrag.active
        zoomWindowDrag = null
        marqueeOverlay.style.display = 'none'
        if (wasActive) onToolRevertedRef.current?.()
      }
      _cancelAnnotationDrag()
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
    // Shift-fov drag/wheel pre-emption (camera-playtest2.md §3): CAPTURE
    // phase on `el` (the canvas's PARENT, not the canvas itself — see
    // onFovDragPointerDownCapture's doc for why registering on the canvas
    // wouldn't run early enough to beat OrbitControls' own listener).
    el.addEventListener('pointerdown', onFovDragPointerDownCapture, true)
    el.addEventListener('wheel', onFovWheelCapture, { capture: true, passive: false })
    // A real pointer-capture loss (unrelated to our own release/abort
    // paths, e.g. the OS reclaiming capture) — abort exactly like Escape,
    // scoped to the arming pointer (findings 1+2, camera-playtest2 DELTA
    // review): `lostpointercapture` fires per-pointer, and OrbitControls
    // itself calls `setPointerCapture` on OTHER pointers for its own
    // gestures (e.g. a second touch mid pinch-zoom) — that pointer's own
    // capture loss must not end a drag it never armed.
    const onFovDragLostPointerCapture = (ev: PointerEvent) => abortFovDrag(ev.pointerId)
    renderer.domElement.addEventListener('lostpointercapture', onFovDragLostPointerCapture)
    // Cmd-Tab / devtools / another window — the button may still be
    // physically held with no further event ever reaching us. Unlike the
    // listeners above, a window blur has no pointer of its own to scope
    // to and genuinely should end the drag regardless of which pointer
    // armed it — the omitted argument gets the unconditional abort.
    const onFovDragWindowBlur = () => abortFovDrag()
    window.addEventListener('blur', onFovDragWindowBlur)

    // ------------------------------------------------------------------ resize
    const resizeObserver = new ResizeObserver(() => {
      const w = el.clientWidth
      const h = el.clientHeight
      rig.setAspect(w / h)
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
      window.removeEventListener('blur', onWindowBlurClearsShiftLock)
      window.removeEventListener('keydown', onCtrlKeyDown, true)
      window.removeEventListener('keyup', onCtrlKeyUp)
      window.removeEventListener('keydown', onPushPullModifierKeyDown)
      window.removeEventListener('keyup', onPushPullModifierKeyUp)
      window.removeEventListener('keydown', onGuideSuppressKeyDown)
      window.removeEventListener('keyup', onGuideSuppressKeyUp)
      window.removeEventListener('blur', onWindowBlurClearsGuideSuppress)
      window.removeEventListener('keydown', onAltKeyDown)
      window.removeEventListener('keyup', onAltKeyUp)
      window.removeEventListener('blur', onWindowBlurClearsEyedropper)
      window.removeEventListener('keydown', onPrecisionKey)
      window.removeEventListener('keyup', onPrecisionKey)
      window.removeEventListener('blur', onWindowBlurClearsPrecision)
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
      el.removeEventListener('pointerdown', onFovDragPointerDownCapture, true)
      el.removeEventListener('wheel', onFovWheelCapture, true)
      renderer.domElement.removeEventListener('lostpointercapture', onFovDragLostPointerCapture)
      window.removeEventListener('blur', onFovDragWindowBlur)
      resizeObserver.disconnect()
      unsubscribeTheme()
      unsubscribeLengthUnit()
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

  // React to activeTool changes from parent without re-mounting the effect.
  //
  // MUST be a layout effect, not a passive one: switchToolRef.current arms
  // the pointer-handling state a tool activation depends on (Zoom Window's
  // `zoomWindowActive`, camera tools' `cameraModeRef`/`mouseButtons.LEFT`,
  // walkthrough tools' `controls.enabled`, …). A passive `useEffect` is
  // flushed on a LATER macrotask, after the browser has already had a
  // chance to paint — so a menu click that sets `activeTool` can be
  // followed by a real pointerdown (a fast click-drag straight from the
  // Camera menu onto the canvas) BEFORE that effect ever runs. That
  // pointerdown then sees the OLD arm-state and gets routed to whatever
  // the PREVIOUS tool was (typically Select's marquee), silently
  // swallowing the gesture — this was the "Zoom Window ignores the first
  // drag" defect: the drag wasn't lost, it was misrouted, and the very
  // next drag "worked" only because by then the effect had long since
  // flushed. `useLayoutEffect` runs synchronously right after the commit,
  // before the browser can paint or dispatch another input event, so the
  // arm-state is always current by the time any subsequent pointer event
  // can possibly arrive.
  useLayoutEffect(() => {
    if (activeToolProp !== undefined && switchToolRef.current !== null) {
      // Last-applied guard (see `shouldSkipToolSwitch`'s and
      // `lastAppliedToolNameRef`'s doc comments): this effect re-runs both
      // for genuine parent-driven switches AND for the ECHO of an internal
      // transition Viewport itself just reported via `onInternalToolChange`
      // (Position Camera's auto-handoff to Look Around, Escape-to-Select,
      // …) — by the time that echo lands here the tool controller has
      // already made the switch, so re-invoking `switchToolRef` would
      // re-run the whole switch body a second time for a transition that
      // already happened (double-applying the walkthrough-exit reseed,
      // tearing down and replacing the tool instance the first invocation
      // just created). Compare against `lastAppliedToolNameRef`, NOT
      // `toolController.activeToolName` — several switch-body branches
      // (Orbit/Pan/Zoom/Zoom Window/default) call `resetToSelect()`, so the
      // controller's name is 'Select' while one of those is active and
      // would wrongly match (and skip) an unrelated later switch TO
      // 'Select'. Skip only when the requested name is already the last one
      // actually applied AND nothing forced a reapply — an explicit
      // reselect of an already-active camera tool still bumps
      // `activeToolSeqProp` (App.tsx `toolActivationSeq`) and must still go
      // through.
      const seqChanged = activeToolSeqProp !== lastAppliedToolSeqRef.current
      if (!shouldSkipToolSwitch(activeToolProp, lastAppliedToolNameRef.current, seqChanged)) {
        switchToolRef.current(activeToolProp)
      }
    }
    lastAppliedToolSeqRef.current = activeToolSeqProp
    // `activeToolSeqProp` is intentionally not read above (beyond the guard)
    // — it exists only to force this effect to re-run when the parent
    // explicitly reselects an unchanged `activeToolProp` (see the prop's
    // doc comment).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeToolProp, activeToolSeqProp])

  // Reflect the editing context path into the renderer (isolation fade) and the
  // active tool (scoped editing) when the parent changes it.
  useEffect(() => {
    activeContextRef.current = activeContext
    const editContext = computeEditContext(wasmSceneRef.current, activeContext)
    wasmSceneRef.current.set_active_inference_instance(
      editContext.kind === 'instance' ? editContext.id : undefined,
    )
    // Compute the lit-instance set for isolation from the WHOLE context
    // chain (component-edit-parity.md Finding 2, plus the delta-review
    // Finding 2/3 fixes on top of it) — member ids alone (`activeLitSet`)
    // can't tell one instance's placement apart from a sibling instance of
    // the SAME definition, since they share the exact same member geometry,
    // and a group context must light its own member instances too. See
    // `computeLitInstances` for the full rationale.
    const litInstances = computeLitInstances(wasmSceneRef.current, activeContext)
    // While a session is open with NO object context pushed on top of it,
    // the session owns the renderer's isolation fade
    // (`refreshSessionScope`): this effect can refire while a session is
    // open even though `activeContext` is (still) empty — App's
    // session-open state churn is enough to hand `activeContext` a fresh
    // (still empty) identity — and re-asserting `activeLitSet ?? null` here
    // clobbered the session's scope back to "nothing dimmed" (verified in a
    // real-browser trace). Re-assert the session's own scope in that case
    // instead, so the two writers agree instead of racing. An object
    // context CAN sit on top of a session now (design: object pushes are
    // allowed while a session is open) — once one is pushed, drilling in
    // narrows scope further, so the ordinary K1/K2 lit sets win instead
    // (matching pre-session behavior exactly at that depth).
    const inSession = activeContext.length === 0
    const objectLit = inSession ? explodeSessionObjectIdsRef.current ?? activeLitSet ?? null : activeLitSet ?? null
    const instanceLit = inSession ? sessionInstanceIdsRef.current ?? litInstances : litInstances
    sceneRendererRef.current?.setActiveContext(objectLit, instanceLit)
    const tool = toolControllerRef.current?.activeTool
    if (tool !== undefined) {
      // The single editing-context channel (component-edit-parity.md phase
      // A1) — replaces the four separate id/boolean channels this effect
      // used to compute by hand (`setActiveContext`/`setComponentContext`/
      // `setContextScoped`/`setActiveGroup`).
      applyEditContext(tool, editContext)
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
    const deepest = activeContext.length > 0 ? activeContext[activeContext.length - 1] : null
    const activeInstance = deepest?.kind === 'instance' ? deepest.id : null
    sceneRendererRef.current?.setSelected(activeInstance === null ? leafIds : [])
    sceneRendererRef.current?.setSelectedInstances(instanceIds)
    sceneRendererRef.current?.setSelectedInstanceMembers(
      activeInstance,
      activeInstance === null ? [] : leafIds,
    )
    sceneRendererRef.current?.setSelectedSketches(sketchIds)
    sceneRendererRef.current?.setSelectedSketchIslands(sketchIslands)
    sceneRendererRef.current?.setSelectedSketchEdges(sketchEdges)
    sceneRendererRef.current?.setSelectedSketchInstance(
      activeInstance,
    )
    scheduleRenderRef.current()
  }, [selectedIds, activeContext])

  // Reflect the selected construction guide into the renderer highlight.
  useEffect(() => {
    sceneRendererRef.current?.setSelectedGuide(selectedGuide)
    scheduleRenderRef.current()
  }, [selectedGuide])

  // Reflect the selected annotation into the renderer highlight
  // (docs/design/dimensions-text.md).
  useEffect(() => {
    sceneRendererRef.current?.setSelectedAnnotation(selectedAnnotation)
    scheduleRenderRef.current()
  }, [selectedAnnotation])

  return (
    <div
      ref={containerRef}
      style={{ width: '100%', height: '100%' }}
    />
  )
}
