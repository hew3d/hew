// Shared types of the Hew Library feature: the item model the browser,
// placement flow, save flow, and storage seam all speak. Pure types — no
// I/O, no React.

/** The curatorial category of a library item. One mechanism, three
 * categories: every item is a `.hew` file; the category only decides where
 * it lists and which action is primary (Insert for components/materials,
 * Open for models). */
export type LibraryCategory = 'component' | 'material' | 'model'

/** The `hew.library` metadata carried in an item file's own document
 * attribute dictionary — the single source of truth (no separate database).
 * All fields optional: a bare `.hew` dropped into the library folder is a
 * valid item with everything derived. */
export interface LibraryItemMeta {
  /** Stable library identity (UUID minted at save). Drives idempotent
   * re-insert and "in this model" matching. */
  id?: string
  /** Display name. Falls back to the item's own definition/root name, then
   * the file name. */
  name?: string
  category?: LibraryCategory
  keywords?: string[]
  /** User collection name ("" / absent = uncollected). */
  collection?: string
  /** ISO-8601 save timestamp. */
  savedAt?: string
  /** The document the item was saved out of (display only). */
  sourceDoc?: string
}

/** One material row of a manifest summary (mirrors the wasm
 * `read_item_summary_json` shape). */
export interface LibraryMaterialSummary {
  name: string
  /** RGBA 0-255; alpha is opacity. */
  color: [number, number, number, number]
  texture_asset: string | null
  texture_format: string | null
  texture_world_size: [number, number] | null
  /** Kernel content hash as a decimal string — compares equal iff the
   * palette would deduplicate the two materials (the cross-window
   * "in palette" key). */
  content_hash: string
}

/** The manifest-only summary of an item file (wasm
 * `read_item_summary_json`, parsed). Cheap: never decodes geometry. */
export interface LibraryItemSummary {
  format_version: number
  objects: number
  materials: number
  components: number
  instances: number
  groups: number
  world_sketches: number
  annotations: number
  guides: number
  first_component_name: string | null
  /** First definition's stable id (decimal string) — `stamp_library_source`'s
   * `def_sid` for marking a saved item's source definition. */
  first_component_sid: string | null
  first_root_name: string | null
  /** namespace → key → JSON value; `hew.library` metadata lives here. */
  doc_attrs: Record<string, Record<string, unknown>>
  material_entries: LibraryMaterialSummary[]
}

/** A file in the library folder, as the storage seam lists it. */
export interface LibraryFileEntry {
  /** File name inside the library folder, including `.hew`. */
  fileName: string
  /** Size in bytes. */
  size: number
  /** Last-modified, ms since epoch (0 when the backend can't say). */
  mtimeMs: number
}

/** A fully-listed library item: file entry + parsed summary + derived
 * fields. Built by the library model from the storage seam's raw parts. */
export interface LibraryItem {
  file: LibraryFileEntry
  summary: LibraryItemSummary
  meta: LibraryItemMeta
  /** Resolved display name (meta.name → item's own names → file stem). */
  displayName: string
  /** Resolved category (meta.category → derived: material-only file =
   * material; single def+instance = component; else model). */
  category: LibraryCategory
  /** SHA-256 hex of the file bytes — the provenance content hash AND the
   * thumbnail cache key. */
  contentHash: string
  /** Files that failed to parse list as errored so the browser can show a
   * typed error state instead of silently hiding them. */
  error?: string
}
