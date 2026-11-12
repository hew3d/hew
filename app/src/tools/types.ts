/**
 * Tool interface and supporting types for the Hew interaction layer.
 * See docs/dev/DEVELOPMENT.md for the design.
 */

import type * as THREE from 'three'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'

/**
 * The single editing-context channel (component-edit-parity.md phase A1):
 * replaces the four ad-hoc duck-typed channels this interface used to grow
 * one at a time (`setActiveContext` — object contexts only, null for an
 * instance context — `setComponentContext`, `setContextScoped`,
 * `setActiveGroup`). One value, pushed to every tool through `setEditContext`,
 * that says exactly what is being edited right now:
 *
 * - `'top'` — nothing entered; ordinary top-level editing.
 * - `'object'` — inside a plain Object's own direct-edit context (double-
 *   clicked a solid that isn't grouped or instanced).
 * - `'group'` — inside a Group's editing context.
 * - `'instance'` — inside a component INSTANCE's shared DEFINITION: `id` is
 *   the instance handle being viewed through (poses/pose⁻¹ math needs the
 *   specific placement, not just which definition), `component` is its
 *   definition (`instance_def(id)`) — the handle every `*_in_instance`/
 *   `*_in_component` wasm surface keys new geometry to.
 */
export type EditContext =
  | { kind: 'top' }
  | { kind: 'object'; id: bigint }
  | { kind: 'group'; id: bigint }
  | { kind: 'instance'; id: bigint; component: bigint }

/** Structural equality for `EditContext` — tools use this to decide whether
 *  a `setEditContext` call is a genuine context change (which should abort
 *  any in-progress gesture, mirroring the old `setActiveContext`'s
 *  re-assertion guard) or a no-op re-push of the same context. */
export function editContextEq(a: EditContext, b: EditContext): boolean {
  if (a.kind !== b.kind) return false
  if (a.kind === 'top') return true
  if (a.kind === 'instance' && b.kind === 'instance') {
    return a.id === b.id && a.component === b.component
  }
  return (a as { id: bigint }).id === (b as { id: bigint }).id
}

/** Snap result shape — mirrors SnapJs from wasm-api, but pure TypeScript */
export interface Snap {
  x: number
  y: number
  z: number
  kind: string
  direction?: [number, number, number]
  /** Under an axis lock, the candidate's own position before it was
   *  projected onto the locked line (`x/y/z` are the projection). The cue
   *  layer draws the dotted tie between the two. */
  projectedFrom?: [number, number, number]
  /** For a reference-line snap (`kind === 'from-point'`, `referenceLines.ts`):
   *  the foot on the drawing plane of the reference point the line runs
   *  through. The cue draws the dashed line from here to the snap point. */
  from?: [number, number, number]
  /** The reference point itself when it is off the drawing plane (the foot's
   *  source), so the cue can draw the thin tie from it to its foot. Also set
   *  on a plain projected snap (a named point dropped onto the plane), where
   *  it is the point before the drop and `x/y/z` is the foot. */
  fromSource?: [number, number, number]
  /** For a reference-line CROSSING: the second line through the snap point,
   *  drawn like the first. */
  fromAlso?: {
    from: [number, number, number]
    direction: [number, number, number]
    fromSource?: [number, number, number]
  }
  /** For an `'intersection'` snap: true when an edge passes THROUGH a face
   *  of another owner (the kernel's `pierced_face` is set), false when two
   *  edges — or an edge and a guide or axis — cross. The edge is the snap's
   *  provenance either way; a tool only words the point differently. */
  piercesFace?: boolean
  /** For an `'on-intersection-line'` snap, or the `'midpoint'` of such a
   *  line: the two ends of the stretch where a face of one owner meets a
   *  face of another (the kernel's `meeting_line`), so the cue layer can
   *  draw the virtual edge under the cursor. Its `elementKind` is
   *  `'meeting-line'`, never `'face'`: it lies on two faces at once and is
   *  a pick of neither, so no tool adopts a face's plane, normal or part
   *  from it. `object`/`element` (or `sketch`/`sketchRegion`) still name
   *  the first of the two faces, the line's owner of record. */
  meetingLine?: [[number, number, number], [number, number, number]]
  object?: bigint
  /** Placing component instance handle, when the snap came from instanced
   *  geometry (pairs with `object`). */
  instance?: bigint
  element?: bigint
  elementKind?: string
  /** Owning sketch handle when the snap derives from a committed sketch
   *  edge (`elementKind === 'sketch-edge'`; `element` is the edge), a drawn
   *  region's fill (`elementKind === 'sketch-region'`; `sketchRegion` is the
   *  region), or a drawn curve's analytic point (`elementKind ===
   *  'sketch-curve'`; `sketchCurve` is the chain). */
  sketch?: bigint
  /** Region handle when the snap is on a drawn sketch region's fill
   *  (`elementKind === 'sketch-region'`). */
  sketchRegion?: bigint
  /** Curve-chain handle when the snap is an analytic point of a drawn
   *  (unextruded) curve — a circle's/arc's exact center, one of its covered
   *  quadrant points, an anchored tangent, or a regular polygon's drawn
   *  center (`elementKind === 'sketch-curve'`). `element` is undefined for
   *  these: the point belongs to the CHAIN, and a center lies on no edge at
   *  all, so there is no honest edge handle to report. */
  sketchCurve?: bigint
}

