/**
 * FollowMeTool — sweep a profile region along a path (the follow-me design).
 *
 * Gesture (SketchUp's preselect-the-path idiom):
 *   1. Activate with the path already selected (sketch edges, a drawn
 *      curve, or a whole island) → the tool starts at "click the profile".
 *      A single selected edge expands to its whole connected island, the
 *      same pickup a path click gives.
 *   2. Or pick the path with the tool itself: clicking a sketch edge takes
 *      that edge's whole connected island as the path; clicking a solid
 *      face means "run around this face's boundary" (molding). While the
 *      cursor is at the path stage, the face-loop (or edge-island) under it
 *      is highlighted — the sweep target is shown *before* the click, so a
 *      solid's face, which cannot be preselected, is discoverable and picked
 *      directly rather than through whatever the ray happens to hit.
 *   3. Click the profile region → immediate commit. The profile outline is
 *      consumed exactly like an extrusion's; the path stays. Clicking a
 *      solid FACE here re-picks the path instead — but only while the path
 *      is a leftover preselection (faces are never profiles in this
 *      release, so that stale-selection recovery is unambiguous). Once a
 *      path is picked deliberately in-tool, a stray face graze is ignored,
 *      never a silent substitution of the swept face; Esc re-picks.
 *   Esc steps back one stage. Kernel refusals surface as toasts; because the
 *   path source (a solid face) changes what "no good" means, the two face-
 *   specific refusals are re-worded against the FACE the user picked
 *   ("that face is parallel to the profile" / "…thinner than the profile is
 *   deep") rather than the generic drawn-path copy.
 *
 * The kernel owns every geometric eligibility decision (perpendicularity,
 * chain validity, tight bends, self-intersection); this tool only gathers
 * the two picks, previews the target, and relays the typed error copy —
 * never a silent no-op when a pick lands on nothing.
 *
 * START AFFORDANCE. Where a profile may legally BEGIN on a path used to be
 * invisible: every wrong placement came back as a post-click
 * `[ProfileNotPerpendicular]` toast. Once a path is picked, the tool now
 * marks the legal starts on it — a circle path's four QUADRANTS (which is
 * where a profile drawn on an axis-aligned plane can actually meet the
 * kernel's radial-plane rule, and which carry extra snap gravity), an open
 * path's two ends, and — since the kernel grew corner-seam support — almost
 * every vertex of a polyline or face-loop path, since a profile CAN now
 * legally start right at a corner (a picture-frame's mitered corner) as long
 * as it stands past it; only a joint between two different drawn curves has
 * no kernel anchor at all and stays marked as truly blocked. Hovering a
 * profile region then outlines it in a verdict color and replaces the status
 * hint with what is wrong and where to move it. `followMeStart.ts` holds that
 * logic, mirrors the kernel branch by branch, and is deliberately
 * three-valued: it claims a placement is fine only for f64-exact sketch-edge
 * paths under the kernel's own tolerances, claims a refusal only when
 * decisively wrong (including the corner's own fold refusal, which — unlike
 * most sweep-wide refusals — IS decidable from the start placement alone),
 * and otherwise says nothing. The kernel remains the only authority — a
 * predicted refusal warns, it never blocks the click.
 *
 * DRAG FOR A PARTIAL SWEEP. Past the profile stage, a plain click still
 * commits the full sweep; pressing and then moving along the highlighted path
 * arms a live partial-sweep preview (a station marker + a VCB length readout)
 * that the RELEASE commits with an explicit `stop_len`. Unlike every other
 * gesture in this codebase (click-move-click — a press arms, a SECOND press
 * commits, since there is normally no reason to tell "a plain click" apart
 * from "the start of a drag"), this one genuinely needs a release: a plain
 * click and a press-drag-release must commit DIFFERENT things from the same
 * first press, which only the real release can disambiguate. So Follow Me is
 * one of the few tools that implements the optional `Tool.onPointerUp` hook
 * (see its doc in types.ts) — the press arms, `onPointerMove` previews, and
 * `onPointerUp` commits: full sweep when the release lands within
 * `MIN_PARTIAL_SWEEP_LEN` of arc length from the press (a plain click),
 * partial otherwise. Typing a length and pressing Enter commits a partial
 * sweep directly, with or without a drag first; Esc during a drag cancels
 * back to the profile stage, keeping the picked path.
 *
 * FACE FRAME GUARD. `face_boundary` and `follow_me_around_face` take only
 * (object, face): they are coordinate-correct ONLY for a plain, identity-
 * placed, top-level world object. A face on a component INSTANCE is stored in
 * definition-local space (its world pose lives on the placement), and a face
 * reached inside a group/instance/object editing context is likewise out of
 * frame — there is no `follow_me_in_component` surface (cf.
 * `push_pull_in_component`). So the face branch is gated to the same
 * plain-top-level set every other face tool uses (faceDraw's
 * `defaultFaceEligible`, injected as `faceDrawEligible`), and any in-context
 * or instanced face is refused with copy rather than swept in the wrong
 * frame. Molding a face of an actively-edited component is a KNOWN
 * LIMITATION pending a kernel/wasm surface. Sketch-EDGE paths carry their own
 * world coordinates and are not gated.
 */

