import { describe, it, expect } from 'vitest'
import {
  IDENTITY,
  translationAffine,
  rotationZAffine,
  uniformScaleAffine,
  composeAffine,
  rotateAboutPivotZ,
  scaleAboutCenter,
  snapAngleDeg,
  angleFromPivot,
  meshBoundingBoxCenter,
  affineToFloat64,
  type Affine,
} from './transformMath'

/** Apply a 3×4 affine to a point [x,y,z] → [x',y',z'] */
function applyAffine(a: Affine, x: number, y: number, z: number): [number, number, number] {
  return [
    a[0]*x + a[1]*y + a[2]*z + a[3],
    a[4]*x + a[5]*y + a[6]*z + a[7],
    a[8]*x + a[9]*y + a[10]*z + a[11],
  ]
}

describe('IDENTITY', () => {
  it('leaves a point unchanged', () => {
    const [x, y, z] = applyAffine(IDENTITY, 3, -1, 5)
    expect(x).toBeCloseTo(3)
    expect(y).toBeCloseTo(-1)
    expect(z).toBeCloseTo(5)
  })

  it('has length 12', () => {
    expect(IDENTITY).toHaveLength(12)
  })
})

describe('translationAffine', () => {
  it('translates a point by (tx, ty, tz)', () => {
    const a = translationAffine(1, 2, 3)
    const [x, y, z] = applyAffine(a, 0, 0, 0)
    expect(x).toBeCloseTo(1)
    expect(y).toBeCloseTo(2)
    expect(z).toBeCloseTo(3)
  })

  it('adds translation to a non-origin point', () => {
    const a = translationAffine(10, -5, 0)
    const [x, y, z] = applyAffine(a, 3, 4, 1)
    expect(x).toBeCloseTo(13)
    expect(y).toBeCloseTo(-1)
    expect(z).toBeCloseTo(1)
  })

  it('does not change the linear part (identity rotation)', () => {
    const a = translationAffine(1, 2, 3)
    expect(a[0]).toBe(1); expect(a[1]).toBe(0); expect(a[2]).toBe(0)
    expect(a[4]).toBe(0); expect(a[5]).toBe(1); expect(a[6]).toBe(0)
    expect(a[8]).toBe(0); expect(a[9]).toBe(0); expect(a[10]).toBe(1)
  })
})

describe('rotationZAffine', () => {
  it('0 radians is the identity', () => {
    const a = rotationZAffine(0)
    const [x, y, z] = applyAffine(a, 1, 2, 3)
    expect(x).toBeCloseTo(1)
    expect(y).toBeCloseTo(2)
    expect(z).toBeCloseTo(3)
  })

  it('90° rotates (1,0,0) → (0,1,0)', () => {
    const a = rotationZAffine(Math.PI / 2)
    const [x, y, z] = applyAffine(a, 1, 0, 0)
    expect(x).toBeCloseTo(0)
    expect(y).toBeCloseTo(1)
    expect(z).toBeCloseTo(0)
  })

  it('180° rotates (1,0,0) → (-1,0,0)', () => {
    const a = rotationZAffine(Math.PI)
    const [x, y, z] = applyAffine(a, 1, 0, 0)
    expect(x).toBeCloseTo(-1)
    expect(y).toBeCloseTo(0)
    expect(z).toBeCloseTo(0)
  })

  it('does not change the Z component', () => {
    const a = rotationZAffine(Math.PI / 3)
    const [, , z] = applyAffine(a, 2, 3, 7)
    expect(z).toBeCloseTo(7)
  })

  it('has no translation', () => {
    const a = rotationZAffine(Math.PI / 4)
    expect(a[3]).toBeCloseTo(0)
    expect(a[7]).toBeCloseTo(0)
    expect(a[11]).toBeCloseTo(0)
  })
})

describe('uniformScaleAffine', () => {
  it('scales a point by f', () => {
    const a = uniformScaleAffine(3)
    const [x, y, z] = applyAffine(a, 1, 2, 3)
    expect(x).toBeCloseTo(3)
    expect(y).toBeCloseTo(6)
    expect(z).toBeCloseTo(9)
  })

  it('f=1 is identity', () => {
    const a = uniformScaleAffine(1)
    const [x, y, z] = applyAffine(a, 5, -3, 0.5)
    expect(x).toBeCloseTo(5)
    expect(y).toBeCloseTo(-3)
    expect(z).toBeCloseTo(0.5)
  })

  it('f=0.5 halves each coordinate', () => {
    const a = uniformScaleAffine(0.5)
    const [x, y, z] = applyAffine(a, 4, 2, 8)
    expect(x).toBeCloseTo(2)
    expect(y).toBeCloseTo(1)
    expect(z).toBeCloseTo(4)
  })
})

