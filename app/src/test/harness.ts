/**
 * `window.__hew_test` — the semantic test harness (docs/DEVELOPMENT.md).
 *
 * Debug/test builds only. It lets a driver (Playwright in, or the console)
 * issue **semantic** actions and read state *by logic, not canvas pixels* — the
 * answer to "the viewport is an opaque WebGL canvas" (docs/DEVELOPMENT.md). Most E2E
 * tests should drive through this rather than synthesize pointer events.
 *
 * Design:
 * - Modeling actions call the same kernel ops the tools commit (`begin_ground_
 *   sketch`/`sketch_add_segment`/`extrude_region`/`push_pull`/`boolean`/…), then
 *   reconcile the app exactly as a tool commit would, so state stays faithful.
 * - **Handles cross the boundary as decimal strings, never bigint.** u64 kernel
 *   handles and the `state_hash` exceed `Number.MAX_SAFE_INTEGER` and bigint is
 *   not structured-cloneable out of `page.evaluate`; strings are exact and
 *   portable. Args accept a string and are `BigInt()`-converted internally.
 * - Picking is pixel-free: `pickFace` casts a **world-space ray** (e.g. straight
 *   down onto a top face), so `pushPull(obj, face, …)` gets a real face handle
 *   without any screen projection.
 * - `startRecording`/`takeRecording` tie the high-level (`Scene`) and
 *   low-level (`inputRecorder`) streams into one session artifact.
 *
 * Not exposed (deliberately): `selectFace`/`selectEdge`/`hoverPoint` from the
 * sketch list — the app has no *persistent* sub-element selection (faces/edges
 * are picked transiently inside tools), and `pushPull` takes an explicit face
 * from `pickFace` instead. Object-level selection is real, via `selectObjects`.
 */

import type { Scene } from '../wasm/loader'
import type { ViewportApi } from '../viewport/Viewport'
import { exportSceneToStl, type StlExportScene } from '../io/exporters/stlExport'
import { nodeKindToNumber, type NodeKind, type NodeRef } from '../panels/treeModel'
import * as inputRecorder from '../recording/inputRecorder'
import { buildSessionRecording } from '../recording/sessionRecording'
import { arcFromChord, arcPolylineOnPlane } from '../tools/arcMath'
import { facePlaneBasis, type V3 } from '../viewport/geoHelpers'
import {
  formatLength,
  getLengthUnit,
  parseLengthToMeters,
  setLengthUnit,
  type LengthFormat,
} from '../settings/units'

type Vec3 = [number, number, number]

export interface CameraPose {
  position: Vec3
  target: Vec3
  up?: Vec3
  fovDeg?: number
}

/** What the harness needs from the app; all live (read at call time). */
export interface HarnessDeps {
  getScene: () => Scene | null
  getViewportApi: () => ViewportApi | null
  /** Reconcile + re-render after a mutation (the app's document-changed path). */
  reconcile: () => void
  /** Current object/group/instance selection. */
  getSelection: () => NodeRef[]
  /** Replace the selection with these object handles. */
  setSelectedObjects: (ids: bigint[]) => void
  /** Replace the selection with arbitrary nodes (objects/groups/instances). */
  setSelection: (nodes: NodeRef[]) => void
  /**
   * Reload `.hew` bytes through the app's real Open path (the same
   * `scene.load` + UI reset + viewport `notifyLoaded` re-tessellation a user's
   * File→Open runs). Returns false if the load was rejected..
   */
  loadBytes: (bytes: Uint8Array) => boolean
  /**
   * Toggle a tag path's hidden flag through the app's real Tags-panel path
   * (session hidden set + union push to renderer/kernel + persisted
   * `set_tag_hidden`) — NOT a bare scene call, which would skip the app's
   * visibility state.
   */
  toggleTagPath: (path: string[]) => void
  /** Delete a tag everywhere through the app's real Tags-panel path
   * (kernel `delete_tag` + tag-visibility resync + document-changed). */
  deleteTag: (path: string[]) => void
}

/** The `ImportReport` shape `scene.import_stl` returns across the boundary
 * (structured-clone-safe: numbers + string arrays). */
export interface StlImportReportJs {
  objects_created: number
  watertight: number
  leaky: number
  skipped: { name: string; reason: string }[]
  textures_missing: string[]
  warnings: string[]
}

export interface HewTestHarness {
  /** True once the kernel scene AND the viewport API are both wired — wait on
   * this before driving any harness op (camera/draw/pick need the viewport). */
  isReady(): boolean
  // modeling
  drawRectangle(p0: Vec3, p1: Vec3): { sketch: string; region: string }
  extrudeRegion(sketch: string, region: string, distance: number): string
  /** Convenience: rectangle on the ground from p0→p1, extruded `height`. */
  drawBox(p0: Vec3, p1: Vec3, height: number): string
  /** `instance` is the placement handle when the ray struck geometry reached
   *  through a component instance (definition-local `object`/`face`), or
   *  `null` for a plain/group-member hit — the same three-way split
   *  `pick_face`'s own `instance()` accessor returns. */
  pickFace(rayOrigin: Vec3, rayDir: Vec3): { object: string; face: string; instance: string | null } | null
  pushPull(object: string, face: string, distance: number): void
  boolean(op: number, a: string, b: string): string
  /**
   * Combine two tree nodes — plain solids or whole groups (`boolean_nodes`,
   * the group-ops design). `op` is 0=union, 1=subtract (a−b), 2=intersect;
   * `a`/`b` are `{kind, id}` refs with kind `'object' | 'group'`. Returns the
   * result root: a single object, or a result group of disjoint pieces.
   */
  booleanNodes(
    op: number,
    a: { kind: string; id: string },
    b: { kind: string; id: string },
  ): { kind: string; id: string }
  /**
   * Group nodes into a merge group (the same `group_nodes` call Edit ▸ Group
   * commits) and return the group handle. Each ref's kind is
   * `'object' | 'group' | 'instance'`.
   */
  groupNodes(nodes: { kind: string; id: string }[]): string
  /**
   * Direct members of a group, as `{kind, id}` refs — for asserting result
   * and copy structure without the Outliner DOM.
   */
  getGroupMembers(id: string): { kind: string; id: string }[]
  /** Whether an object is currently a watertight solid (`object_solid`). */
  isObjectSolid(id: string): boolean
  /**
   * World-axis-aligned bounding box of `object`'s current mesh, as
   * `[minX,minY,minZ,maxX,maxY,maxZ]` (meters) — read from `object_mesh`'s
   * position buffer, the same source ScaleTool's gizmo box is computed from
   * on the UI side.
   */
  getObjectBounds(id: string): [number, number, number, number, number, number]
  /**
   * Deep-copy ANY tree node — object, group, or instance — offset by
   * `(dx, dy, dz)` meters: `duplicate_node` on the node's own kind, exactly
   * the Move+Alt copy commit. Returns the new root node.
   */
  copyNode(
    kind: string,
    id: string,
    dx: number,
    dy: number,
    dz: number,
  ): { kind: string; id: string }
  deleteObject(id: string): void
  selectObjects(ids: string[]): void
  /** Edit ▸ Select All: every visible top-level node + free sketch (or a
   * group context's direct members) — the same path ⌘A takes. */
  selectAll(): void
  setCamera(pose: CameraPose): void
  /**
   * Render the scene at two camera poses and count the pixels that differ
   * between the two frames: `differing` (any channel off by > 8/255) and
   * `hard` (> 60/255 — a high-contrast flip, e.g. a dark edge line trading
   * places with the face fill behind it). With poses a sub-pixel rotation
   * apart, `hard` must stay near zero: a spray of hard flips means the
   * depth test is resolving coplanar edge/face fragments by rounding noise
   * (the edge-shimmer defect — every repaint of an orbit's damping tail
   * re-rolls that noise). Counts, not pixels, cross the boundary — frames
   * are megabytes and the verdict is a number.
   */
  frameStability(poseA: CameraPose, poseB: CameraPose): {
    width: number
    height: number
    differing: number
    hard: number
  }
  /**
   * Show/hide the origin axes (View ▸ Axes). Docs-screenshot convenience so a
   * capture can drop the axes when they'd overshadow the modeled solids,
   * without driving the View menu through the DOM. Delegates to the same
   * `ViewportApi.setAxesVisible` the menu item calls.
   */
  setAxesVisible(visible: boolean): void
  /** Show/hide the ground grid (View ▸ Grid). Same rationale as setAxesVisible. */
  setGridVisible(visible: boolean): void
  /** Show/hide all construction guides (View ▸ Guides). */
  setGuidesVisible(visible: boolean): void

