// GENERATED from crates/api registry — do not edit; regenerate with:
//   REGENERATE_API_ARTIFACTS=1 cargo test -p api --test generate_artifacts
//
// A typed client over a caller-supplied transport for the Hew API
// (docs/agents/HEW_API.md — the normative reference; §9 is this file's
// contract). Every `Params`/`Result` pair and every method below is
// derived mechanically from the command registry in `crates/api`,
// which is their single source of truth — this file has no
// hand-maintained copy of a command's shape.

/** One JSON-RPC 2.0 request frame (docs/agents/HEW_API.md §4.1). `params` is
 * always a single object, never positional. */
export interface JsonRpcRequest {
  jsonrpc: '2.0'
  id: number | string
  method: string
  params?: unknown
  /** The one document this request addresses (§4.2): an id, path, label, or title. */
  document?: string
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
 * stdio MCP adapter all satisfy this one shape (docs/agents/HEW_API.md
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
 * (an untightened `{"type": "object"}` — docs/agents/HEW_API.md §14's
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
 * `hew.doc.attach` (v1) — Bind this connection to one open document, named by `document` (id, path, label, or title; see hew.meta.documents). With no `document`, the connection's current document.
 * Tier: Required · Class: solitary · Served: kernel
 * Refusals: unknown_document, ambiguous_document
 */
export interface DocAttachParams {
  /** a document id (doc_N), or its exact path, label, or title when unambiguous */
  document?: string
}

export interface DocAttachResult {
  document?: { attached?: boolean; dirty: boolean; front?: boolean; id: string; label?: string; path: string | null; title: string }
}

/**
 * `hew.doc.close` (v1) — Close an open document window (the attached one, or the named `document`). Refuses unsaved changes unless `discard` is set.
 * Tier: Standard · Class: solitary · Served: host
 * Refusals: host_capability_missing, unknown_document, ambiguous_document, unsaved_changes, last_document
 */
export interface DocCloseParams {
  /** close even with unsaved changes (they are lost); default false */
  discard?: boolean
  document?: string
}

export interface DocCloseResult {
  closed?: { attached?: boolean; dirty: boolean; front?: boolean; id: string; label?: string; path: string | null; title: string }
}

/**
 * `hew.doc.export` (v1) — Export the attached document — STL, 3MF, glTF/GLB, USDZ (solids only), or a SketchUp .skp ("skp", 2026 container; "skp2017") — bytes base64, or a path on hosts with filesystem access.
 * Tier: Required · Class: solitary · Served: host
 * Refusals: export_failed, host_capability_missing, nothing_to_export, save_failed
 */
export interface DocExportParams {
  /** "gltf" is an alias for "glb" — every host that implements one implements both. "skp" is the SketchUp 2026 container, "skp2017" the 2017 format; exporting never changes the document's own file */
  format: "stl" | "3mf" | "glb" | "gltf" | "usdz" | "skp" | "skp2017"
  path?: string
  segments_per_turn?: number
  /** SketchUp formats only: the display units the file opens in (hew.view.units's formats). Absent: the host's display units (the app's preference live; meters headless). */
  units?: "m" | "cm" | "mm" | "arch" | "frac_in" | "dec_in"
}

export interface DocExportResult {
  bytes_base64?: string
  format: string
}

/**
 * `hew.doc.focus` (v1) — Raise a document's window to the front. Every other command leaves the user's focus alone; this is the explicit opt-in.
 * Tier: Standard · Class: solitary · Served: host
 * Refusals: host_capability_missing, unknown_document, ambiguous_document
 */
export interface DocFocusParams {
  document?: string
}

export interface DocFocusResult {
  document?: { attached?: boolean; dirty: boolean; front?: boolean; id: string; label?: string; path: string | null; title: string }
}

/**
 * `hew.doc.health` (v1) — Model health: every open object (world or component member) with its verdict, open rims and seam family, and counts by kind.
 * Tier: Standard · Class: read-only · Served: kernel
 * Refusals: unknown_entity
 */
export interface DocHealthParams {
  /** obj_ ids to diagnose; omitted = every live object, component members included */
  objects?: string[]
}

export interface DocHealthResult {
  /** objects, solids, flat_sheets, bent_sheets, cappable, holes, mixed, non_manifold, seam_families */
  counts: UnspecifiedShape
  /** objects an importer split from one shape, joined by shared rims */
  families: { members?: string[]; shared_rims?: number }[]
  /** one per open (non-solid) object; a component member is one row however often it is placed */
  rows: ({ accepted: boolean; branching_vertices?: number; definition?: string | null; definition_name?: string | null; faces?: number; family?: number | null; instances?: string[]; islands?: number; name?: string | null; object: string; placements: number; rims: ({ cappable?: boolean; closed?: boolean; edges?: number; extent?: number; perimeter?: number; planar?: boolean; role?: "outer" | "hole"; shared?: boolean; sheet_edge?: boolean })[]; verdict: { holes?: number; kind: "flat_sheet" | "bent_sheet" | "cappable" | "holes" | "mixed" | "non_manifold"; planar?: number; warped?: number } })[]
}

/**
 * `hew.doc.import` (v1) — Merge a foreign-format file (STL, glTF/GLB, COLLADA, SketchUp) into the attached document through the shared healing pipeline, or place a PNG/JPEG picture (a component holding one face painted with it, at origin/u/v, width meters wide). Additive: existing geometry is untouched.
 * Tier: Standard · Class: solitary · Served: host
 * Refusals: host_capability_missing, units_required, load_failed, unsupported_format
 */
export interface DocImportParams {
  /** the file's bytes, for a host with no filesystem (the desktop app); `path` then only names the file. hew-cli supplies this for you */
  bytes_base64?: string
  /** COLLADA texture images by the uri the file uses: {bytes_base64, format: png|jpeg} */
  images?: UnspecifiedShape
  /** a picture's (.png/.jpg/.jpeg) lower-left corner, meters; default [0,0,0] */
  origin?: [number, number, number]
  path: string
  /** a picture's width direction; default [1,0,0] */
  u?: [number, number, number]
  units?: "m" | "mm" | "cm" | "in"
  /** toward a picture's top (not parallel to u); default [0,1,0] */
  v?: [number, number, number]
  /** a picture's width in meters (its height follows from its proportions); default 1 */
  width?: number
}

export interface DocImportResult {
  report: UnspecifiedShape
}

/**
 * `hew.doc.new` (v1) — Create a fresh document. The desktop app opens it in a new window — behind the user's current one unless `focus` says otherwise — and attaches the connection to it; headless hosts replace the working document.
 * Tier: Required · Class: solitary · Served: host
 * Refusals: host_capability_missing, window_not_ready
 */
export interface DocNewParams {
  /** where the new window goes; background (the default) never takes the user's focus */
  focus?: "background" | "front"
  /** a name to find the document by later, resolvable by hew.doc.attach and listed by hew.meta.documents */
  label?: string
}

export interface DocNewResult {
  document?: { attached?: boolean; dirty: boolean; front?: boolean; id: string; label?: string; path: string | null; title: string }
}

/**
 * `hew.doc.open` (v1) — Open a .hew or SketchUp .skp document. The desktop app opens it in a new window (or attaches to the window that already has that file open), behind the user's current window unless `focus` says otherwise; headless hosts replace the working document.
 * Tier: Required · Class: solitary · Served: host
 * Refusals: host_capability_missing, load_failed, window_not_ready
 */
export interface DocOpenParams {
  /** where the new window goes; background (the default) never takes the user's focus */
  focus?: "background" | "front"
  /** a name to find the document by later, resolvable by hew.doc.attach and listed by hew.meta.documents */
  label?: string
  /** a .hew file, or a SketchUp .skp file (either container) when the extension is .skp; a later hew.doc.save with no format writes the same container back */
  path: string
}

export interface DocOpenResult {
  /** the file was already open in a window; the connection was attached to it instead of opening a second copy */
  already_open?: boolean
  document?: { attached?: boolean; dirty: boolean; front?: boolean; id: string; label?: string; path: string | null; title: string }
}

/**
 * `hew.doc.purge_unused` (v1) — Delete every unused material and component definition, as one undo entry.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: none.
 */
export interface DocPurgeUnusedParams {}

export interface DocPurgeUnusedResult {
  definitions: number
  materials: number
}

/**
 * `hew.doc.revert` (v1) — Revert to Saved: re-read the document's saved file in place, discarding every change since the last save. The undo and redo history is cleared and the document reads clean. Refuses `nothing_to_revert` when the document has no saved file or has no changes since the last save. There is no confirmation step: the caller is deliberate.
 * Tier: Standard · Class: solitary · Served: host
 * Refusals: host_capability_missing, load_failed, nothing_to_revert, unknown_document, ambiguous_document
 */
export interface DocRevertParams {
  document?: string
}

export interface DocRevertResult {}

/**
 * `hew.doc.save` (v1) — Save the attached document (Save / Save As) as .hew or SketchUp .skp: the file is written, the unsaved mark cleared, and the document bound to it. `copy` writes a copy without any of that. With no `path` it re-saves in place; a document with no file yet, in a host that cannot ask where, hands the bytes back instead.
 * Tier: Required · Class: solitary · Served: host
 * Refusals: file_open_elsewhere, format_mismatch, host_capability_missing, path_required, save_failed
 */
export interface DocSaveParams {
  /** write a copy to `path` — the document is not rebound to it and keeps its unsaved mark; requires `path` */
  copy?: boolean
  /** "skp" is the 2026 container (what current SketchUp saves), "skp2017" the 2017 format (opens in SketchUp Make 2017 and later). Absent: a .skp path keeps the working file's own .skp container (the 2026 one for a document that did not come from a .skp), any other path writes .hew, and no path uses the working file's own format. A format that contradicts the path's extension is refused (format_mismatch). */
  format?: "hew" | "skp" | "skp2017"
  path?: string
  /** hand the document's bytes back and write nothing: no file, no binding, no saved mark; takes no `path` */
  return_bytes?: boolean
  /** The display units a SketchUp file opens in (hew.view.units's formats), a save back over a .skp included. Absent: a save back keeps the file's own units and a new file takes the host's display units (the app's preference live; meters headless). A .hew ignores it. */
  units?: "m" | "cm" | "mm" | "arch" | "frac_in" | "dec_in"
}

export interface DocSaveResult {
  bytes_base64?: string
  /** the format the document was saved in */
  format?: "hew" | "skp" | "skp2017"
  /** present for a .skp: what the file could not hold exactly */
  simplified?: string[]
  /** present for a .skp: what the 2017 writer dropped */
  writer_notes?: string[]
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
 * Refusals: unknown_entity, unknown_object, unknown_group, unknown_instance, unknown_sketch, unknown_guide, unknown_edge, unknown_material, unknown_component, definition_nested_in_definition, explode_session_scope
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
 * Refusals: unknown_entity, rename_unsupported, unknown_object, unknown_group, unknown_instance, unknown_component, unknown_material
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
 * `hew.group.reparent` (v1) — Move live nodes into a group, or out to the top level, without touching their geometry.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_entity, empty_ids, group_cycle, explode_session_scope, unknown_object, unknown_group, unknown_instance
 */
export interface GroupReparentParams {
  ids: string[]
  parent?: string | null
}

export interface GroupReparentResult {}

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
 * `hew.history.status` (v1) — History depth, the top entry's label and origin, the saved depth, and every undo/redo entry's label and origin.
 * Tier: Required · Class: solitary · Served: kernel
 * Refusals: none.
 */
export interface HistoryStatusParams {}

export interface HistoryStatusResult {
  entries: { redo: { bookkeeping: boolean; label: string; origin: UnspecifiedShape }[]; undo: { bookkeeping: boolean; label: string; origin: UnspecifiedShape }[] }
  redo_depth: number
  saved_depth: UnspecifiedShape
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
 * `hew.library.describe` (v1) — One library item's full metadata, manifest summary, and document attributes.
 * Tier: Standard · Class: read-only · Served: host
 * Refusals: host_capability_missing, unknown_library_item
 */
export interface LibraryDescribeParams {
  /** the item's id or relative path */
  item: string
}

export interface LibraryDescribeResult {
  /** the item's full document attribute dictionaries, namespace -> key -> value */
  attrs: UnspecifiedShape
  category: "component" | "material" | "model"
  collection?: string
  /** present only when the file failed to parse */
  error?: string
  id?: string
  keywords: string[]
  mtime_ms: number
  name: string
  path: string
  saved_at?: string
  size: number
  /** manifest counts (objects, materials, components, ...); absent when error is present */
  summary?: UnspecifiedShape
}

/**
 * `hew.library.insert` (v1) — Insert a library item into the attached document, wired for idempotent re-insert exactly like the UI's own placement; a material item goes into the palette instead, recording no undo entry (§6.4).
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: host_capability_missing, unknown_library_item, load_failed, component_expansion_exceeded, singular, degenerate_axis, reflection, explode_session_scope
 */
export interface LibraryInsertParams {
  /** a translation applied to the item's world roots; identity when omitted */
  at?: [number, number, number] | UnspecifiedShape
  /** the item's raw .hew bytes — exactly one of item/bytes_base64 */
  bytes_base64?: string
  /** sha256 hex of bytes_base64, for idempotent re-insert matching; ignored (and provenance is skipped) without it */
  content_hash?: string
  /** the item's id or relative path — exactly one of item/bytes_base64 */
  item?: string
  name?: string
}

export interface LibraryInsertResult {
  annotations_skipped: number
  definitions_added: number
  definitions_reused: number
  guides_added: number
  /** a material item only: its palette materials, added or reused, in item order */
  materials?: string[]
  materials_added: number
  materials_reused: number
  objects_added: number
  roots: string[]
  world_sketches_skipped: number
}

/**
 * `hew.library.list` (v1) — List items in the Hew Library, optionally filtered by category, collection, or a text query.
 * Tier: Standard · Class: read-only · Served: host
 * Refusals: host_capability_missing
 */
export interface LibraryListParams {
  category?: "component" | "material" | "model"
  /** matches this collection path and its subtree */
  collection?: string
  /** case-insensitive substring match against name and keywords */
  query?: string
}

export interface LibraryListResult {
  folder: string
  items: ({ category: "component" | "material" | "model"; collection?: string; error?: string; id?: string; keywords: string[]; mtime_ms: number; name: string; path: string; saved_at?: string; size: number; summary?: UnspecifiedShape })[]
}

/**
 * `hew.library.remove` (v1) — Delete a library item.
 * Tier: Standard · Class: read-only · Served: host
 * Refusals: host_capability_missing, unknown_library_item
 */
export interface LibraryRemoveParams {
  /** the item's id or relative path */
  item: string
}

export interface LibraryRemoveResult {
  removed: string
}

/**
 * `hew.library.save` (v1) — Save a selection, one material, or the whole document to the library. Records no undo entry (§6.4).
 * Tier: Standard · Class: read-only · Served: kernel
 * Refusals: host_capability_missing, unknown_entity, empty_component, duplicate_member, unknown_object, unknown_group, unknown_instance, explode_session_scope, save_failed
 */
export interface LibrarySaveParams {
  /** defaults to component (a selection), material (a material), or model (the whole document) */
  category?: "component" | "material" | "model"
  collection?: string
  keywords?: string[]
  /** one palette material to save alone as a material item, as the palette's Save to Library does — not with selection */
  material?: string
  name: string
  /** also return the saved item's bytes base64 */
  return_bytes?: boolean
  /** node ids to save as a component item; omitted saves the whole document as a model item */
  selection?: string[]
  /** display-only "saved from" bookkeeping; defaults to the host's current document path when known (e.g. hew-cli --file), else omitted */
  source_doc?: string
}

export interface LibrarySaveResult {
  /** present only when return_bytes was true */
  bytes_base64?: string
  id: string
  path: string
}

/**
 * `hew.library.update_meta` (v1) — Edit a library item's name, keywords, or collection in place.
 * Tier: Standard · Class: read-only · Served: host
 * Refusals: host_capability_missing, unknown_library_item, save_failed
 */
export interface LibraryUpdateMetaParams {
  collection?: string
  /** the item's id or relative path */
  item: string
  keywords?: string[]
  name?: string
}

export interface LibraryUpdateMetaResult {
  category: "component" | "material" | "model"
  collection?: string
  id?: string
  keywords: string[]
  name: string
  saved_at?: string
}

/**
 * `hew.material.create` (v1) — Add a color or texture material to the palette. Registry-state: records no undo entry (§6.4).
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: load_failed, host_capability_missing
 */
export interface MaterialCreateParams {
  /** required without a texture; with one it tints the image (default white: the image as-is) */
  color?: number[]
  name: string
  /** an image texture: a PNG or JPEG, inline or a file path (hew-cli reads it), and the meters one tile covers */
  texture?: { bytes_base64?: string; path?: string; world_size: [number, number] }
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
 * `hew.material.position_texture` (v1) — Position a face's texture by three tile-corner pins, or reset it to the default projection.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_entity, unknown_object, unknown_face, locator_missed, ambiguous_locator, face_token_unknown, face_token_stale, degenerate_pins
 */
export interface MaterialPositionTextureParams {
  /** a face locator (§5.2) */
  face: UnspecifiedShape
  /** three corners of one texture tile, in the face's object frame (as a locator's `at`), projected onto the face: origin, one tile along the width (u), one tile along the height (v) — exactly one of pins/reset */
  pins?: { origin: [number, number, number]; u: [number, number, number]; v: [number, number, number] }
  /** true: back to the default projection at the material's tile size — exactly one of pins/reset */
  reset?: boolean
}

export interface MaterialPositionTextureResult {}

/**
 * `hew.material.replace` (v1) — Replace one material with another everywhere it is painted, or within one object.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_entity, unknown_object
 */
export interface MaterialReplaceParams {
  /** the material to replace; null: unpainted object bases */
  from: string | null
  /** only within this object; omitted: the whole document */
  object?: string
  /** the material it becomes; null: unpaint */
  to: string | null
}

export interface MaterialReplaceResult {}

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
 * `hew.meta.documents` (v1) — The host's open documents: id, title, path, dirty, and — in the desktop app — label, front, and attached.
 * Tier: Required · Class: read-only · Served: host
 * Refusals: none.
 */
export interface MetaDocumentsParams {}

export interface MetaDocumentsResult {
  documents: ({ attached?: boolean; dirty: boolean; front?: boolean; id: string; label?: string; path: string | null; title: string })[]
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
  documents: ({ attached?: boolean; dirty: boolean; front?: boolean; id: string; label?: string; path: string | null; title: string })[]
  encoding: string
  profile: "core" | "app"
  protocol: number
}

/**
 * `hew.print.pdf` (v1) — Print the document to a PDF the way File ▸ Print… does: standard (one page, the view as-is) or scaled (parallel projection at an exact drawing scale, tiled across pages with overlap bands, crop/trim marks, a scale bar, and a title block). Line art is vector (hidden lines removed); shaded is a software-rasterized bitmap per page. Bytes base64 inline, or written to path.
 * Tier: Standard · Class: solitary · Served: host
 * Refusals: host_capability_missing, nothing_to_render, too_complex, save_failed, unknown_scene
 */
export interface PrintPdfParams {
  /** as hew.view.snapshot; scaled prints use its direction with parallel projection */
  camera?: UnspecifiedShape
  /** line art: dashed hidden lines */
  include_hidden?: boolean
  /** defaults to 12.7 (½ in) */
  margin_mm?: number
  /** crop/trim marks and neighbour labels; defaults to true */
  marks?: boolean
  /** defaults to scaled */
  mode?: "scaled" | "standard"
  /** defaults to auto (fewer tiles; standard follows the view's aspect) */
  orientation?: "auto" | "portrait" | "landscape"
  /** tile overlap band, defaults to 10; 0 for none */
  overlap_mm?: number
  /** "letter" | "legal" | "tabloid" | "a5" | "a4" | "a3" (default a4) or {w_mm, h_mm} */
  paper?: string | { h_mm: number; w_mm: number }
  /** write the PDF here instead of returning it inline */
  path?: string
  /** scaled: paper/model, defaults to 0.1 (1:10) */
  scale?: number
  /** defaults to true */
  scale_bar?: boolean
  /** title-block scale text; defaults to the ratio ("1:10") */
  scale_label?: string
  scene?: string
  /** defaults to line_art (vector) */
  style?: "line_art" | "shaded"
  /** title-block document name; defaults to the document's */
  title?: string
  /** defaults to true */
  title_block?: boolean
  /** scale-bar family, defaults to metric */
  units?: "metric" | "imperial"
  view?: "iso" | "front" | "back" | "left" | "right" | "top" | "bottom"
}

export interface PrintPdfResult {
  cols: number
  pages: number
  path?: string
  /** present only when path was not given */
  pdf_base64?: string
  rows: number
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
  camera?: { eye: [number, number, number]; fov_deg?: number; parallel_height?: number; projection?: "perspective" | "parallel"; target: [number, number, number]; up?: [number, number, number] }
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
  camera?: { eye: [number, number, number]; fov_deg: number; parallel_height: number; projection: "perspective" | "parallel"; target: [number, number, number]; up: [number, number, number] }
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
  scenes: ({ camera?: { eye: [number, number, number]; fov_deg: number; parallel_height: number; projection: "perspective" | "parallel"; target: [number, number, number]; up: [number, number, number] }; description: string; display?: { axes: boolean; grid: boolean; guides: boolean }; id: string; name: string; props: { camera: boolean; display: boolean; hidden_nodes: boolean; hidden_tags: boolean; section: boolean }; section?: null | { active: boolean; normal: [number, number, number]; origin: [number, number, number] }; sid: number; view_style?: { axes: boolean; background: [number, number, number]; ground: [number, number, number] | null; sky: [number, number, number] | null } })[]
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
  camera?: { eye: [number, number, number]; fov_deg?: number; parallel_height?: number; projection?: "perspective" | "parallel"; target: [number, number, number]; up?: [number, number, number] }
  /** opaque editor display toggles: stored and returned, never interpreted by the kernel */
  display?: { axes: boolean; grid: boolean; guides: boolean }
  id: string
  /** which of the five capturable properties to (re-)capture; each defaults to true */
  properties?: { camera?: boolean; display?: boolean; hidden_nodes?: boolean; hidden_tags?: boolean; section?: boolean }
}

export interface ScenesUpdateResult {}

/**
 * `hew.sketch.copy` (v1) — Copy drawing units on a plane by an offset.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_unit, draft_boundary_edge_touched, unknown_sketch, unknown_entity, locator_missed, ambiguous_locator, point_off_plane, degenerate_segment
 */
export interface SketchCopyParams {
  offset: [number, number, number]
  /** HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>} */
  plane: UnspecifiedShape
  /** unit keys from hew.sketch.units: isl_<hex16> (an island: all its edges), edg_<hex16> (one edge), crv_<hex16> (every facet of one circle), rgn_<hex16> (a region's ring edges that are drawing edges) */
  units: string[]
}

export interface SketchCopyResult {
  /** the plane units the call touched: every drawing edge lying along what was drawn, then every region whose boundary carries it */
  units: ({ key: string; kind: "edge" | "region"; object_id?: string; sketch_id?: string; store: "topology" | "owned" | "free" })[]
}

/**
 * `hew.sketch.delete` (v1) — Delete drawing units from a plane.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_unit, draft_boundary_edge_touched, unknown_sketch, unknown_entity, locator_missed, ambiguous_locator, point_off_plane, degenerate_segment
 */
export interface SketchDeleteParams {
  /** HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>} */
  plane: UnspecifiedShape
  /** unit keys from hew.sketch.units: isl_<hex16> (an island: all its edges), edg_<hex16> (one edge), crv_<hex16> (every facet of one circle), rgn_<hex16> (a region's ring edges that are drawing edges) */
  units: string[]
}

export interface SketchDeleteResult {
  /** the plane units the call touched: every drawing edge lying along what was drawn, then every region whose boundary carries it */
  units: ({ key: string; kind: "edge" | "region"; object_id?: string; sketch_id?: string; store: "topology" | "owned" | "free" })[]
}

/**
 * `hew.sketch.draw_arc` (v1) — Draw an arc on a plane spec.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: point_off_plane, degenerate_curve, degenerate_segment, unknown_sketch, segments_above_cap, unknown_entity, locator_missed, ambiguous_locator, curve_claim_off_loop, unknown_object, unknown_component, unknown_face, point_not_on_face, would_corrupt
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

export type SketchDrawArcResult = { curve_id?: string; region_id?: string; region_ids: string[]; sketch: string } | { object_id: string; units: ({ key: string; kind: "edge" | "region"; object_id?: string; sketch_id?: string; store: "topology" | "owned" | "free" })[] }

/**
 * `hew.sketch.draw_circle` (v1) — Draw a circle on a plane spec.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: point_off_plane, degenerate_curve, degenerate_segment, unknown_sketch, segments_below_floor, segments_above_cap, unknown_entity, locator_missed, ambiguous_locator, curve_claim_off_loop, unknown_object, unknown_component, unknown_face, point_not_on_face, would_corrupt
 */
export interface SketchDrawCircleParams {
  center: [number, number, number] | UnspecifiedShape
  /** HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>} */
  plane: UnspecifiedShape
  radius: number
  /** facet count; defaults to 48, must fall in [MIN_CIRCLE_SEGMENTS, MAX_CIRCLE_SEGMENTS] = [24, 1024] */
  segments?: number
}

export type SketchDrawCircleResult = { curve_id?: string; region_id?: string; region_ids: string[]; sketch: string } | { object_id: string; units: ({ key: string; kind: "edge" | "region"; object_id?: string; sketch_id?: string; store: "topology" | "owned" | "free" })[] }

/**
 * `hew.sketch.draw_line` (v1) — Draw a line (chain) on a plane spec.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: point_off_plane, degenerate_segment, unknown_sketch, unknown_entity, locator_missed, ambiguous_locator, unknown_object, unknown_component, unknown_face, point_not_on_face, would_corrupt
 */
export interface SketchDrawLineParams {
  /** HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>} */
  plane: UnspecifiedShape
  points: ([number, number, number] | UnspecifiedShape)[]
}

export type SketchDrawLineResult = { curve_id?: string; region_id?: string; region_ids: string[]; sketch: string } | { object_id: string; units: ({ key: string; kind: "edge" | "region"; object_id?: string; sketch_id?: string; store: "topology" | "owned" | "free" })[] }

/**
 * `hew.sketch.draw_polygon` (v1) — Draw a regular N-gon on a plane spec.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: point_off_plane, degenerate_curve, degenerate_segment, unknown_sketch, unknown_entity, locator_missed, ambiguous_locator, sides_above_cap, unknown_object, unknown_component, unknown_face, point_not_on_face, would_corrupt
 */
export interface SketchDrawPolygonParams {
  center: [number, number, number] | UnspecifiedShape
  /** HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>} */
  plane: UnspecifiedShape
  radius: number
  sides: number
}

export type SketchDrawPolygonResult = { curve_id?: string; region_id?: string; region_ids: string[]; sketch: string } | { object_id: string; units: ({ key: string; kind: "edge" | "region"; object_id?: string; sketch_id?: string; store: "topology" | "owned" | "free" })[] }

/**
 * `hew.sketch.draw_rect` (v1) — Draw an axis-aligned rectangle on a plane spec.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: point_off_plane, degenerate_segment, unknown_sketch, unknown_entity, locator_missed, ambiguous_locator, unknown_object, unknown_component, unknown_face, point_not_on_face, would_corrupt
 */
export interface SketchDrawRectParams {
  corner_a: [number, number, number] | UnspecifiedShape
  corner_b: [number, number, number] | UnspecifiedShape
  /** HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>} */
  plane: UnspecifiedShape
}

export type SketchDrawRectResult = { curve_id?: string; region_id?: string; region_ids: string[]; sketch: string } | { object_id: string; units: ({ key: string; kind: "edge" | "region"; object_id?: string; sketch_id?: string; store: "topology" | "owned" | "free" })[] }

/**
 * `hew.sketch.move` (v1) — Move drawing units (islands, edges, circles, regions) on a plane by an offset.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_unit, draft_boundary_edge_touched, unknown_sketch, unknown_entity, locator_missed, ambiguous_locator, point_off_plane, degenerate_segment
 */
export interface SketchMoveParams {
  /** translation [x, y, z] in meters; must stay in the plane */
  offset: [number, number, number]
  /** HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>} */
  plane: UnspecifiedShape
  /** unit keys from hew.sketch.units: isl_<hex16> (an island: all its edges), edg_<hex16> (one edge), crv_<hex16> (every facet of one circle), rgn_<hex16> (a region's ring edges that are drawing edges) */
  units: string[]
}

export interface SketchMoveResult {
  /** the plane units the call touched: every drawing edge lying along what was drawn, then every region whose boundary carries it */
  units: ({ key: string; kind: "edge" | "region"; object_id?: string; sketch_id?: string; store: "topology" | "owned" | "free" })[]
}

/**
 * `hew.sketch.offset` (v1) — Offset a region boundary within its sketch, or a face or plane region by its unit key.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: unknown_region, malformed_region, offset_too_small, offset_collapsed, unknown_sketch, unknown_entity, unknown_unit, draft_boundary_edge_touched
 */
export interface SketchOffsetParams {
  /** positive grows the material, negative shrinks it */
  distance: number
  /** HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>} */
  plane?: UnspecifiedShape
  /** a sketch region id; with `plane`, a region unit key rgn_<hex16> from hew.sketch.units (a face or plane region) */
  region: string
}

export type SketchOffsetResult = { curve_ids: string[]; region_id?: string; region_ids: string[]; sketch: string } | { units: ({ key: string; kind: "edge" | "region"; object_id?: string; sketch_id?: string; store: "topology" | "owned" | "free" })[] }

/**
 * `hew.sketch.rotate` (v1) — Rotate drawing units on a plane about the plane normal through a point.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_unit, draft_boundary_edge_touched, unknown_sketch, unknown_entity, locator_missed, ambiguous_locator, point_off_plane, degenerate_segment
 */
export interface SketchRotateParams {
  /** radians, counter-clockwise seen from the plane normal side */
  angle: number
  axis_point: [number, number, number] | UnspecifiedShape
  /** HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>} */
  plane: UnspecifiedShape
  /** unit keys from hew.sketch.units: isl_<hex16> (an island: all its edges), edg_<hex16> (one edge), crv_<hex16> (every facet of one circle), rgn_<hex16> (a region's ring edges that are drawing edges) */
  units: string[]
}

export interface SketchRotateResult {
  /** the plane units the call touched: every drawing edge lying along what was drawn, then every region whose boundary carries it */
  units: ({ key: string; kind: "edge" | "region"; object_id?: string; sketch_id?: string; store: "topology" | "owned" | "free" })[]
}

/**
 * `hew.sketch.scale` (v1) — Scale drawing units on a plane about a center, uniformly or per axis.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_unit, draft_boundary_edge_touched, unknown_sketch, unknown_entity, locator_missed, ambiguous_locator, point_off_plane, degenerate_segment
 */
export interface SketchScaleParams {
  center: [number, number, number] | UnspecifiedShape
  /** uniform scale; give this or `factors` */
  factor?: number
  /** per-axis [sx, sy, sz] in world axes; must keep the drawing in its plane */
  factors?: [number, number, number]
  /** HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>} */
  plane: UnspecifiedShape
  /** unit keys from hew.sketch.units: isl_<hex16> (an island: all its edges), edg_<hex16> (one edge), crv_<hex16> (every facet of one circle), rgn_<hex16> (a region's ring edges that are drawing edges) */
  units: string[]
}

export interface SketchScaleResult {
  /** the plane units the call touched: every drawing edge lying along what was drawn, then every region whose boundary carries it */
  units: ({ key: string; kind: "edge" | "region"; object_id?: string; sketch_id?: string; store: "topology" | "owned" | "free" })[]
}

/**
 * `hew.sketch.units` (v1) — List a plane's drawing as units: islands of edges (with where each is stored) and regions.
 * Tier: Standard · Class: read-only · Served: kernel
 * Refusals: unknown_sketch, unknown_entity, locator_missed, ambiguous_locator
 */
export interface SketchUnitsParams {
  /** HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>} */
  plane: UnspecifiedShape
}

export interface SketchUnitsResult {
  regions: { extrudable: boolean; face?: UnspecifiedShape; holes: [number, number, number][][]; key: string; object_id?: string; outer: [number, number, number][] }[]
  units: ({ edges: ({ a: [number, number, number]; b: [number, number, number]; curve?: { center?: [number, number, number]; key?: string; radius?: number }; key: string; object_id?: string; sketch_id?: string; store: "topology" | "owned" | "free" })[]; key: string })[]
}

/**
 * `hew.solid.cap` (v1) — Close an open object's flat openings with faces; an object closed all round becomes a solid.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: object_is_solid, unknown_component, unknown_entity, explode_session_scope, invalid, unknown_rim, rim_not_planar, rim_self_intersecting, cap_would_be_coplanar, non_manifold_rim, caps_overlap
 */
export interface SolidCapParams {
  /** obj_ id (a world object or a component member) */
  object: string
  /** indices into the object's hew.doc.health rims; omitted = every cappable rim (when none is, all of them, so the refusal says why) */
  rims?: number[]
}

export interface SolidCapResult {
  /** the object is now a solid */
  closed: boolean
  /** faces added (face tokens cap.1, cap.2, ... in a transaction) */
  faces: number
  object_id: string
  /** an island the caps closed was inside out, so that island's faces were turned over (a cavity inside another island is left as is) */
  reversed: boolean
}

/**
 * `hew.solid.delete_faces` (v1) — Remove faces from an open object; islands cut loose become objects of their own. Solids refuse.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: object_is_solid, unknown_component, unknown_entity, explode_session_scope, invalid, unknown_face, last_definition_member, locator_missed, ambiguous_locator, face_token_unknown, face_token_stale
 */
export interface SolidDeleteFacesParams {
  /** faces of that object; omitted = every face, which deletes the object (a component member too, unless it is its component's last object: last_definition_member) */
  faces?: UnspecifiedShape[]
  object: string
}

export interface SolidDeleteFacesResult {
  /** every face went, so the object was deleted */
  deleted: boolean
  object_id: string
  /** islands cut loose, now objects of their own */
  object_ids: string[]
}

/**
 * `hew.solid.delete_imprint` (v1) — Kept for compatibility: delete a drawn shape (sub-face or chord island) back into its face; hew.sketch.delete is the general form.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: not_in_plane, not_an_imprint, draft_boundary_edge_touched, unknown_object, unknown_entity, locator_missed, ambiguous_locator, face_token_unknown, face_token_stale
 */
export interface SolidDeleteImprintParams {
  /** HEW_API.md §5.2 face locator ({object,at} | {object,ray} | {"$face":"label#key"}) naming a sub-face; {object,at} also picks a CHORD when the point lies on one of its lines (a chord is tried first, then the sub-face containing the point) */
  imprint: UnspecifiedShape
}

export interface SolidDeleteImprintResult {
  object_id: string
}

/**
 * `hew.solid.extrude` (v1) — Extrude a region into a new Object, consuming the profile.
 * Tier: Required · Class: model-mutating · Served: kernel
 * Refusals: distance_too_small, degenerate_geometry, unknown_region, region_has_no_face, unknown_entity, unknown_unit, region_not_pushable, region_touches_hole
 */
export interface SolidExtrudeParams {
  distance: number
  /** HEW_API.md §7 plane spec naming the drafting context a unit-key `region` belongs to. A region over no face extrudes as a new Object; a region that is a face of a solid push/pulls that face (result as hew.solid.push_pull); a region over a face that cannot hold it refuses region_not_pushable */
  plane?: UnspecifiedShape
  /** a sketch region id; with `plane`, a region unit key rgn_<hex16> from hew.sketch.units */
  region: string
}

export interface SolidExtrudeResult {
  object_id?: string
  /** a through-cut's resulting pieces (unit-key form over a face, as hew.solid.push_pull) */
  object_ids?: string[]
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
 * `hew.solid.imprints` (v1) — Kept for compatibility: list an object's drawn shapes (sub-faces and chords) per drawing plane; hew.sketch.units is the general form.
 * Tier: Standard · Class: read-only · Served: kernel
 * Refusals: unknown_object, unknown_entity
 */
export interface SolidImprintsParams {
  object: string
}

export interface SolidImprintsResult {
  /** drawn circles and arcs (whole or cut by other drawing into several runs) on the object's drawing planes: each {closed, curve: {center, radius}, runs: [{at, path}]}, a run that is a chord above carrying that chord's at locator — the whole drawn curve, a circle when closed, else an arc */
  curve_units: { closed: boolean; curve: UnspecifiedShape; runs: UnspecifiedShape[] }[]
  imprints: ({ at: [number, number, number]; curve?: UnspecifiedShape; curves?: unknown[]; holes?: unknown[][]; kind: "sub_face" | "chord"; loop?: unknown[][]; nested?: number; normal?: number[]; path?: unknown[][]; region?: string; unit?: string })[]
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
 * `hew.solid.mark_surface` (v1) — Mark an open object as a surface on purpose (or clear the mark); hew.doc.health keeps listing it, flagged accepted.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: object_is_solid, unknown_entity
 */
export interface SolidMarkSurfaceParams {
  /** false clears the mark */
  accepted?: boolean
  object: string
}

export interface SolidMarkSurfaceResult {
  accepted: boolean
  object_id: string
}

/**
 * `hew.solid.move_imprint` (v1) — Kept for compatibility: slide a drawn shape (sub-face or chord island) in its plane; hew.sketch.move is the general form.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: not_in_plane, not_an_imprint, draft_boundary_edge_touched, unknown_object, unknown_entity, locator_missed, ambiguous_locator, face_token_unknown, face_token_stale
 */
export interface SolidMoveImprintParams {
  /** HEW_API.md §5.2 face locator ({object,at} | {object,ray} | {"$face":"label#key"}) naming a sub-face; {object,at} also picks a CHORD when the point lies on one of its lines (a chord is tried first, then the sub-face containing the point) */
  imprint: UnspecifiedShape
  offset: [number, number, number]
}

export interface SolidMoveImprintResult {
  object_id: string
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
 * `hew.solid.rotate_imprint` (v1) — Kept for compatibility: turn a drawn shape about its plane's normal through a pivot; hew.sketch.rotate is the general form.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: not_in_plane, not_an_imprint, draft_boundary_edge_touched, unknown_object, unknown_entity, locator_missed, ambiguous_locator, face_token_unknown, face_token_stale, no_such_point
 */
export interface SolidRotateImprintParams {
  about: [number, number, number] | UnspecifiedShape
  /** radians */
  angle: number
  /** HEW_API.md §5.2 face locator ({object,at} | {object,ray} | {"$face":"label#key"}) naming a sub-face; {object,at} also picks a CHORD when the point lies on one of its lines (a chord is tried first, then the sub-face containing the point) */
  imprint: UnspecifiedShape
}

export interface SolidRotateImprintResult {
  object_id: string
}

/**
 * `hew.solid.scale_imprint` (v1) — Kept for compatibility: uniformly scale a drawn shape about an anchor in its plane; hew.sketch.scale is the general form.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: not_in_plane, not_an_imprint, draft_boundary_edge_touched, unknown_object, unknown_entity, locator_missed, ambiguous_locator, face_token_unknown, face_token_stale, no_such_point
 */
export interface SolidScaleImprintParams {
  about: [number, number, number] | UnspecifiedShape
  /** uniform scale factor */
  factor: number
  /** HEW_API.md §5.2 face locator ({object,at} | {object,ray} | {"$face":"label#key"}) naming a sub-face; {object,at} also picks a CHORD when the point lies on one of its lines (a chord is tried first, then the sub-face containing the point) */
  imprint: UnspecifiedShape
}

export interface SolidScaleImprintResult {
  object_id: string
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
 * Refusals: boolean_operand_has_instance, boolean_operand_not_solid, boolean_operand_empty, grouped_operand, degenerate_contact, unknown_object, unknown_group, unknown_instance, unknown_entity, nothing_removed
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
 * `hew.solid.thicken` (v1) — Pull a flat surface into a solid of the given thickness.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: object_is_solid, unknown_component, unknown_entity, explode_session_scope, invalid, not_a_flat_sheet, thickness_too_small
 */
export interface SolidThickenParams {
  /** split the thickness evenly on both sides of the surface */
  centered?: boolean
  /** meters along each face's front normal (negative: behind it) */
  distance: number
  object: string
}

export interface SolidThickenResult {
  /** the surface, now a solid */
  object_id: string
  /** further separate pieces of the surface, each now a solid of its own */
  object_ids: string[]
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
 * `hew.tag.rename` (v1) — Rename a tag path (and every tag nested under it), keeping its identity.
 * Tier: Standard · Class: model-mutating · Served: kernel
 * Refusals: unknown_tag, duplicate_tag, invalid_tag_path
 */
export interface TagRenameParams {
  new_path: string[]
  path: string[]
}

export interface TagRenameResult {}

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
  camera?: { eye: [number, number, number]; fov_deg?: number; parallel_height?: number; projection?: "perspective" | "parallel"; target: [number, number, number]; up?: [number, number, number] }
  /** a named standard view; mutually exclusive with camera */
  view?: "iso" | "front" | "back" | "left" | "right" | "top" | "bottom"
}

export interface ViewCameraResult {}

/**
 * `hew.view.line_drawing` (v1) — Hidden-line drawing of the visible document from a camera (crates/hlr): hard edges, curved-wall silhouettes, section-cut outlines, optionally dashed hidden lines — as a true-size SVG at a drawing scale (inline or written to path), or as raw segments in view-plane meters.
 * Tier: Standard · Class: solitary · Served: host
 * Refusals: host_capability_missing, nothing_to_render, too_complex, save_failed, unknown_scene
 */
export interface ViewLineDrawingParams {
  /** identical vocabulary to hew.view.snapshot's camera; mutually exclusive with view and scene */
  camera?: UnspecifiedShape
  /** defaults to svg */
  format?: "svg" | "segments"
  /** defaults to false; when true, hidden pieces are returned too (kind "hidden", dashed in SVG) */
  include_hidden?: boolean
  /** defaults to false; when true, non-silhouette curved-wall facet seams are returned (kind "soft") */
  include_soft?: boolean
  /** svg only: write the file here instead of returning it inline (hosts with filesystem access) */
  path?: string
  /** svg only: paper/model drawing scale, defaults to 1 (full size); the SVG's width/height are millimeters at that scale */
  scale?: number
  /** a Scene's id: its camera and hidden sets, as hew.view.snapshot */
  scene?: string
  /** a named standard view (parallel projection); mutually exclusive with camera and scene */
  view?: "iso" | "front" | "back" | "left" | "right" | "top" | "bottom"
}

export interface ViewLineDrawingResult {
  /** [min_x, min_y, max_x, max_y] in view-plane meters; null when empty */
  bounds?: [number, number, number, number]
  count: number
  /** public id of the entity each segment belongs to */
  ids?: string[]
  kinds?: ("hard" | "silhouette" | "soft" | "section" | "hidden")[]
  /** format svg with path */
  path?: string
  /** format segments: [ax, ay, bx, by] in view-plane meters, y up, origin at the camera target */
  segments?: [number, number, number, number][]
  /** format svg without path */
  svg?: string
}

/**
 * `hew.view.snapshot` (v1) — Render the attached document to PNG, headless-rendered via a software rasterizer (a live host may render through its viewport instead) — bytes base64 by default, or a path on hosts with filesystem access.
 * Tier: Standard · Class: solitary · Served: host
 * Refusals: host_capability_missing, nothing_to_render, save_failed, unknown_scene
 */
export interface ViewSnapshotParams {
  /** mutually exclusive with view */
  camera?: { eye: [number, number, number]; fov_deg?: number; parallel_height?: number; projection?: "perspective" | "parallel"; target: [number, number, number]; up?: [number, number, number] }
  /** bits per pixel of the depth PNG (default 8); only with include_depth */
  depth_bits?: 8 | 16
  /** defaults to 512; out-of-range values are clamped, not refused */
  height?: number
  /** defaults to false; when true, also returns a depth pass as a grayscale PNG — NEAR is WHITE, background 0 — with depth_near/depth_far (meters from the eye along the view axis) so absolute depth is recoverable: depth = depth_far - (v - 1) / (max - 1) * (depth_far - depth_near), max = 255 or 65535. The conditioning input depth-aware image models expect */
  include_depth?: boolean
  /** defaults to false; when true, also returns a per-pixel id-buffer and its palette */
  include_ids?: boolean
  /** defaults to false; when true, also returns a camera-space normals pass as an RGB PNG: (n * 0.5 + 0.5) * 255 per axis, x right, y up, z toward the viewer, background (128, 128, 255) */
  include_normals?: boolean
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
  /** with a depth pass: 8 or 16 */
  depth_bits?: number
  /** with a depth pass: the farthest drawn surface */
  depth_far?: number
  /** with a depth pass: meters from the eye, along the view axis, of the nearest drawn surface */
  depth_near?: number
  /** present only when include_depth and path were both given: "<path>.depth.png" */
  depth_path?: string
  /** present only when include_depth was true and path was not given */
  depth_png_base64?: string
  height: number
  /** present only when include_ids was true and path was not given: u16 little-endian per pixel, index into id_palette (0 = background) */
  id_buffer_base64?: string
  /** present only when include_ids and path were both given: "<path>.ids.bin", the same u16 little-endian per-pixel encoding written to disk */
  id_buffer_path?: string
  /** public ids; id_palette[i] is what the id-buffer (inline or on disk) reports as index i+1 */
  id_palette?: string[]
  /** present only when include_normals and path were both given: "<path>.normals.png" */
  normals_path?: string
  /** present only when include_normals was true and path was not given */
  normals_png_base64?: string
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
    close: (params: DocCloseParams): Promise<DocCloseResult> => this.call('hew.doc.close', params),
    export: (params: DocExportParams): Promise<DocExportResult> => this.call('hew.doc.export', params),
    focus: (params: DocFocusParams): Promise<DocFocusResult> => this.call('hew.doc.focus', params),
    health: (params: DocHealthParams): Promise<DocHealthResult> => this.call('hew.doc.health', params),
    import: (params: DocImportParams): Promise<DocImportResult> => this.call('hew.doc.import', params),
    new: (params: DocNewParams): Promise<DocNewResult> => this.call('hew.doc.new', params),
    open: (params: DocOpenParams): Promise<DocOpenResult> => this.call('hew.doc.open', params),
    purgeUnused: (params: DocPurgeUnusedParams): Promise<DocPurgeUnusedResult> => this.mutate('hew.doc.purge_unused', params),
    revert: (params: DocRevertParams): Promise<DocRevertResult> => this.call('hew.doc.revert', params),
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
    reparent: (params: GroupReparentParams): Promise<GroupReparentResult> => this.mutate('hew.group.reparent', params),
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

  readonly library = {
    describe: (params: LibraryDescribeParams): Promise<LibraryDescribeResult> => this.call('hew.library.describe', params),
    insert: (params: LibraryInsertParams): Promise<LibraryInsertResult> => this.mutate('hew.library.insert', params),
    list: (params: LibraryListParams): Promise<LibraryListResult> => this.call('hew.library.list', params),
    remove: (params: LibraryRemoveParams): Promise<LibraryRemoveResult> => this.call('hew.library.remove', params),
    save: (params: LibrarySaveParams): Promise<LibrarySaveResult> => this.call('hew.library.save', params),
    updateMeta: (params: LibraryUpdateMetaParams): Promise<LibraryUpdateMetaResult> => this.call('hew.library.update_meta', params),
  }

  readonly material = {
    create: (params: MaterialCreateParams): Promise<MaterialCreateResult> => this.mutate('hew.material.create', params),
    paint: (params: MaterialPaintParams): Promise<MaterialPaintResult> => this.mutate('hew.material.paint', params),
    positionTexture: (params: MaterialPositionTextureParams): Promise<MaterialPositionTextureResult> => this.mutate('hew.material.position_texture', params),
    replace: (params: MaterialReplaceParams): Promise<MaterialReplaceResult> => this.mutate('hew.material.replace', params),
    setDefault: (params: MaterialSetDefaultParams): Promise<MaterialSetDefaultResult> => this.mutate('hew.material.set_default', params),
    setOpacity: (params: MaterialSetOpacityParams): Promise<MaterialSetOpacityResult> => this.mutate('hew.material.set_opacity', params),
  }

  readonly meta = {
    capabilities: (params: MetaCapabilitiesParams): Promise<MetaCapabilitiesResult> => this.call('hew.meta.capabilities', params),
    documents: (params: MetaDocumentsParams): Promise<MetaDocumentsResult> => this.call('hew.meta.documents', params),
    hello: (params: MetaHelloParams): Promise<MetaHelloResult> => this.call('hew.meta.hello', params),
  }

  readonly print = {
    pdf: (params: PrintPdfParams): Promise<PrintPdfResult> => this.call('hew.print.pdf', params),
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
    copy: (params: SketchCopyParams): Promise<SketchCopyResult> => this.mutate('hew.sketch.copy', params),
    delete: (params: SketchDeleteParams): Promise<SketchDeleteResult> => this.mutate('hew.sketch.delete', params),
    drawArc: (params: SketchDrawArcParams): Promise<SketchDrawArcResult> => this.mutate('hew.sketch.draw_arc', params),
    drawCircle: (params: SketchDrawCircleParams): Promise<SketchDrawCircleResult> => this.mutate('hew.sketch.draw_circle', params),
    drawLine: (params: SketchDrawLineParams): Promise<SketchDrawLineResult> => this.mutate('hew.sketch.draw_line', params),
    drawPolygon: (params: SketchDrawPolygonParams): Promise<SketchDrawPolygonResult> => this.mutate('hew.sketch.draw_polygon', params),
    drawRect: (params: SketchDrawRectParams): Promise<SketchDrawRectResult> => this.mutate('hew.sketch.draw_rect', params),
    move: (params: SketchMoveParams): Promise<SketchMoveResult> => this.mutate('hew.sketch.move', params),
    offset: (params: SketchOffsetParams): Promise<SketchOffsetResult> => this.mutate('hew.sketch.offset', params),
    rotate: (params: SketchRotateParams): Promise<SketchRotateResult> => this.mutate('hew.sketch.rotate', params),
    scale: (params: SketchScaleParams): Promise<SketchScaleResult> => this.mutate('hew.sketch.scale', params),
    units: (params: SketchUnitsParams): Promise<SketchUnitsResult> => this.call('hew.sketch.units', params),
  }

  readonly solid = {
    cap: (params: SolidCapParams): Promise<SolidCapResult> => this.mutate('hew.solid.cap', params),
    deleteFaces: (params: SolidDeleteFacesParams): Promise<SolidDeleteFacesResult> => this.mutate('hew.solid.delete_faces', params),
    deleteImprint: (params: SolidDeleteImprintParams): Promise<SolidDeleteImprintResult> => this.mutate('hew.solid.delete_imprint', params),
    extrude: (params: SolidExtrudeParams): Promise<SolidExtrudeResult> => this.mutate('hew.solid.extrude', params),
    followMe: (params: SolidFollowMeParams): Promise<SolidFollowMeResult> => this.mutate('hew.solid.follow_me', params),
    imprints: (params: SolidImprintsParams): Promise<SolidImprintsResult> => this.call('hew.solid.imprints', params),
    intersect: (params: SolidIntersectParams): Promise<SolidIntersectResult> => this.mutate('hew.solid.intersect', params),
    markSurface: (params: SolidMarkSurfaceParams): Promise<SolidMarkSurfaceResult> => this.mutate('hew.solid.mark_surface', params),
    moveImprint: (params: SolidMoveImprintParams): Promise<SolidMoveImprintResult> => this.mutate('hew.solid.move_imprint', params),
    pushPull: (params: SolidPushPullParams): Promise<SolidPushPullResult> => this.mutate('hew.solid.push_pull', params),
    rotateImprint: (params: SolidRotateImprintParams): Promise<SolidRotateImprintResult> => this.mutate('hew.solid.rotate_imprint', params),
    scaleImprint: (params: SolidScaleImprintParams): Promise<SolidScaleImprintResult> => this.mutate('hew.solid.scale_imprint', params),
    slice: (params: SolidSliceParams): Promise<SolidSliceResult> => this.mutate('hew.solid.slice', params),
    subtract: (params: SolidSubtractParams): Promise<SolidSubtractResult> => this.mutate('hew.solid.subtract', params),
    thicken: (params: SolidThickenParams): Promise<SolidThickenResult> => this.mutate('hew.solid.thicken', params),
    union: (params: SolidUnionParams): Promise<SolidUnionResult> => this.mutate('hew.solid.union', params),
  }

  readonly tag = {
    assign: (params: TagAssignParams): Promise<TagAssignResult> => this.mutate('hew.tag.assign', params),
    create: (params: TagCreateParams): Promise<TagCreateResult> => this.mutate('hew.tag.create', params),
    delete: (params: TagDeleteParams): Promise<TagDeleteResult> => this.mutate('hew.tag.delete', params),
    rename: (params: TagRenameParams): Promise<TagRenameResult> => this.mutate('hew.tag.rename', params),
    setVisible: (params: TagSetVisibleParams): Promise<TagSetVisibleResult> => this.mutate('hew.tag.set_visible', params),
  }

  readonly view = {
    camera: (params: ViewCameraParams): Promise<ViewCameraResult> => this.call('hew.view.camera', params),
    lineDrawing: (params: ViewLineDrawingParams): Promise<ViewLineDrawingResult> => this.call('hew.view.line_drawing', params),
    snapshot: (params: ViewSnapshotParams): Promise<ViewSnapshotResult> => this.call('hew.view.snapshot', params),
    units: (params: ViewUnitsParams): Promise<ViewUnitsResult> => this.call('hew.view.units', params),
    zoomExtents: (params: ViewZoomExtentsParams): Promise<ViewZoomExtentsResult> => this.call('hew.view.zoom_extents', params),
  }
}
