// GENERATED from crates/api registry — do not edit; regenerate with:
//   REGENERATE_API_ARTIFACTS=1 cargo test -p api --test generate_artifacts
//
// A typed client over a caller-supplied transport for the Hew API
// (docs/HEW_API.md — the normative reference; §9 is this file's
// contract). Every `Params`/`Result` pair and every method below is
// derived mechanically from the command registry in `crates/api`,
// which is their single source of truth — this file has no
// hand-maintained copy of a command's shape.

/** One JSON-RPC 2.0 request frame (docs/HEW_API.md §4.1). `params` is
 * always a single object, never positional. */
export interface JsonRpcRequest {
  jsonrpc: '2.0'
  id: number | string
  method: string
  params?: unknown
}

/** One JSON-RPC 2.0 response frame: exactly one of `result` / `error`
 * is present (§4.1). */
export interface JsonRpcResponse {
  jsonrpc: '2.0'
  id: number | string | null
  result?: unknown
  error?: JsonRpcErrorObject
}

/** A JSON-RPC 2.0 error object (§4.4). */
export interface JsonRpcErrorObject {
  code: number
  message: string
  data?: unknown
}

/** The transport this client dispatches every envelope through — an
 * in-process call, the desktop app's local socket, or `hew-cli`'s
 * stdio MCP adapter all satisfy this one shape (docs/HEW_API.md
 * §11). The client owns request framing (`id` assignment) and
 * result/error unwrapping; the transport owns only the wire. */
export interface HewTransport {
  dispatch(request: JsonRpcRequest): Promise<JsonRpcResponse>
}

/** The protocol's error-code inventory (§4.4). Additive only (§9). */
export const HewErrorCode = {
  PARSE_ERROR: -32700,
  METHOD_NOT_FOUND: -32601,
  INVALID_PARAMS: -32602,
  REFUSED: -32000,
  NOT_PERMITTED: -32001,
  NO_DOCUMENT: -32002,
  INTERNAL_FAULT: -32003,
  NOT_READY: -32004,
} as const

export type HewErrorCode = (typeof HewErrorCode)[keyof typeof HewErrorCode]

/** The canonical `error.data` shape of a refusal (`code ===
 * HewErrorCode.REFUSED`) — §4.4: always all five fields, whether the
 * envelope was a transaction or a plain request. */
export interface HewRefusal {
  refusal: string
  failed_index: number
  failed_method: string
  detail: unknown
  explanation: string
}

/** Thrown by every generated method when the dispatcher answers an
 * error frame. `refusal` is populated (typed as {@link HewRefusal})
 * exactly when `code === HewErrorCode.REFUSED`; other codes are
 * protocol errors or internal faults (§4.4) and carry `data` as-is,
 * if any. */
export class HewApiError extends Error {
  readonly code: number
  readonly data: unknown
  readonly refusal: HewRefusal | undefined

  constructor(error: JsonRpcErrorObject) {
    super(error.message)
    this.name = 'HewApiError'
    this.code = error.code
    this.data = error.data
    this.refusal = error.code === HewErrorCode.REFUSED ? (error.data as HewRefusal) : undefined
  }
}

/** A command whose registry schema is still the scaffold placeholder
 * (an untightened `{"type": "object"}` — docs/HEW_API.md §14's
 * burn-down posture) falls back to this rather than a fabricated
 * shape. */
export type UnspecifiedShape = Record<string, unknown>

/**
 * `hew.attr.delete` (v1) — Delete one attribute key or a whole namespace.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: unknown_entity, invalid_attr_name, reserved_attr_namespace, unknown_attr, unknown_object, unknown_group, unknown_instance, unknown_sketch, unknown_guide, unknown_material, unknown_component, unknown_tag
 */
export interface AttrDeleteParams {
  key?: string
  ns: string
  target: string
}

export interface AttrDeleteResult {}

/**
 * `hew.attr.get` (v1) — Read a target's attribute dictionaries.
 * Tier: Required · Class: read-only · Served: kernel
 * Refusals: unknown_entity, unknown_object, unknown_group, unknown_instance, unknown_sketch, unknown_guide, unknown_material, unknown_component, unknown_tag
 */
export interface AttrGetParams {
  ns?: string
  target: string
}

export type AttrGetResult = UnspecifiedShape

/**
 * `hew.attr.set` (v1) — Write one attribute key.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: unknown_entity, invalid_attr_name, reserved_attr_namespace, non_finite_attr_value, attr_value_too_deep, unrepresentable_attr_value, unknown_object, unknown_group, unknown_instance, unknown_sketch, unknown_guide, unknown_material, unknown_component, unknown_tag
 */
export interface AttrSetParams {
  key: string
  ns: string
  target: string
  value: UnspecifiedShape
}

export interface AttrSetResult {}

/**
 * `hew.component.create` (v1) — Fold a selection into a definition plus one instance.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_entity, empty_component, duplicate_member, nested_component_unsupported, unknown_object, unknown_group, unknown_instance
 */
export interface ComponentCreateParams {
  members: string[]
}

export interface ComponentCreateResult {
  component: string
  instance: string
}

/**
 * `hew.component.explode` (v1) — Bake an instance into world geometry.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_entity, unknown_instance, cannot_explode_reflected
 */
export interface ComponentExplodeParams {
  instance: string
}

export interface ComponentExplodeResult {
  objects: string[]
}

/**
 * `hew.component.make_unique` (v1) — Deep-copy an instance's definition into a private one.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_entity, unknown_instance, unknown_component
 */
export interface ComponentMakeUniqueParams {
  instance: string
}

export interface ComponentMakeUniqueResult {
  component: string
}

/**
 * `hew.component.place` (v1) — Place an instance of a definition at a pose.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_entity, unknown_component, singular
 */
export interface ComponentPlaceParams {
  component: string
  pose?: UnspecifiedShape
}

export interface ComponentPlaceResult {
  instance: string
}

/**
 * `hew.context.enter` (v1) — Open a group/component editing frame (transaction-balanced only).
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: unknown_entity, explode_session_open, explode_session_nested_group, explode_session_pose_unsupported, explode_session_grouped_instance, unknown_group, unknown_instance
 */