/**
 * Snap constraints a tool injects into the next `snapService.resolve()`
 * call (see `Tool.snapConstraint`):
 *
 * - `anchor` + `lockAxis`: axis-lock for distance tools (e.g. MoveTool).
 * - `constraintPlane`: restrict candidates to a plane (e.g. RectangleTool
 *   in face mode, to avoid snapping to occluded off-plane geometry).
 * - `offPlanePoints`: keep precise POINT candidates (endpoint, midpoint,
 *   center, quadrant, intersection) that lie OFF `constraintPlane`. A tool
 *   sets this only if it can HONOUR an off-plane point one of three ways:
 *   re-homing onto a new plane through it (LineTool's plane-mode chain),
 *   projecting the reported point back onto the frozen plane WHILE
 *   disclosing that it did so via `Tool.snapProjected` (TapeMeasureTool's
 *   `measure` stage), or — a third pattern, TapeMeasureTool's `parallel`
 *   stage — consuming only the candidate's projection along an in-plane
 *   offset direction rather than the plane itself, with the SAME
 *   `Tool.snapProjected` disclosure obligation whenever that discards a
 *   nonzero off-plane component. In every disclosing case, the disclosure is
 *   what keeps the projection from being a silent lie about the snap. A tool
 *   that does none of the three must leave `offPlanePoints` unset.
 * - `facesOnly`: restrict the winning candidate to the on-face point under
 *   the cursor (PushPullTool's idle hover). Inference's rank order is a
 *   hard total order (`crates/inference` `rank_group` — `on-face` can never
 *   outrank a precise point kind like `endpoint`/`midpoint`), so a face the
 *   user is aiming at from near its edge would otherwise show — and drag
 *   from — a corner/edge chip instead of the face itself. Set alongside
 *   `constraintPlane`: `SnapService.resolve` replaces any winner whose kind
 *   isn't `on-face`/`plane` with the plane fallback (ray ∩
 *   `constraintPlane`), so the hover cue is always the on-face dot under the
 *   cursor on the face that will actually be pushed.
 */
export interface SnapConstraint {
  anchor?: [number, number, number]
  lockAxis?: 0 | 1 | 2
  constraintPlane?: { point: [number, number, number]; normal: [number, number, number] }
  offPlanePoints?: boolean
  /**
   * The tool drops an off-plane snap onto `constraintPlane` and discloses it
   * through `snapProjected` (the draw tools' `cursorOnFrozenPlane`), so in
   * X-ray and Wireframe hidden points and edges behind the face it is held
   * to may be offered. A tool that commits the snapped point as-is must
   * leave this unset: it then gets the plain plane filter in every mode.
   */
  seeThroughPoints?: boolean
  facesOnly?: boolean
  /**
   * A plane used ONLY for the snap service's ray∩plane fallback when the
   * kernel offers no candidate; never passed to the kernel, so geometry
   * anywhere still snaps. The idle draw tools set it to the axis plane the
   * view faces (view-plane design, picker step 7), so a click in empty space
   * lands on that plane while a nearby corner or edge still wins.
   * `constraintPlane`, when set, takes precedence.
   */
  fallbackPlane?: { point: [number, number, number]; normal: [number, number, number] }
}

