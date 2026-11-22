/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * profiles — the Library's profile items (WP11): the built-in starter set,
 * reading a saved profile's shape, thumbnails, and building profile item
 * bytes from a drawn chain. A thin async layer over the wasm functions of
 * `curves_api.rs`; like `itemFiles.ts`, every export waits for the kernel
 * once.
 */

import { loadKernel } from '../wasm/loader'
import { parseKernelErrorCode } from '../kernelErrors'
import {
  bundled_profiles_json,
  profile_item_bytes_from_bundled,
  profile_item_bytes_from_polyline,
  profile_shape_from_item,
  render_profile_thumbnail,
} from '../wasm/pkg/wasm_api.js'
import type { FilletProfile } from '../tools/filletSettings'
import type { ProfileShapeJson } from '../tools/filletMath'

let readyPromise: Promise<void> | null = null
function ready(): Promise<void> {
  if (readyPromise === null) readyPromise = loadKernel().then(() => undefined)
  return readyPromise
}

/** A built-in starter profile. */
export interface BundledProfile {
  /** `builtin:ogee` and so on. */
  id: string
  name: string
  shape: ProfileShapeJson
}

let bundledCache: BundledProfile[] | null = null

/** The built-in profiles, in display order. */
export async function bundledProfiles(): Promise<BundledProfile[]> {
  if (bundledCache === null) {
    await ready()
    // Profile names are data labels (not catalog keys). Normalize this built-in
    // presentation name here so Library and tool choices agree; ids and shapes
    // remain exactly as supplied by the kernel, and saved names are untouched.
    bundledCache = (JSON.parse(bundled_profiles_json()) as BundledProfile[]).map(profile =>
      profile.id === 'builtin:roman-ogee' ? { ...profile, name: 'Roman Ogee' } : profile,
    )
  }
  return bundledCache
}

/** The Fillet tool's profile for a shape. */
export function shapeProfile(id: string, name: string, shape: ProfileShapeJson): FilletProfile {
  return { kind: 'shape', id, name, shape }
}

/** A built-in as a Fillet profile. */
export async function builtInFilletProfile(id: string): Promise<FilletProfile | null> {
  const found = (await bundledProfiles()).find((p) => p.id === id)
  return found === undefined ? null : shapeProfile(found.id, found.name, found.shape)
}

/** The shape and drawn width of a profile item's bytes; null when the bytes
 *  are not a valid profile. With `strict`, only a kernel refusal (a coded
 *  error) means "not a profile"; anything else (the module failing to start,
 *  a crash) is thrown, so a caller guarding a write never mistakes a broken
 *  module for a bad drawing. */
export async function profileShapeOfBytes(
  bytes: Uint8Array,
  opts: { strict?: boolean } = {},
): Promise<{ shape: ProfileShapeJson; width: number } | null> {
  await ready()
  return readProfileShape(profile_shape_from_item, bytes, opts.strict === true)
}

/** `read(bytes)` as parsed JSON, or null when `read` refuses. In `strict`
 *  mode only a coded kernel refusal counts as a refusal; any other failure
 *  is rethrown. Split out so the rule can be tested without the module. */
export function readProfileShape(
  read: (bytes: Uint8Array) => string,
  bytes: Uint8Array,
  strict: boolean,
): { shape: ProfileShapeJson; width: number } | null {
  try {
    return JSON.parse(read(bytes)) as { shape: ProfileShapeJson; width: number }
  } catch (err) {
    if (strict && parseKernelErrorCode(err) === null) throw err
    return null
  }
}

/** A built-in saved as a profile item (50 mm wide), with `meta` stamped. */
export async function builtInItemBytes(id: string, meta: Record<string, unknown>): Promise<Uint8Array> {
  await ready()
  return profile_item_bytes_from_bundled(id, JSON.stringify(meta))
}

/** One segment of a drawn chain, in order from the profile's start. */
export interface ChainSegment {
  a: [number, number, number]
  b: [number, number, number]
  /** The analytic circle the segment belongs to, if any. */
  curve: { center: [number, number, number]; radius: number } | null
}

/**
 * Orders loose segments into one open chain: starts at the end of the
 * first segment that touches no other segment, then follows the rest. Null
 * when they are not one open chain.
 */