export interface ContextEnterParams {
  id: string
}

export interface ContextEnterResult {}

/**
 * `hew.context.exit` (v1) — Close the innermost frame this envelope opened.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: explode_session_not_open
 */
export interface ContextExitParams {}

export interface ContextExitResult {}

/**
 * `hew.doc.attach` (v1) — Bind this connection to one open document.
 * Tier: Required · Class: solitary · Served: kernel
 * Refusals: none.
 */
export interface DocAttachParams {
  document?: string
}

export type DocAttachResult = UnspecifiedShape

/**
 * `hew.doc.export` (v1) — Export the attached document — STL, 3MF, glTF/GLB, or USDZ — solids only, bytes base64, or a path on hosts with filesystem access.
 * Tier: Required · Class: solitary · Served: host
 * Refusals: export_failed, host_capability_missing, nothing_to_export, save_failed
 */
export interface DocExportParams {
  /** "gltf" is an alias for "glb" — every host that implements one implements both */
  format: "stl" | "3mf" | "glb" | "gltf" | "usdz"
  path?: string
  segments_per_turn?: number
}

export interface DocExportResult {
  bytes_base64?: string
  format: string
}

/**
 * `hew.doc.import` (v1) — Merge a foreign-format file into the attached document through the shared healing pipeline.
 * Tier: Standard · Class: solitary · Served: host
 * Refusals: host_capability_missing, units_required, load_failed, unsupported_format
 */
export interface DocImportParams {
  path: string
  units?: "m" | "mm" | "cm" | "in"
}

export interface DocImportResult {
  report: UnspecifiedShape
}

/**
 * `hew.doc.new` (v1) — Create a fresh document (headless hosts; live hosts advertise via capabilities).
 * Tier: Required · Class: solitary · Served: host
 * Refusals: host_capability_missing
 */
export interface DocNewParams {}

export interface DocNewResult {}

/**
 * `hew.doc.open` (v1) — Open a .hew document (headless hosts; live hosts advertise via capabilities).
 * Tier: Required · Class: solitary · Served: host
 * Refusals: host_capability_missing, load_failed
 */
export interface DocOpenParams {
  path: string
}

export interface DocOpenResult {}

/**
 * `hew.doc.save` (v1) — Save the attached document — written by hosts with filesystem access, bytes base64 by those without.
 * Tier: Required · Class: solitary · Served: host
 * Refusals: host_capability_missing, path_required, save_failed
 */
export interface DocSaveParams {
  path?: string
}

export interface DocSaveResult {
  bytes_base64?: string
}

/**
 * `hew.doc.transact` (v1) — Execute commands in order, atomically, as one labeled undo entry.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: ref_resolution_failed
 */
export interface DocTransactParams {
  commands: { as?: string; method: string; params?: UnspecifiedShape }[]
  label?: string
}

export interface DocTransactResult {
  label: string
  results: unknown[]
}

/**
 * `hew.entity.delete` (v1) — Delete an entity.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: unknown_entity, delete_unsupported, unknown_object, unknown_group, unknown_instance, unknown_sketch, unknown_guide, unknown_edge
 */
export interface EntityDeleteParams {
  /** any public id; a sketch edge id ("edg_…") erases just that one edge — the eraser's own kernel path (Sketch::remove_edge) — as one undo entry, rather than the whole sketch */
  id: string
}

export interface EntityDeleteResult {}

/**
 * `hew.entity.move` (v1) — Translate (with copy/array forms) by vector or from→to points.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: unknown_entity, mixed_selection_unsupported, sketch_copy_unsupported, array_count_too_large, empty_selection, duplicate_member, unknown_object, unknown_group, unknown_instance, unknown_sketch
 */
export interface EntityMoveParams {
  copy?: { count?: number }
  from?: UnspecifiedShape
  ids: string[]
  to?: UnspecifiedShape
  translation?: [number, number, number]
}

export interface EntityMoveResult {
  ids?: string[]
}

/**
 * `hew.entity.rename` (v1) — Rename an entity.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: unknown_entity, rename_unsupported, unknown_object, unknown_group, unknown_instance, unknown_component
 */
export interface EntityRenameParams {
  id: string
  name?: string | null
}

export interface EntityRenameResult {}

/**
 * `hew.entity.rotate` (v1) — Rotate about a pivot and axis by an angle.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: unknown_entity, mixed_selection_unsupported, sketch_copy_unsupported, array_count_too_large, empty_selection, duplicate_member, unknown_object, unknown_group, unknown_instance, unknown_sketch
 */
export interface EntityRotateParams {
  angle: number
  axis: [number, number, number]
  copy?: { count?: number }
  ids: string[]
  pivot: UnspecifiedShape
}

export interface EntityRotateResult {
  ids?: string[]
}

/**
 * `hew.entity.scale` (v1) — Scale about an anchor with per-axis factors.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: unknown_entity, mixed_selection_unsupported, empty_selection, unknown_object, unknown_group, unknown_instance, unknown_sketch
 */
export interface EntityScaleParams {
  anchor: UnspecifiedShape
  factors: [number, number, number]
  ids: string[]
}

export interface EntityScaleResult {}

/**
 * `hew.group.create` (v1) — Group sibling nodes non-destructively.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: unknown_entity, empty_group, duplicate_member, mixed_parents, unknown_object, unknown_group, unknown_instance
 */
export interface GroupCreateParams {
  members: string[]
}

export interface GroupCreateResult {
  group: string
}

/**
 * `hew.group.explode` (v1) — Dissolve a group, re-homing its members.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: unknown_entity, unknown_group
 */
export interface GroupExplodeParams {
  id: string
}

export interface GroupExplodeResult {}

/**
 * `hew.guide.angular` (v1) — Add an angular construction guide.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: degenerate_guide, unknown_entity, locator_missed, ambiguous_locator, no_such_point
 */
export interface GuideAngularParams {
  /** radians, right-handed about plane_normal */
  angle: number
  base_dir: [number, number, number]
  origin: UnspecifiedShape
  plane_normal: [number, number, number]
}

