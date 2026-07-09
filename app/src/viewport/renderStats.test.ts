import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import {
  NOTIFY_INTERVAL_MS,
  STREAMING_GAP_MS,
  getSnapshot,
  isRenderStatsActive,
  recordRender,
  resetRenderStats,
  subscribe,
  type RendererInfoLike,
  type RenderStatsSnapshot,
} from './renderStats'

/** Build a renderer.info-shaped object with overridable counters. */
function info(overrides: Partial<RendererInfoLike['render'] & RendererInfoLike['memory']> = {}): RendererInfoLike {
  return {
    render: { calls: overrides.calls ?? 42, triangles: overrides.triangles ?? 1000 },
    memory: { geometries: overrides.geometries ?? 7, textures: overrides.textures ?? 2 },
  }
}

beforeEach(() => {
  resetRenderStats()
  // The trailing-edge notification uses setTimeout; drive it deterministically.
  vi.useFakeTimers()
})

afterEach(() => {
  resetRenderStats()
  vi.useRealTimers()
})

describe('renderStats', () => {
  it('is inactive with no subscribers and active with one', () => {
    expect(isRenderStatsActive()).toBe(false)
    const unsub = subscribe(() => {})
    expect(isRenderStatsActive()).toBe(true)
    unsub()
    expect(isRenderStatsActive()).toBe(false)
  })

  it('records nothing without subscribers (zero cost when the readout is closed)', () => {
    recordRender(info(), 4.2, 1000)
    expect(getSnapshot()).toBeNull()
  })

  it('captures the renderer.info counters and cpu duration of the last render', () => {
    subscribe(() => {})
    recordRender(info({ calls: 38, triangles: 123456, geometries: 87, textures: 3 }), 4.2, 1000)
    expect(getSnapshot()).toMatchObject({
      drawCalls: 38,
      triangles: 123456,
      geometries: 87,
      textures: 3,
      cpuMs: 4.2,
      at: 1000,
    })
  })

  it('reports fps: null for an isolated render (on-demand loop at rest)', () => {
    subscribe(() => {})
    recordRender(info(), 1, 1000)
    expect(getSnapshot()?.fps).toBeNull()
  })

  it('reports a smoothed fps while renders are streaming', () => {
    subscribe(() => {})
    // A steady 16ms cadence → ~62.5 fps regardless of smoothing weights.
    for (let i = 0; i < 10; i++) {
      recordRender(info(), 1, 1000 + i * 16)
    }
    const fps = getSnapshot()?.fps
    expect(fps).not.toBeNull()
    expect(fps as number).toBeCloseTo(62.5, 1)
  })

  it('drops back to fps: null after a gap longer than STREAMING_GAP_MS', () => {
    subscribe(() => {})
    recordRender(info(), 1, 1000)
    recordRender(info(), 1, 1016)
    expect(getSnapshot()?.fps).not.toBeNull()
    recordRender(info(), 1, 1016 + STREAMING_GAP_MS + 1)
    expect(getSnapshot()?.fps).toBeNull()
  })

  it('notifies immediately for the first render, then throttles with a trailing update', () => {
    const seen: RenderStatsSnapshot[] = []
    subscribe((s) => seen.push(s))

    recordRender(info({ calls: 10 }), 1, 1000)
    expect(seen).toHaveLength(1)
    expect(seen[0].drawCalls).toBe(10)

    // A burst inside the throttle window: no synchronous notifications…
    recordRender(info({ calls: 11 }), 1, 1016)
    recordRender(info({ calls: 12 }), 1, 1033)
    expect(seen).toHaveLength(1)

    // …but the trailing edge delivers the burst's final values.
    vi.advanceTimersByTime(NOTIFY_INTERVAL_MS)
    expect(seen).toHaveLength(2)
    expect(seen[1].drawCalls).toBe(12)
  })

  it('delivers the current snapshot immediately on subscribe', () => {
    subscribe(() => {})
    recordRender(info({ calls: 99 }), 1, 1000)

    const seen: RenderStatsSnapshot[] = []
    subscribe((s) => seen.push(s))
    expect(seen).toHaveLength(1)
    expect(seen[0].drawCalls).toBe(99)
  })

  it('cancels a pending trailing notification when the last subscriber leaves', () => {
    const seen: RenderStatsSnapshot[] = []
    const unsub = subscribe((s) => seen.push(s))
    recordRender(info(), 1, 1000)
    recordRender(info(), 1, 1016) // schedules the trailing notify
    unsub()
    vi.advanceTimersByTime(NOTIFY_INTERVAL_MS * 2)
    expect(seen).toHaveLength(1)
  })
})
