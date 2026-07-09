/**
 * Component tests for RenderStatsReadout.
 *
 * The readout subscribes to the real renderStats singleton, so tests drive
 * frames through recordRender() with explicit timestamps and fake timers
 * (both the store's notify throttle and the readout's idle timer are
 * setTimeout-based).
 */

import { render, screen, act } from '@testing-library/react'
import { describe, expect, it, beforeEach, afterEach, vi } from 'vitest'
import { RenderStatsReadout } from './RenderStatsReadout'
import {
  NOTIFY_INTERVAL_MS,
  recordRender,
  resetRenderStats,
  type RendererInfoLike,
} from '../viewport/renderStats'

const INFO: RendererInfoLike = {
  render: { calls: 38, triangles: 1234567 },
  memory: { geometries: 87, textures: 3 },
}

beforeEach(() => {
  resetRenderStats()
  vi.useFakeTimers()
})

afterEach(() => {
  resetRenderStats()
  vi.useRealTimers()
})

describe('RenderStatsReadout', () => {
  it('shows a placeholder before any frame has been recorded', () => {
    render(<RenderStatsReadout />)
    expect(screen.getByText(/no frames yet/)).toBeInTheDocument()
  })

  it('shows the last frame counters, with draw calls first', () => {
    render(<RenderStatsReadout />)
    act(() => {
      recordRender(INFO, 4.25, 1000)
    })
    expect(screen.getByText('draw 38')).toBeInTheDocument()
    expect(screen.getByText(/tri 1\.23M/)).toBeInTheDocument()
    expect(screen.getByText(/geo 87/)).toBeInTheDocument()
    expect(screen.getByText(/tex 3/)).toBeInTheDocument()
    expect(screen.getByText(/4\.3 cpu ms/)).toBeInTheDocument()
  })

  it('shows "idle" (not an fps) for an isolated on-demand render', () => {
    render(<RenderStatsReadout />)
    act(() => {
      recordRender(INFO, 1, 1000)
    })
    expect(screen.getByText('idle')).toBeInTheDocument()
    expect(screen.queryByText(/fps/)).not.toBeInTheDocument()
  })

  it('shows a smoothed fps while renders stream, then flips back to idle', () => {
    render(<RenderStatsReadout />)
    act(() => {
      recordRender(INFO, 1, 0) // notifies immediately (fps still null)
      recordRender(INFO, 1, 16) // burst: throttled…
      recordRender(INFO, 1, 32)
      vi.advanceTimersByTime(NOTIFY_INTERVAL_MS) // …trailing notify lands the fps
    })
    expect(screen.getByText(/\d+ fps/)).toBeInTheDocument()
    expect(screen.queryByText('idle')).not.toBeInTheDocument()

    // No further renders: the idle timer expires and the fps figure is
    // retired rather than left frozen on screen.
    act(() => {
      vi.advanceTimersByTime(1000)
    })
    expect(screen.getByText('idle')).toBeInTheDocument()
    expect(screen.queryByText(/fps/)).not.toBeInTheDocument()
  })
})