export interface GuideAngularResult {
  guide: string
}

/**
 * `hew.guide.clear` (v1) — Delete all guides.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: none.
 */
export interface GuideClearParams {}

export interface GuideClearResult {}

/**
 * `hew.guide.line` (v1) — Add an infinite construction guide line.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: degenerate_guide, unknown_entity, locator_missed, ambiguous_locator, no_such_point
 */
export interface GuideLineParams {
  direction: UnspecifiedShape
  origin: UnspecifiedShape
}

export interface GuideLineResult {
  guide: string
}

/**
 * `hew.guide.point` (v1) — Add a construction guide point.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: degenerate_guide, unknown_entity, locator_missed, ambiguous_locator, no_such_point
 */
export interface GuidePointParams {
  position: UnspecifiedShape
}

export interface GuidePointResult {
  guide: string
}

/**
 * `hew.history.redo` (v1) — Redo the most recently undone entry.
 * Tier: Required · Class: solitary · Served: kernel
 * Refusals: nothing_to_redo, inverse_failed, inverse_diverged
 */
export interface HistoryRedoParams {}

export interface HistoryRedoResult {}

/**
 * `hew.history.status` (v1) — History depth and the top entry's label and origin.
 * Tier: Required · Class: solitary · Served: kernel
 * Refusals: none.
 */
export interface HistoryStatusParams {}

export interface HistoryStatusResult {
  redo_depth: number
  top: UnspecifiedShape
  undo_depth: number
}

/**
 * `hew.history.undo` (v1) — Undo the top history entry (optionally guarded by expected_label).
 * Tier: Required · Class: solitary · Served: kernel
 * Refusals: expected_label_mismatch, nothing_to_undo, inverse_failed, inverse_diverged
 */
export interface HistoryUndoParams {
  expected_label?: string
}

export interface HistoryUndoResult {}

/**
 * `hew.material.create` (v1) — Add a color or texture material to the palette. Registry-state: records no undo entry (§6.4).
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: none.
 */
export interface MaterialCreateParams {
  color: number[]
  name: string
}

export interface MaterialCreateResult {
  material: string
}

/**
 * `hew.material.paint` (v1) — Paint a face or entity.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_entity, unknown_object, unknown_face, unknown_material, locator_missed, ambiguous_locator
 */
export interface MaterialPaintParams {
  face?: UnspecifiedShape
  id?: string
  material: string | null
}

export interface MaterialPaintResult {}

/**
 * `hew.material.set_default` (v1) — Set an object's default material.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_entity, unknown_object, unknown_material
 */
export interface MaterialSetDefaultParams {
  id: string
  material: string | null
}

export interface MaterialSetDefaultResult {}

/**
 * `hew.material.set_opacity` (v1) — Set a material's opacity.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_entity, unknown_material
 */
export interface MaterialSetOpacityParams {
  alpha: number
  material: string
}

export interface MaterialSetOpacityResult {}

/**
 * `hew.meta.capabilities` (v1) — The registry as data: every granted command's schemas, summary, and refusal inventory.
 * Tier: Required · Class: read-only · Served: kernel
 * Refusals: none.
 */
export interface MetaCapabilitiesParams {}

export interface MetaCapabilitiesResult {
  commands: ({ class: "model_mutating" | "read_only" | "solitary"; implemented: boolean; name: string; params: UnspecifiedShape; refusals: string[]; result: UnspecifiedShape; summary: string; version: number })[]
}

/**
 * `hew.meta.documents` (v1) — The host's open documents.
 * Tier: Required · Class: read-only · Served: host
 * Refusals: none.
 */
export interface MetaDocumentsParams {}

export interface MetaDocumentsResult {
  documents: unknown[]
}

/**
 * `hew.meta.hello` (v1) — Open the connection: negotiate protocol and encoding, learn the granted profile and open documents.
 * Tier: Required · Class: read-only · Served: kernel
 * Refusals: none.
 */
export interface MetaHelloParams {
  client?: { name?: string; version?: string }
  encodings?: string[]
  protocol: number
  token?: string
}

export interface MetaHelloResult {
  app: { name: string; version: string }
  documents: unknown[]
  encoding: string
  profile: "core" | "app"
  protocol: number
}

/**
 * `hew.query.context` (v1) — The open editing-context frame stack.
 * Tier: Required · Class: read-only · Served: kernel
 * Refusals: none.
 */
export interface QueryContextParams {}

export interface QueryContextResult {
  direct_members?: UnspecifiedShape
  stack: unknown[]
}

/**
 * `hew.query.entity` (v1) — One entity's details.
 * Tier: Required · Class: read-only · Served: kernel
 * Refusals: unknown_entity
 */
export interface QueryEntityParams {
  /** any public id, including a sketch's own edge id ("edg_…", HEW_API.md §5.2) — a sketch's `hew.query.scene`/`hew.query.entity` listing hands these out, and this command answers them directly with `{kind:"edge", sketch, from, to, length, curve}` */
  id: string
}

export interface QueryEntityResult {
  id: string
  kind: string
}

/**
 * `hew.query.faces` (v1) — A solid's faces: planes, areas, centroids, boundary loops.
 * Tier: Required · Class: read-only · Served: kernel
 * Refusals: unknown_entity
 */
export interface QueryFacesParams {
  object: string
}

export interface QueryFacesResult {
  faces: unknown[]
  object: string
}

/**
 * `hew.query.measure` (v1) — Distances and angles between points, edges, and faces.
 * Tier: Required · Class: read-only · Served: kernel
 * Refusals: unknown_entity, locator_missed, ambiguous_locator, no_such_point
 */
export interface QueryMeasureParams {
  from: UnspecifiedShape
  to: UnspecifiedShape
}

export interface QueryMeasureResult {
  delta: unknown[]
  distance: number
}

/**
 * `hew.query.raycast` (v1) — First hit along a ray — the programmatic form of clicking.
 * Tier: Required · Class: read-only · Served: kernel
 * Refusals: locator_missed, ambiguous_locator
 */