describe('composeAffine', () => {
  it('composing with IDENTITY on either side is identity', () => {
    const a = translationAffine(1, 2, 3)
    const ab = composeAffine(a, IDENTITY)
    const ba = composeAffine(IDENTITY, a)
    const pt = applyAffine(a, 5, 6, 7)
    const ptAB = applyAffine(ab, 5, 6, 7)
    const ptBA = applyAffine(ba, 5, 6, 7)
    expect(ptAB[0]).toBeCloseTo(pt[0])
    expect(ptAB[1]).toBeCloseTo(pt[1])
    expect(ptAB[2]).toBeCloseTo(pt[2])
    expect(ptBA[0]).toBeCloseTo(pt[0])
    expect(ptBA[1]).toBeCloseTo(pt[1])
    expect(ptBA[2]).toBeCloseTo(pt[2])
  })

  it('two translations compose to sum', () => {
    const a = translationAffine(1, 2, 3)
    const b = translationAffine(10, 20, 30)
    const c = composeAffine(a, b)
    const [x, y, z] = applyAffine(c, 0, 0, 0)
    expect(x).toBeCloseTo(11)
    expect(y).toBeCloseTo(22)
    expect(z).toBeCloseTo(33)
  })

  it('translate-then-scale is applied in order (A first, then B)', () => {
    // Translate by (1,0,0) then scale by 2 → point (0,0,0) becomes (2,0,0)
    const translate = translationAffine(1, 0, 0)
    const scale = uniformScaleAffine(2)
    const c = composeAffine(translate, scale)
    const [x, y, z] = applyAffine(c, 0, 0, 0)
    expect(x).toBeCloseTo(2) // (0 + 1) * 2 = 2
    expect(y).toBeCloseTo(0)
    expect(z).toBeCloseTo(0)
  })

  it('rotate 90° twice = 180° rotation', () => {
    const r90 = rotationZAffine(Math.PI / 2)
    const r180 = composeAffine(r90, r90)
    const [x, y] = applyAffine(r180, 1, 0, 0)
    expect(x).toBeCloseTo(-1)
    expect(y).toBeCloseTo(0)
  })
})

describe('rotateAboutPivotZ', () => {
  it('rotates a point 90° about the origin', () => {
    const a = rotateAboutPivotZ(0, 0, 0, Math.PI / 2)
    const [x, y, z] = applyAffine(a, 1, 0, 0)
    expect(x).toBeCloseTo(0)
    expect(y).toBeCloseTo(1)
    expect(z).toBeCloseTo(0)
  })

  it('pivot at (1,1,0): rotating (2,1,0) by 90° → (1,2,0)', () => {
    // Point is 1 unit to the right of pivot. After 90° CCW, it should be 1
    // unit above the pivot.
    const a = rotateAboutPivotZ(1, 1, 0, Math.PI / 2)
    const [x, y, z] = applyAffine(a, 2, 1, 0)
    expect(x).toBeCloseTo(1)
    expect(y).toBeCloseTo(2)
    expect(z).toBeCloseTo(0)
  })

  it('pivot point itself is unchanged by rotation', () => {
    const a = rotateAboutPivotZ(3, 4, 0, Math.PI / 3)
    const [x, y, z] = applyAffine(a, 3, 4, 0)
    expect(x).toBeCloseTo(3)
    expect(y).toBeCloseTo(4)
    expect(z).toBeCloseTo(0)
  })

  it('360° rotation returns point to original position', () => {
    const a = rotateAboutPivotZ(1, 1, 0, 2 * Math.PI)
    const [x, y, z] = applyAffine(a, 5, 3, 2)
    expect(x).toBeCloseTo(5)
    expect(y).toBeCloseTo(3)
    expect(z).toBeCloseTo(2)
  })

  it('does not change Z', () => {
    const a = rotateAboutPivotZ(0, 0, 0, Math.PI / 6)
    const [, , z] = applyAffine(a, 1, 2, 7)
    expect(z).toBeCloseTo(7)
  })
})

describe('scaleAboutCenter', () => {
  it('scale 2× about origin doubles distances', () => {
    const a = scaleAboutCenter(0, 0, 0, 2)
    const [x, y, z] = applyAffine(a, 3, 4, 5)
    expect(x).toBeCloseTo(6)
    expect(y).toBeCloseTo(8)
    expect(z).toBeCloseTo(10)
  })

  it('scale 2× about (1,1,1): center point unchanged', () => {
    const a = scaleAboutCenter(1, 1, 1, 2)
    const [x, y, z] = applyAffine(a, 1, 1, 1)
    expect(x).toBeCloseTo(1)
    expect(y).toBeCloseTo(1)
    expect(z).toBeCloseTo(1)
  })

  it('scale 2× about (1,0,0): (2,0,0) → (3,0,0)', () => {
    // Distance from center (1,0,0) to (2,0,0) is 1. After 2×, distance is 2.
    // New point: center + 2*(1,0,0) = (3,0,0)
    const a = scaleAboutCenter(1, 0, 0, 2)
    const [x, y, z] = applyAffine(a, 2, 0, 0)
    expect(x).toBeCloseTo(3)
    expect(y).toBeCloseTo(0)
    expect(z).toBeCloseTo(0)
  })

  it('scale 0.5× about center halves the extents', () => {
    const a = scaleAboutCenter(0, 0, 0, 0.5)
    const [x, y, z] = applyAffine(a, 4, -2, 6)
    expect(x).toBeCloseTo(2)
    expect(y).toBeCloseTo(-1)
    expect(z).toBeCloseTo(3)
  })
})