export function orderChain(segs: readonly ChainSegment[]): ChainSegment[] | null {
  if (segs.length === 0) return null
  const same = (p: readonly number[], q: readonly number[]): boolean =>
    Math.hypot(p[0] - q[0], p[1] - q[1], p[2] - q[2]) < 1e-7
  const touches = (p: readonly number[], skip: number): number =>
    segs.reduce((n, s, i) => (i !== skip && (same(s.a, p) || same(s.b, p)) ? n + 1 : n), 0)
  // The first drawn segment whose `a` end is free starts the chain; failing
  // that its `b` end (reversed).
  let first = -1
  let reverse = false
  for (let i = 0; i < segs.length && first < 0; i++) {
    if (touches(segs[i].a, i) === 0) {
      first = i
    } else if (touches(segs[i].b, i) === 0) {
      first = i
      reverse = true
    }
  }
  if (first < 0) return null
  const flip = (s: ChainSegment): ChainSegment => ({ a: s.b, b: s.a, curve: s.curve })
  const out: ChainSegment[] = [reverse ? flip(segs[first]) : segs[first]]
  const used = new Set<number>([first])
  while (out.length < segs.length) {
    const tail = out[out.length - 1].b
    let next = -1
    let flipNext = false
    for (let i = 0; i < segs.length; i++) {
      if (used.has(i)) continue
      if (same(segs[i].a, tail)) {
        next = i
        break
      }
      if (same(segs[i].b, tail)) {
        next = i
        flipNext = true
        break
      }
    }
    if (next < 0) return null
    used.add(next)
    out.push(flipNext ? flip(segs[next]) : segs[next])
  }
  // A single open chain: the far end touches nothing either.
  return touches(out[out.length - 1].b, -1) === 1 ? out : null
}

/**
 * Why a drawing cannot be saved as a profile yet, in plain words and as an
 * instruction, or null when it is one open chain of lines and arcs (the
 * kernel may still refuse its shape; that is a separate check on the bytes).
 */
export function chainFirstFix(segs: readonly ChainSegment[]): string | null {
  if (orderChain(segs) !== null) return null
  const same = (p: readonly number[], q: readonly number[]): boolean =>
    Math.hypot(p[0] - q[0], p[1] - q[1], p[2] - q[2]) < 1e-7
  const ends: Array<readonly number[]> = segs.flatMap((s) => [s.a, s.b])
  const count = (p: readonly number[]): number => ends.filter((q) => same(p, q)).length
  if (ends.some((p) => count(p) > 2)) {
    return 'That drawing branches. Select one line or arc that runs from one end to the other, then save it as a profile.'
  }
  if (ends.length > 0 && ends.every((p) => count(p) === 2)) {
    return 'That shape is closed. Select an open line or arc (one that does not come back to where it started), then save it as a profile.'
  }
  return 'Those lines are not joined end to end. Select one connected line or arc, then save it as a profile.'
}

/** A drawn chain, as a profile item's bytes with `meta` stamped. Null (not
 *  a throw) when the chain is not a valid profile. */
export async function chainItemBytes(
  chain: readonly ChainSegment[],
  normal: readonly number[],
  meta: Record<string, unknown>,
): Promise<Uint8Array | null> {
  await ready()
  const ordered = orderChain(chain)
  if (ordered === null) return null
  const points: number[] = [...ordered[0].a]
  const centers: number[] = []
  const radii: number[] = []
  for (const s of ordered) {
    points.push(...s.b)
    centers.push(...(s.curve?.center ?? [0, 0, 0]))
    radii.push(s.curve?.radius ?? Number.NaN)
  }
  try {
    return profile_item_bytes_from_polyline(
      Float64Array.from(normal),
      Float64Array.from(points),
      Float64Array.from(centers),
      Float64Array.from(radii),
      JSON.stringify(meta),
    )
  } catch {
    return null
  }
}

/** A square PNG thumbnail of `shape` (the end-on section of an edge cut
 *  with it). */
export async function profileThumbnailPng(shape: ProfileShapeJson, px: number): Promise<Uint8Array> {
  await ready()
  return render_profile_thumbnail(JSON.stringify(shape), px)
}

const thumbUrls = new Map<string, Promise<string>>()

/** An object URL of a thumbnail of `shape`, cached in memory by shape for
 *  the chip's rows (they live as long as the page). */
export function profileThumbnailUrl(shape: ProfileShapeJson, px: number): Promise<string> {
  const key = `${px}|${JSON.stringify(shape)}`
  let hit = thumbUrls.get(key)
  if (hit === undefined) {
    hit = profileThumbnailPng(shape, px).then((png) =>
      URL.createObjectURL(new Blob([new Uint8Array(png)], { type: 'image/png' })),
    )
    thumbUrls.set(key, hit)
    // A failed render is not cached: the next ask tries again.
    hit.catch(() => thumbUrls.delete(key))
  }
  return hit
}