/**
 * What a click right now would pick up from under the cursor — the
 * transform tools' empty-selection auto-acquire (Move/Rotate/Scale) — so the
 * viewport can highlight it on hover before the click (SketchUp's
 * pre-selection highlight). A tool offers it through an optional
 * `clickAcquire(): ClickAcquire | null` (null: a click acquires nothing).
 * `guides` before `nodes`, as Move's click takes a guide over the object
 * beneath it.
 */
export interface ClickAcquire {
  guides: boolean
  nodes: boolean
}

/**
 * One active tool at a time. ToolController owns the routing; each method
 * is called with the resolved snap result (or null when snap is unavailable
 * and fallback was also null).
 */
export interface Tool {
  /** Called on every pointer move — update preview / cues */
  onPointerMove(snap: Snap | null, ray: Ray): void
  /** Called on pointer down — advance the gesture */
  onPointerDown(snap: Snap | null, ray: Ray): void
  /** Called on key events — esc cancels current stage */
  onKey(ev: KeyboardEvent): void
  /** Full reset — clear all preview / ephemeral state */
  cancel(): void
  /** Human-readable name shown in the status bar */
  readonly name: string

  /**
   * (optional) Per-render-tick hooks for tools that draw screen-constant
   * widgets — grips (ScaleTool, FollowMeTool, PositionTextureTool) and single
   * disks (RotateTool, ProtractorTool, SliceTool, SectionPlaneTool). The
   * Viewport's render loop feature-detects these and calls them once per
   * frame BEFORE `renderer.render()`, passing a `worldPerPixel(dist)`
   * callback derived from the active `CameraRig` — projection-agnostic, so
   * the same widget sizing works under perspective and parallel alike.
   *
   * Declared HERE, on `Tool`, rather than left to the Viewport's `as` cast:
   * a cast asserts a shape without checking any implementor, so a tool that
   * wrote its own incompatible signature (taking a viewport height in pixels
   * where the callback actually arrives) type-checked cleanly and then
   * computed NaN sizes at runtime — invisible, unpickable widgets. With the
   * members declared, that mismatch is a compile error in the tool itself.
   */
  updateGripScale?(camera: THREE.Camera, worldPerPixel: (dist: number) => number): void
  updateDiskScale?(camera: THREE.Camera, worldPerPixel: (dist: number) => number): void

  /**
   * (optional) The live camera, pushed once per render tick by the same loop.
   * `onPointerMove`/`onPointerDown` carry only a per-pixel cursor ray, never
   * the camera itself, so this is the one feed for tools that need the actual
   * view direction — DimensionTool and TextTool, for `axisDimensionPlane`'s
   * face-on candidate scoring. Declared here for the same reason as the two
   * hooks above: a cast at the call site checks nothing.
   */
  updateCamera?(camera: THREE.Camera): void

  /**
   * (optional) True for a tool whose idle hover follows the view: the plane
   * a click in empty space lands on is inferred from the camera
   * (docs/design/view-plane-inference.md), so a still cursor's answer
   * changes when an idle key, Shift or the camera does. Only these tools get
   * the Viewport's held-pointer hover replay after those events
   * (`followsViewPlane`, heldPointerReplay.ts); every other tool keeps its
   * hover where the last real pointer move left it.
   */
  readonly followsViewPlane?: boolean

  /**
   * (optional) The plane state the view die, pips and cursor cue read
   * (docs/design/view-die.md §3.4). Null / absent when the tool is not a
   * plane-drawing tool. Implemented by the five `followsViewPlane` draw tools.
   */
  drawPlaneState?(): import('./drawPlane').DrawPlaneState | null