export interface QueryRaycastParams {
  dir: [number, number, number]
  origin: [number, number, number]
}

export interface QueryRaycastResult {
  distance: number
  kind: "object" | "instance"
  normal: unknown[]
  /** the world object's or, for an instance hit, the instance's public id */
  object: string
  point: unknown[]
}

/**
 * `hew.query.resolve` (v1) — Resolve any locator (point, face, edge) to its concrete value without mutating.
 * Tier: Required · Class: read-only · Served: kernel
 * Refusals: unknown_entity, locator_missed, ambiguous_locator, no_such_point, face_token_unknown, face_token_stale
 */
export interface QueryResolveParams {
  /** HEW_API.md §5.2's edge locator: a solid edge by {object,at}, a sketch edge's own public id ("edg_…") as a bare string, or a sketch edge by {sketch,at} / {sketch,from,to} */
  edge?: UnspecifiedShape
  face?: UnspecifiedShape
  point?: UnspecifiedShape
}

export type QueryResolveResult = UnspecifiedShape

/**
 * `hew.query.scene` (v1) — The document tree with per-entity summaries.
 * Tier: Required · Class: read-only · Served: kernel
 * Refusals: none.
 */
export interface QuerySceneParams {}

export interface QuerySceneResult {
  components: unknown[]
  document: UnspecifiedShape
  guides: unknown[]
  materials: unknown[]
  sketches: unknown[]
  tags: unknown[]
  tree: unknown[]
}

/**
 * `hew.scenes.add` (v1) — Add a Scene capturing the document's current view state. Records no undo entry (§6.4).
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: duplicate_scene_name, empty_scene_name, unknown_scene
 */
export interface ScenesAddParams {
  /** insert after this Scene's id; appended at the end when omitted */
  after?: string
  /** an explicit camera to capture — no named-view shorthand, a Scene captures a concrete eye/target, not a fitted view; when omitted and the camera property is captured, falls back to the document's own saved working camera */
  camera?: { eye: [number, number, number]; fov_deg?: number; projection?: "perspective" | "parallel"; target: [number, number, number]; up?: [number, number, number] }
  description?: string
  /** opaque editor display toggles: stored and returned, never interpreted by the kernel */
  display?: { axes: boolean; grid: boolean; guides: boolean }
  /** must be non-empty and unused; auto-named "Scene N" when omitted */
  name?: string
  /** which of the five capturable properties to (re-)capture; each defaults to true */
  properties?: { camera?: boolean; display?: boolean; hidden_nodes?: boolean; hidden_tags?: boolean; section?: boolean }
}

export interface ScenesAddResult {
  id: string
  name: string
  sid: number
}

/**
 * `hew.scenes.apply` (v1) — Apply a Scene: write its captured camera/hidden-set/section state into the document. Records no undo entry (§6.4).
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_scene
 */
export interface ScenesApplyParams {
  id: string
}

export interface ScenesApplyResult {
  camera?: { eye: [number, number, number]; fov_deg: number; projection: "perspective" | "parallel"; target: [number, number, number]; up: [number, number, number] }
  hidden_instance_ids?: string[]
  hidden_object_ids?: string[]
  /** null means captured-but-no-plane-placed */
  section?: null | { active: boolean; normal: [number, number, number]; origin: [number, number, number] }
}

/**
 * `hew.scenes.describe` (v1) — Set a Scene's free-text description. Records no undo entry (§6.4).
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_scene
 */
export interface ScenesDescribeParams {
  description: string
  id: string
}

export interface ScenesDescribeResult {}

/**
 * `hew.scenes.list` (v1) — Every Scene, in tab order.
 * Tier: Standard · Class: read-only · Served: kernel
 * Refusals: none.
 */
export interface ScenesListParams {}

export interface ScenesListResult {
  scenes: ({ camera?: { eye: [number, number, number]; fov_deg: number; projection: "perspective" | "parallel"; target: [number, number, number]; up: [number, number, number] }; description: string; display?: { axes: boolean; grid: boolean; guides: boolean }; id: string; name: string; props: { camera: boolean; display: boolean; hidden_nodes: boolean; hidden_tags: boolean; section: boolean }; section?: null | { active: boolean; normal: [number, number, number]; origin: [number, number, number] }; sid: number })[]
}

/**
 * `hew.scenes.remove` (v1) — Delete a Scene. Records no undo entry (§6.4).
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_scene
 */
export interface ScenesRemoveParams {
  id: string
}

export interface ScenesRemoveResult {}

/**
 * `hew.scenes.rename` (v1) — Rename a Scene. Records no undo entry (§6.4).
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_scene, duplicate_scene_name, empty_scene_name
 */
export interface ScenesRenameParams {
  id: string
  name: string
}

export interface ScenesRenameResult {}

/**
 * `hew.scenes.reorder` (v1) — Move a Scene to a new position in tab order. Records no undo entry (§6.4).
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_scene
 */
export interface ScenesReorderParams {
  id: string
  /** tab-order position; clamped to the end for an out-of-range index, never refused */
  index: number
}

export interface ScenesReorderResult {}

/**
 * `hew.scenes.update` (v1) — Re-capture a Scene's properties from the document's current state. Records no undo entry (§6.4).
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_scene
 */
export interface ScenesUpdateParams {
  /** an explicit camera to capture — no named-view shorthand, a Scene captures a concrete eye/target, not a fitted view; when omitted and the camera property is captured, falls back to the document's own saved working camera */
  camera?: { eye: [number, number, number]; fov_deg?: number; projection?: "perspective" | "parallel"; target: [number, number, number]; up?: [number, number, number] }
  /** opaque editor display toggles: stored and returned, never interpreted by the kernel */
  display?: { axes: boolean; grid: boolean; guides: boolean }
  id: string
  /** which of the five capturable properties to (re-)capture; each defaults to true */
  properties?: { camera?: boolean; display?: boolean; hidden_nodes?: boolean; hidden_tags?: boolean; section?: boolean }
}

export interface ScenesUpdateResult {}

