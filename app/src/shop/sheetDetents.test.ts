import { describe, it, expect } from 'vitest'
import {
  detentHeightPx,
  clampDragHeightPx,
  nearestDetent,
  PEEK_HEIGHT_PX,
} from './sheetDetents'

describe('detentHeightPx', () => {
  it('peek is the fixed content-driven height, independent of container size', () => {
    expect(detentHeightPx('peek', 800)).toBe(PEEK_HEIGHT_PX)
    expect(detentHeightPx('peek', 2000)).toBe(PEEK_HEIGHT_PX)
  })

  it('half is 50% of the container height', () => {
    expect(detentHeightPx('half', 800)).toBe(400)
  })

  it('full is 92% of the container height, short of 100%', () => {
    expect(detentHeightPx('full', 1000)).toBe(920)
  })
})

describe('clampDragHeightPx', () => {
  it('passes through a height already within [peek, full]', () => {
    expect(clampDragHeightPx(500, 800)).toBe(500)
  })

  it('clamps below peek up to the peek height', () => {
    expect(clampDragHeightPx(10, 800)).toBe(PEEK_HEIGHT_PX)
  })

  it('clamps above full down to the full height', () => {
    expect(clampDragHeightPx(10000, 800)).toBe(detentHeightPx('full', 800))
  })
})

describe('nearestDetent', () => {
  it('snaps to peek when close to it', () => {
    expect(nearestDetent(70, 800)).toBe('peek')
  })

  it('snaps to half when close to it', () => {
    expect(nearestDetent(390, 800)).toBe('half')
  })

  it('snaps to full when close to it', () => {
    expect(nearestDetent(900, 800)).toBe('full')
  })

  it('breaks an exact peek/half tie toward peek (the smaller detent)', () => {
    const containerHeightPx = 800
    const peek = detentHeightPx('peek', containerHeightPx)
    const half = detentHeightPx('half', containerHeightPx)
    const midpoint = (peek + half) / 2
    expect(nearestDetent(midpoint, containerHeightPx)).toBe('peek')
  })

  it('breaks an exact half/full tie toward half (the smaller detent)', () => {
    const containerHeightPx = 800
    const half = detentHeightPx('half', containerHeightPx)
    const full = detentHeightPx('full', containerHeightPx)
    const midpoint = (half + full) / 2
    expect(nearestDetent(midpoint, containerHeightPx)).toBe('half')
  })
})