  /**
   * (optional) Toggle the Shift plane pin as one fresh Shift press would
   * while idle — the view die's custom pip (docs/design/view-die.md §3.3).
   * A no-op mid-gesture. Paired with `pinnablePlane`.
   */
  togglePlanePin?(): void

  /**
   * (optional) The plane `togglePlanePin` acts on right now: the pinned
   * plane, else the tracked hover plane a pin would take; null mid-gesture
   * or with nothing to pin.
   */
  pinnablePlane?(): import('./drawPlane').DrawPlane | null

  /**
   * (optional) The plane the view die's custom pip acts on: a pinned custom
   * plane, a custom tracked hover plane, or the live custom candidate (the
   * last custom plane hovered, kept for 2 s and while the pointer is off
   * the canvas on its way to the pip); null mid-gesture or with none, which
   * disables the pip (docs/design/view-die.md §3.3).
   */
  customPipPlane?(): import('./drawPlane').DrawPlane | null

  /**
   * (optional) The custom pip's click: release an active pin, else pin the
   * custom candidate's plane. A no-op mid-gesture.
   */
  pinCustomCandidate?(): void

  /**
   * (optional) Milliseconds until the custom candidate's window closes, or
   * null with no timed expiry pending — the host schedules one pip refresh
   * then.
   */
  customPipTtl?(): number | null

  /**
   * (optional) Whether the arrow `key` at the tool's current stage would
   * release the held plane lock rather than set one — so the view die's
   * pip tooltip reads "Release" only when a click would release
   * (docs/design/view-die.md §3.3). Line mid-chain answers false: its
   * arrows lock the segment's axis there.
   */
  planeKeyReleases?(key: string): boolean

  /**
   * (optional) True when the snap most recently handed to `onPointerMove` was
   * PROJECTED onto the tool's drawing plane rather than used as-is — i.e. the
   * point the gesture will actually commit is not the point the inference
   * chip is naming.
   *
   * Only the draw tools can answer this, and only they know it: a shape has to
   * be planar on the plane it is drawn on, so a snap to a vertex floating
   * above the ground contributes its x/y and has its z discarded. That is the
   * intended behaviour — it is how you align a rectangle under a solid's
   * corner — but reporting it as a plain "Endpoint" claims a snap that was not
   * honoured. The Viewport passes this through to the inference chip so the
   * label can say so.
   *
   * False (or absent) means the snap was taken exactly as reported, which
   * includes every idle hover: before a gesture is anchored, a click on that
   * floating vertex legitimately starts a sketch THERE, so nothing is
   * projected and nothing needs qualifying.
   */
  snapProjected?(): boolean

  /**
   * (optional) A cursor-side explanation for what is under the pointer when
   * the tool cannot act on it — Push/Pull hovering a face that belongs to a
   * component not being edited ("double-click it to edit the component").
   * The Viewport passes it through to the inference chip, which shows it as
   * its own line beside the cursor, so the guidance sits where the person
   * is looking instead of only in a bottom toast after the click is
   * refused. `null` (or absent) means nothing to explain right now.
   */
  hoverHint?(): string | null

  /**
   * (optional) The camera moved (a wheel zoom, a Scene tween, Fly) while the
   * pointer rests on the canvas: `ray` is the ray at the pointer's position
   * for the camera as it is now, undefined when no cursor is over the canvas
   * to aim one (a button held, a panel covering it). A tool that lights
   * something under the cursor re-aims it here, as a pointer move would.
   */
  onViewChanged?(ray: Ray | undefined): void

  /**
   * (optional) The pointer left the canvas. A tool that memoizes anything
   * about what is hovered (`hoverHint`'s refusal line, a draw tool's idle
   * plane patch) drops it here, so the status bar and the drawing-plane cue
   * (both re-polled right after) do not keep describing a face or plane the
   * cursor is no longer over.
   */
  onPointerLeave?(): void