/**
 * `hew.sketch.draw_arc` (v1) — Draw an arc on a plane spec.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: point_off_plane, degenerate_curve, degenerate_segment, unknown_sketch, segments_above_cap, unknown_entity, locator_missed, ambiguous_locator, path_too_short, endpoint_not_on_boundary, path_not_simple, loop_not_strictly_inside, loop_self_intersects, curve_claim_off_loop, unknown_object, unknown_component, unknown_face, point_not_on_face, would_corrupt
 */
export interface SketchDrawArcParams {
  center: [number, number, number] | UnspecifiedShape
  /** how the arc's ends are closed: "open" (default, a bare arc), "pie" (closed wedge — two radii to the center), or "segment" (closed circular segment — the chord). "pie"/"segment" commit a closed profile (a region in plane/sketch mode, a SplitFaceInner loop in face mode) like draw_rect/draw_circle, and need at least 2 segments (see "segments"). Must be "open" when the sweep is a full turn (already closed). */
  close?: "open" | "pie" | "segment"
  /** radians */
  end_angle: number
  /** HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>} */
  plane: UnspecifiedShape
  radius: number
  /** facet count; defaults proportionally to the sweep, capped at MAX_CIRCLE_SEGMENTS = 1024. A single chord (1) is fine for close: "open", but close: "pie"/"segment" needs at least 2 — a single chord can't form a non-degenerate closed loop — and the proportional default is floored at 2 for those modes too. */
  segments?: number
  /** radians */
  start_angle: number
}

export type SketchDrawArcResult = { curve_id?: string; region_id?: string; region_ids: string[]; sketch: string } | { object_id: string }

/**
 * `hew.sketch.draw_circle` (v1) — Draw a circle on a plane spec.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: point_off_plane, degenerate_curve, degenerate_segment, unknown_sketch, segments_below_floor, segments_above_cap, unknown_entity, locator_missed, ambiguous_locator, loop_not_strictly_inside, loop_self_intersects, curve_claim_off_loop, unknown_object, unknown_component, unknown_face, point_not_on_face, would_corrupt
 */
export interface SketchDrawCircleParams {
  center: [number, number, number] | UnspecifiedShape
  /** HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>} */
  plane: UnspecifiedShape
  radius: number
  /** facet count; defaults to 48, must fall in [MIN_CIRCLE_SEGMENTS, MAX_CIRCLE_SEGMENTS] = [24, 1024] */
  segments?: number
}

export type SketchDrawCircleResult = { curve_id?: string; region_id?: string; region_ids: string[]; sketch: string } | { object_id: string }

/**
 * `hew.sketch.draw_line` (v1) — Draw a line (chain) on a plane spec.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: point_off_plane, degenerate_segment, unknown_sketch, unknown_entity, locator_missed, ambiguous_locator, path_too_short, endpoint_not_on_boundary, path_not_simple, unknown_object, unknown_component, unknown_face, point_not_on_face, would_corrupt
 */
export interface SketchDrawLineParams {
  /** HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>} */
  plane: UnspecifiedShape
  points: ([number, number, number] | UnspecifiedShape)[]
}

export type SketchDrawLineResult = { curve_id?: string; region_id?: string; region_ids: string[]; sketch: string } | { object_id: string }

/**
 * `hew.sketch.draw_polygon` (v1) — Draw a regular N-gon on a plane spec.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: point_off_plane, degenerate_curve, degenerate_segment, unknown_sketch, unknown_entity, locator_missed, ambiguous_locator, loop_not_strictly_inside, loop_self_intersects, unknown_object, unknown_component, unknown_face, point_not_on_face, would_corrupt
 */
export interface SketchDrawPolygonParams {
  center: [number, number, number] | UnspecifiedShape
  /** HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>} */
  plane: UnspecifiedShape
  radius: number
  sides: number
}

export type SketchDrawPolygonResult = { curve_id?: string; region_id?: string; region_ids: string[]; sketch: string } | { object_id: string }

/**
 * `hew.sketch.draw_rect` (v1) — Draw an axis-aligned rectangle on a plane spec.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: point_off_plane, degenerate_segment, unknown_sketch, unknown_entity, locator_missed, ambiguous_locator, loop_not_strictly_inside, loop_self_intersects, unknown_object, unknown_component, unknown_face, point_not_on_face, would_corrupt
 */
export interface SketchDrawRectParams {
  corner_a: [number, number, number] | UnspecifiedShape
  corner_b: [number, number, number] | UnspecifiedShape
  /** HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>} */
  plane: UnspecifiedShape
}

export type SketchDrawRectResult = { curve_id?: string; region_id?: string; region_ids: string[]; sketch: string } | { object_id: string }

/**
 * `hew.sketch.offset` (v1) — Offset a region boundary within its sketch.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: unknown_region, malformed_region, offset_too_small, offset_collapsed, unknown_sketch, unknown_entity
 */
export interface SketchOffsetParams {
  /** positive grows the material, negative shrinks it */
  distance: number
  region: string
}

export interface SketchOffsetResult {
  curve_ids: string[]
  /** present when exactly one region resulted */
  region_id?: string
  region_ids: string[]
  sketch: string
}

/**
 * `hew.solid.extrude` (v1) — Extrude a region into a new Object, consuming the profile.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: distance_too_small, degenerate_geometry, unknown_region, unknown_entity
 */
export interface SolidExtrudeParams {
  distance: number
  region: string
}

export interface SolidExtrudeResult {
  object_id: string
}

/**
 * `hew.solid.follow_me` (v1) — Sweep a profile along an edge-chain path, as the tool does.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: empty_path, unknown_path_edge, path_branches, path_disconnected, path_segment_too_short, profile_not_perpendicular, follow_me_in_component_unsupported, path_detached_from_profile, path_reverses, path_too_tight, profile_crosses_axis, partial_sweep_on_pole, sweep_self_intersects, sweep_degenerate, unknown_region, unknown_object, unknown_face, unknown_entity, locator_missed, ambiguous_locator, unimplemented
 */
export interface SolidFollowMeParams {
  path: { edges: unknown[] } | { face: UnspecifiedShape } | { curve: string }
  profile: string | { face: UnspecifiedShape }
}

