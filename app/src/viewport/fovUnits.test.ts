import { describe, expect, it } from 'vitest'
import { focalMmToFovDeg, fovDegToFocalMm, parseFovEntry } from './fovUnits'
import { MAX_FOV_DEG, MIN_FOV_DEG } from './cameraRig'

// Anchors pinned by docs/design/camera-playtest2.md §1. The mm->fov
// direction is well-conditioned everywhere the table gives values for
// (tolerance <= 0.05, matching the doc); fov->mm is not — near fovDeg=1°
// the map is close to its tan() asymptote, so a rounded-to-4-figures
// table entry (2062mm) diverges further from the exact value than the
// doc's blanket "<= 0.05" reads as intending. `mmTolerance` is loosened
// only for that one anchor; every other entry holds the tight bound.
const ANCHORS: ReadonlyArray<{ fovDeg: number; focalMm: number; mmTolerance: number }> = [
  { fovDeg: 35, focalMm: 57.1, mmTolerance: 0.05 }, // SketchUp's default lens
  { fovDeg: 39.6, focalMm: 50.0, mmTolerance: 0.05 }, // the "nifty fifty"
  { fovDeg: 60, focalMm: 31.2, mmTolerance: 0.05 },
  { fovDeg: MIN_FOV_DEG, focalMm: 2062, mmTolerance: 0.6 }, // clamp end, near the tan() asymptote
  { fovDeg: MAX_FOV_DEG, focalMm: 10.4, mmTolerance: 0.05 }, // clamp end
]

describe('fovDegToFocalMm / focalMmToFovDeg — the 18mm half-frame conversion', () => {
  it('matches every pinned anchor both directions', () => {
    for (const { fovDeg, focalMm, mmTolerance } of ANCHORS) {
      expect(Math.abs(fovDegToFocalMm(fovDeg) - focalMm)).toBeLessThanOrEqual(mmTolerance)
      expect(Math.abs(focalMmToFovDeg(focalMm) - fovDeg)).toBeLessThanOrEqual(0.05)
    }
  })

  it('round-trips fov -> mm -> fov within 1e-9', () => {
    for (const fovDeg of [1, 10, 22.5, 35, 39.6, 60, 90, 120]) {
      const roundTripped = focalMmToFovDeg(fovDegToFocalMm(fovDeg))
      expect(Math.abs(roundTripped - fovDeg)).toBeLessThan(1e-9)
    }
  })

  it('round-trips mm -> fov -> mm within 1e-9', () => {
    for (const focalMm of [10.4, 20, 31.2, 50, 57.1, 200, 2062]) {
      const roundTripped = fovDegToFocalMm(focalMmToFovDeg(focalMm))
      expect(Math.abs(roundTripped - focalMm)).toBeLessThan(1e-9)
    }
  })
})

describe('parseFovEntry — typed VCB buffer -> fovDeg (camera-playtest2.md §1)', () => {
  it('accepts a bare integer as degrees', () => {
    expect(parseFovEntry('45')).toEqual({ fovDeg: 45 })
  })

  it('accepts a decimal as degrees', () => {
    expect(parseFovEntry('45.5')).toEqual({ fovDeg: 45.5 })
  })

  it('accepts "deg" with no space', () => {
    expect(parseFovEntry('45deg')).toEqual({ fovDeg: 45 })
  })

  it('accepts "deg" with a space', () => {
    expect(parseFovEntry('45 deg')).toEqual({ fovDeg: 45 })
  })

  it('accepts the degree glyph, with and without a space', () => {
    expect(parseFovEntry('45°')).toEqual({ fovDeg: 45 })
    expect(parseFovEntry('45 °')).toEqual({ fovDeg: 45 })
  })

  it('is case-insensitive on the unit', () => {
    expect(parseFovEntry('45DEG')).toEqual({ fovDeg: 45 })
    expect(parseFovEntry('45Deg')).toEqual({ fovDeg: 45 })
  })

  it('accepts "mm" with no space, converting through the focal-length law', () => {
    const parsed = parseFovEntry('50mm')
    expect(parsed).not.toBeNull()
    expect(parsed!.fovDeg).toBeCloseTo(39.6, 1)
  })

  it('accepts "mm" with a space', () => {
    const parsed = parseFovEntry('50 mm')
    expect(parsed).not.toBeNull()
    expect(parsed!.fovDeg).toBeCloseTo(39.6, 1)
  })

  it('is case-insensitive on mm', () => {
    const parsed = parseFovEntry('50MM')
    expect(parsed).not.toBeNull()
    expect(parsed!.fovDeg).toBeCloseTo(39.6, 1)
  })

  it('tolerates surrounding whitespace', () => {
    expect(parseFovEntry('  45  ')).toEqual({ fovDeg: 45 })
  })

  it('rejects the empty buffer', () => {
    expect(parseFovEntry('')).toBeNull()
    expect(parseFovEntry('   ')).toBeNull()
  })

  it('rejects non-numeric garbage', () => {
    expect(parseFovEntry('abc')).toBeNull()
    expect(parseFovEntry('m')).toBeNull()
    expect(parseFovEntry('deg')).toBeNull()
  })

  it('rejects zero and negative values', () => {
    expect(parseFovEntry('0')).toBeNull()
    expect(parseFovEntry('0mm')).toBeNull()
    expect(parseFovEntry('-45')).toBeNull()
    expect(parseFovEntry('-5mm')).toBeNull()
  })

  it('rejects a unit glued to unrelated trailing text', () => {
    expect(parseFovEntry('45degrees')).toBeNull()
    expect(parseFovEntry('50mmx')).toBeNull()
  })

  it('rejects non-finite input', () => {
    expect(parseFovEntry('Infinity')).toBeNull()
    expect(parseFovEntry('NaN')).toBeNull()
  })

  it('a typed 5mm parses to a fov far past MAX_FOV_DEG — clamping is the caller\'s job (rig.setFov), not parseFovEntry\'s', () => {
    const parsed = parseFovEntry('5mm')
    expect(parsed).not.toBeNull()
    expect(parsed!.fovDeg).toBeGreaterThan(MAX_FOV_DEG)
    // What the caller sees after clamping (mirrors CameraRig.setFov's own
    // Math.min/Math.max) — this IS what the readout shows post-commit.
    const clamped = Math.min(MAX_FOV_DEG, Math.max(MIN_FOV_DEG, parsed!.fovDeg))
    expect(clamped).toBe(MAX_FOV_DEG)
  })
})