  /**
   * (optional) What a click right now would pick up from under the cursor
   * (`ClickAcquire`; null: nothing), for the viewport's hover highlight.
   */
  clickAcquire?(): ClickAcquire | null

  /**
   * (optional) Return snap constraints the tool wants injected into the next
   * snapService.resolve() call.  Viewport feature-detects with
   * `'snapConstraint' in tool`.
   *
   * - `anchor` + `lockAxis`: axis-lock for distance tools (e.g. MoveTool)
   * - `constraintPlane`: restrict candidates to a plane (e.g. RectangleTool
   *   in face mode, to avoid snapping to occluded off-plane geometry)
   *
   * The optional `ray` argument is the current pointer ray; tools that need to
   * pick the hovered face (e.g. RectangleTool idle) can use it.  Tools that
   * don't need it may omit the parameter.
   */
  snapConstraint?(ray?: Ray): SnapConstraint | null

  /**
   * (optional) When true the tool is capturing raw keyboard input (e.g. VCB
   * numeric entry) and the Viewport should route key events to it BEFORE any
   * tool-switch shortcuts.  Viewport feature-detects with
   * `'capturingInput' in tool`.
   */
  capturingInput?(): boolean

  /**
   * (optional) True when the tool has an armed, in-progress gesture that
   * Escape belongs to (component-edit-parity.md phase A2) — distinct from
   * `capturingInput`, which for most tools happens to mean the same thing
   * but for SliceTool does NOT: Slice reports `capturingInput() === true`
   * unconditionally (so a hovering typed VCB offset never leaks into
   * tool-switch shortcuts), even though it has no cancelable gesture beyond
   * its plane LOCK. `toolHasArmedGesture` below checks this first, falling
   * back to `capturingInput()` for every other tool. Feature-detected with
   * `'hasArmedGesture' in tool`.
   */
  hasArmedGesture?(): boolean

  /**
   * (optional) Per-key refinement of `capturingInput`: whether THIS key
   * belongs to the tool's input capture. Lets a tool capture only the keys
   * its buffer actually needs (e.g. Move's armed ×N / /N window takes
   * digits and mode tokens but must never eat Space, which always resets
   * to Select) while a full VCB gesture still captures the whole keyboard.
   * Consulted by the Viewport's key routing and by App-level shortcut
   * gates via `ViewportApi.isCapturingInput(key)`; when absent, the plain
   * `capturingInput()` verdict applies to every key. Feature-detected with
   * `'capturesKey' in tool`.
   */
  capturesKey?(key: string): boolean

  /**
   * (optional) Called on a double-click BEFORE the Viewport's default
   * "enter context" gesture. Return `true` if the tool consumed the
   * double-click (e.g. LineTool ending a chain) so the Viewport skips
   * entering a group/instance/object; return `false`/omit to fall through to
   * the default behavior. Viewport feature-detects with
   * `'onDoubleClick' in tool`.
   */
  onDoubleClick?(snap: Snap | null, ray: Ray): boolean

  /**
   * (optional) Called on a real pointer-UP (button release). Every OTHER
   * gesture in this codebase is click-move-click (a press arms a stage, a
   * SECOND press commits — there is no generic release hook; see
   * PushPullTool/SectionPlaneTool), which works because "click to commit"
   * and "press-drag-release to commit a different value" were never both
   * needed on the SAME first press. A tool that needs exactly that — a
   * plain click (negligible movement) commits one thing, a real press-drag-
   * release commits another (Follow Me's drag-to-partial-sweep, E4) —
   * cannot tell those apart from `onPointerDown` alone, since a press only
   * ever arms; only the matching release can distinguish "the user let go
   * without moving" from "the user is still deciding". Viewport feature-
   * detects with `'onPointerUp' in tool` and calls this on a real DOM
   * `pointerup` (button 0), AFTER resolving the same snap/ray a move would
   * get. Most tools should NOT implement this — it exists for the one
   * gesture shape click-move-click cannot express, not as a general
   * alternative to it.
   */
  onPointerUp?(snap: Snap | null, ray: Ray): void