  // -------- section plane (session view state, non-destructive) --------
  /** "Toggle Section Plane Active" — flips the placed section's clip on/off
   *  (Tools menu / palette). No-op when none is placed. */
  toggleSectionActive(): void
  /** The current session section (`{ origin, normal, active }`) or null —
   *  observe-only, reads the same SectionManager the tool/menu mutate. */
  getSectionState(): { origin: Vec3; normal: Vec3; active: boolean } | null
  /** Section-plane render diagnostics for a rendered object/instance: whether
   *  the widget overlay is built, the widget's own clip count (must be 0),
   *  the clip-plane count on that node's face material (-1 if not rendered),
   *  and the active clip plane's world normal + constant (null when inactive)
   *  for asserting the clip SIDE. `kind` is 'object'/'instance'; `id` decimal. */
  getSectionRenderInfo(
    kind: 'object' | 'instance',
    id: string,
  ): {
    widget: boolean
    widgetClipCount: number
    nodeClipCount: number
    clipPlane: { normal: Vec3; constant: number } | null
  }

  replay(recordingJson: string): string
  // serialization ( — round-trips the live `.hew` container through the
  // app's real save/open path). Bytes cross `page.evaluate` as a plain number[]
  // (portable + structured-cloneable); a box is tiny, so the cost is moot.
  save(): number[]
  load(bytes: number[]): void
  /** Export the current solid geometry to binary STL bytes (mm, Z-up) through
   * the app's REAL exporter (`exportSceneToStl`); instances flatten into the
   * soup. `segmentsPerTurn` re-facets curved walls (0 = stored facets). Null
   * when nothing solid. Test-only — the round-trip verification uses it. */
  exportStl(segmentsPerTurn: number): { bytes: number[]; triangleCount: number } | null
  /** Import binary/ASCII STL bytes through the REAL `scene.import_stl`,
   * returning the ImportReport (object/watertight/leaky/skipped/warnings).
   * Additive, exactly like File▸Import's kernel call. Test-only. */
  importStl(bytes: number[], unitScale: number): StlImportReportJs
  // recording ( high + low, one artifact)
  startRecording(): void
  stopRecording(): void
  isRecording(): boolean
  takeRecording(): string
  // queries
  getStateHash(): string
  getObjectCount(): number
  getObjectIds(): string[]
  getSelection(): { kind: string; id: string }[]
  getLastError(): string | null

  // -------- NEW in  --------

  /**
   * Draw a chain of line segments in a new ground sketch. Points are an ordered
   * list of positions; segments are added between consecutive pairs. Returns the
   * sketch handle (string) and all closed regions that formed (as handle strings).
   * Equivalent to what LineTool commits: begin_ground_sketch → N × sketch_add_segment.
   */
  drawLineChain(points: Vec3[]): { sketch: string; regions: string[] }

  /**
   * Draw a regular N-gon approximation of a circle in a new ground sketch.
   * `center` is the XY centroid (Z ignored), `radius` in meters, `nSegments`
   * defaults to a fixed 24 for test determinism (CircleTool itself adapts
   * the count to the radius — the true-curves design §6; pass an
   * explicit count to exercise other densities). Returns the sketch and the
   * one closed region that forms (the N-gon). Equivalent to CircleTool's
   * commit, including the analytic curve bracket.
   */
  drawCircle(center: Vec3, radius: number, nSegments?: number): { sketch: string; region: string }

  /**
   * Imprint a circle onto an existing solid face (draw-on-face), carrying the
   * circle's analytic identity so a later push-through of the imprinted disk
   * shades smooth and offsets its radius (the true-curves design, playtest
   * fix C3). `center` must lie on the face plane; `radius` in meters. Mirrors
   * CircleTool face mode (`split_face_inner_with_curve`). Returns the new disk
   * sub-face handle — pick nothing, push it straight through with `pushPull`.
   */
  imprintCircleOnFace(
    object: string,
    face: string,
    center: Vec3,
    radius: number,
    nSegments?: number,
  ): string

  /**
   * Offset a sketch region's whole boundary (outer loop and holes) by a
   * signed distance — negative shrinks the material, positive grows it.
   * The Offset tool's sketch commit: `sketch_offset_region` inside a
   * gesture bracket (one undo step). Returns the handles of the regions the
   * offset created.
   */
  offsetRegion(sketch: string, region: string, distance: number): string[]

  /**
   * Offset a solid face's outer boundary by a signed distance (negative =
   * into the face, the only direction that can land on it) and imprint the
   * loop — the Offset tool's face commit (`offset_face`). Returns the new
   * sub-face handle; push/pull it to boss/recess.
   */
  offsetFace(object: string, face: string, distance: number): string

  /**
   * Draw a faceted 2-point arc in a new ground sketch. `a`/`b` are the
   * chord endpoints (z should be 0 — ground plane), `sagitta` is the signed
   * bulge distance from the chord midpoint (positive on the CCW side of a→b,
   * viewed from +Z). With `close`, also adds the chord b→a so the arc closes
   * into a region. Equivalent to ArcTool's ground commit:
   * begin_ground_sketch → N × sketch_add_segment along `arcPolylineOnPlane`.
   */
  drawArc(a: Vec3, b: Vec3, sagitta: number, close?: boolean): { sketch: string; regions: string[] }

  /**
   * Cut `face` of `object` along a faceted 2-point arc. `a`/`b` must
   * lie on the face (endpoints on its boundary for a boundary-to-boundary
   * cut); `sagitta` is signed in the face's `facePlaneBasis(normal)` (u, v)
   * frame. Equivalent to ArcTool's face commit: one `split_face` call with
   * the arc polyline path.
   */
  drawArcOnFace(object: string, face: string, a: Vec3, b: Vec3, sagitta: number): void

  /**
   * Move `object` by a world translation (meters). Calls `transform_object` with
   * a pure-translation 3×4 affine, exactly as MoveTool commits. The handle
   * is unchanged; the document state_hash changes.
   */
  moveObject(object: string, dx: number, dy: number, dz: number): void

