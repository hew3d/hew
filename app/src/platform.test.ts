// @vitest-environment jsdom
import { describe, it, expect } from 'vitest'
import { isCoarsePointer, prefersReducedMotion } from './platform'

/** Stubs `window.matchMedia` for one query, restoring whatever was there
 *  before — mirrors settings/theme.test.ts's matchMedia mocking pattern. */
function withMatchMedia<T>(matches: boolean, fn: () => T): T {
  const original = window.matchMedia
  window.matchMedia = ((query: string) => ({
    matches: query === '(pointer: coarse)' && matches,
  })) as typeof window.matchMedia
  try {
    return fn()
  } finally {
    window.matchMedia = original
  }
}

/** Same shape as `withMatchMedia` above, scoped to the reduced-motion query
 *  `prefersReducedMotion` reads. */
function withReducedMotionMedia<T>(matches: boolean, fn: () => T): T {
  const original = window.matchMedia
  window.matchMedia = ((query: string) => ({
    matches: query === '(prefers-reduced-motion: reduce)' && matches,
  })) as typeof window.matchMedia
  try {
    return fn()
  } finally {
    window.matchMedia = original
  }
}

describe('isCoarsePointer', () => {
  it('reflects a coarse-pointer match', () => {
    expect(withMatchMedia(true, () => isCoarsePointer())).toBe(true)
  })

  it('reflects a fine-pointer (no match)', () => {
    expect(withMatchMedia(false, () => isCoarsePointer())).toBe(false)
  })

  it('returns false when matchMedia is unavailable rather than throwing', () => {
    const original = window.matchMedia
    // @ts-expect-error — simulating an environment without matchMedia
    delete window.matchMedia
    try {
      expect(isCoarsePointer()).toBe(false)
    } finally {
      window.matchMedia = original
    }
  })

  it('returns false when matchMedia throws rather than propagating', () => {
    const original = window.matchMedia
    window.matchMedia = (() => {
      throw new Error('not implemented')
    }) as typeof window.matchMedia
    try {
      expect(isCoarsePointer()).toBe(false)
    } finally {
      window.matchMedia = original
    }
  })
})

describe('prefersReducedMotion', () => {
  it('reflects a reduced-motion match', () => {
    expect(withReducedMotionMedia(true, () => prefersReducedMotion())).toBe(true)
  })

  it('reflects no reduced-motion preference', () => {
    expect(withReducedMotionMedia(false, () => prefersReducedMotion())).toBe(false)
  })

  it('returns false when matchMedia is unavailable rather than throwing', () => {
    const original = window.matchMedia
    // @ts-expect-error — simulating an environment without matchMedia
    delete window.matchMedia
    try {
      expect(prefersReducedMotion()).toBe(false)
    } finally {
      window.matchMedia = original
    }
  })

  it('returns false when matchMedia throws rather than propagating', () => {
    const original = window.matchMedia
    window.matchMedia = (() => {
      throw new Error('not implemented')
    }) as typeof window.matchMedia
    try {
      expect(prefersReducedMotion()).toBe(false)
    } finally {
      window.matchMedia = original
    }
  })
})
