/**
 * isVisiblePartSnap — the parts-only gate for Shop Mode's read-only Tape
 * Measure (shop-mode playtest: parts-only + tap-off cancels). A tape point
 * may only land on a VISIBLE part; a tap that resolves to anything else
 * CANCELS the in-progress measurement rather than dropping a free-space
 * point.
 *
 * "A visible part" means the snap carries real OBJECT provenance
 * (`snap.object` defined) on an object/instance that isn't hidden. Gating on
 * object provenance — rather than blocklisting `kind` strings by name —
 * rejects EVERY topology-less kernel inference snap in a single check: the
 * ground/plane empty-space fallback AND the soft on-axis / on-guide inference
 * lines, all of which report `snap.object === undefined` because their kernel
 * candidates carry `Provenance::None`. That distinction matters in Shop Mode:
 * a construction guide inherited from a desktop-authored document is still
 * snappable here (Shop Mode never disables guide snapping), yet a guide is not
 * a part — an on-guide tap must cancel, not silently measure to a point off
 * every solid.
 */
import type { Snap } from '../tools/types'

export function isVisiblePartSnap(
  snap: Snap | null,
  hiddenObjectIds: ReadonlySet<bigint>,
  hiddenInstanceIds: ReadonlySet<bigint>,
): boolean {
  if (snap === null) return false
  // Provenance-less inference snaps (ground/plane fallback, on-axis, on-guide)
  // all report `object === undefined` — none of them is a part.
  if (snap.object === undefined) return false
  if (snap.instance !== undefined && hiddenInstanceIds.has(snap.instance)) return false
  if (hiddenObjectIds.has(snap.object)) return false
  return true
}

/**
 * Whether `snap` is a pick on the CLEAR FACE of a visible part — an
 * `elementKind: 'face'` hit on a non-hidden object. Shop Mode's Tape HOLD
 * rule keys the isolate-vs-magnify choice off this.
 *
 * CRITICAL (shop-mode playtest adversarial review): the caller MUST resolve
 * this `snap` from a NARROW, mouse-tuned aperture — NOT Tape Measure's own
 * touch-widened aperture, which the kernel resolves to `'edge'` for most
 * casual taps comfortably inside a face's silhouette on a phone (Viewport's
 * own aperture doc). Passing the widened-aperture press snap here would make
 * "hold a clear face to isolate" almost never fire on the target device.
 */
export function isClearFaceSnap(
  snap: Snap | null,
  hiddenObjectIds: ReadonlySet<bigint>,
  hiddenInstanceIds: ReadonlySet<bigint>,
): boolean {
  return snap !== null && snap.elementKind === 'face' && isVisiblePartSnap(snap, hiddenObjectIds, hiddenInstanceIds)
}

/** What a HELD Tape Measure press resolves to, by target. */
export type TapeHoldAction = 'grab' | 'isolate' | 'magnify'

/**
 * Decide what a HELD Tape press does (shop-mode playtest) — kept pure and
 * total so the priority/gating is unit-testable without the Viewport's
 * pointer/timer/WebGL machinery:
 *   - `grab`    — the hold landed on an existing tape point → move it. Highest
 *     priority: a grab on a point that happens to sit on a face still moves.
 *   - `isolate` — a hold on a clear face while NOT mid-measurement → isolate
 *     the part. An inspect-setup gesture, done before or between measurements.
 *   - `magnify` — everything else: on/near an edge or endpoint, empty space,
 *     OR a clear-face hold while a measurement is IN PROGRESS (`midGesture`),
 *     where the hold is the precise placement of the next point, not isolate.
 */
export function classifyTapeHold(opts: {
  grabEndpoint: 0 | 1 | null
  midGesture: boolean
  onClearFace: boolean
}): TapeHoldAction {
  if (opts.grabEndpoint !== null) return 'grab'
  if (!opts.midGesture && opts.onClearFace) return 'isolate'
  return 'magnify'
}