  /**
   * (optional) Click-drag-release participation (GitHub #52b, SketchUp's
   * second way to give a gesture its two points): `true` for a tool whose
   * gesture has a start point and an end point. With it, a left press that
   * STARTS the gesture (the tool idle before the press, mid-gesture after)
   * and travels past the drag threshold before its release has that release
   * delivered as the second click — through `onPointerDown`, at the release
   * snap, so commits, typed values and inference are exactly a click's. A
   * short press stays a plain click; a press in any later stage is never
   * armed. The shared mechanism lives in viewport/dragCommit.ts; a tool
   * declares only this flag. Leave it unset for a tool whose press or
   * release already means something of its own (Select's drag-move and
   * marquee, Follow Me's `onPointerUp` sweep, Walk, Position Texture, …).
   */
  readonly dragCommits?: boolean

  /**
   * (optional) "A gesture is between its first point and its last", for the
   * drag gate above, when `capturingInput()` cannot say it alone because it
   * is also true for a post-commit typing window (Move's and Rotate's array
   * copies, the chain Line holds for a dragged line's retype). Absent, the
   * gate reads `capturingInput()`.
   */
  gestureInFlight?(): boolean

  /**
   * (optional) Called right after a drag-release was delivered as the
   * second click (see `dragCommits`), for a tool whose second click alone
   * would not end the gesture the way a drag should. Line: a dragged line
   * is one segment and the chain ends (SketchUp), where a clicked one
   * keeps chaining.
   */
  onDragCommitted?(): void

  /**
   * (optional) Raw canvas-relative CSS-pixel pointer position, called on
   * EVERY pointer move alongside `onPointerMove` (both fire — this is
   * additive, not a replacement) with the live `PointerEvent.buttons`
   * bitmask and Shift's live state. For a gesture that is fundamentally a
   * SCREEN-SPACE drag rather than a world pick — Look Around's mouse-look
   * and Walk's forward/turn (docs/design/camera.md §4) — `onPointerMove`'s
   * resolved `Snap`/`Ray` carry no useful signal (there is no "point in the
   * scene" being aimed at) and no pixel delta at all; this hook exists
   * for exactly that shape. Most tools should NOT implement this. Viewport
   * feature-detects with `'onPointerRawMove' in tool`.
   */
  onPointerRawMove?(xPx: number, yPx: number, buttons: number, mods: { shift: boolean }): void

  /**
   * (optional) The pointer's canvas-pixel position on every ordinary move,
   * delivered right AFTER `onPointerMove` for the same event — unlike
   * `onPointerRawMove` this does NOT opt the tool out of snap resolution,
   * it only adds the screen coordinate a world-space tool cannot recover
   * from its ray (under parallel projection every ray shares one direction,
   * so "has the pointer moved a few pixels?" is unanswerable from rays
   * alone). LineTool's post-click retype window uses it to tell "type the
   * length of the segment just placed" from "move, then type the next
   * segment". Feature-detected with `'onPointerScreenMove' in tool`.
   */
  onPointerScreenMove?(xPx: number, yPx: number): void

  /**
   * (optional) Raw canvas-relative CSS-pixel pointer position at a genuine
   * left-button pointerDOWN, called alongside `onPointerDown` (both fire).
   * Pairs with `onPointerRawMove` for a screen-space drag gesture that needs
   * to measure movement FROM THE PRESS POINT (Walk's forward/turn speed,
   * design §4 — proportional to net drag distance, not a per-frame
   * velocity) rather than incrementally since the last move. Viewport
   * feature-detects with `'onPointerRawDown' in tool`.
   */
  onPointerRawDown?(xPx: number, yPx: number): void

  /**
   * (optional) A new/loaded document has replaced the Scene. Tools that cache
   * kernel handles across gestures (e.g. a ground-sketch handle) must drop
   * them here — reusing a handle from the previous document throws
   * UnknownSketch. The Viewport calls this on the active tool from its
   * `notifyLoaded`. Feature-detected with `'onDocumentReset' in tool`.
   */
  onDocumentReset?(): void