import * as THREE from 'three'
import { LineSegments2 } from 'three/examples/jsm/lines/LineSegments2.js'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from '../panels/treeModel'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { defaultFaceEligible, type FaceEligible } from './faceDraw'
import { clearPreview } from './transformPreview'
import { getResolvedTheme } from '../settings/theme'
import { makeFatSegments, disposeFatSegments, PREVIEW_LINE_STYLE } from '../viewport/fatLine'
import { EDGE_COLOR_SELECTED } from '../viewport/SceneRenderer'
import { editLengthBuffer, isLengthInputKey } from './moveInput'
import { formatLength, parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'
import {
  startAffordance,
  evaluateStart,
  refusalGuidance,
  chainPath,
  type PathPolyline,
  type PathSegment,
  type PlaneDef,
  type StartVerdict,
  type Vec3,
} from './followMeStart'
import { seamWalk, nearestOnWalk, subWalkTo, type SeamWalk } from './followMeDrag'

export type PathTarget =
  | { kind: 'edges'; sketchHandle: bigint; edgeHandles: bigint[] }
  // `instance` is carried only to keep the hover-dedup key distinct per
  // placement; a followable face is always a plain world object (instance
  // `undefined`) — `face_boundary`/`follow_me_around_face` take just
  // (object, face) and are coordinate-correct for nothing else.
  | { kind: 'face'; objectHandle: bigint; faceHandle: bigint; instance: bigint | undefined }

/**
 * The outcome of resolving a path-stage pick: a usable target, a real face
 * the tool cannot correctly sweep (refused with copy, never a wrong-frame
 * sweep), or nothing under the cursor.
 */
type PathPick =
  | { kind: 'target'; path: PathTarget }
  | { kind: 'ineligible-face'; message: string }
  | { kind: 'none' }

/**
 * Where the current path came from. A path inherited from the
 * activation-time selection (`preselection`) may be a stale/hijacked
 * leftover from placing the profile, so a solid-face click at the profile
 * stage is allowed to re-target it (the molding recovery). A path the user
 * picked deliberately in the tool (`in-tool`) is never silently replaced by
 * a stray face graze.
 */
type PathSource = 'preselection' | 'in-tool'

/**
 * A partial sweep in progress: the profile region has been pressed and the
 * cursor is dragging along the picked path (E4 — drag the profile along the
 * path). `walk` is fixed for the whole gesture (computed once at arm time from
 * the profile's plane + centroid — see `followMeDrag.ts`); `armLen` is the
 * arc length at the press, used only to tell a plain click (negligible
 * movement) from a real drag at commit time; `liveLen` is the current arc
 * length under the cursor, updated every move.
 */
interface DraggingStage {
  kind: 'dragging'
  path: PathTarget
  pathSource: PathSource
  sketchHandle: bigint
  regionHandle: bigint
  walk: SeamWalk
  armLen: number
  liveLen: number
}

type Stage =
  | { kind: 'pick-path' }
  | { kind: 'pick-profile'; path: PathTarget; pathSource: PathSource }
  | DraggingStage

export type OnFollowMeCommit = (objectId: bigint) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

/**
 * Path/hover highlight colors. The picked path adopts the SELECTION highlight
 * color (`EDGE_COLOR_SELECTED`, SceneRenderer.ts — the same orange a selected
 * sketch/edge is drawn in, already proven legible on both themes without
 * forking) on FAT lines (`makeFatSegments`) at preview width — the 1px native
 * lines this replaces were nearly invisible. The pre-click hover preview is a
 * lighter TINT of that same hue, theme-forked like `axisColors.ts`/CUE_COLORS
 * below: a naive lighten (a paler, higher-lightness orange) reads fine on the
 * dark canvas but nearly vanishes against the light theme's near-white
 * background, so the light variant is a softened, LOWER-lightness step
 * instead — still visibly the "weaker" cousin of the path color, just reached
 * by desaturating rather than lightening. `sweep` is the drag preview's
 * "swept so far" overlay color — always a stark white-on-orange contrast, so
 * it needs no theme fork (it reads against the path color, not the canvas).
 */
const PATH_HOVER_COLORS = {
  dark: { path: EDGE_COLOR_SELECTED, hover: 0xffcc77, sweep: 0xffffff },
  light: { path: EDGE_COLOR_SELECTED, hover: 0xd99a4e, sweep: 0xffffff },
} as const

export function pathHoverColors(): { path: number; hover: number; sweep: number } {
  return PATH_HOVER_COLORS[getResolvedTheme() === 'light' ? 'light' : 'dark']
}

/**
 * Start-affordance colors: "may start here" (circle quadrants, open-path ends,
 * a profile that checks out) and "may not" (a real path corner, a profile that
 * will be refused).
 *
 * Theme-forked the way `axisColors.ts` forks the world axes, and for the same
 * reason: the light theme's canvas is near-white, so the bright greens that
 * read well on the dark canvas lose most of their contrast there. Colors are
 * chosen at build time (a path pick / a hover), matching how ScaleTool reads
 * `axisColorsForTheme(getResolvedTheme())` when it draws its gizmo.
 *
 * Color is never the only encoding — a legal start is an open RING and a
 * blocked one a solid X, so the two read apart without it.
 */
const CUE_COLORS = {
  dark: { ok: 0x33dd77, blocked: 0xff5544 },
  light: { ok: 0x0f8f4d, blocked: 0xd0311f },
} as const

export function cueColors(): { ok: number; blocked: number } {
  return CUE_COLORS[getResolvedTheme() === 'light' ? 'light' : 'dark']
}

/** Start markers hold a constant on-screen size: they are handles/annotations,
 *  not model geometry, so a world-sized marker would vanish on a 100 m path
 *  and swamp a 10 cm one. Same perspective inverse ScaleTool's grips use —
 *  `worldHalf = px · dist · tan(fov/2) / viewportHeight` — driven from the
 *  Viewport render loop through the shared `updateGripScale` hook. */
const START_MARKER_PX = 14
/** The verdict badge on the hovered profile is deliberately LARGER than a
 *  start marker: it lands on the profile's centre, which is exactly where the
 *  inference cursor's own snap indicator sits, so anything smaller is simply
 *  covered by it. At this size the badge reads as a ring/X around that dot. */
const VERDICT_BADGE_PX = 24
/** Floor on a marker's world half-size; guards a degenerate viewport only. */
const MIN_MARKER_WORLD_HALF = 1e-5
/** Half-size a marker renders at for the one frame before the render loop
 *  first calls `updateGripScale` (and in unit tests, which never drive one). */
const FALLBACK_MARKER_HALF_M = 0.02
/** Cap on blocked-corner markers. A hand-drawn path has a handful; a dense
 *  imported outline could have hundreds, at which point the markers stop
 *  informing and start hiding the path they annotate. */
const MAX_BLOCKED_MARKERS = 64

/**
 * The two marker shapes, built once and SHARED by every marker — the same
 * pattern ScaleTool's gizmo uses for its grips. Only the material differs per
 * marker (the verdict color), and only the material is disposed with a marker;
 * these outlive any one path pick, so `_dispose` skips geometry flagged as
 * shared rather than tearing down a buffer the next pick will reuse.
 */
let sharedRingGeometry: THREE.BufferGeometry | null = null
let sharedCrossGeometry: THREE.BufferGeometry | null = null

function markerGeometry(shape: 'ring' | 'cross'): THREE.BufferGeometry {
  if (shape === 'ring') {
    sharedRingGeometry ??= new THREE.RingGeometry(0.62, 1, 28)
    return sharedRingGeometry
  }
  sharedCrossGeometry ??= crossGeometry()
  return sharedCrossGeometry
}

/** A unit-sized solid "X" in the XY plane — two crossing bars, as four
 *  triangles. Used for the "cannot start here" marker. */
function crossGeometry(): THREE.BufferGeometry {
  const w = 0.2 // bar half-width
  const d = Math.SQRT1_2
  const verts: number[] = []
  for (const [dx, dy] of [
    [d, d],
    [-d, d],
  ]) {
    // Along (dx, dy), half-width across its perpendicular (-dy, dx).
    const ax = dx + -dy * w, ay = dy + dx * w
    const bx = dx - -dy * w, by = dy - dx * w
    verts.push(-ax, -ay, 0, ax, ay, 0, bx, by, 0)
    verts.push(-ax, -ay, 0, bx, by, 0, -bx, -by, 0)
  }
  const geo = new THREE.BufferGeometry()
  geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(verts), 3))
  return geo
}

/** Guidance when a path-stage click lands on no followable geometry. */
const PATH_MISS =
  'Click the flat face to run the profile around it — or a drawn line or curve to follow.'

/**
 * Below this much arc length of movement between the arming press and the
 * real release (`onPointerUp`), the gesture reads as a plain CLICK (full
 * sweep) rather than a drag — a real click's down and up land at essentially
 * the same point, so this must still read as "no drag" despite the tiny,
 * inevitable jitter between the two events. 1mm is comfortably above
 * camera-projection float noise and comfortably below any deliberate drag.
 */
const MIN_PARTIAL_SWEEP_LEN = 1e-3

/** The mean of a flat `[x,y,z,x,y,z,…]` point array (a region/face boundary
 *  loop) — the same "profile centroid" the kernel's own seam choice uses
 *  (`profile_centroid`, ops.rs), reused here as the closed-path seam
 *  approximation's anchor (see `followMeDrag.ts`). */
function _centroidOfFlat(flat: ArrayLike<number>): Vec3 {
  let cx = 0
  let cy = 0
  let cz = 0
  const n = flat.length / 3
  for (let i = 0; i < flat.length; i += 3) {
    cx += flat[i]
    cy += flat[i + 1]
    cz += flat[i + 2]
  }
  return [cx / n, cy / n, cz / n]
}

export class FollowMeTool implements Tool {
  readonly name = 'Follow Me'

