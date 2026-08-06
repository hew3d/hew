/**
 * itemFiles — async, typed wrapper over the wasm free functions that read
 * and write standalone `.hew` library item bytes (`read_item_summary_json`,
 * `render_item_thumbnail`, `update_item_meta`, `read_item_asset`).
 *
 * The wasm module must be initialized (`wasm/loader.ts`'s `loadKernel()`)
 * before any of these free functions can be called; every export here
 * awaits that once, lazily, so callers never have to think about init
 * ordering. `sha256Hex` needs no wasm at all — it's the browser's own
 * SubtleCrypto — but lives here because it's the third leg of the "identify
 * and read a library item file" trio the browser modal needs on every load.
 */

import { loadKernel } from '../wasm/loader'
import {
  read_item_asset,
  read_item_summary_json,
  render_item_thumbnail,
  update_item_meta,
} from '../wasm/pkg/wasm_api.js'
import type { LibraryItemMeta, LibraryItemSummary } from './types'

let readyPromise: Promise<void> | null = null

/** Ensures the wasm module is initialized exactly once, no matter how many
 * call sites await it concurrently (mirrors `wasm/loader.ts`'s own
 * memoization of `getInitPromise`). */
function ready(): Promise<void> {
  if (readyPromise === null) {
    readyPromise = loadKernel().then(() => undefined)
  }
  return readyPromise
}

/** The manifest-only summary of an item file's bytes — cheap, never decodes
 * geometry. Throws the kernel's typed load error (as a plain `Error`,
 * wasm-bindgen's `JsError` convention) for bytes that don't parse as a
 * `.hew` container; callers building a browser listing should catch that
 * and fall back to an error-state tile. */
export async function readItemSummary(bytes: Uint8Array): Promise<LibraryItemSummary> {
  await ready()
  const json = read_item_summary_json(bytes)
  return JSON.parse(json) as LibraryItemSummary
}

/** Renders a square PNG thumbnail from a fitted isometric view. `null` when
 * the item has nothing visible to render (an honest "no thumbnail" — e.g. a
 * material item, whose swatch is drawn directly from its palette data
 * instead). Throws on bytes that don't load as a document at all. */
export async function renderItemThumbnail(bytes: Uint8Array, size: number): Promise<Uint8Array | null> {
  await ready()
  const png = render_item_thumbnail(bytes, size)
  return png ?? null
}

/** Rewrites an item's `hew.library` metadata (rename / re-keyword /
 * re-collection) and returns its full new bytes; the input bytes are
 * untouched. `meta` is written key-by-key into the `hew.library` namespace —
 * a key set to `undefined` is simply omitted (use the wasm layer's own
 * `null`-deletes-a-key convention explicitly if a caller ever needs to
 * clear a field outright). */
export async function updateItemMeta(
  bytes: Uint8Array,
  meta: Partial<LibraryItemMeta> & Record<string, unknown>,
): Promise<Uint8Array> {
  await ready()
  return update_item_meta(bytes, JSON.stringify(meta))
}

/** One named zip entry of an item's bytes, verbatim — the on-demand fetch
 * for a material summary's `texture_asset` path. */
export async function readItemAsset(bytes: Uint8Array, path: string): Promise<Uint8Array> {
  await ready()
  return read_item_asset(bytes, path)
}

/** SHA-256 of `bytes` as lowercase hex — the provenance content hash AND
 * the thumbnail cache key (`LibraryItem.contentHash`). Pure browser
 * SubtleCrypto; no wasm involved. */
export async function sha256Hex(bytes: Uint8Array): Promise<string> {
  // `Uint8Array<ArrayBufferLike>` (this project's TS/DOM lib version) isn't
  // structurally a `BufferSource` (which wants a plain `ArrayBuffer`, not
  // the `ArrayBuffer | SharedArrayBuffer` union) — a wasm-returned view in
  // particular could be backed by either. The bytes are never mutated here,
  // so the assertion is safe.
  const digest = await crypto.subtle.digest('SHA-256', bytes as BufferSource)
  return Array.from(new Uint8Array(digest))
    .map((b) => b.toString(16).padStart(2, '0'))
    .join('')
}