  /**
   * Copy `object` to a new object offset by `(dx, dy, dz)` meters. Calls
   * `duplicate_node(0, id, affine)` with a translation affine, matching the
   * "Option-drag" branch of MoveTool. Returns the new node's kind and id.
   */
  copyObject(id: string, dx: number, dy: number, dz: number): { kind: string; id: string }

  /**
   * Rotate `object` by `angleDeg` degrees around `axis` (default: Z = [0,0,1]).
   * Builds a Rodrigues rotation matrix and calls `transform_object`, matching
   * RotateTool's commit.
   */
  rotateObject(object: string, angleDeg: number, axis?: Vec3): void

  /**
   * Scale `object` by `(sx, sy, sz)` about `pivot` (world meters, non-uniform
   * allowed). Builds the same "diag(sx,sy,sz) about a world pivot" affine as
   * `nonUniformScaleAboutPivot` in `tools/transformMath.ts` and calls
   * `transform_object`, matching ScaleTool's grip-gizmo commit. The handle is
   * unchanged; the document state_hash changes.
   */
  scaleObject(object: string, sx: number, sy: number, sz: number, pivot: Vec3): void

  /**
   * Slice a watertight solid by a plane. `plane` is 6 floats `[px,py,pz,nx,ny,nz]`
   * (a point on the plane + its normal). Returns `[positiveId, negativeId]` as
   * decimal strings. Calls `slice_object` directly (SliceTool's commit).
   */
  sliceObject(
    object: string,
    plane: [number, number, number, number, number, number],
  ): [string, string]

  /**
   * Add a construction guide line through `(ox,oy,oz)` along direction
   * `(dx,dy,dz)`. Returns the guide handle as a decimal string. Matches
   * TapeMeasureTool's parallel-guide commit and ProtractorTool's angular-guide
   * commit.
   */
  addGuideLine(ox: number, oy: number, oz: number, dx: number, dy: number, dz: number): string

  /**
   * Add a construction guide point at `(x,y,z)`. Returns the guide handle as a
   * decimal string. Matches TapeMeasureTool's point-mode commit.
   */
  addGuidePoint(x: number, y: number, z: number): string

  /** Delete one construction guide by handle string. */
  deleteGuide(id: string): void

  /** Delete all construction guides in one undo step. */
  deleteAllGuides(): void

  /** Handles of all currently visible construction guides, as decimal strings. */
  getGuideIds(): string[]

  /** `'line' | 'point'` for a guide handle, or undefined if stale/hidden. */
  getGuideKind(id: string): string | undefined

  /** A guide's geometry: `[ox,oy,oz, dx,dy,dz]` for a line (origin + unit
   * direction), `[x,y,z]` for a point; undefined if stale/hidden. */
  getGuideGeometry(id: string): number[] | undefined

  /**
   * Document-level undo (same kernel path as Cmd+Z). Calls `scene_undo()` and
   * reconciles the viewport. Throws if there is nothing to undo.
   */
  undo(): void

  /**
   * Document-level redo. Calls `scene_redo()` and reconciles the viewport.
   * Throws if there is nothing to redo.
   */
  redo(): void

  /** True if there is a document action to undo. */
  canUndo(): boolean

  /** True if there is a document action to redo. */
  canRedo(): boolean

  /**
   * Set the active display/parse length unit ( VCB). `format` is one
   * of `'m'|'cm'|'mm'|'arch'|'frac_in'|'dec_in'`. Writes through to
   * `setLengthUnit` (the same singleton the tools use) so subsequent
   * `formatLength` / `parseLength` calls reflect the new format.
   */
  setLengthUnit(format: string): void

  /** The currently active length format string (e.g. `'cm'`). */
  getLengthUnit(): string

  /**
   * Format `meters` (a kernel f64 length) using the current display unit, as
   * the VCB and status bar do. E.g. 1.5 m → "150 cm" when unit is cm.
   */
  formatLength(meters: number): string

  /**
   * Parse a typed length string to meters using the current unit ( VCB).
   * Returns `null` on empty/invalid input. E.g. "100 cm" → 1.0.
   */
  parseLength(input: string): number | null

  // -------- materials --------

  /**
   * Add a solid-color material to the palette and return its handle (decimal
   * string). `r`/`g`/`b`/`a` are 0–255; `a` < 255 is translucent. Wraps
   * `add_material`. Palette additions are not individually undoable — only
   * assignment (via `paintObject`/`paintFace`) is.
   */
  addMaterial(name: string, r: number, g: number, b: number, a: number): string

  /**
   * Set `object`'s base material — the color the whole solid (and faces grown
   * later by extrude/boolean) renders with, unless a face is explicitly painted.
   * Wraps `set_object_material`. Pass `null` to clear back to the renderer
   * default. Undoable.
   */
  paintObject(object: string, material: string | null): void

  /**
   * Paint a single `face` of `object` with `material`, overriding the object's
   * base material for that face. Wraps `paint_face`. Pass `null` for `material`
   * to reset the face to the (unpainted) default. Undoable.
   */
  paintFace(object: string, face: string, material: string | null): void

  // -------- components --------

  /**
   * Copy an instance to a sibling instance of the same definition, offset by
   * `(dx, dy, dz)` meters — `duplicate_node(2, id, affine)`, the instance arm
   * of MoveTool's Option-drag. Returns the new instance handle.
   */
  copyInstance(id: string, dx: number, dy: number, dz: number): string

  /**
   * Detach an instance onto its own private definition copy (Make Unique).
   * Returns the new definition handle. A set instance name is promoted to
   * the new definition's name; otherwise the copy is named "<def> Copy"
   * (disambiguated "<def> Copy 2", …).
   */
  makeUnique(instance: string): string

  /** Rename a node (object/group/instance); `null` clears. Undoable. */
  setNodeName(kind: string, id: string, name: string | null): void

  /** A node's own kernel name, or `null` if unnamed. */
  getNodeName(kind: string, id: string): string | null

  /** Add a tag path (root-first segments) to a node. Undoable. */
  addNodeTag(kind: string, id: string, path: string[]): void

  /** A node's tag paths, `/`-joined (e.g. `"Objects/Boxes"`). */
  getNodeTags(kind: string, id: string): string[]

  /** Rename a component definition; `null` clears. Undoable, renames every
   * instance's shared label. */
  setComponentName(component: string, name: string | null): void

  /** A component definition's name, or `null` if unnamed. */
  getComponentName(component: string): string | null

  /** The definition an instance places, or `null` if the handle is stale. */
  getInstanceDef(instance: string): string | null

  /** Every visible instance of a definition, as decimal handle strings. */
  getInstancesOf(component: string): string[]

  /** Replace the selection with arbitrary nodes (kind + handle string). */
  selectNodes(nodes: { kind: string; id: string }[]): void
  // -------- follow me --------

  /**
   * Rotate a free-standing sketch by `angleDeg` about the world `axis`
   * through `origin` — the same `transform_sketch` bake the Move/Rotate
   * tools' sketch branch commits. Lets a driver stand a ground-drawn
   * profile up perpendicular to a path before a `followMe*` call, exactly
   * as a user would with the Rotate tool.
   */
  rotateSketch(sketch: string, angleDeg: number, axis: Vec3, origin: Vec3): void

  /** Every edge handle of `sketch` (union of its islands' edges), as
   * decimal strings — the raw material for a `followMeAlongEdges` path. */
  getSketchEdgeIds(sketch: string): string[]