  private stage: Stage = { kind: 'pick-path' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnFollowMeCommit
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  /** The picked-path highlight (persists from pick until commit/cancel). A
   *  fat line (`makeFatSegments`) — see `PATH_HOVER_COLORS`. */
  private pathHighlight: LineSegments2 | null = null
  /** The pre-click hover preview of the target under the cursor. */
  private hoverHighlight: LineSegments2 | null = null
  /** The verdict badge on the hovered profile region (screen-constant, so it
   *  reads even when the profile itself is a few pixels across). */
  private hoverMarker: THREE.Object3D | null = null
  /** Identity of what the hover currently shows, so a still cursor doesn't
   *  rebuild the same geometry every move. */
  private hoverKey: string | null = null
  /** Whether the current run of empty/refused clicks has already been called
   *  out, so repeated clicks on nothing (or on the same ineligible face)
   *  don't stack identical toasts (cleared the moment a real face — eligible
   *  or not — appears under the cursor, or a pick lands). */
  private missNotified = false

  /** Start-affordance markers for the picked path (quadrants/ends in the
   *  "may start here" color, real corners in the refusal color). Screen-
   *  constant — see `updateGripScale`. */
  private startMarkers: THREE.Object3D[] = []
  /** The picked path as this tool understands it geometrically, resolved once
   *  per path pick and reused for every hover verdict. Null when the path
   *  cannot be chained (branching/disconnected — the kernel refuses it too, so
   *  no cue is offered rather than a cue for one component). */
  private pathPoly: PathPolyline | null = null
  /** The path sketch's plane normal — the frame the quadrant markers are
   *  phased in, matching the inference engine's own quadrant snaps. Null for a
   *  face-loop path. */
  private pathSketchNormal: Vec3 | null = null
  /** Identity of the profile region the last hover verdict was computed for,
   *  so a still cursor doesn't re-query the kernel every move. */
  private profileHoverKey: string | null = null
  /** The current hover's start verdict — drives the status hint. */
  private profileVerdict: StartVerdict | null = null
  /** The region currently under the cursor at the profile stage (or null),
   *  kept live so typing a length + Enter can commit a partial sweep against
   *  it WITHOUT requiring a drag to have armed first (E4). */
  private hoveredRegion: { sketchHandle: bigint; regionHandle: bigint } | null = null

  /** VCB buffer — raw string being typed by the user, live at the profile
   *  stage and while dragging (see `capturesKey`). */
  private typed: string = ''
  /** The "swept so far" overlay during a drag — a brighter sub-segment of the
   *  path highlight, from the seam up to the live drag point. */
  private dragHighlight: LineSegments2 | null = null
  /** The live station marker at the drag point. */
  private stationMarker: THREE.Object3D | null = null

  /** The entered-object context id (null at top level); mirrors PushPullTool.
   *  Only the shared `defaultFaceEligible` fallback reads it — in production
   *  the Viewport injects `faceDrawEligible`, which knows the full path. */
  private _activeContext: bigint | null = null
  /** Richer eligibility injected by the Viewport (knows the full
   *  group/instance context path); null = the shared default policy. */
  private _faceEligible: FaceEligible | null = null
  /** True while ANY editing context is entered (object, group, or instance) —
   *  Follow Me runs at the top level only, so this refuses in-context face
   *  molding wholesale (see the FACE FRAME GUARD note). */
  private _contextScoped = false

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onCommit: OnFollowMeCommit,
    onToast: OnToast,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
    initialSelection: readonly NodeRef[] = [],
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.onCommit = onCommit
    this.onToast = onToast
    this.onMeasurementCb = onMeasurement

    const preselected = this._pathFromSelection(initialSelection)
    if (preselected !== null) {
      this.stage = { kind: 'pick-profile', path: preselected, pathSource: 'preselection' }
      this._highlightPath(preselected)
    }
  }

