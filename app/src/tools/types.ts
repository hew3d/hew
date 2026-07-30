/**
 * Tool interface and supporting types for the Hew interaction layer.
 * See docs/DEVELOPMENT.md for the design.
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
 *   center, quadrant, intersection) that lie OFF `constraintPlane`. Only a
 *   tool that can HONOUR an off-plane point may set it — today that is
 *   LineTool's plane-mode chain, whose commit re-homes onto a new sketch
 *   plane through the anchor and the snapped point. A tool that commits
 *   into one frozen plane must leave it unset: it would have to project
 *   the reported point back onto the plane, i.e. lie about the snap.
 */
export interface SnapConstraint {
  anchor?: [number, number, number]
  lockAxis?: 0 | 1 | 2
  constraintPlane?: { point: [number, number, number]; normal: [number, number, number] }
  offPlanePoints?: boolean
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
 * Select, TapeMeasure) are never "armed" by this definition; their Escape
 * has no gesture to protect.
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