export interface SolidFollowMeResult {
  object_id: string
}

/**
 * `hew.solid.intersect` (v1) — Boolean intersection of two solids.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: boolean_operand_has_instance, boolean_operand_not_solid, boolean_operand_empty, grouped_operand, degenerate_contact, unknown_object, unknown_group, unknown_instance, unknown_entity
 */
export interface SolidIntersectParams {
  /** a node id: obj_/grp_/ins_ */
  a: string
  /** a node id: obj_/grp_/ins_ */
  b: string
}

export interface SolidIntersectResult {
  result: string
}

/**
 * `hew.solid.push_pull` (v1) — Push/pull a face of a solid with the tool's full semantics.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: object_not_solid, distance_too_small, would_vanish, non_manifold_result, not_a_sub_face, radius_vanishes, wall_neighbor_non_planar, unknown_face, unknown_object, unknown_component, grouped_operand, unknown_entity, locator_missed, ambiguous_locator, face_token_unknown, face_token_stale
 */
export interface SolidPushPullParams {
  distance: number
  /** HEW_API.md §5.2 face locator: {object,at} | {object,ray} | {"$face":"label#key"} */
  face: UnspecifiedShape
}

export interface SolidPushPullResult {
  /** the pushed/pulled object, still standing (in-place or sub-face case) */
  object_id?: string
  /** a through-cut's resulting pieces, replacing the source object */
  object_ids?: string[]
}

/**
 * `hew.solid.slice` (v1) — Slice a solid by a plane into two solids.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: not_solid, plane_misses_solid, degenerate, unknown_object, unknown_entity
 */
export interface SolidSliceParams {
  object: string
  plane: { normal: [number, number, number] | UnspecifiedShape; origin: [number, number, number] | UnspecifiedShape }
}

export interface SolidSliceResult {
  negative: string
  positive: string
}

/**
 * `hew.solid.subtract` (v1) — Boolean subtraction of two solids.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: boolean_operand_has_instance, boolean_operand_not_solid, boolean_operand_empty, grouped_operand, degenerate_contact, unknown_object, unknown_group, unknown_instance, unknown_entity
 */
export interface SolidSubtractParams {
  /** a node id: obj_/grp_/ins_ */
  a: string
  /** a node id: obj_/grp_/ins_ */
  b: string
}

export interface SolidSubtractResult {
  result: string
}

/**
 * `hew.solid.union` (v1) — Boolean union of two solids.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: boolean_operand_has_instance, boolean_operand_not_solid, boolean_operand_empty, grouped_operand, degenerate_contact, unknown_object, unknown_group, unknown_instance, unknown_entity
 */
export interface SolidUnionParams {
  /** a node id: obj_/grp_/ins_ */
  a: string
  /** a node id: obj_/grp_/ins_ */
  b: string
}

export interface SolidUnionResult {
  result: string
}

/**
 * `hew.tag.assign` (v1) — Assign a tag to nodes.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_entity, unknown_object, unknown_group, unknown_instance
 */
export interface TagAssignParams {
  id: string
  path: string[]
  remove?: boolean
}

export interface TagAssignResult {}

/**
 * `hew.tag.create` (v1) — Register a tag path. Registry-state: records no undo entry (§6.4).
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: none.
 */
export interface TagCreateParams {
  hidden?: boolean
  path: string[]
}

export interface TagCreateResult {
  tag: string
}

/**
 * `hew.tag.delete` (v1) — Delete a tag path, unassigning it everywhere.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_tag
 */
export interface TagDeleteParams {
  path: string[]
}

export interface TagDeleteResult {}

/**
 * `hew.tag.set_visible` (v1) — Toggle a tag's visibility. Registry-state: records no undo entry (§6.4).
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: none.
 */
export interface TagSetVisibleParams {
  path: string[]
  visible: boolean
}

export interface TagSetVisibleResult {}

/**
 * `hew.view.camera` (v1) — Set the live desktop viewport's camera. A host effect on the view, not a document edit (mutates_document = false: never recorded, never resyncs the document). Headless clients pass a camera per hew.view.snapshot call instead.
 * Tier: Standard · Class: solitary · Served: host
 * Refusals: host_capability_missing
 */
export interface ViewCameraParams {
  /** mutually exclusive with view; identical vocabulary to hew.view.snapshot's camera */
  camera?: { eye: [number, number, number]; fov_deg?: number; projection?: "perspective" | "parallel"; target: [number, number, number]; up?: [number, number, number] }
  /** a named standard view; mutually exclusive with camera */
  view?: "iso" | "front" | "back" | "left" | "right" | "top" | "bottom"
}

export interface ViewCameraResult {}

/**
 * `hew.view.snapshot` (v1) — Render the attached document to PNG, headless-rendered via a software rasterizer (a live host may render through its viewport instead) — bytes base64 by default, or a path on hosts with filesystem access.
 * Tier: Standard · Class: solitary · Served: host
 * Refusals: host_capability_missing, nothing_to_render, save_failed, unknown_scene
 */
export interface ViewSnapshotParams {
  /** mutually exclusive with view */
  camera?: { eye: [number, number, number]; fov_deg?: number; projection?: "perspective" | "parallel"; target: [number, number, number]; up?: [number, number, number] }
  /** defaults to 512; out-of-range values are clamped, not refused */
  height?: number
  /** defaults to false; when true, also returns a per-pixel id-buffer and its palette */
  include_ids?: boolean
  /** when given, the PNG is written here instead of returned inline, honored by hosts with filesystem access and refused typed elsewhere (mirrors hew.doc.export) */
  path?: string
  /** a Scene's id: renders through its resolved camera and hidden sets (Document::resolve_scene) instead of the document's own — falls back to the usual cameraless resolution when the Scene captures no camera. Mutually exclusive with camera and view. The Scene's section plane, if any, is NOT rendered headlessly at 1.0. */
  scene?: string
  /** a named standard view fitted to the scene bounding box; mutually exclusive with camera and scene */
  view?: "iso" | "front" | "back" | "left" | "right" | "top" | "bottom"
  /** defaults to 512; out-of-range values are clamped, not refused */
  width?: number
}

