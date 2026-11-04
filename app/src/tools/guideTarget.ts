/**
 * guideTarget — the construction guide a transform tool (Move, Rotate) acts
 * on instead of nodes. The app selects a guide OR nodes, never both; the
 * Viewport pushes the selected guide in (`selected`) and injects the click
 * pick and the post-commit refresh (`setHooks`).
 *
 * A guide the tool takes by the gesture's first click, rather than one
 * selected beforehand, is selected only for the gesture: a highlight while it
 * moves. Once the gesture is over — landed, cancelled, or clicked in place —
 * the selection returns to what it was before the click. The post-commit
 * windows (a retyped value, an ×N / /N array) land the same way, until the
 * next gesture starts.
 */
import type { Ray } from '../viewport/math'

/** A guide transform landed, or a gesture on a guide ended: redraw the
 *  guides and make `guide` the selected guide (null: none). */
export type OnGuideCommit = (guide: bigint | null) => void

export class GuideTarget {
  /** The app's selected guide, kept in step by the tool's `setGuideTarget`. */
  selected: bigint | null = null
  private acquire: ((ray: Ray) => bigint | null) | null = null
  private onCommit: OnGuideCommit = () => { /* no-op */ }
  /** Set while the guide in hand was taken by the click: `to` is the guide
   *  selected before it (null: none), restored once the gesture is over. */
  private restore: { to: bigint | null } | null = null

  /** `acquire` picks the guide under the click ray, selects it in the app,
   *  and returns it (null: none there). */
  setHooks(acquire: ((ray: Ray) => bigint | null) | null, onCommit: OnGuideCommit): void {
    this.acquire = acquire
    this.onCommit = onCommit
  }

  /**
   * The guide a gesture starting with this click acts on, or null to act on
   * nodes. `eligible` is false while nodes are selected: the click is then
   * their base point or pivot, a guide's snap included. Otherwise a selected
   * guide is kept unless `retarget` lets a click on another guide take that
   * one (Move); with none selected, the guide under the click is taken.
   */
  start(ray: Ray, eligible: boolean, retarget: boolean): bigint | null {
    this.restore = null
    if (!eligible) return null
    const before = this.selected
    if (before !== null && !retarget) return before
    const hit = this.acquire?.(ray) ?? null
    if (hit === null) return before
    this.selected = hit
    if (hit !== before) this.restore = { to: before }
    return hit
  }

  /** A transform landed on `guide` (the moved guide, or the last copy): it
   *  becomes the selected guide — unless the click took the guide, then
   *  the selection returns to what it was before. */
  landed(guide: bigint): void {
    const next = this.restore !== null ? this.restore.to : guide
    this.selected = next
    this.onCommit(next)
  }

  /** The gesture ended without landing (Esc, a tool switch, a click in
   *  place): give a click-taken guide's selection back. */
  ended(): void {
    const restore = this.restore
    if (restore === null) return
    this.restore = null
    this.selected = restore.to
    this.onCommit(restore.to)
  }
}