  /**
   * Re-draw the at-rest overlay after the tool becomes active.
   *
   * `ToolController.setTool(makeFollowMeTool())` evaluates the constructor
   * FIRST and only then calls the OUTGOING tool's `cancel()` — and most tools'
   * `cancel()` runs `clearPreview()`, which empties the whole shared preview
   * group. So a preselected path picked up in the constructor had its
   * highlight, and now its start markers, silently wiped whenever Follow Me
   * was entered from a tool with a preview to clear (Push/Pull, Move, Offset…)
   * — exactly the bounce-between-tools flow the affordance has to survive.
   * `ToolController` documents this hook for precisely that case; ScaleTool's
   * always-on gizmo was the only other user. Idempotent: `_highlightPath`
   * clears before it draws, and three.js disposal is safe to repeat.
   */
  activate(): void {
    if (this.stage.kind === 'pick-profile') this._highlightPath(this.stage.path)
  }

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    if (this.stage.kind === 'pick-path') {
      return 'Click the path to follow — a drawn line or curve, or a solid face to run around.'
    }
    if (this.stage.kind === 'dragging') {
      return 'Drag along the path for a partial sweep, or type a length — release to commit. Esc backs up.'
    }
    // A predicted refusal outranks the generic stage copy: the whole point of
    // the start affordance is that being wrong is legible BEFORE the click,
    // not as a toast after it. `unknown` deliberately says nothing extra —
    // this tool never claims certainty it cannot compute (see followMeStart).
    if (this.profileVerdict !== null) {
      if (this.profileVerdict.kind === 'refused') {
        return refusalGuidance(this.profileVerdict.reason)
      }
      if (this.profileVerdict.kind === 'ok') {
        // A carried (perpendicular-but-detached) open-path end is honest
        // about what the sweep will actually do: it does NOT start on the
        // path as drawn — the path's shape is carried to wherever the
        // profile stands (design §2a).
        return this.profileVerdict.carried === true
          ? 'The sweep starts at the profile and follows the path’s shape — click to sweep it, or drag along the path for a partial sweep.'
          : 'This profile starts cleanly on the path — click to sweep it, or drag along the path for a partial sweep.'
      }
    }
    // The face-click re-pick is offered only while the path is a leftover
    // preselection (the recovery); once a path is deliberately picked, the
    // hint stops promising a face click will retarget it.
    return this.stage.pathSource === 'preselection'
      ? 'Click the profile to sweep along the highlighted path — a solid-face click follows that face instead; Esc re-picks the path.'
      : 'Click the profile to sweep along the highlighted path, or drag along it for a partial sweep. Esc re-picks the path.'
  }

  /**
   * Per-key VCB capture (see the routing note at MoveTool.capturesKey).
   * `isLengthInputKey` accepts unit-suffix letters (m/c/k/f/t/i/n) so an
   * explicit unit can be typed in any display format — but several of those
   * ARE ALSO bare tool-switch shortcuts (m→Move, c→Circle, f→Offset). Once
   * genuinely mid-entry (`dragging`, where digits are the only way to name a
   * length at all, or `pick-profile` once a digit has already started the
   * buffer) every length-input key must capture, or a shortcut letter typed
   * as a unit suffix (e.g. the "m" in "5m") would switch tools mid-entry and
   * silently eat the rest of the number. But an EMPTY buffer at `pick-
   * profile` is not yet "typing a length" — the user has done nothing but
   * hover — so only tokens that can legitimately START a fresh entry capture
   * there (never a bare unit letter out of context): letting `m`/`c`/`f`
   * through uncaptured in that case is what keeps ordinary tool-switching
   * working while a profile is merely being looked at, matching how
   * MoveTool/SectionPlaneTool only capture the whole keyboard once their own
   * drag has actually started. Escape is deliberately NOT listed here: it is
   * always let through so cancel/back-up keeps working regardless of buffer
   * state.
   */
  capturesKey(key: string): boolean {
    if (this.stage.kind === 'pick-path') return false
    if (this.stage.kind === 'dragging' || this.typed !== '') {
      return key === 'Enter' || isLengthInputKey(key)
    }
    // pick-profile, nothing typed yet: only what can START a fresh entry —
    // the non-letter subset of isLengthInputKey (digits, sign, decimal
    // point, feet/inch/fraction marks, space, Backspace) — never a bare
    // unit-suffix letter, which is indistinguishable from its shortcut here.
    return key === 'Backspace' || /^[0-9.\-'"/ ]$/.test(key)
  }

  /** Set the active editing context (entered object), or null for top level.
   *  Wired by the Viewport exactly like PushPullTool. */
  setActiveContext(objectId: bigint | null): void {
    this._activeContext = objectId
  }

  /** Inject the Viewport's context-path-aware face policy (or null for the
   *  shared default). Wired like PushPullTool so Follow Me is on the same
   *  face-eligibility system as every other face tool. */
  setFaceEligibility(pred: FaceEligible | null): void {
    this._faceEligible = pred
  }

  /** True while any editing context is entered — the Viewport sets it (the
   *  object/instance id channels don't cover a GROUP context). Follow Me is a
   *  top-level act, so this makes in-context face molding refuse. */
  setContextScoped(scoped: boolean): void {
    this._contextScoped = scoped
  }

  /** Whether the face on `object` (hit through `instance`) may become a path.
   *  Only a plain, top-level, non-instanced world object is coordinate-correct
   *  for `follow_me_around_face` (see the FACE FRAME GUARD note). */
  private _faceFollowable(object: bigint, instance: bigint | undefined): boolean {
    // No in-component variant: an instanced face is definition-local and would
    // sweep in the wrong frame.
    if (instance !== undefined) return false
    // Follow Me runs at the top level; refuse any in-context face for now
    // (known limitation) rather than sweep a face the profile can't meet.
    if (this._contextScoped || this._activeContext !== null) return false
    // The shared plain-top-level policy (grouped faces refused, etc.).
    return this._faceEligible !== null
      ? this._faceEligible(object, instance)
      : defaultFaceEligible(this.wasmScene, this._activeContext, object, instance)
  }

  /** Why an ineligible face refused, directing the user to a plain object's
   *  face (Follow Me can't mold instanced/grouped/in-context geometry). */
  private _ineligibleFaceHint(instance: bigint | undefined): string {
    if (this._contextScoped || this._activeContext !== null) {
      return 'Follow Me runs at the top level — press Esc to step out of what you are editing first.'
    }
    if (instance !== undefined) {
      return 'That face belongs to a component — Follow Me needs a plain object. Explode the instance, then follow the face.'
    }
    return 'That face is inside a group — Follow Me needs a plain object. Ungroup it, then follow the face.'
  }

  onPointerMove(_snap: Snap | null, ray: Ray): void {
    // Mid-drag: the cursor's projection onto the picked path IS the live
    // partial-sweep length (E4) — update the station marker, the brightened
    // "swept so far" overlay, and the VCB readout.
    if (this.stage.kind === 'dragging') {
      const { arcLen } = nearestOnWalk(this.stage.walk, ray.origin, ray.direction)
      this.stage = { ...this.stage, liveLen: arcLen }
      this._drawDragPreview(arcLen)
      this._reportDragMeasurement(arcLen)
      return
    }
    // At the profile stage the picked path stays highlighted and the hover
    // instead answers the question the UI used to leave until after the click:
    // would THIS profile be accepted as a start?
    if (this.stage.kind === 'pick-profile') {
      this._hoverProfile(ray)
      return
    }
    // Path stage: show the sweep target — the face-loop or edge-chain under
    // the cursor — before the click commits to it.
    let pick: PathPick
    try {
      pick = this._pickPath(ray)
    } catch {
      pick = { kind: 'none' }
    }
    if (pick.kind === 'target') {
      // A usable target is under the cursor — any earlier "nothing there" note
      // is stale, so the next genuine miss speaks up again.
      this.missNotified = false
      const { key, points } = this._targetHighlight(pick.path)
      this._showHover(key, points)
      return
    }
    // An ineligible face still counts as "a real face is here", so the next
    // click should speak (arming the refusal toast); but it is never previewed
    // as a sweep target.
    if (pick.kind === 'ineligible-face') this.missNotified = false
    this._clearHover()
  }

  onPointerDown(_snap: Snap | null, ray: Ray): void {
    if (this.stage.kind === 'pick-path') {
      const pick = this._pickPath(ray)
      if (pick.kind === 'target') {
        this.missNotified = false
        this.stage = { kind: 'pick-profile', path: pick.path, pathSource: 'in-tool' }
        this._highlightPath(pick.path)
        return
      }
      // Never a silent no-op: a solid face cannot be preselected, so a
      // path-stage click that hits nothing — or lands on a face the tool
      // cannot correctly sweep (instanced / in-context) — says what to aim at.
      this._notifyMiss(pick.kind === 'ineligible-face' ? pick.message : PATH_MISS)
      return
    }

    if (this.stage.kind === 'dragging') {
      // A real mouse always alternates down/up, so a SECOND down while
      // already dragging (no up in between) should not happen — but stay
      // inert rather than double-arm or double-commit if it somehow does;
      // `onPointerUp` is the only thing that ends this gesture.
      return
    }

    // pick-profile: a region click commits. A solid-face click instead
    // RE-PICKS the path, but ONLY when the current path is a leftover
    // preselection — the recovery that matters when a stale selection from
    // placing the profile silently became the path: the user "clicks the
    // box's top face" expecting to pick it, and before this fallback that
    // click was a dead no-op. A path the user picked deliberately in-tool
    // is NEVER retargeted by a stray face graze: doing so would silently
    // swap the swept face out from under the very next profile click. A
    // near-miss of a small profile's interior lands here constantly, so an
    // in-tool region miss stays quiet (the picked path is highlighted; the
    // user simply clicks again) — surfacing a toast on every near-miss would
    // be noise, not guidance.
    const regionPick = this.wasmScene.pick_sketch_region(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (regionPick === undefined) {
      if (this.stage.pathSource !== 'preselection') return
      const facePick = this.wasmScene.pick_face(
        ray.origin[0], ray.origin[1], ray.origin[2],
        ray.direction[0], ray.direction[1], ray.direction[2],
      )
      if (facePick !== undefined) {
        let object: bigint
        let face: bigint
        let instance: bigint | undefined
        try {
          object = facePick.object()
          face = facePick.face()
          instance = facePick.instance()
        } finally {
          facePick.free()
        }
        // Same frame guard as the path stage: a stale-preselection recovery
        // must not adopt an instanced/in-context face the sweep can't place.
        // Route through _notifyMiss so repeated clicks on the same ineligible
        // face don't stack toasts, matching the path stage's anti-spam dedup.
        if (!this._faceFollowable(object, instance)) {
          this._notifyMiss(this._ineligibleFaceHint(instance))
          return
        }
        // The recovered face is now a deliberate pick — mark it in-tool so a
        // further stray graze can't re-target it in turn.
        const path: PathTarget = { kind: 'face', objectHandle: object, faceHandle: face, instance }
        this.stage = { kind: 'pick-profile', path, pathSource: 'in-tool' }
        this._highlightPath(path)
      }
      return
    }
    let sketchHandle: bigint
    let regionHandle: bigint
    try {
      sketchHandle = regionPick.sketch()
      regionHandle = regionPick.region()
    } finally {
      regionPick.free()
    }
    this._armOrCommit(this.stage.path, this.stage.pathSource, sketchHandle, regionHandle, ray)
  }

  /**
   * The real release (E4): commits the armed drag. A negligible move from
   * the arming press (within `MIN_PARTIAL_SWEEP_LEN` of arc length) commits
   * the FULL sweep — a plain click, exactly as it always has; a real
   * press-drag-release commits the dragged partial length. Feature-detected
   * by the Viewport (`Tool.onPointerUp`) — see that interface doc for why
   * this is the one gesture click-move-click can't express: "click commits
   * one thing, press-drag-release commits another" needs the real release
   * to tell a plain click from a drag that never moved anywhere near the
   * seam, which `onPointerDown` alone cannot.
   */
  onPointerUp(_snap: Snap | null, ray: Ray): void {
    if (this.stage.kind !== 'dragging') return
    const { path, sketchHandle, regionHandle, walk, armLen } = this.stage
    const { arcLen } = nearestOnWalk(walk, ray.origin, ray.direction)
    const stopLen = Math.abs(arcLen - armLen) < MIN_PARTIAL_SWEEP_LEN ? undefined : arcLen
    this._commit(path, sketchHandle, regionHandle, stopLen)
  }

  /**
   * Arm the drag-to-partial-sweep gesture (E4): build the seam walk for this
   * profile region and enter `dragging`. When a walk can't be built — the
   * path can't be chained (branching/disconnected), or the profile's plane
   * can't be read (a stale handle mid-click) — there is nothing to preview or
   * measure, so this falls back to the pre-drag behavior: a click commits the
   * full sweep outright, exactly as it always has. The kernel is unaffected
   * either way; this only decides whether a live preview is possible.
   */
  private _armOrCommit(
    path: PathTarget,
    pathSource: PathSource,
    sketchHandle: bigint,
    regionHandle: bigint,
    ray: Ray,
  ): void {
    const walk = this._buildWalk(sketchHandle, regionHandle)
    if (walk === null) {
      this._commit(path, sketchHandle, regionHandle)
      return
    }
    const { arcLen } = nearestOnWalk(walk, ray.origin, ray.direction)
    this.typed = ''
    this.stage = {
      kind: 'dragging',
      path,
      pathSource,
      sketchHandle,
      regionHandle,
      walk,
      armLen: arcLen,
      liveLen: arcLen,
    }
    // The profile-region verdict outline/badge is retired the moment the
    // gesture starts — the picked-path highlight (undisturbed) and the new
    // drag overlay are the only cues from here.
    this._clearHover()
    this._drawDragPreview(arcLen)
    this._reportDragMeasurement(arcLen)
  }

  /** The seam-relative arc-length walk for a profile region — see
   *  `followMeDrag.ts` for what this approximates and why. Null when the
   *  path can't be chained or the plane/boundary can't be read. */
  private _buildWalk(sketchHandle: bigint, regionHandle: bigint): SeamWalk | null {
    if (this.pathPoly === null) return null
    try {
      const pl = this.wasmScene.sketch_plane(sketchHandle)
      if (pl === undefined || pl.length < 6) return null
      const plane: PlaneDef = { point: [pl[0], pl[1], pl[2]], normal: [pl[3], pl[4], pl[5]] }
      const loop = this.wasmScene.region_boundary(sketchHandle, regionHandle)
      if (loop.length < 9) return null
      const centroid = _centroidOfFlat(loop)
      return seamWalk(this.pathPoly, plane, centroid)
    } catch {
      return null
    }
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      if (this.stage.kind === 'dragging') {
        // Cancels back to the PROFILE stage (not the path stage) — the path
        // is still picked and highlighted; only the in-progress drag is
        // abandoned.
        const { path, pathSource } = this.stage
        this.stage = { kind: 'pick-profile', path, pathSource }
        this.typed = ''
        this._clearDragPreview()
        this.onMeasurementCb('')
        return
      }
      if (this.stage.kind === 'pick-profile') {
        // Step back one stage: drop the picked path, keep the tool.
        this.stage = { kind: 'pick-path' }
        this.missNotified = false
        this.typed = ''
        this.hoveredRegion = null
        this._clearPath()
        this._clearHover()
        this.onMeasurementCb('')
      } else {
        this.cancel()
      }
      return
    }

    if (this.stage.kind === 'pick-path') return

    // ── Numeric VCB — live at the profile stage AND while dragging ──
    if (ev.key === 'Enter') {
      const meters = parseLengthToMeters(this.typed)
      if (meters === null) return
      if (this.stage.kind === 'dragging') {
        this._commit(this.stage.path, this.stage.sketchHandle, this.stage.regionHandle, meters)
        return
      }
      // pick-profile: a typed length commits a partial sweep directly
      // against whatever profile is under the cursor right now — a drag is
      // not required first (E4: "typing a length + Enter at the profile
      // stage commits a partial sweep of that length").
      if (this.hoveredRegion !== null) {
        this._commit(
          this.stage.path,
          this.hoveredRegion.sketchHandle,
          this.hoveredRegion.regionHandle,
          meters,
        )
      }
      return
    }

    if (isLengthInputKey(ev.key)) {
      this.typed = editLengthBuffer(this.typed, ev.key, getLengthUnit())
      this.onMeasurementCb(typedReadout(this.typed))
    }
  }

  cancel(): void {
    this.stage = { kind: 'pick-path' }
    this.missNotified = false
    this.typed = ''
    this.hoveredRegion = null
    // Route every live overlay through its own proper disposal FIRST — a fat
    // line (`LineSegments2`) needs `disposeFatSegments` to drop its material
    // from the resolution registry (`fatLine.ts`), which the generic
    // `clearPreview` sweep below does not know to do (it disposes the
    // geometry/material fine via the `instanceof THREE.Mesh` branch, since
    // `LineSegments2` extends `Mesh`, but never calls `disposeFatSegments`,
    // leaking the material reference in that registry forever). `clearPreview`
    // then only needs to catch anything these don't already own.
    this._clearPath()
    this._clearHover()
    this._clearDragPreview()
    this.onMeasurementCb('')
    clearPreview(this.preview)
  }

  /**
   * Resolve a preselection into a path. Sketch-scoped refs (edges, curves,
   * islands) contribute their edges; they must all live in ONE sketch.
   * Anything else (objects, groups…) yields no preselected path — the tool
   * starts at pick-path instead. Stale handles resolve to nothing.
   *
   * A selection of exactly ONE edge expands to its whole connected island —
   * the same one-click pickup the in-tool path click gives (the guide's
   * "clicking one line picks up the whole connected shape"); a Select click
   * on a line yields a single sketch-edge ref, and without this expansion
   * the preselect flow swept just that segment. An explicit multi-edge
   * selection is honored as picked (a deliberate partial path).
   */
  private _pathFromSelection(selection: readonly NodeRef[]): PathTarget | null {
    let sketchHandle: bigint | null = null
    const edges = new Set<bigint>()
    let sketchRefs = 0
    let soleEdgeRef: NodeRef | null = null
    for (const ref of selection) {
      if (ref.sketch === undefined) continue
      if (ref.kind !== 'sketch-edge' && ref.kind !== 'sketch-curve' && ref.kind !== 'sketch-island') {
        continue
      }
      sketchRefs += 1
      soleEdgeRef = sketchRefs === 1 && ref.kind === 'sketch-edge' ? ref : null
      if (sketchHandle === null) sketchHandle = ref.sketch
      else if (sketchHandle !== ref.sketch) return null // spans two sketches
      try {
        if (ref.kind === 'sketch-edge') {
          edges.add(ref.id)
        } else if (ref.kind === 'sketch-curve') {
          for (const e of this.wasmScene.sketch_curve_edges(ref.sketch, ref.id)) edges.add(e)
        } else {
          for (const e of this.wasmScene.sketch_island_edges(ref.sketch, ref.id)) edges.add(e)
        }
      } catch {
        return null // stale handle — no usable preselection
      }
    }
    if (sketchHandle === null || edges.size === 0) return null
    if (soleEdgeRef !== null) {
      try {
        const island = this.wasmScene.sketch_edge_island(sketchHandle, soleEdgeRef.id)
        if (island !== undefined) {
          for (const e of this.wasmScene.sketch_island_edges(sketchHandle, island)) edges.add(e)
        }
      } catch {
        // stale mid-query — fall back to the bare edge
      }
    }
    return { kind: 'edges', sketchHandle, edgeHandles: [...edges] }
  }

  /**
   * Resolve a path-stage pick. A sketch edge expands to its whole connected
   * island (the shape the user drew — the kernel refuses a branching island
   * with its own guidance) and is always a usable target — sketch edges carry
   * world coordinates and are NOT frame-gated (only the face branch is).
   *
   * A solid face means "run around this face's outer boundary", but ONLY when
   * it is a face the sweep can place correctly (plain, top-level, non-
   * instanced — the FACE FRAME GUARD). An ineligible face is reported as such
   * so the click is refused with copy instead of silently sweeping the wrong
   * frame, and so the hover preview never renders on it.
   */
  private _pickPath(ray: Ray): PathPick {
    const edgePick = this.wasmScene.pick_sketch_edge(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (edgePick !== undefined) {
      let sketchHandle: bigint
      let edgeHandle: bigint
      try {
        sketchHandle = edgePick.sketch()
        edgeHandle = edgePick.edge()
      } finally {
        edgePick.free()
      }
      try {
        const island = this.wasmScene.sketch_edge_island(sketchHandle, edgeHandle)
        const edgeHandles = island !== undefined
          ? [...this.wasmScene.sketch_island_edges(sketchHandle, island)]
          : [edgeHandle]
        return { kind: 'target', path: { kind: 'edges', sketchHandle, edgeHandles } }
      } catch {
        return { kind: 'target', path: { kind: 'edges', sketchHandle, edgeHandles: [edgeHandle] } }
      }
    }

    const facePick = this.wasmScene.pick_face(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (facePick !== undefined) {
      let object: bigint
      let face: bigint
      let instance: bigint | undefined
      try {
        object = facePick.object()
        face = facePick.face()
        instance = facePick.instance()
      } finally {
        facePick.free()
      }
      if (!this._faceFollowable(object, instance)) {
        return { kind: 'ineligible-face', message: this._ineligibleFaceHint(instance) }
      }
      return { kind: 'target', path: { kind: 'face', objectHandle: object, faceHandle: face, instance } }
    }
    return { kind: 'none' }
  }

  /**
   * Commit a sweep. `stopLen` is the optional partial-sweep arc length (E4)
   * — `undefined` sweeps the full path exactly as before.
   *
   * `fallback` is computed BEFORE the kernel call from whatever stage this
   * commit is running from (`pick-profile` or `dragging`), so a refusal drops
   * back to the PROFILE stage (path still picked/highlighted) rather than
   * leaving a stale, already-released `dragging` stage behind.
   */
  private _commit(
    path: PathTarget,
    sketchHandle: bigint,
    regionHandle: bigint,
    stopLen?: number,
  ): void {
    const fallback: Stage =
      this.stage.kind === 'dragging' || this.stage.kind === 'pick-profile'
        ? { kind: 'pick-profile', path: this.stage.path, pathSource: this.stage.pathSource }
        : { kind: 'pick-path' }
    try {
      const objectId = path.kind === 'edges'
        ? this.wasmScene.follow_me_along_edges(
            sketchHandle,
            regionHandle,
            path.sketchHandle,
            new BigUint64Array(path.edgeHandles),
            stopLen,
          )
        : this.wasmScene.follow_me_around_face(
            sketchHandle,
            regionHandle,
            path.objectHandle,
            path.faceHandle,
            stopLen,
          )
      this.stage = { kind: 'pick-path' }
      this.missNotified = false
      this.typed = ''
      this.hoveredRegion = null
      this._clearPath()
      this._clearHover()
      this._clearDragPreview()
      this.onMeasurementCb('')
      this.onCommit(objectId)
    } catch (err) {
      // Typed refusal: keep the picked path so the user can adjust the
      // profile and click (or drag/type) again.
      this.stage = fallback
      this.typed = ''
      this._clearDragPreview()
      this.onMeasurementCb('')
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      this.onToast(this._refusalMessage(path, code, rawMsg), code ?? undefined)
    }
  }

  /**
   * Plain-language copy for a refused sweep. Two refusals mean something
   * different when the path is a *solid face* the user clicked directly: the
   * generic drawn-path copy talks about placing the profile on a
   * perpendicular surface, but here the profile is already placed and the
   * FACE is the wrong one — so name the face. Everything else defers to the
   * shared kernel-error table (kernelErrors.ts), the one surfacing path.
   */
  private _refusalMessage(path: PathTarget, code: string | null, rawMsg: string): string {
    if (path.kind === 'face') {
      if (code === 'ProfileNotPerpendicular') {
        return 'That face is parallel to the profile — pick the flat face the profile stands across, not one it runs along.'
      }
      if (code === 'PathTooTight') {
        return 'That face is thinner than the profile is deep — pick a wider face, or use a shallower profile.'
      }
    }
    return kernelErrorMessage(code ?? 'Unknown', rawMsg)
  }

  /** Toast a path-stage miss once per run of empty clicks (anti-spam). */
  private _notifyMiss(message: string): void {
    if (this.missNotified) return
    this.missNotified = true
    this.onToast(message)
  }

  /** Draw the picked path as the persistent highlight (viewport ephemera),
   *  and with it the start affordance: the markers saying where on this path
   *  a profile may (and may not) begin. */
  private _highlightPath(path: PathTarget): void {
    this._clearPath()
    // Committing to a path retires the pre-click hover preview.
    this._clearHover()
    this._resolvePathGeometry(path)
    this._buildStartCues()
    const { points } = this._targetHighlight(path)
    if (points.length === 0) return
    this.pathHighlight = this._buildLines(points, pathHoverColors().path, 996)
    this.preview.add(this.pathHighlight)
  }

  /** Show the pre-click hover preview of the target under the cursor. */
  private _showHover(key: string, points: number[]): void {
    if (key === this.hoverKey) return // same target — nothing to rebuild
    this._clearHover()
    if (points.length === 0) return
    this.hoverHighlight = this._buildLines(points, pathHoverColors().hover, 995)
    this.preview.add(this.hoverHighlight)
    this.hoverKey = key
  }

  // ------------------------------------------------------ start affordance

  /**
   * Answer, while the cursor is still moving, whether the profile under it
   * would be accepted as a start for the picked path — and outline it in the
   * verdict's color. The kernel stays the sole authority: a predicted refusal
   * never BLOCKS the click, it only warns, and an `unknown` verdict warns
   * about nothing (see followMeStart's three-valued contract).
   */
  private _hoverProfile(ray: Ray): void {
    let sketchHandle: bigint
    let regionHandle: bigint
    try {
      const pick = this.wasmScene.pick_sketch_region(
        ray.origin[0], ray.origin[1], ray.origin[2],
        ray.direction[0], ray.direction[1], ray.direction[2],
      )
      if (pick === undefined) {
        this.profileHoverKey = null
        this.profileVerdict = null
        this.hoveredRegion = null
        this._clearHover()
        return
      }
      try {
        sketchHandle = pick.sketch()
        regionHandle = pick.region()
      } finally {
        pick.free()
      }
    } catch {
      this.profileHoverKey = null
      this.profileVerdict = null
      this.hoveredRegion = null
      this._clearHover()
      return
    }
    // Kept live even when the region is unchanged (below), so a typed
    // Enter can commit against it at any moment (E4).
    this.hoveredRegion = { sketchHandle, regionHandle }
    const key = `region:${sketchHandle}:${regionHandle}`
    if (key === this.profileHoverKey) return // same region — verdict still current
    this.profileHoverKey = key

    // The profile's own boundary — needed for BOTH the outline drawn below
    // AND the corner-seam fold test (design §2b), which needs the actual
    // profile geometry to judge, not just its plane (see followMeStart.ts).
    const ring: Vec3[] = []
    try {
      const loop = this.wasmScene.region_boundary(sketchHandle, regionHandle)
      for (let i = 0; i < loop.length; i += 3) ring.push([loop[i], loop[i + 1], loop[i + 2]])
    } catch {
      // Stale region mid-query: no outline/ring; the verdict below degrades
      // gracefully (a corner placement reads 'unknown' without ring data).
    }
    this.profileVerdict = this._startVerdict(sketchHandle, ring)

    const points: number[] = []
    for (let i = 0; i < ring.length; i++) {
      const j = (i + 1) % ring.length
      points.push(ring[i][0], ring[i][1], ring[i][2], ring[j][0], ring[j][1], ring[j][2])
    }
    this._clearHover()
    if (points.length === 0) return
    const verdictColors = cueColors()
    const hoverTint = pathHoverColors().hover
    const color =
      this.profileVerdict.kind === 'refused'
        ? verdictColors.blocked
        : this.profileVerdict.kind === 'ok'
          ? verdictColors.ok
          : hoverTint
    this.hoverHighlight = this._buildLines(points, color, 995)
    this.preview.add(this.hoverHighlight)
    this.hoverKey = key
    // A verdict badge at the region's centre. The outline alone is a poor
    // signal — a profile is often only a few pixels across, exactly when the
    // answer matters most — and the badge is screen-constant, so it reads at
    // any profile size. `unknown` gets no badge: nothing is being claimed.
    //
    // The anchor is the plain MEAN of the boundary vertices, not an
    // area-weighted centroid, and deliberately so: it is the same point the
    // kernel calls `profile_centroid` (ops.rs, "the mean of its outer ring
    // vertices") and anchors the seam nearest to. The badge therefore sits
    // where the sweep would actually start from, which is the thing being
    // judged. For a strongly concave profile that point can fall outside the
    // outline — the same caveat the kernel's own seam choice carries.
    if (this.profileVerdict.kind !== 'unknown' && ring.length > 0) {
      const centroid = _centroidOfFlat(ring.flat())
      this.hoverMarker = this._makeMarker(
        centroid,
        this.profileVerdict.kind === 'ok' ? verdictColors.ok : verdictColors.blocked,
        this.profileVerdict.kind === 'ok' ? 'ring' : 'cross',
        VERDICT_BADGE_PX,
      )
      this.preview.add(this.hoverMarker)
    }
  }

  /** The start verdict for a profile region living in `profileSketch`. The
   *  kernel builds the profile on its SKETCH's plane verbatim
   *  (`Sketch::profile` → `Profile::new(self.plane, …)`), so the sketch plane
   *  IS the plane the sweep tests — no re-derivation from the polygon.
   *  `ring` is the profile's own boundary — consulted only for a closed-path
   *  CORNER seam's fold test (design §2b); every other branch ignores it. */
  private _startVerdict(profileSketch: bigint, ring: readonly Vec3[]): StartVerdict {
    if (this.pathPoly === null) return { kind: 'unknown' }
    try {
      const pl = this.wasmScene.sketch_plane(profileSketch)
      if (pl === undefined || pl.length < 6) return { kind: 'unknown' }
      const plane: PlaneDef = {
        point: [pl[0], pl[1], pl[2]],
        normal: [pl[3], pl[4], pl[5]],
      }
      return evaluateStart(this.pathPoly, plane, ring)
    } catch {
      return { kind: 'unknown' }
    }
  }

  /**
   * Resolve the picked path into the geometry the start affordance reasons
   * over: segments with their analytic curve attribution, whether the chain
   * closes, and whether the coordinates are f64-exact.
   *
   * Sketch edges come across as f64 (`sketch_edge_endpoints`), reproducing the
   * kernel's own arithmetic; a face loop comes across as f32
   * (`face_boundary`), far coarser than the kernel's 1e-9 tolerances, which is
   * why it is flagged inexact and can only ever earn a refusal or an
   * "unknown", never an affirmative "this will start cleanly".
   */
  private _resolvePathGeometry(path: PathTarget): void {
    this.pathPoly = null
    this.pathSketchNormal = null
    try {
      if (path.kind === 'edges') {
        const geomCache = new Map<string, PathSegment['curve']>()
        const segments: PathSegment[] = []
        for (const edge of path.edgeHandles) {
          const ends = this.wasmScene.sketch_edge_endpoints(path.sketchHandle, edge)
          if (ends === undefined || ends.length < 6) return
          // A chain with no analytic definition is NOT a curve as far as the
          // sweep is concerned — every `same_curve`/tangent branch in the
          // kernel is gated on the `CurveGeom` being present — so it stays
          // null and its joints are treated as ordinary corners.
          let curve: PathSegment['curve'] = null
          const cid = this.wasmScene.sketch_edge_curve(path.sketchHandle, edge)
          if (cid !== undefined) {
            const idKey = cid.toString()
            if (!geomCache.has(idKey)) {
              const g = this.wasmScene.sketch_curve_geom(path.sketchHandle, cid)
              geomCache.set(
                idKey,
                g !== undefined && g.length >= 4
                  ? { center: [g[0], g[1], g[2]], radius: g[3] }
                  : null,
              )
            }
            curve = geomCache.get(idKey) ?? null
          }
          segments.push({
            a: [ends[0], ends[1], ends[2]],
            b: [ends[3], ends[4], ends[5]],
            curve,
          })
        }
        // Walk the edge graph into ONE oriented chain, exactly as the
        // kernel's `chain_sketch_edges` does — sketch edges arrive in
        // whatever direction they were drawn, and the start rule is only
        // sound against a consistently oriented path (see `chainPath`).
        const chained = chainPath(segments)
        if (chained === null) return // branching/disconnected: the kernel refuses it too
        this.pathPoly = { ...chained, exact: true }
        const pl = this.wasmScene.sketch_plane(path.sketchHandle)
        if (pl !== undefined && pl.length >= 6) {
          this.pathSketchNormal = [pl[3], pl[4], pl[5]]
        }
        return
      }
      const loop = this.wasmScene.face_boundary(path.objectHandle, path.faceHandle)
      if (loop.length < 9) return
      const segments: PathSegment[] = []
      for (let i = 0; i < loop.length; i += 3) {
        const j = (i + 3) % loop.length
        segments.push({
          a: [loop[i], loop[i + 1], loop[i + 2]],
          b: [loop[j], loop[j + 1], loop[j + 2]],
          curve: null,
        })
      }
      // A face loop already arrives in boundary order, but run it through the
      // same walk so the invariant `pathPoly` carries is established in one
      // place rather than assumed here.
      const chained = chainPath(segments)
      if (chained === null || !chained.closed) return
      this.pathPoly = { ...chained, exact: false }
    } catch {
      this.pathPoly = null
      this.pathSketchNormal = null
    }
  }

  /** Draw the start markers for the picked path: where a profile MAY start
   *  (a circle's four quadrants, an open path's two ends) and the vertices it
   *  may NOT (real corners — the refusal the kernel makes on purpose). */
  private _buildStartCues(): void {
    this._clearStartCues()
    if (this.pathPoly === null) return
    let legal: Vec3[]
    let blocked: Vec3[]
    try {
      ;({ legal, blocked } = startAffordance(this.pathPoly, this.pathSketchNormal))
    } catch {
      return // never let a cue failure take the path highlight down with it
    }
    const { ok, blocked: blockedColor } = cueColors()
    const add = (p: Vec3, color: number, shape: 'ring' | 'cross'): void => {
      const marker = this._makeMarker(p, color, shape)
      this.preview.add(marker)
      this.startMarkers.push(marker)
    }
    for (const p of legal) add(p, ok, 'ring')
    // A dense polyline (an imported outline) could carry hundreds of corners;
    // past a point the markers stop informing and start obscuring the path.
    for (const p of blocked.slice(0, MAX_BLOCKED_MARKERS)) {
      add(p, blockedColor, 'cross')
    }
  }

  private _makeMarker(
    p: Vec3,
    color: number,
    shape: 'ring' | 'cross',
    screenPx: number = START_MARKER_PX,
  ): THREE.Object3D {
    // Both markers are flat shapes in their own XY plane, turned to face the
    // camera every frame (see `updateGripScale`). Billboarding is what makes
    // them legible: a marker sits exactly ON the path highlight it annotates,
    // and a solid blob there is simply swallowed by the highlight's own
    // stroke, whereas a screen-space ring or X reads around it.
    const material = new THREE.MeshBasicMaterial({
      color,
      depthTest: false,
      side: THREE.DoubleSide,
    })
    // 'ring' is an open ring — "aim here", with the point it marks visible
    // through the middle rather than covered. 'cross' is a solid X — "not
    // here", told apart from the ring by shape, not by color alone.
    const obj = new THREE.Mesh(markerGeometry(shape), material)
    obj.userData.sharedGeometry = true
    obj.position.set(p[0], p[1], p[2])
    obj.scale.setScalar(FALLBACK_MARKER_HALF_M)
    // Above the path/hover highlights (996/995) so a marker on the rim is
    // never buried under the very line it annotates, and above CueLayer's
    // inference guide line (998), which is live at the same time and would
    // otherwise be an undefined tie.
    obj.renderOrder = 999
    obj.userData.screenPx = screenPx
    return obj
  }

  /**
   * Hold every start marker at a constant on-screen size. Named to match the
   * hook the Viewport render loop already feature-detects
   * (`'updateGripScale' in tool`), and using the same perspective inverse
   * ScaleTool documents: `worldHalf = px · dist · tan(fov/2) / viewportHeight`,
   * which is stable under both fov change and viewport resize (unlike the
   * `K · dist` shorthand, which bakes those in).
   */
  updateGripScale(camera: THREE.Camera, viewportHeight: number): void {
    if (viewportHeight <= 0) return
    if (!(camera instanceof THREE.PerspectiveCamera)) return
    const tanHalfFov = Math.tan((camera.fov * Math.PI) / 360)
    const all = [...this.startMarkers]
    if (this.hoverMarker !== null) all.push(this.hoverMarker)
    if (this.stationMarker !== null) all.push(this.stationMarker)
    for (const marker of all) {
      marker.quaternion.copy(camera.quaternion) // face the viewer
      const dist = camera.position.distanceTo(marker.position)
      const px = (marker.userData.screenPx as number | undefined) ?? START_MARKER_PX
      marker.scale.setScalar(
        Math.max((px * dist * tanHalfFov) / viewportHeight, MIN_MARKER_WORLD_HALF),
      )
    }
  }

  private _clearStartCues(): void {
    for (const marker of this.startMarkers) this._dispose(marker)
    this.startMarkers = []
  }

  private _clearPath(): void {
    this._dispose(this.pathHighlight)
    this.pathHighlight = null
    this._clearStartCues()
    this.pathPoly = null
    this.pathSketchNormal = null
    this.profileHoverKey = null
    this.profileVerdict = null
  }

  private _clearHover(): void {
    this._dispose(this.hoverHighlight)
    this.hoverHighlight = null
    this._dispose(this.hoverMarker)
    this.hoverMarker = null
    this.hoverKey = null
  }

  /** Drop the drag-in-progress overlay (the brightened "swept so far"
   *  sub-path and the live station marker) — see `_drawDragPreview`. */
  private _clearDragPreview(): void {
    this._dispose(this.dragHighlight)
    this.dragHighlight = null
    this._dispose(this.stationMarker)
    this.stationMarker = null
  }

  /**
   * Draw (replacing any previous frame's) the drag-in-progress preview: the
   * path from the seam up to `arcLen`, brightened over the base path
   * highlight, and a live station marker at the drag point — E4's "reuse the
   * path highlight, brightening the swept portion up to the drag point".
   */
  private _drawDragPreview(arcLen: number): void {
    if (this.stage.kind !== 'dragging') return
    this._clearDragPreview()
    const sub = subWalkTo(this.stage.walk, arcLen)
    const points: number[] = []
    for (let i = 0; i < sub.length - 1; i++) {
      const a = sub[i]
      const b = sub[i + 1]
      points.push(a[0], a[1], a[2], b[0], b[1], b[2])
    }
    const sweepColor = pathHoverColors().sweep
    if (points.length > 0) {
      // Above the base path highlight (996), below the badge/CueLayer tier
      // (998/999) — an overlay on top of the path it brightens, not a
      // competing annotation.
      this.dragHighlight = makeFatSegments(new Float32Array(points), {
        color: sweepColor,
        widthPx: PREVIEW_LINE_STYLE.widthPx + 1,
        depthTest: false,
        renderOrder: 997,
      })
      this.preview.add(this.dragHighlight)
    }
    const tip = sub[sub.length - 1]
    this.stationMarker = this._makeMarker(tip, sweepColor, 'ring', VERDICT_BADGE_PX)
    this.preview.add(this.stationMarker)
  }

  /** Report the live drag length to the VCB — the typed buffer's readout
   *  when the user is typing, otherwise the formatted live arc length. */
  private _reportDragMeasurement(arcLen: number): void {
    if (this.typed !== '') {
      this.onMeasurementCb(typedReadout(this.typed))
      return
    }
    this.onMeasurementCb(formatLength(arcLen))
  }

  private _dispose(obj: THREE.Object3D | null): void {
    if (obj === null) return
    this.preview.remove(obj)
    if (obj instanceof LineSegments2) {
      // Fat lines are registered with `fatLine.ts`'s resolution registry at
      // construction (`makeFatSegments`); the generic Mesh path below WOULD
      // dispose the geometry/material fine (LineSegments2 extends Mesh), but
      // would never drop the material from that registry, leaking the
      // reference (and re-touching a disposed material on every resize)
      // forever. `disposeFatSegments` is the one correct teardown.
      disposeFatSegments(obj)
      return
    }
    if (!(obj instanceof THREE.Mesh) && !(obj instanceof THREE.LineSegments)) return
    // Marker geometry is shared across every marker and every path pick (see
    // `markerGeometry`); disposing it here would free a buffer the next pick
    // still needs. Materials are per-marker and always disposed.
    if (obj.userData.sharedGeometry !== true) obj.geometry.dispose()
    const mat = obj.material
    if (Array.isArray(mat)) mat.forEach((m) => m.dispose())
    else mat.dispose()
  }

  private _buildLines(points: number[], color: number, renderOrder: number): LineSegments2 {
    return makeFatSegments(new Float32Array(points), {
      color,
      widthPx: PREVIEW_LINE_STYLE.widthPx,
      depthTest: false,
      renderOrder,
    })
  }

  /**
   * The highlight geometry for a path target: a flat list of world-space
   * line-segment endpoints (pairs) tracing the swept loop/chain, plus a
   * stable identity key so the hover preview only rebuilds when the target
   * changes. A stale handle yields no points (the commit will surface the
   * error instead of a misleading highlight).
   */
  private _targetHighlight(path: PathTarget): { key: string; points: number[] } {
    const points: number[] = []
    try {
      if (path.kind === 'edges') {
        for (const edge of path.edgeHandles) {
          const ends = this.wasmScene.sketch_edge_endpoints(path.sketchHandle, edge)
          if (ends !== undefined && ends.length >= 6) {
            points.push(ends[0], ends[1], ends[2], ends[3], ends[4], ends[5])
          }
        }
        const key = `edges:${path.sketchHandle}:${[...path.edgeHandles].sort().join(',')}`
        return { key, points }
      }
      const loop = this.wasmScene.face_boundary(path.objectHandle, path.faceHandle)
      for (let i = 0; i < loop.length; i += 3) {
        const j = (i + 3) % loop.length
        points.push(loop[i], loop[i + 1], loop[i + 2], loop[j], loop[j + 1], loop[j + 2])
      }
      // The instance is part of the key so it can never collide across two
      // placements of one definition (defensive — a followable face is always
      // a plain world object, so this is `world` in practice).
      const at = path.instance === undefined ? 'world' : path.instance.toString()
      return { key: `face:${path.objectHandle}:${path.faceHandle}:${at}`, points }
    } catch {
      return { key: 'stale', points: [] }
    }
  }
}