export interface ViewSnapshotResult {
  height: number
  /** present only when include_ids was true and path was not given: u16 little-endian per pixel, index into id_palette (0 = background) */
  id_buffer_base64?: string
  /** present only when include_ids and path were both given: "<path>.ids.bin", the same u16 little-endian per-pixel encoding written to disk */
  id_buffer_path?: string
  /** public ids; id_palette[i] is what the id-buffer (inline or on disk) reports as index i+1 */
  id_palette?: string[]
  /** present only when path was given: echoes it back */
  path?: string
  /** present only when path was not given */
  png_base64?: string
  width: number
}

/**
 * `hew.view.units` (v1) — Set the app's displayed length-unit format (app/src/settings/units.ts's LengthFormat) — an app-level display PREFERENCE, never document state or file-format data.
 * Tier: Standard · Class: solitary · Served: host
 * Refusals: host_capability_missing
 */
export interface ViewUnitsParams {
  /** metric: m, cm, mm; imperial: arch (feet+inches, e.g. 5' 3-1/8"), frac_in (fractional inches), dec_in (decimal inches) */
  format: "m" | "cm" | "mm" | "arch" | "frac_in" | "dec_in"
}

export interface ViewUnitsResult {}

/**
 * `hew.view.zoom_extents` (v1) — Frame all visible geometry in the live viewport (View > Zoom Extents). A view effect, not a document edit (mutates_document = false).
 * Tier: Standard · Class: solitary · Served: host
 * Refusals: host_capability_missing
 */
export interface ViewZoomExtentsParams {}

export interface ViewZoomExtentsResult {}

/** A typed client over a caller-supplied {@link HewTransport}. One
 * method per registry command, grouped by namespace
 * (`client.sketch.drawRect(…)`), generated from the registry above. */
export class HewApiClient {
  private nextId = 1

  constructor(private readonly transport: HewTransport) {}

  private async call<TResult>(method: string, params: unknown): Promise<TResult> {
    const request: JsonRpcRequest = {
      jsonrpc: '2.0',
      id: this.nextId++,
      method,
      params,
    }
    const response = await this.transport.dispatch(request)
    if (response.error) {
      throw new HewApiError(response.error)
    }
    return response.result as TResult
  }

  // A plain request to a model-mutating command is wrapped by the
  // dispatcher into a one-command transaction (HEW_API.md section 6) and
  // answered with {results: [<own result>], label} -- unwrap it so
  // every method resolves with its own declared result type.
  private async mutate<TResult>(method: string, params: unknown): Promise<TResult> {
    const envelope = await this.call<{ results: [TResult]; label: string }>(method, params)
    return envelope.results[0]
  }

  readonly attr = {
    delete: (params: AttrDeleteParams): Promise<AttrDeleteResult> => this.mutate('hew.attr.delete', params),
    get: (params: AttrGetParams): Promise<AttrGetResult> => this.call('hew.attr.get', params),
    set: (params: AttrSetParams): Promise<AttrSetResult> => this.mutate('hew.attr.set', params),
  }

  readonly component = {
    create: (params: ComponentCreateParams): Promise<ComponentCreateResult> => this.mutate('hew.component.create', params),
    explode: (params: ComponentExplodeParams): Promise<ComponentExplodeResult> => this.mutate('hew.component.explode', params),
    makeUnique: (params: ComponentMakeUniqueParams): Promise<ComponentMakeUniqueResult> => this.mutate('hew.component.make_unique', params),
    place: (params: ComponentPlaceParams): Promise<ComponentPlaceResult> => this.mutate('hew.component.place', params),
  }

  readonly context = {
    enter: (params: ContextEnterParams): Promise<ContextEnterResult> => this.mutate('hew.context.enter', params),
    exit: (params: ContextExitParams): Promise<ContextExitResult> => this.mutate('hew.context.exit', params),
  }

  readonly doc = {
    attach: (params: DocAttachParams): Promise<DocAttachResult> => this.call('hew.doc.attach', params),
    export: (params: DocExportParams): Promise<DocExportResult> => this.call('hew.doc.export', params),
    import: (params: DocImportParams): Promise<DocImportResult> => this.call('hew.doc.import', params),
    new: (params: DocNewParams): Promise<DocNewResult> => this.call('hew.doc.new', params),
    open: (params: DocOpenParams): Promise<DocOpenResult> => this.call('hew.doc.open', params),
    save: (params: DocSaveParams): Promise<DocSaveResult> => this.call('hew.doc.save', params),
    transact: (params: DocTransactParams): Promise<DocTransactResult> => this.call('hew.doc.transact', params),
  }

  readonly entity = {
    delete: (params: EntityDeleteParams): Promise<EntityDeleteResult> => this.mutate('hew.entity.delete', params),
    move: (params: EntityMoveParams): Promise<EntityMoveResult> => this.mutate('hew.entity.move', params),
    rename: (params: EntityRenameParams): Promise<EntityRenameResult> => this.mutate('hew.entity.rename', params),
    rotate: (params: EntityRotateParams): Promise<EntityRotateResult> => this.mutate('hew.entity.rotate', params),
    scale: (params: EntityScaleParams): Promise<EntityScaleResult> => this.mutate('hew.entity.scale', params),
  }

  readonly group = {
    create: (params: GroupCreateParams): Promise<GroupCreateResult> => this.mutate('hew.group.create', params),
    explode: (params: GroupExplodeParams): Promise<GroupExplodeResult> => this.mutate('hew.group.explode', params),
  }

  readonly guide = {
    angular: (params: GuideAngularParams): Promise<GuideAngularResult> => this.mutate('hew.guide.angular', params),
    clear: (params: GuideClearParams): Promise<GuideClearResult> => this.mutate('hew.guide.clear', params),
    line: (params: GuideLineParams): Promise<GuideLineResult> => this.mutate('hew.guide.line', params),
    point: (params: GuidePointParams): Promise<GuidePointResult> => this.mutate('hew.guide.point', params),
  }

