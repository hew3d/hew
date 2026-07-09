/**
 * renderStats — observable render-statistics store (module-level singleton,
 * same shape as log/LogStore.ts and settings/debugMode.ts).
 *
 * Viewport.tsx calls recordRender() immediately after each `renderer.render`
 * with the frame's `WebGLRenderer.info` counters and the CPU-side duration of
 * the call. The render loop is ON-DEMAND (dirty-flag `needsRender`), so a
 * classic rolling FPS is meaningless while idle: instead the store keeps the
 * most recent frame's numbers, and derives a smoothed frames-per-second ONLY
 * while renders are streaming (successive frames closer than STREAMING_GAP_MS
 * — orbiting, dragging a tool). An isolated one-off render reports fps: null.
 *
 * Two guarantees keep this free when nobody is looking:
 *   - Viewport checks isRenderStatsActive() before even timing the render;
 *     with the readout closed the per-frame cost is one boolean check.
 *   - Subscriber notification is throttled to NOTIFY_INTERVAL_MS (with a
 *     trailing notification so the final frame of a burst always lands), so
 *     the readout's setState can never become its own render load.
 *
 * `cpuMs` is the CPU-side duration of the renderer.render() call. WebGL GPU
 * work is asynchronous, so this is an approximation of frame cost, not GPU
 * time — the readout labels it "cpu ms" accordingly.
 */

/** Structural subset of THREE.WebGLRenderer.info that the store consumes. */
export interface RendererInfoLike {
  render: { calls: number; triangles: number }
  memory: { geometries: number; textures: number }
}

export interface RenderStatsSnapshot {
  /** Draw calls issued for the most recent frame (`info.render.calls`). */
  drawCalls: number
  /** Triangles rasterized in the most recent frame (`info.render.triangles`). */
  triangles: number
  /** Live GPU geometries (`info.memory.geometries`, cumulative not per-frame). */
  geometries: number
  /** Live GPU textures (`info.memory.textures`, cumulative not per-frame). */
  textures: number
  /** CPU-side duration of the most recent renderer.render() call, in ms. */
  cpuMs: number
  /** Smoothed frames/sec while renders are streaming; null when idle/isolated. */
  fps: number | null
  /** Timestamp (performance.now() clock) of the most recent render. */
  at: number
}

export type Subscriber = (snapshot: RenderStatsSnapshot) => void

/** Notify subscribers at most once per this interval (trailing edge included). */
export const NOTIFY_INTERVAL_MS = 250
/** Two renders further apart than this are "isolated", not a stream — no fps. */
export const STREAMING_GAP_MS = 500
/** EMA weight of the newest frame interval in the smoothed fps. */
const EMA_ALPHA = 0.2

const subscribers = new Set<Subscriber>()

let snapshot: RenderStatsSnapshot | null = null
let lastRenderAt: number | null = null
let emaFrameMs: number | null = null
let lastNotifyAt = -Infinity
let pendingNotify: ReturnType<typeof setTimeout> | null = null

function notify(): void {
  if (snapshot === null) return
  for (const sub of subscribers) {
    sub(snapshot)
  }
}

function notifyThrottled(now: number): void {
  if (now - lastNotifyAt >= NOTIFY_INTERVAL_MS) {
    lastNotifyAt = now
    notify()
    return
  }
  if (pendingNotify === null) {
    const delay = NOTIFY_INTERVAL_MS - (now - lastNotifyAt)
    pendingNotify = setTimeout(() => {
      pendingNotify = null
      lastNotifyAt += NOTIFY_INTERVAL_MS
      notify()
    }, delay)
  }
}

/**
 * True when at least one subscriber (the debug-log readout) is mounted.
 * Viewport checks this BEFORE timing the render, so a closed readout costs
 * one boolean check per rendered frame and nothing while fully idle.
 */
export function isRenderStatsActive(): boolean {
  return subscribers.size > 0
}

/**
 * Record one completed render. Call immediately after `renderer.render` so
 * `info.render.*` still holds this frame's counters (three.js auto-resets
 * them at the start of the next render).
 *
 * `now` is injectable for tests; production callers omit it.
 */
export function recordRender(info: RendererInfoLike, cpuMs: number, now: number = performance.now()): void {
  if (subscribers.size === 0) return

  if (lastRenderAt !== null) {
    const delta = now - lastRenderAt
    if (delta <= STREAMING_GAP_MS) {
      emaFrameMs = emaFrameMs === null ? delta : emaFrameMs + EMA_ALPHA * (delta - emaFrameMs)
    } else {
      // Gap since the previous render — the stream broke; start fresh.
      emaFrameMs = null
    }
  }
  lastRenderAt = now

  snapshot = {
    drawCalls: info.render.calls,
    triangles: info.render.triangles,
    geometries: info.memory.geometries,
    textures: info.memory.textures,
    cpuMs,
    fps: emaFrameMs !== null && emaFrameMs > 0 ? 1000 / emaFrameMs : null,
    at: now,
  }
  notifyThrottled(now)
}

/** Get the most recent snapshot, or null if nothing has been recorded. */
export function getSnapshot(): RenderStatsSnapshot | null {
  return snapshot
}

/**
 * Subscribe to (throttled) snapshot updates. Immediately delivers the current
 * snapshot if one exists. Returns an unsubscribe function.
 */
export function subscribe(fn: Subscriber): () => void {
  subscribers.add(fn)
  if (snapshot !== null) fn(snapshot)
  return () => {
    subscribers.delete(fn)
    if (subscribers.size === 0 && pendingNotify !== null) {
      clearTimeout(pendingNotify)
      pendingNotify = null
    }
  }
}

/** Reset all module state (tests only — the singleton persists across suites). */
export function resetRenderStats(): void {
  if (pendingNotify !== null) {
    clearTimeout(pendingNotify)
    pendingNotify = null
  }
  subscribers.clear()
  snapshot = null
  lastRenderAt = null
  emaFrameMs = null
  lastNotifyAt = -Infinity
}