  /** Every live sketch handle, as decimal strings — lets a spec find the
   * sketch a TOOL created (the tool's cached handle is internal). */
  getSketchIds(): string[]

  /** All of `sketch`'s edges as flat endpoint pairs
   * `[ax,ay,az, bx,by,bz, ...]` — the geometry probe for asserting where a
   * sketch's lines actually stand (e.g. upright after a rotate). */
  getSketchLines(sketch: string): number[]

  /** `sketch`'s islands (connected components) with their member edges, as
   * decimal strings — the probe for asserting what one path click should
   * pick up (Follow Me grabs the clicked edge's whole island). */
  getSketchIslands(sketch: string): { island: string; edges: string[] }[]

  /** How many closed regions `sketch` currently holds — for asserting that
   * deleting a shared partition edge merges the two regions it divided. */
  getSketchRegionCount(sketch: string): number

  /**
   * Follow Me along sketch edges (the follow-me design): sweeps the
   * closed profile `region` of `sketch` along the chain the `edges` of
   * `pathSketch` form. Returns the new object handle. Equivalent to
   * FollowMeTool's edge-path commit (`follow_me_along_edges`). `group`
   * births the result inside that group (design §2f) instead of
   * top-level. `stopLen` (trailing, so existing `group` callers are
   * unaffected) is the partial-sweep arc length from the seam — NEGATIVE
   * sweeps `|stopLen|` the OTHER way around a closed loop (K2 — the same
   * value FollowMeTool's drag gesture computes and passes verbatim);
   * `undefined` sweeps the full path.
   */
  followMeAlongEdges(
    sketch: string,
    region: string,
    pathSketch: string,
    edges: string[],
    group?: string,
    stopLen?: number,
  ): string

  /**
   * Follow Me around a solid face's outer boundary loop (molding). Returns
   * the new object handle. Equivalent to FollowMeTool's face-path commit
   * (`follow_me_around_face`). `group` births the result inside that group
   * (design §2f) instead of top-level.
   */
  followMeAroundFace(sketch: string, region: string, object: string, face: string, group?: string): string

  /**
   * Follow Me around a face reached THROUGH a component instance (design
   * §2e): the definition face's loop rides the instance's pose into world
   * space. Equivalent to FollowMeTool's instanced-face-path commit
   * (`follow_me_around_instance_face`).
   */
  followMeAroundInstanceFace(
    sketch: string,
    region: string,
    instance: string,
    pathObject: string,
    pathFace: string,
  ): string

  /**
   * `followMeAroundFace` that MERGES the swept molding with the path's own
   * solid in one gesture and one undo step (design §3b) — Subtract when the
   * sweep overlaps the solid's interior, Union when it only rides the
   * surface, decided by the kernel itself. Equivalent to FollowMeTool's
   * Ctrl/Cmd-click commit (`follow_me_merged_around_face`).
   */
  followMeMergedAroundFace(sketch: string, region: string, pathObject: string, pathFace: string): string

  /**
   * Follow Me with a solid FACE as the profile (design §3a), along a chain
   * of sketch edges: the face's boundary (holes become tunnels) sweeps into
   * a NEW object; the source solid is untouched unless the profile face
   * belongs to the path's own solid, which auto-merges (design §3b) — see
   * `followMeFaceAroundFace`. Equivalent to FollowMeTool's face-profile
   * commit (`follow_me_face_along_edges`).
   */
  followMeFaceAlongEdges(
    profileObject: string,
    profileFace: string,
    pathSketch: string,
    edges: string[],
  ): string

  /**
   * Follow Me with a solid FACE as the profile around another face's outer
   * boundary loop — `followMeFaceAlongEdges`'s face-path sibling
   * (`follow_me_face_around_face`). Auto-merges with the path's own solid
   * when the profile face belongs to it (design §3b) — no separate call for
   * that case, the kernel decides from object identity alone.
   */
  followMeFaceAroundFace(
    profileObject: string,
    profileFace: string,
    pathObject: string,
    pathFace: string,
  ): string

  // -------- tags --------

  /**
   * Toggle a tag path's hidden flag through the app's real Tags-panel eye
   * path — session hidden set, renderer/kernel union push, and the
   * persisted `set_tag_hidden` flag together.
   */
  toggleTagHidden(path: string[]): void

  /**
   * Delete a tag everywhere through the app's real Tags-panel delete path
   * (undoable kernel `delete_tag` + tag-visibility resync).
   */
  deleteTag(path: string[]): void

  // -------- components & instances --------

  /**
   * Fold objects into a shared component definition plus an identity-posed
   * instance (Edit ▸ Make Component, without the selection side effect —
   * the placement renders batched, not materialized). Returns the instance
   * and definition handles as decimal strings.
   */
  makeComponent(objectIds: string[]): { instance: string; component: string }

  /**
   * Stamp another instance of `component`, translated by `(dx, dy, dz)`
   * meters. Returns the new instance handle as a decimal string.
   */
  placeInstance(component: string, dx: number, dy: number, dz: number): string

  // -------- camera --------

  /** Frame all visible geometry (View ▸ Zoom Extents). */
  zoomExtents(): void

  /** The camera's current pose — the read complement of `setCamera`, for
   * asserting framing (e.g. Zoom Extents re-targeting onto an instance). */
  getCamera(): { position: Vec3; target: Vec3; fovDeg: number }

  /** Project a world point to canvas-relative CSS pixels at the current
   * camera, for pointer-driven E2E that must target an on-screen widget (e.g.
   * grabbing a Scale-gizmo grip by its exact projected position instead of a
   * hard-coded pixel). `behind` true = the point is behind the camera. */
  worldToScreen(world: Vec3): { x: number; y: number; behind: boolean }
}

declare global {
  interface Window {
    __hew_test?: HewTestHarness
  }
}

/**
 * Install `window.__hew_test`. Returns an uninstall function (for HMR / unmount).
 * Caller gates on a debug/test build.
 */