  readonly history = {
    redo: (params: HistoryRedoParams): Promise<HistoryRedoResult> => this.call('hew.history.redo', params),
    status: (params: HistoryStatusParams): Promise<HistoryStatusResult> => this.call('hew.history.status', params),
    undo: (params: HistoryUndoParams): Promise<HistoryUndoResult> => this.call('hew.history.undo', params),
  }

  readonly material = {
    create: (params: MaterialCreateParams): Promise<MaterialCreateResult> => this.mutate('hew.material.create', params),
    paint: (params: MaterialPaintParams): Promise<MaterialPaintResult> => this.mutate('hew.material.paint', params),
    setDefault: (params: MaterialSetDefaultParams): Promise<MaterialSetDefaultResult> => this.mutate('hew.material.set_default', params),
    setOpacity: (params: MaterialSetOpacityParams): Promise<MaterialSetOpacityResult> => this.mutate('hew.material.set_opacity', params),
  }

  readonly meta = {
    capabilities: (params: MetaCapabilitiesParams): Promise<MetaCapabilitiesResult> => this.call('hew.meta.capabilities', params),
    documents: (params: MetaDocumentsParams): Promise<MetaDocumentsResult> => this.call('hew.meta.documents', params),
    hello: (params: MetaHelloParams): Promise<MetaHelloResult> => this.call('hew.meta.hello', params),
  }

  readonly query = {
    context: (params: QueryContextParams): Promise<QueryContextResult> => this.call('hew.query.context', params),
    entity: (params: QueryEntityParams): Promise<QueryEntityResult> => this.call('hew.query.entity', params),
    faces: (params: QueryFacesParams): Promise<QueryFacesResult> => this.call('hew.query.faces', params),
    measure: (params: QueryMeasureParams): Promise<QueryMeasureResult> => this.call('hew.query.measure', params),
    raycast: (params: QueryRaycastParams): Promise<QueryRaycastResult> => this.call('hew.query.raycast', params),
    resolve: (params: QueryResolveParams): Promise<QueryResolveResult> => this.call('hew.query.resolve', params),
    scene: (params: QuerySceneParams): Promise<QuerySceneResult> => this.call('hew.query.scene', params),
  }

  readonly scenes = {
    add: (params: ScenesAddParams): Promise<ScenesAddResult> => this.mutate('hew.scenes.add', params),
    apply: (params: ScenesApplyParams): Promise<ScenesApplyResult> => this.mutate('hew.scenes.apply', params),
    describe: (params: ScenesDescribeParams): Promise<ScenesDescribeResult> => this.mutate('hew.scenes.describe', params),
    list: (params: ScenesListParams): Promise<ScenesListResult> => this.call('hew.scenes.list', params),
    remove: (params: ScenesRemoveParams): Promise<ScenesRemoveResult> => this.mutate('hew.scenes.remove', params),
    rename: (params: ScenesRenameParams): Promise<ScenesRenameResult> => this.mutate('hew.scenes.rename', params),
    reorder: (params: ScenesReorderParams): Promise<ScenesReorderResult> => this.mutate('hew.scenes.reorder', params),
    update: (params: ScenesUpdateParams): Promise<ScenesUpdateResult> => this.mutate('hew.scenes.update', params),
  }

  readonly sketch = {
    drawArc: (params: SketchDrawArcParams): Promise<SketchDrawArcResult> => this.mutate('hew.sketch.draw_arc', params),
    drawCircle: (params: SketchDrawCircleParams): Promise<SketchDrawCircleResult> => this.mutate('hew.sketch.draw_circle', params),
    drawLine: (params: SketchDrawLineParams): Promise<SketchDrawLineResult> => this.mutate('hew.sketch.draw_line', params),
    drawPolygon: (params: SketchDrawPolygonParams): Promise<SketchDrawPolygonResult> => this.mutate('hew.sketch.draw_polygon', params),
    drawRect: (params: SketchDrawRectParams): Promise<SketchDrawRectResult> => this.mutate('hew.sketch.draw_rect', params),
    offset: (params: SketchOffsetParams): Promise<SketchOffsetResult> => this.mutate('hew.sketch.offset', params),
  }

  readonly solid = {
    extrude: (params: SolidExtrudeParams): Promise<SolidExtrudeResult> => this.mutate('hew.solid.extrude', params),
    followMe: (params: SolidFollowMeParams): Promise<SolidFollowMeResult> => this.mutate('hew.solid.follow_me', params),
    intersect: (params: SolidIntersectParams): Promise<SolidIntersectResult> => this.mutate('hew.solid.intersect', params),
    pushPull: (params: SolidPushPullParams): Promise<SolidPushPullResult> => this.mutate('hew.solid.push_pull', params),
    slice: (params: SolidSliceParams): Promise<SolidSliceResult> => this.mutate('hew.solid.slice', params),
    subtract: (params: SolidSubtractParams): Promise<SolidSubtractResult> => this.mutate('hew.solid.subtract', params),
    union: (params: SolidUnionParams): Promise<SolidUnionResult> => this.mutate('hew.solid.union', params),
  }

  readonly tag = {
    assign: (params: TagAssignParams): Promise<TagAssignResult> => this.mutate('hew.tag.assign', params),
    create: (params: TagCreateParams): Promise<TagCreateResult> => this.mutate('hew.tag.create', params),
    delete: (params: TagDeleteParams): Promise<TagDeleteResult> => this.mutate('hew.tag.delete', params),
    setVisible: (params: TagSetVisibleParams): Promise<TagSetVisibleResult> => this.mutate('hew.tag.set_visible', params),
  }

  readonly view = {
    camera: (params: ViewCameraParams): Promise<ViewCameraResult> => this.call('hew.view.camera', params),
    snapshot: (params: ViewSnapshotParams): Promise<ViewSnapshotResult> => this.call('hew.view.snapshot', params),
    units: (params: ViewUnitsParams): Promise<ViewUnitsResult> => this.call('hew.view.units', params),
    zoomExtents: (params: ViewZoomExtentsParams): Promise<ViewZoomExtentsResult> => this.call('hew.view.zoom_extents', params),
  }
}