  /**
   * (optional) Kernel history changed under this tool via undo/redo — ANY
   * entry point (Edit menu, command palette, or the viewport's own Cmd+Z /
   * Cmd+Shift+Z), all funneled through the Viewport's shared `runUndo`/
   * `runRedo` choke point. Tools that cache UI-side descriptions of
   * ALREADY-COMMITTED kernel geometry across a multi-click gesture (e.g.
   * LineTool's `_chainVertices`, used only for its same-chain cross-sketch
   * coincidence check) must drop that cached state here — an undo can
   * remove the very segments it describes, leaving a phantom that no
   * longer corresponds to real geometry and can misfire against a
   * perfectly legitimate later action. This does NOT reset the gesture
   * itself (`planeStage`/`faceStage` stay anchored) — only stale
   * descriptions of committed geometry are invalidated; unlike
   * `onDocumentReset`, the tool is not rewound to idle. Feature-detected
   * with `'onHistoryChanged' in tool`.
   */
  onHistoryChanged?(): void

  /**
   * (optional) The app selection changed while this tool is active. Tools
   * that snapshot the selection at creation (Move/Rotate/Scale) implement
   * this so the NEXT gesture starts from live handles — without it, an
   * undo that killed selected nodes left the tool committing against dead
   * handles (UnknownObject). The Viewport pushes every selection change
   * (clicks, Outliner, undo/redo pruning) into the active tool.
   * Feature-detected with `'setSelection' in tool`.
   */
  setSelection?(nodes: NodeRef[]): void

  /**
   * (optional) One live "what do I do next" line for the status bar,
   * reflecting the tool's CURRENT stage — "Click the opposite corner", not
   * a static tool description. The Viewport re-polls after every routed
   * event (move/down/key/double-click/cancel/switch) and pushes changes up;
   * tools without it fall back to the palette's static description.
   * Feature-detected with `'statusHint' in tool`.
   */
  statusHint?(): string

  /**
   * (optional) The current editing context (component-edit-parity.md phase
   * A1) — see `EditContext`'s doc. Called by the Viewport whenever the
   * active context path changes, AND once right after a tool is constructed
   * (switching tools doesn't itself change the context, so a freshly made
   * tool needs the CURRENT value pushed to it explicitly). Feature-detected
   * with `'setEditContext' in tool`. Replaces `setActiveContext`/
   * `setComponentContext`/`setContextScoped`/`setActiveGroup`, which no
   * tool implements any more.
   */
  setEditContext?(ctx: EditContext): void
}

/**
 * True when `tool` has an armed, in-progress gesture that Escape belongs to
 * (a two-click Move/Rotate/Scale/PushPull/Offset drag, a Follow Me sweep
 * past `pick-path`, a draw tool's anchored chain, Slice's plane lock, …).
 *
 * Prefers the tool's own `hasArmedGesture()` when present (SliceTool: its
 * `capturingInput()` is unconditionally `true` for an unrelated reason — see
 * that method's doc — so it opts out with a precise `hasArmedGesture` of its
 * own). Every other armed tool's `capturingInput()` already means exactly
 * "gesture in flight", so that is the fallback. Tools with neither (e.g.
 * Select) are never "armed" by this definition; their Escape has no gesture
 * to protect.
 *
 * The Viewport consults this BEFORE popping a level off the edit-context
 * path on Escape (component-edit-parity.md phase A2): without it, Escape
 * would pop the context out from under an armed gesture, which then gets
 * the new (shallower) context silently pushed into it via `setEditContext`
 * — retargeting the gesture's eventual commit instead of the plain cancel
 * the user asked for. Escape reaching an armed tool routes to `onKey`
 * instead, which cancels/steps back the gesture; only an UNARMED tool lets
 * Escape fall through to its traditional context-pop meaning.
 */
export function toolHasArmedGesture(tool: Tool): boolean {
  if ('hasArmedGesture' in tool) {
    return (tool as { hasArmedGesture(): boolean }).hasArmedGesture()
  }
  return 'capturingInput' in tool && (tool as { capturingInput(): boolean }).capturingInput()
}