export function installTestHarness(deps: HarnessDeps): () => void {
  let lastError: string | null = null

  const scene = (): Scene => {
    const s = deps.getScene()
    if (s === null) throw new Error('__hew_test: scene not ready')
    return s
  }

  // Run a mutation, reconcile + re-tessellate on success, and track the last
  // error. When the viewport is mounted we drive its `refreshScene` (which
  // re-tessellates the new geometry to the GPU *and* reconciles the app, exactly
  // like a tool commit) — a bare `reconcile()` updates React state but leaves the
  // canvas stale, so harness geometry would never render. Headless
  // callers with no viewport fall back to a plain reconcile.
  function act<T>(fn: (s: Scene) => T): T {
    try {
      const out = fn(scene())
      const api = deps.getViewportApi()
      if (api !== null) api.refreshScene()
      else deps.reconcile()
      lastError = null
      return out
    } catch (e) {
      lastError = e instanceof Error ? e.message : String(e)
      throw e
    }
  }
  function query<T>(fn: (s: Scene) => T): T {
    try {
      return fn(scene())
    } catch (e) {
      lastError = e instanceof Error ? e.message : String(e)
      throw e
    }
  }

  // The kernel's "no/clear material" sentinel is u64::MAX (see paint_face /
  // set_object_material in wasm-api). Map the harness's `null` to it.
  const MATERIAL_NONE = (1n << 64n) - 1n
  const materialHandle = (m: string | null): bigint => (m === null ? MATERIAL_NONE : BigInt(m))

  // Map a harness kind string to the numeric FFI kind tag; sketch kinds have
  // no kernel NodeId and must never reach a node_id-keyed wasm call.
  const kindNum = (kind: string): number => {
    const n = nodeKindToNumber(kind as NodeKind)
    if (n < 0) throw new Error(`__hew_test: '${kind}' has no kernel NodeId`)
    return n
  }

  // Add the four edges of the axis-aligned rectangle p0→p1 (on p0's z plane) to
  // `sketch`, returning the first closed region handle it forms. Bracketed in
  // one gesture (M-sketches-first-class) so a harness-drawn rectangle has the
  // same one-undo-step semantics as a tool-drawn one.
  function addRectangle(s: Scene, sketch: bigint, p0: Vec3, p1: Vec3): bigint {
    const z = p0[2]
    const corners: Vec3[] = [
      [p0[0], p0[1], z],
      [p1[0], p0[1], z],
      [p1[0], p1[1], z],
      [p0[0], p1[1], z],
    ]
    s.sketch_begin_gesture(sketch)
    try {
      for (let i = 0; i < 4; i++) {
        const a = corners[i]
        const b = corners[(i + 1) % 4]
        s.sketch_add_segment(sketch, a[0], a[1], a[2], b[0], b[1], b[2])
      }
    } finally {
      s.sketch_end_gesture(sketch)
    }
    const regions = s.sketch_regions(sketch)
    if (regions.length === 0) {
      throw new Error('drawRectangle: no closed region formed')
    }
    return regions[0]
  }

  const harness: HewTestHarness = {
    // Ready = kernel scene live AND the viewport API wired. Both are needed
    // before callers drive the harness: drawBox/setCamera/pickFace go through the
    // viewport (setCamera throws "viewport not ready" without it). The viewport
    // registers its API a tick after the scene becomes non-null, so gating only
    // on the scene leaves a race that a slower-mounting engine (webkit) loses —
    // `waitForFunction(isReady)` then `setCamera` flaked. Gate on both.
    isReady: () => deps.getScene() !== null && deps.getViewportApi() !== null,

    drawRectangle: (p0, p1) =>
      act((s) => {
        const sketch = s.begin_ground_sketch()
        const region = addRectangle(s, sketch, p0, p1)
        return { sketch: sketch.toString(), region: region.toString() }
      }),

    extrudeRegion: (sketch, region, distance) =>
      act((s) =>
        s.extrude_region(BigInt(sketch), BigInt(region), distance).toString(),
      ),

    drawBox: (p0, p1, height) =>
      act((s) => {
        const sketch = s.begin_ground_sketch()
        const region = addRectangle(s, sketch, p0, p1)
        return s.extrude_region(sketch, region, height).toString()
      }),

    pickFace: (o, d) =>
      query((s) => {
        // FacePickJs exposes object()/face()/instance() as methods; the
        // first two return bigint always, instance() is bigint|undefined.
        // pick_face returns undefined on a miss.
        const p = s.pick_face(o[0], o[1], o[2], d[0], d[1], d[2])
        if (!p) return null
        const instance = p.instance()
        return {
          object: p.object().toString(),
          face: p.face().toString(),
          instance: instance === undefined ? null : instance.toString(),
        }
      }),

    pushPull: (object, face, distance) => {
      act((s) => s.push_pull(BigInt(object), BigInt(face), distance))
    },

    boolean: (op, a, b) =>
      act((s) => s.boolean(op, BigInt(a), BigInt(b)).toString()),

    booleanNodes: (op, a, b) =>
      act((s) => {
        const kindNum = (k: string) => (k === 'group' ? 1 : k === 'instance' ? 2 : 0)
        const node = s.boolean_nodes(op, kindNum(a.kind), BigInt(a.id), kindNum(b.kind), BigInt(b.id))
        return { kind: node.kind, id: node.id.toString() }
      }),

    groupNodes: (nodes) =>
      act((s) => {
        const kinds = new Uint8Array(
          nodes.map((n) => (n.kind === 'group' ? 1 : n.kind === 'instance' ? 2 : 0)),
        )
        const ids = new BigUint64Array(nodes.map((n) => BigInt(n.id)))
        return s.group_nodes(kinds, ids).toString()
      }),

    getGroupMembers: (id) =>
      query((s) =>
        s.group_members(BigInt(id)).map((n) => ({ kind: n.kind, id: n.id.toString() })),
      ),

    isObjectSolid: (id) => query((s) => s.object_solid(BigInt(id))),

    getObjectBounds: (id) =>
      query((s) => {
        const mesh = s.object_mesh(BigInt(id))
        try {
          const pos = mesh.positions()
          if (pos.length < 3) {
            throw new Error(`getObjectBounds: object ${id} has no geometry`)
          }
          let minX = pos[0], maxX = pos[0]
          let minY = pos[1], maxY = pos[1]
          let minZ = pos[2], maxZ = pos[2]
          for (let i = 3; i < pos.length; i += 3) {
            const x = pos[i], y = pos[i + 1], z = pos[i + 2]
            if (x < minX) minX = x
            if (x > maxX) maxX = x
            if (y < minY) minY = y
            if (y > maxY) maxY = y
            if (z < minZ) minZ = z
            if (z > maxZ) maxZ = z
          }
          return [minX, minY, minZ, maxX, maxY, maxZ]
        } finally {
          mesh.free()
        }
      }),

    copyNode: (kind, id, dx, dy, dz) => {
      const affine = new Float64Array([1, 0, 0, dx, 0, 1, 0, dy, 0, 0, 1, dz])
      const kindNum = kind === 'group' ? 1 : kind === 'instance' ? 2 : 0
      return act((s) => {
        const node = s.duplicate_node(kindNum, BigInt(id), affine)
        return { kind: node.kind, id: node.id.toString() }
      })
    },

    deleteObject: (id) => {
      act((s) => s.delete_node(0, BigInt(id))) // kind 0 = object
    },

    selectObjects: (ids) => deps.setSelectedObjects(ids.map((id) => BigInt(id))),

    selectAll: () => {
      const api = deps.getViewportApi()
      if (api === null) throw new Error('__hew_test: viewport not ready')
      api.selectAll()
    },

    setCamera: (pose) => {
      const api = deps.getViewportApi()
      if (api === null) throw new Error('__hew_test: viewport not ready')
      api.setCamera(pose.position, pose.target, pose.up ?? [0, 0, 1], pose.fovDeg ?? 45)
    },

    frameStability: (poseA, poseB) => {
      const api = deps.getViewportApi()
      if (api === null) throw new Error('__hew_test: viewport not ready')
      const capture = (pose: CameraPose) => {
        api.setCamera(pose.position, pose.target, pose.up ?? [0, 0, 1], pose.fovDeg ?? 45)
        return api.captureFrame()
      }
      const a = capture(poseA)
      const b = capture(poseB)
      if (a.width !== b.width || a.height !== b.height) {
        throw new Error('__hew_test: frame size changed between captures')
      }
      let differing = 0
      let hard = 0
      for (let p = 0; p < a.pixels.length; p += 4) {
        const d = Math.max(
          Math.abs(a.pixels[p] - b.pixels[p]),
          Math.abs(a.pixels[p + 1] - b.pixels[p + 1]),
          Math.abs(a.pixels[p + 2] - b.pixels[p + 2]),
        )
        if (d > 8) differing++
        if (d > 60) hard++
      }
      return { width: a.width, height: a.height, differing, hard }
    },

    setAxesVisible: (visible) => {
      const api = deps.getViewportApi()
      if (api === null) throw new Error('__hew_test: viewport not ready')
      api.setAxesVisible(visible)
    },

    setGridVisible: (visible) => {
      const api = deps.getViewportApi()
      if (api === null) throw new Error('__hew_test: viewport not ready')
      api.setGridVisible(visible)
    },

    setGuidesVisible: (visible) => {
      const api = deps.getViewportApi()
      if (api === null) throw new Error('__hew_test: viewport not ready')
      api.setGuidesVisible(visible)
    },

    toggleSectionActive: () => {
      const api = deps.getViewportApi()
      if (api === null) throw new Error('__hew_test: viewport not ready')
      api.toggleSectionActive()
    },
    getSectionState: () => {
      const api = deps.getViewportApi()
      if (api === null) throw new Error('__hew_test: viewport not ready')
      return api.getSectionState()
    },
    getSectionRenderInfo: (kind, id) => {
      const api = deps.getViewportApi()
      if (api === null) throw new Error('__hew_test: viewport not ready')
      return api.getSectionRenderInfo(kind, BigInt(id))
    },

    replay: (json) => act((s) => s.replay(json).toString()),

    save: () => query((s) => Array.from(s.save())),
    load: (bytes) => {
      // Route through the app's real Open path, not a bare `scene.load`, so the
      // viewport re-tessellates exactly as File→Open does. `loadBytes` reports
      // failure via its boolean (it has already toasted); surface it as both a
      // lastError and a throw, matching the `act` helper's contract.
      const ok = deps.loadBytes(new Uint8Array(bytes))
      if (!ok) {
        lastError = '__hew_test: load rejected'
        throw new Error(lastError)
      }
      lastError = null
    },

    exportStl: (segmentsPerTurn) =>
      query((s) => {
        // The wasm `Scene` structurally satisfies StlExportScene (object_ids /
        // instance_ids / instance_def / instance_pose / object_export_triangles).
        const out = exportSceneToStl(s as unknown as StlExportScene, segmentsPerTurn)
        return out === null
          ? null
          : { bytes: Array.from(out.bytes), triangleCount: out.triangleCount }
      }),

    importStl: (bytes, unitScale) =>
      act((s) => s.import_stl(new Uint8Array(bytes), unitScale) as StlImportReportJs),

    startRecording: () => {
      scene().start_recording()
      inputRecorder.start()
    },
    stopRecording: () => {
      scene().stop_recording()
      inputRecorder.stop()
    },
    isRecording: () => scene().is_recording(),
    takeRecording: () =>
      buildSessionRecording(scene().take_recording(), inputRecorder.take()),

    getStateHash: () => query((s) => s.state_hash().toString()),
    getObjectCount: () => query((s) => s.object_ids().length),
    getObjectIds: () => query((s) => Array.from(s.object_ids()).map(String)),
    getSelection: () =>
      deps.getSelection().map((n) => ({ kind: n.kind, id: n.id.toString() })),
    getLastError: () => lastError,

    // -------- NEW in  --------

    drawLineChain: (points) =>
      act((s) => {
        if (points.length < 2) throw new Error('drawLineChain: need at least 2 points')
        const sketch = s.begin_ground_sketch()
        const regionsAll = new Set<bigint>()
        s.sketch_begin_gesture(sketch)
        try {
          for (let i = 0; i < points.length - 1; i++) {
            const a = points[i]
            const b = points[i + 1]
            const added = s.sketch_add_segment(sketch, a[0], a[1], a[2], b[0], b[1], b[2])
            for (const r of added.regions_created()) regionsAll.add(r)
          }
        } finally {
          s.sketch_end_gesture(sketch)
        }
        return {
          sketch: sketch.toString(),
          regions: Array.from(regionsAll).map(String),
        }
      }),

    drawCircle: (center, radius, nSegments = 24) =>
      act((s) => {
        if (radius <= 0) throw new Error('drawCircle: radius must be positive')
        const sketch = s.begin_ground_sketch()
        const pts: Vec3[] = []
        for (let i = 0; i < nSegments; i++) {
          const theta = (2 * Math.PI * i) / nSegments
          pts.push([center[0] + radius * Math.cos(theta), center[1] + radius * Math.sin(theta), center[2]])
        }
        let lastRegion: bigint | null = null
        s.sketch_begin_gesture(sketch)
        try {
          // One curve chain carrying the analytic circle, exactly as
          // CircleTool commits (the true-curves design).
          s.sketch_begin_curve_with(sketch, center[0], center[1], center[2], radius)
          for (let i = 0; i < nSegments; i++) {
            const a = pts[i]
            const b = pts[(i + 1) % nSegments]
            const added = s.sketch_add_segment(sketch, a[0], a[1], a[2], b[0], b[1], b[2])
            for (const r of added.regions_created()) lastRegion = r
          }
          s.sketch_end_curve(sketch)
        } finally {
          s.sketch_end_gesture(sketch)
        }
        if (lastRegion === null) {
          // Fallback: ask the scene for any open regions
          const regions = s.sketch_regions(sketch)
          lastRegion = regions.length > 0 ? regions[0] : null
        }
        if (lastRegion === null) throw new Error('drawCircle: no region formed')
        return { sketch: sketch.toString(), region: lastRegion.toString() }
      }),

    imprintCircleOnFace: (object, face, center, radius, nSegments = 24) =>
      act((s) => {
        if (radius <= 0) throw new Error('imprintCircleOnFace: radius must be positive')
        const nrm = s.face_normal(BigInt(object), BigInt(face))
        const normal: V3 = [nrm[0], nrm[1], nrm[2]]
        const basis = facePlaneBasis(normal)
        if (basis === null) throw new Error('imprintCircleOnFace: degenerate face normal')
        const { u, v } = basis
        const loop = new Float64Array(nSegments * 3)
        for (let i = 0; i < nSegments; i++) {
          const theta = (2 * Math.PI * i) / nSegments
          const cs = Math.cos(theta) * radius
          const sn = Math.sin(theta) * radius
          loop[i * 3 + 0] = center[0] + u[0] * cs + v[0] * sn
          loop[i * 3 + 1] = center[1] + u[1] * cs + v[1] * sn
          loop[i * 3 + 2] = center[2] + u[2] * cs + v[2] * sn
        }
        const sub = s.split_face_inner_with_curve(
          BigInt(object),
          BigInt(face),
          loop,
          new Float64Array([center[0], center[1], center[2]]),
          radius,
        )
        return sub.toString()
      }),

    offsetRegion: (sketch, region, distance) =>
      act((s) => {
        const sid = BigInt(sketch)
        s.sketch_begin_gesture(sid)
        try {
          const report = s.sketch_offset_region(sid, BigInt(region), distance)
          try {
            return Array.from(report.regions_created(), (r) => r.toString())
          } finally {
            report.free()
          }
        } finally {
          s.sketch_end_gesture(sid)
        }
      }),

    offsetFace: (object, face, distance) =>
      act((s) => s.offset_face(BigInt(object), BigInt(face), distance).toString()),

    drawArc: (a, b, sagitta, close = false) =>
      act((s) => {
        const pts = arcPolylineOnPlane(a, b, sagitta, [1, 0, 0], [0, 1, 0])
        if (pts === null) throw new Error('drawArc: degenerate chord or flat sagitta')
        const arc = arcFromChord([a[0], a[1]], [b[0], b[1]], sagitta)
        const sketch = s.begin_ground_sketch()
        const regionsAll = new Set<bigint>()
        const addSeg = (p: V3, q: V3): void => {
          const added = s.sketch_add_segment(sketch, p[0], p[1], p[2], q[0], q[1], q[2])
          for (const r of added.regions_created()) regionsAll.add(r)
        }
        // The arc's facets are one curve chain carrying the analytic circle,
        // exactly as ArcTool commits; the closing chord stays a plain line.
        if (arc !== null) {
          s.sketch_begin_curve_with(sketch, arc.center[0], arc.center[1], 0, arc.radius)
        }
        for (let i = 0; i < pts.length - 1; i++) addSeg(pts[i], pts[i + 1])
        s.sketch_end_curve(sketch)
        if (close) addSeg(pts[pts.length - 1], pts[0])
        return {
          sketch: sketch.toString(),
          regions: Array.from(regionsAll).map(String),
        }
      }),

    drawArcOnFace: (object, face, a, b, sagitta) => {
      act((s) => {
        const objId = BigInt(object)
        const faceId = BigInt(face)
        const n = s.face_normal(objId, faceId)
        const basis = facePlaneBasis([n[0], n[1], n[2]])
        if (basis === null) throw new Error('drawArcOnFace: degenerate face normal')
        const pts = arcPolylineOnPlane(a, b, sagitta, basis.u, basis.v)
        if (pts === null) throw new Error('drawArcOnFace: degenerate chord or flat sagitta')
        const path = new Float64Array(pts.length * 3)
        for (let i = 0; i < pts.length; i++) {
          path[i * 3 + 0] = pts[i][0]
          path[i * 3 + 1] = pts[i][1]
          path[i * 3 + 2] = pts[i][2]
        }
        s.split_face(objId, faceId, path).free()
      })
    },

    moveObject: (object, dx, dy, dz) => {
      // Pure-translation 3×4 row-major affine: identity rotation + (dx,dy,dz) column.
      const affine = new Float64Array([1, 0, 0, dx, 0, 1, 0, dy, 0, 0, 1, dz])
      act((s) => s.transform_object(BigInt(object), affine))
    },

    copyObject: (id, dx, dy, dz) => {
      const affine = new Float64Array([1, 0, 0, dx, 0, 1, 0, dy, 0, 0, 1, dz])
      return act((s) => {
        const node = s.duplicate_node(0, BigInt(id), affine)
        return { kind: node.kind, id: node.id.toString() }
      })
    },

    rotateObject: (object, angleDeg, axis = [0, 0, 1]) => {
      // Rodrigues rotation matrix around a normalized axis by angleDeg degrees.
      const [ux, uy, uz] = axis
      const len = Math.hypot(ux, uy, uz)
      if (len < 1e-12) throw new Error('rotateObject: axis must be non-zero')
      const ax = ux / len, ay = uy / len, az = uz / len
      const theta = (angleDeg * Math.PI) / 180
      const c = Math.cos(theta), s_ = Math.sin(theta), t = 1 - c
      // Row-major 3×4 affine; translation column is zero.
      const affine = new Float64Array([
        t * ax * ax + c,       t * ax * ay - s_ * az, t * ax * az + s_ * ay, 0,
        t * ay * ax + s_ * az, t * ay * ay + c,       t * ay * az - s_ * ax, 0,
        t * az * ax - s_ * ay, t * az * ay + s_ * ax, t * az * az + c,       0,
      ])
      act((s) => s.transform_object(BigInt(object), affine))
    },

    scaleObject: (object, sx, sy, sz, pivot) => {
      // diag(sx,sy,sz) about `pivot`: translation = pivot componentwise
      // (1 - s) — matches `nonUniformScaleAboutPivot` in tools/transformMath.ts.
      const [px, py, pz] = pivot
      const affine = new Float64Array([
        sx, 0, 0, px * (1 - sx),
        0, sy, 0, py * (1 - sy),
        0, 0, sz, pz * (1 - sz),
      ])
      act((s) => s.transform_object(BigInt(object), affine))
    },

    sliceObject: (object, plane) =>
      act((s) => {
        const ids = s.slice_object(BigInt(object), new Float64Array(plane))
        return [ids[0].toString(), ids[1].toString()] as [string, string]
      }),

    addGuideLine: (ox, oy, oz, dx, dy, dz) =>
      act((s) => s.add_guide_line(ox, oy, oz, dx, dy, dz).toString()),

    addGuidePoint: (x, y, z) =>
      act((s) => s.add_guide_point(x, y, z).toString()),

    deleteGuide: (id) => {
      act((s) => s.delete_guide(BigInt(id)))
    },

    deleteAllGuides: () => {
      act((s) => s.delete_all_guides())
    },

    getGuideIds: () => query((s) => Array.from(s.guide_ids()).map(String)),

    getGuideKind: (id) => query((s) => s.guide_kind(BigInt(id))),

    getGuideGeometry: (id) =>
      query((s) => {
        const g = s.guide_geometry(BigInt(id))
        return g !== undefined ? Array.from(g) : undefined
      }),

    // Undo/redo route through the viewport's runUndo/runRedo — the shared
    // choke point that fires post-history reconciliation (onHistoryChanged →
    // tag visibility + pick/inference exclusion) — so a harness-driven undo
    // behaves exactly like the menu, palette, and keyboard paths. runUndo is
    // deliberately a silent no-op when there is nothing to undo, so the
    // harness's documented throw-on-nothing contract is checked up front.
    // Headless callers (no viewport) fall back to the bare kernel call.
    undo: () => {
      const api = deps.getViewportApi()
      if (api === null) {
        act((s) => s.scene_undo().free())
        return
      }
      if (!scene().can_scene_undo()) throw new Error('__hew_test: nothing to undo')
      api.runUndo()
    },

    redo: () => {
      const api = deps.getViewportApi()
      if (api === null) {
        act((s) => s.scene_redo().free())
        return
      }
      if (!scene().can_scene_redo()) throw new Error('__hew_test: nothing to redo')
      api.runRedo()
    },

    canUndo: () => query((s) => s.can_scene_undo()),
    canRedo: () => query((s) => s.can_scene_redo()),

    setLengthUnit: (format) => setLengthUnit(format as LengthFormat),
    getLengthUnit: () => getLengthUnit(),
    formatLength: (meters) => formatLength(meters),
    parseLength: (input) => parseLengthToMeters(input),

    // -------- materials --------

    addMaterial: (name, r, g, b, a) =>
      act((s) => s.add_material(name, r, g, b, a).toString()),

    paintObject: (object, material) => {
      act((s) => s.set_object_material(BigInt(object), materialHandle(material)))
    },

    paintFace: (object, face, material) => {
      act((s) => s.paint_face(BigInt(object), BigInt(face), materialHandle(material)))
    },

    // -------- follow me --------

    rotateSketch: (sketch, angleDeg, axis, origin) => {
      // Rodrigues rotation about `axis` through `origin`:
      // affine = T(origin) · R · T(−origin), i.e. translation = origin − R·origin.
      const [ux, uy, uz] = axis
      const len = Math.hypot(ux, uy, uz)
      if (len < 1e-12) throw new Error('rotateSketch: axis must be non-zero')
      const ax = ux / len, ay = uy / len, az = uz / len
      const theta = (angleDeg * Math.PI) / 180
      const c = Math.cos(theta), s_ = Math.sin(theta), t = 1 - c
      const r = [
        t * ax * ax + c,       t * ax * ay - s_ * az, t * ax * az + s_ * ay,
        t * ay * ax + s_ * az, t * ay * ay + c,       t * ay * az - s_ * ax,
        t * az * ax - s_ * ay, t * az * ay + s_ * ax, t * az * az + c,
      ]
      const [px, py, pz] = origin
      const tx = px - (r[0] * px + r[1] * py + r[2] * pz)
      const ty = py - (r[3] * px + r[4] * py + r[5] * pz)
      const tz = pz - (r[6] * px + r[7] * py + r[8] * pz)
      const affine = new Float64Array([
        r[0], r[1], r[2], tx,
        r[3], r[4], r[5], ty,
        r[6], r[7], r[8], tz,
      ])
      act((s) => s.transform_sketch(BigInt(sketch), affine))
    },

    getSketchIds: () => query((s) => Array.from(s.sketch_ids(), (id) => id.toString())),

    getSketchLines: (sketch) => query((s) => Array.from(s.sketch_lines(BigInt(sketch)))),

    getSketchEdgeIds: (sketch) =>
      query((s) => {
        const out: string[] = []
        for (const island of s.sketch_island_ids(BigInt(sketch))) {
          for (const edge of s.sketch_island_edges(BigInt(sketch), island)) {
            out.push(edge.toString())
          }
        }
        return out
      }),

    getSketchIslands: (sketch) =>
      query((s) =>
        Array.from(s.sketch_island_ids(BigInt(sketch)), (island) => ({
          island: island.toString(),
          edges: Array.from(s.sketch_island_edges(BigInt(sketch), island), (e) => e.toString()),
        })),
      ),

    getSketchRegionCount: (sketch) => query((s) => s.sketch_regions(BigInt(sketch)).length),

    followMeAlongEdges: (sketch, region, pathSketch, edges, group, stopLen) =>
      act((s) =>
        s
          .follow_me_along_edges(
            BigInt(sketch),
            BigInt(region),
            BigInt(pathSketch),
            new BigUint64Array(edges.map((e) => BigInt(e))),
            stopLen,
            group === undefined ? undefined : BigInt(group),
          )
          .toString(),
      ),

    followMeAroundFace: (sketch, region, object, face, group) =>
      act((s) =>
        s
          .follow_me_around_face(
            BigInt(sketch),
            BigInt(region),
            BigInt(object),
            BigInt(face),
            undefined,
            group === undefined ? undefined : BigInt(group),
          )
          .toString(),
      ),

    followMeAroundInstanceFace: (sketch, region, instance, pathObject, pathFace) =>
      act((s) =>
        s
          .follow_me_around_instance_face(
            BigInt(sketch),
            BigInt(region),
            BigInt(instance),
            BigInt(pathObject),
            BigInt(pathFace),
          )
          .toString(),
      ),

    followMeMergedAroundFace: (sketch, region, pathObject, pathFace) =>
      act((s) =>
        s
          .follow_me_merged_around_face(BigInt(sketch), BigInt(region), BigInt(pathObject), BigInt(pathFace))
          .toString(),
      ),

    followMeFaceAlongEdges: (profileObject, profileFace, pathSketch, edges) =>
      act((s) =>
        s
          .follow_me_face_along_edges(
            BigInt(profileObject),
            BigInt(profileFace),
            BigInt(pathSketch),
            new BigUint64Array(edges.map((e) => BigInt(e))),
          )
          .toString(),
      ),

    followMeFaceAroundFace: (profileObject, profileFace, pathObject, pathFace) =>
      act((s) =>
        s
          .follow_me_face_around_face(
            BigInt(profileObject),
            BigInt(profileFace),
            BigInt(pathObject),
            BigInt(pathFace),
          )
          .toString(),
      ),

    // -------- components --------

    copyInstance: (id, dx, dy, dz) => {
      const affine = new Float64Array([1, 0, 0, dx, 0, 1, 0, dy, 0, 0, 1, dz])
      return act((s) => s.duplicate_node(2, BigInt(id), affine).id.toString())
    },

    makeUnique: (instance) =>
      act((s) => s.make_unique(BigInt(instance)).toString()),

    setNodeName: (kind, id, name) => {
      act((s) => s.set_node_name(kindNum(kind), BigInt(id), name ?? undefined))
    },

    getNodeName: (kind, id) =>
      query((s) => {
        const i = BigInt(id)
        const name =
          kind === 'object' ? s.object_name(i) :
          kind === 'group' ? s.group_name(i) :
          s.instance_name(i)
        return name ?? null
      }),

    addNodeTag: (kind, id, path) => {
      act((s) => s.add_node_tag(kindNum(kind), BigInt(id), path))
    },

    getNodeTags: (kind, id) =>
      query((s) => Array.from(s.node_tags(kindNum(kind), BigInt(id)))),

    setComponentName: (component, name) => {
      act((s) => s.set_component_name(BigInt(component), name ?? undefined))
    },

    getComponentName: (component) =>
      query((s) => s.component_name(BigInt(component)) ?? null),

    getInstanceDef: (instance) =>
      query((s) => s.instance_def(BigInt(instance))?.toString() ?? null),

    getInstancesOf: (component) =>
      query((s) => Array.from(s.instances_of(BigInt(component))).map(String)),

    selectNodes: (nodes) =>
      deps.setSelection(
        nodes.map((n) => ({ kind: n.kind as NodeKind, id: BigInt(n.id) })),
      ),

    // -------- tags --------

    toggleTagHidden: (path) => deps.toggleTagPath(path),

    deleteTag: (path) => deps.deleteTag(path),

    // -------- components & instances --------

    makeComponent: (objectIds) =>
      act((s) => {
        const kinds = new Uint8Array(objectIds.length) // all zeros = object
        const ids = new BigUint64Array(objectIds.map((id) => BigInt(id)))
        const instance = s.make_component(kinds, ids)
        const component = s.instance_def(instance)
        if (component === undefined) throw new Error('makeComponent: instance has no definition')
        return { instance: instance.toString(), component: component.toString() }
      }),

    placeInstance: (component, dx, dy, dz) =>
      act((s) => {
        const affine = new Float64Array([1, 0, 0, dx, 0, 1, 0, dy, 0, 0, 1, dz])
        return s.place_instance(BigInt(component), affine).toString()
      }),

    // -------- camera --------

    zoomExtents: () => {
      const api = deps.getViewportApi()
      if (api === null) throw new Error('__hew_test: viewport not ready')
      api.zoomExtents()
    },

    getCamera: () => {
      const api = deps.getViewportApi()
      if (api === null) throw new Error('__hew_test: viewport not ready')
      return api.getCamera()
    },

    worldToScreen: (world) => {
      const api = deps.getViewportApi()
      if (api === null) throw new Error('__hew_test: viewport not ready')
      return api.worldToScreen(world)
    },
  }

  window.__hew_test = harness
  return () => {
    if (window.__hew_test === harness) delete window.__hew_test
  }
}