describe('snapAngleDeg', () => {
  it('snaps to nearest 15° multiple', () => {
    const r15 = (15 * Math.PI) / 180
    // 0° → 0
    expect(snapAngleDeg(0, 15)).toBeCloseTo(0)
    // exactly 15° → 15°
    expect(snapAngleDeg(r15, 15)).toBeCloseTo(r15)
    // 7° (half-step) → 0
    expect(snapAngleDeg((7 * Math.PI) / 180, 15)).toBeCloseTo(0)
    // 8° (just over half-step) → 15°
    expect(snapAngleDeg((8 * Math.PI) / 180, 15)).toBeCloseTo(r15)
  })

  it('handles negative angles', () => {
    const r15 = (15 * Math.PI) / 180
    expect(snapAngleDeg(-r15, 15)).toBeCloseTo(-r15)
    expect(snapAngleDeg((-7 * Math.PI) / 180, 15)).toBeCloseTo(0)
  })

  it('snaps to 45° increments', () => {
    const r45 = (45 * Math.PI) / 180
    expect(snapAngleDeg((40 * Math.PI) / 180, 45)).toBeCloseTo(r45)
    expect(snapAngleDeg((20 * Math.PI) / 180, 45)).toBeCloseTo(0)
  })
})

describe('angleFromPivot', () => {
  it('(1,0) from origin is 0°', () => {
    expect(angleFromPivot(0, 0, 1, 0)).toBeCloseTo(0)
  })

  it('(0,1) from origin is 90°', () => {
    expect(angleFromPivot(0, 0, 0, 1)).toBeCloseTo(Math.PI / 2)
  })

  it('(-1,0) from origin is 180°', () => {
    expect(angleFromPivot(0, 0, -1, 0)).toBeCloseTo(Math.PI)
  })

  it('accounts for pivot offset', () => {
    // (2,1) from pivot (1,0) → direction (1,1) → 45°
    expect(angleFromPivot(1, 0, 2, 1)).toBeCloseTo(Math.PI / 4)
  })
})

describe('meshBoundingBoxCenter', () => {
  it('center of a unit cube', () => {
    // 2 triangles of the top face: corners at (0,0,0), (1,0,0), (1,1,0), (0,1,1)
    const pos = new Float32Array([
      0, 0, 0,
      1, 0, 0,
      0, 1, 0,
      1, 1, 1,
      0, 0, 0,
      1, 0, 0,
    ])
    const [cx, cy, cz] = meshBoundingBoxCenter(pos)
    expect(cx).toBeCloseTo(0.5)
    expect(cy).toBeCloseTo(0.5)
    expect(cz).toBeCloseTo(0.5)
  })

  it('single vertex returns that vertex', () => {
    const pos = new Float32Array([3, 4, 5])
    const [cx, cy, cz] = meshBoundingBoxCenter(pos)
    expect(cx).toBeCloseTo(3)
    expect(cy).toBeCloseTo(4)
    expect(cz).toBeCloseTo(5)
  })

  it('empty array returns (0,0,0)', () => {
    const pos = new Float32Array([])
    const [cx, cy, cz] = meshBoundingBoxCenter(pos)
    expect(cx).toBe(0)
    expect(cy).toBe(0)
    expect(cz).toBe(0)
  })

  it('two opposite points → midpoint', () => {
    const pos = new Float32Array([-2, -4, -6,  2, 4, 6])
    const [cx, cy, cz] = meshBoundingBoxCenter(pos)
    expect(cx).toBeCloseTo(0)
    expect(cy).toBeCloseTo(0)
    expect(cz).toBeCloseTo(0)
  })
})

describe('affineToFloat64', () => {
  it('converts IDENTITY to a Float64Array of length 12', () => {
    const f = affineToFloat64(IDENTITY)
    expect(f).toBeInstanceOf(Float64Array)
    expect(f.length).toBe(12)
    expect(f[0]).toBe(1)
    expect(f[5]).toBe(1)
    expect(f[10]).toBe(1)
    expect(f[3]).toBe(0)
  })

  it('round-trips a translation affine', () => {
    const a = translationAffine(5, -3, 1)
    const f = affineToFloat64(a)
    expect(f[3]).toBe(5)
    expect(f[7]).toBe(-3)
    expect(f[11]).toBe(1)
  })
})
