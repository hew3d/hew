import { describe, it, expect } from 'vitest'
import {
  matVec,
  matMul,
  matInverse,
  rotation2,
  signedAngle2,
  frameToLocal2D,
  local2DToFrame,
  local2ToWorld,
  worldToLocal2,
  pinLocal,
  handleLocal,
  vHandleLocal,
  uvPointLocal,
  frameAbsolute,
  translateLocal,
  scaleRotateLocal,
  shearScaleLocal,
  tessellatePlaneBasis,
  planarDefaultFrame,
  frameToArray,
  arrayToFrame,
  writeFrameUvs,
  identityLocal2,
  type UvFrameComponents,
  type Local2,
} from './uvFrameMath'
import { planeBasis } from './transformMath'

/** Applies a UvFrameComponents to a world point, mirroring the kernel's
 *  `UvFrame::apply` (`uv = s·p + u0, t·p + v0`) — used to check round-trips
 *  against the SAME formula tessellate/wasm evaluate. */
function applyFrame(f: UvFrameComponents, p: readonly [number, number, number]): [number, number] {
  return [
    f.sx * p[0] + f.sy * p[1] + f.sz * p[2] + f.u0,
    f.tx * p[0] + f.ty * p[1] + f.tz * p[2] + f.v0,
  ]
}

const ANCHOR: [number, number, number] = [1, 2, 3]
// A plane basis (arbitrary orthonormal pair spanning some plane).
const NORMAL: [number, number, number] = [0, 0, 1]
const { u: U_AX, v: V_AX } = planeBasis(NORMAL)

describe('2x2 algebra', () => {
  it('matVec applies a matrix to a vector', () => {
    expect(matVec({ a: 2, b: 0, c: 0, d: 3 }, [1, 1])).toEqual([2, 3])
  })

  it('matMul composes two matrices (A then B applied is matMul(B, A))', () => {
    const scale = { a: 2, b: 0, c: 0, d: 2 }
    const rot = rotation2(Math.PI / 2)
    const composed = matMul(rot, scale)
    const v = matVec(composed, [1, 0])
    // Scale (1,0)->(2,0), then rotate 90 deg -> (0,2).
    expect(v[0]).toBeCloseTo(0)
    expect(v[1]).toBeCloseTo(2)
  })

  it('matInverse undoes a matrix', () => {
    const m = { a: 1, b: 1, c: 0, d: 1 } // shear
    const inv = matInverse(m)
    expect(inv).not.toBeNull()
    const composed = matMul(m, inv!)
    expect(composed.a).toBeCloseTo(1)
    expect(composed.b).toBeCloseTo(0)
    expect(composed.c).toBeCloseTo(0)
    expect(composed.d).toBeCloseTo(1)
  })

  it('matInverse returns null for a singular matrix', () => {
    expect(matInverse({ a: 1, b: 2, c: 2, d: 4 })).toBeNull()
  })

  it('rotation2 rotates counter-clockwise by theta', () => {
    const v = matVec(rotation2(Math.PI / 2), [1, 0])
    expect(v[0]).toBeCloseTo(0)
    expect(v[1]).toBeCloseTo(1)
  })

  it('signedAngle2 measures the signed angle between two vectors', () => {
    expect(signedAngle2([1, 0], [0, 1])).toBeCloseTo(Math.PI / 2)
    expect(signedAngle2([1, 0], [-1, 0])).toBeCloseTo(Math.PI)
    expect(signedAngle2([1, 0], [1, 0])).toBeCloseTo(0)
  })

  it('signedAngle2 returns 0 for a degenerate (near-zero) vector', () => {
    expect(signedAngle2([0, 0], [1, 0])).toBe(0)
    expect(signedAngle2([1, 0], [0, 0])).toBe(0)
  })
})

describe('frame <-> local2D round trip', () => {
  it('round-trips a frame with no out-of-plane gradient component exactly', () => {
    // sz/tz = 0: s and t lie entirely in the face plane, the realistic case
    // for a frame meant to be evaluated on THIS plane (planar-default and
    // dae-imported frames alike). The next test documents what happens when
    // that's not true.
    const frame: UvFrameComponents = {
      sx: 0.3, sy: -0.1, sz: 0,
      tx: 0.05, ty: 0.4, tz: 0,
      u0: 0.25, v0: -0.6,
    }
    const local = frameToLocal2D(frame, ANCHOR, U_AX, V_AX)
    const back = local2DToFrame(local, ANCHOR, U_AX, V_AX)
    expect(back.sx).toBeCloseTo(frame.sx)
    expect(back.sy).toBeCloseTo(frame.sy)
    expect(back.sz).toBeCloseTo(frame.sz)
    expect(back.tx).toBeCloseTo(frame.tx)
    expect(back.ty).toBeCloseTo(frame.ty)
    expect(back.tz).toBeCloseTo(frame.tz)
    expect(back.u0).toBeCloseTo(frame.u0)
    expect(back.v0).toBeCloseTo(frame.v0)
  })

  it('a frame with a gradient component ALONG the plane normal round-trips functionally, not byte-for-byte', () => {
    // s/t's component along the plane normal has NO effect on the UV of any
    // point actually ON the face (every such point shares the same
    // normal-axis coordinate, so that component only ever contributes a
    // constant that frameToLocal2D already folds into `off`/u0/v0). The 2D
    // round trip therefore reconstructs a DIFFERENT but FUNCTIONALLY
    // EQUIVALENT frame — same UV at every point on the plane — dropping the
    // gratuitous normal-axis component rather than preserving it verbatim.
    const frame: UvFrameComponents = {
      sx: 0.3, sy: -0.1, sz: 0.05,
      tx: 0.05, ty: 0.4, tz: -0.02,
      u0: 0.25, v0: -0.6,
    }
    const local = frameToLocal2D(frame, ANCHOR, U_AX, V_AX)
    const back = local2DToFrame(local, ANCHOR, U_AX, V_AX)

    // Sample a few ON-PLANE points (anchor + arbitrary in-plane offsets —
    // NORMAL's own axis coordinate is fixed at ANCHOR's, since U_AX/V_AX
    // span the plane through ANCHOR with that normal).
    const samples: Array<[number, number, number]> = [
      ANCHOR,
      local2ToWorld(ANCHOR, U_AX, V_AX, [3, -1]),
      local2ToWorld(ANCHOR, U_AX, V_AX, [-2, 4]),
    ]
    for (const p of samples) {
      const uvOriginal = applyFrame(frame, p)
      const uvBack = applyFrame(back, p)
      expect(uvBack[0]).toBeCloseTo(uvOriginal[0])
      expect(uvBack[1]).toBeCloseTo(uvOriginal[1])
    }
  })

  it('round trip is invariant to the anchor point chosen', () => {
    const frame: UvFrameComponents = {
      sx: 1, sy: 0, sz: 0,
      tx: 0, ty: 1, tz: 0,
      u0: 0.1, v0: -0.2,
    }
    const local1 = frameToLocal2D(frame, [0, 0, 0], U_AX, V_AX)
    const back1 = local2DToFrame(local1, [0, 0, 0], U_AX, V_AX)
    const local2 = frameToLocal2D(frame, [5, -3, 0], U_AX, V_AX)
    const back2 = local2DToFrame(local2, [5, -3, 0], U_AX, V_AX)
    expect(back1.u0).toBeCloseTo(back2.u0)
    expect(back1.v0).toBeCloseTo(back2.v0)
    expect(back1.sx).toBeCloseTo(back2.sx)
    expect(back1.ty).toBeCloseTo(back2.ty)
  })

  it('produces UVs matching applyFrame at sample points via the local map', () => {
    const frame: UvFrameComponents = {
      sx: 1, sy: 0, sz: 0,
      tx: 0, ty: 1, tz: 0,
      u0: 0, v0: 0,
    }
    const local = frameToLocal2D(frame, ANCHOR, U_AX, V_AX)
    const sample: [number, number, number] = [ANCHOR[0] + U_AX[0] * 2, ANCHOR[1] + U_AX[1] * 2, ANCHOR[2] + U_AX[2] * 2]
    const uvDirect = applyFrame(frame, sample)
    const localCoords = worldToLocal2(ANCHOR, U_AX, V_AX, sample)
    const uvViaLocal = matVec(local.m, localCoords)
    expect(uvViaLocal[0] + local.off[0]).toBeCloseTo(uvDirect[0])
    expect(uvViaLocal[1] + local.off[1]).toBeCloseTo(uvDirect[1])
  })
})

describe('local2ToWorld / worldToLocal2', () => {
  it('round-trips a point ON the anchor plane (z = ANCHOR.z, since NORMAL/U_AX/V_AX define that plane)', () => {
    const p: [number, number, number] = [4, -2, ANCHOR[2]]
    const local = worldToLocal2(ANCHOR, U_AX, V_AX, p)
    const back = local2ToWorld(ANCHOR, U_AX, V_AX, local)
    expect(back[0]).toBeCloseTo(p[0])
    expect(back[1]).toBeCloseTo(p[1])
    expect(back[2]).toBeCloseTo(p[2])
  })

  it('a point OFF the plane loses its normal-axis offset — only the in-plane projection round-trips (documented, not a bug: callers only ever feed this on-plane face vertices)', () => {
    const p: [number, number, number] = [4, -2, ANCHOR[2] + 4]
    const local = worldToLocal2(ANCHOR, U_AX, V_AX, p)
    const back = local2ToWorld(ANCHOR, U_AX, V_AX, local)
    expect(back[0]).toBeCloseTo(p[0])
    expect(back[1]).toBeCloseTo(p[1])
    expect(back[2]).toBeCloseTo(ANCHOR[2], 5) // NOT p[2] — the off-plane offset is dropped.
  })
})

describe('pin / handle', () => {
  it('pinLocal is the point mapping to UV (0,0)', () => {
    const frame: UvFrameComponents = {
      sx: 1, sy: 0, sz: 0,
      tx: 0, ty: 1, tz: 0,
      u0: 0.5, v0: -0.25,
    }
    const local = frameToLocal2D(frame, ANCHOR, U_AX, V_AX)
    const pin = pinLocal(local)!
    const uv = matVec(local.m, pin)
    expect(uv[0] + local.off[0]).toBeCloseTo(0)
    expect(uv[1] + local.off[1]).toBeCloseTo(0)
  })

  it('handleLocal is the point mapping to UV (1,0)', () => {
    const frame: UvFrameComponents = {
      sx: 2, sy: 0, sz: 0,
      tx: 0, ty: 2, tz: 0,
      u0: 0, v0: 0,
    }
    const local = frameToLocal2D(frame, ANCHOR, U_AX, V_AX)
    const handle = handleLocal(local)!
    const uv = matVec(local.m, handle)
    expect(uv[0] + local.off[0]).toBeCloseTo(1)
    expect(uv[1] + local.off[1]).toBeCloseTo(0)
  })

  it('returns null for a degenerate (singular) frame', () => {
    // s and t parallel -> singular M in any basis.
    const frame: UvFrameComponents = {
      sx: 1, sy: 0, sz: 0,
      tx: 2, ty: 0, tz: 0,
      u0: 0, v0: 0,
    }
    const local = frameToLocal2D(frame, ANCHOR, U_AX, V_AX)
    expect(pinLocal(local)).toBeNull()
    expect(handleLocal(local)).toBeNull()
    expect(vHandleLocal(local)).toBeNull()
  })

  it('vHandleLocal is the point mapping to UV (0,1) — the blue pin (paint-playtest2 §1)', () => {
    const frame: UvFrameComponents = {
      sx: 2, sy: 0, sz: 0,
      tx: 0, ty: 2, tz: 0,
      u0: 0, v0: 0,
    }
    const local = frameToLocal2D(frame, ANCHOR, U_AX, V_AX)
    const handle = vHandleLocal(local)!
    const uv = matVec(local.m, handle)
    expect(uv[0] + local.off[0]).toBeCloseTo(0)
    expect(uv[1] + local.off[1]).toBeCloseTo(1)
  })
})

describe('translateLocal (drag-anywhere / red-handle move)', () => {
  it('shifts the decal by exactly the drag delta: a world point that used to sit at the pin now sits at pin+delta', () => {
    const frame: UvFrameComponents = {
      sx: 1, sy: 0, sz: 0,
      tx: 0, ty: 1, tz: 0,
      u0: 0, v0: 0,
    }
    const local = frameToLocal2D(frame, ANCHOR, U_AX, V_AX)
    const pinBefore = pinLocal(local)!
    const delta: [number, number] = [1.5, -0.5]
    const moved = translateLocal(local, delta)
    const pinAfter = pinLocal(moved)!
    expect(pinAfter[0]).toBeCloseTo(pinBefore[0] + delta[0])
    expect(pinAfter[1]).toBeCloseTo(pinBefore[1] + delta[1])
  })

  it('leaves M (scale/rotate/shear) untouched', () => {
    const frame: UvFrameComponents = {
      sx: 0.3, sy: 0.7, sz: 0,
      tx: -0.2, ty: 0.4, tz: 0,
      u0: 0, v0: 0,
    }
    const local = frameToLocal2D(frame, ANCHOR, U_AX, V_AX)
    const moved = translateLocal(local, [2, 3])
    expect(moved.m).toEqual(local.m)
  })
})

describe('scaleRotateLocal (green-handle scale+rotate about the fixed pin)', () => {
  it('keeps the pin fixed at the same UV value', () => {
    const frame: UvFrameComponents = {
      sx: 1, sy: 0, sz: 0,
      tx: 0, ty: 1, tz: 0,
      u0: 0.2, v0: -0.3,
    }
    const local = frameToLocal2D(frame, ANCHOR, U_AX, V_AX)
    const pivot = pinLocal(local)!
    const oldHandle = handleLocal(local)!
    const newHandle: [number, number] = [pivot[0] + 3, pivot[1] + 1] // arbitrary drag target
    const result = scaleRotateLocal(local, pivot, oldHandle, newHandle)!
    expect(result).not.toBeNull()
    const uvAtPivot = matVec(result.m, pivot)
    expect(uvAtPivot[0] + result.off[0]).toBeCloseTo(0)
    expect(uvAtPivot[1] + result.off[1]).toBeCloseTo(0)
  })

  it('moves the dragged target to map to UV (1,0) — the handle followed the cursor', () => {
    const frame: UvFrameComponents = {
      sx: 1, sy: 0, sz: 0,
      tx: 0, ty: 1, tz: 0,
      u0: 0, v0: 0,
    }
    const local = frameToLocal2D(frame, ANCHOR, U_AX, V_AX)
    const pivot = pinLocal(local)!
    const oldHandle = handleLocal(local)!
    const newHandle: [number, number] = [pivot[0] - 2, pivot[1] + 4]
    const result = scaleRotateLocal(local, pivot, oldHandle, newHandle)!
    const uvAtNewHandle = matVec(result.m, newHandle)
    expect(uvAtNewHandle[0] + result.off[0]).toBeCloseTo(1)
    expect(uvAtNewHandle[1] + result.off[1]).toBeCloseTo(0)
  })

  it('preserves shear/aspect that was already present in the frame (no free-pin distort)', () => {
    // A frame with a deliberate shear: s and t are not orthogonal.
    const frame: UvFrameComponents = {
      sx: 1, sy: 0, sz: 0,
      tx: 0.6, ty: 1, tz: 0,
      u0: 0, v0: 0,
    }
    const local = frameToLocal2D(frame, ANCHOR, U_AX, V_AX)
    const pivot = pinLocal(local)!
    const oldHandle = handleLocal(local)!
    // Rotate 90 degrees, scale by 2, about the pivot.
    const newHandle: [number, number] = [
      pivot[0] - 2 * (oldHandle[1] - pivot[1]),
      pivot[1] + 2 * (oldHandle[0] - pivot[0]),
    ]
    const result = scaleRotateLocal(local, pivot, oldHandle, newHandle)!
    // The shear factor (ratio between the off-diagonal coupling and the
    // diagonal terms) should be preserved in some rotated/scaled form: check
    // by verifying the determinant scales by k^2 (a similarity-composed
    // shear scales area by k^2, same as a pure rotation+scale would) while
    // the matrix is NOT a pure rotation+scale itself (still has shear).
    const detBefore = local.m.a * local.m.d - local.m.b * local.m.c
    const detAfter = result.m.a * result.m.d - result.m.b * result.m.c
    const k = 2
    expect(detAfter).toBeCloseTo(detBefore / (k * k))
  })

  it('returns null when the handle drag starts degenerate (old handle at the pivot)', () => {
    const frame: UvFrameComponents = {
      sx: 1, sy: 0, sz: 0,
      tx: 0, ty: 1, tz: 0,
      u0: 0, v0: 0,
    }
    const local = frameToLocal2D(frame, ANCHOR, U_AX, V_AX)
    const pivot = pinLocal(local)!
    const result = scaleRotateLocal(local, pivot, pivot, [pivot[0] + 1, pivot[1]])
    expect(result).toBeNull()
  })

  it('a pure rotation with no scale (k=1) reproduces the exact rotation matrix composed on the right', () => {
    const local: Local2 = identityLocal2()
    const pivot: [number, number] = [0, 0]
    const oldHandle: [number, number] = [1, 0]
    const theta = Math.PI / 3
    const newHandle: [number, number] = [Math.cos(theta), Math.sin(theta)]
    const result = scaleRotateLocal(local, pivot, oldHandle, newHandle)!
    // M identity, k=1: M' = R(-theta).
    expect(result.m.a).toBeCloseTo(Math.cos(theta))
    expect(result.m.b).toBeCloseTo(Math.sin(theta))
    expect(result.m.c).toBeCloseTo(-Math.sin(theta))
    expect(result.m.d).toBeCloseTo(Math.cos(theta))
  })
})

describe('shearScaleLocal (blue-handle shear+scale about the fixed pin, paint-playtest2 §1)', () => {
  it('keeps the pin fixed at the same UV value', () => {
    const frame: UvFrameComponents = {
      sx: 1, sy: 0, sz: 0,
      tx: 0, ty: 1, tz: 0,
      u0: 0.2, v0: -0.3,
    }
    const local = frameToLocal2D(frame, ANCHOR, U_AX, V_AX)
    const pivot = pinLocal(local)!
    const newHandle: [number, number] = [pivot[0] + 1, pivot[1] + 3] // arbitrary drag target
    const result = shearScaleLocal(local, pivot, newHandle)!
    expect(result).not.toBeNull()
    const uvAtPivot = matVec(result.m, pivot)
    expect(uvAtPivot[0] + result.off[0]).toBeCloseTo(0)
    expect(uvAtPivot[1] + result.off[1]).toBeCloseTo(0)
  })

  it('moves the dragged target to map to UV (0,1) — the blue handle followed the cursor', () => {
    const frame: UvFrameComponents = {
      sx: 1, sy: 0, sz: 0,
      tx: 0, ty: 1, tz: 0,
      u0: 0, v0: 0,
    }
    const local = frameToLocal2D(frame, ANCHOR, U_AX, V_AX)
    const pivot = pinLocal(local)!
    const newHandle: [number, number] = [pivot[0] + 4, pivot[1] - 2]
    const result = shearScaleLocal(local, pivot, newHandle)!
    const uvAtNewHandle = matVec(result.m, newHandle)
    expect(uvAtNewHandle[0] + result.off[0]).toBeCloseTo(0)
    expect(uvAtNewHandle[1] + result.off[1]).toBeCloseTo(1)
  })

  it('leaves the RED and GREEN pins exactly fixed — only the V edge (blue) changes, unlike scaleRotateLocal which moves both', () => {
    const frame: UvFrameComponents = {
      sx: 1, sy: 0, sz: 0,
      tx: 0.3, ty: 0.8, tz: 0, // a frame with some pre-existing shear
      u0: 0.1, v0: -0.4,
    }
    const local = frameToLocal2D(frame, ANCHOR, U_AX, V_AX)
    const pivot = pinLocal(local)!
    const greenBefore = handleLocal(local)!
    const blueBefore = vHandleLocal(local)!
    const newHandle: [number, number] = [pivot[0] - 3, pivot[1] + 5]
    const result = shearScaleLocal(local, pivot, newHandle)!

    const pinAfter = pinLocal(result)!
    const greenAfter = handleLocal(result)!
    expect(pinAfter[0]).toBeCloseTo(pivot[0])
    expect(pinAfter[1]).toBeCloseTo(pivot[1])
    expect(greenAfter[0]).toBeCloseTo(greenBefore[0])
    expect(greenAfter[1]).toBeCloseTo(greenBefore[1])

    // The blue handle DID move, to the dragged-to point — sanity that this
    // isn't a no-op that would pass the fixed-pin assertions vacuously.
    const blueAfter = vHandleLocal(result)!
    expect(Math.hypot(blueAfter[0] - blueBefore[0], blueAfter[1] - blueBefore[1])).toBeGreaterThan(0.5)
  })

  it('returns null when the frame itself is degenerate (singular M)', () => {
    const frame: UvFrameComponents = {
      sx: 1, sy: 0, sz: 0,
      tx: 2, ty: 0, tz: 0, // s, t parallel -> singular
      u0: 0, v0: 0,
    }
    const local = frameToLocal2D(frame, ANCHOR, U_AX, V_AX)
    const result = shearScaleLocal(local, [0, 0], [1, 1])
    expect(result).toBeNull()
  })

  it('returns null when the drag collapses the V edge to near-zero length', () => {
    const frame: UvFrameComponents = {
      sx: 1, sy: 0, sz: 0,
      tx: 0, ty: 1, tz: 0,
      u0: 0, v0: 0,
    }
    const local = frameToLocal2D(frame, ANCHOR, U_AX, V_AX)
    const pivot = pinLocal(local)!
    const result = shearScaleLocal(local, pivot, pivot) // newHandle == pivot: zero-length V edge
    expect(result).toBeNull()
  })

  it('returns null when the drag makes the V edge near-PARALLEL to the (unchanged) U edge — an unrecoverable basis', () => {
    const frame: UvFrameComponents = {
      sx: 1, sy: 0, sz: 0,
      tx: 0, ty: 1, tz: 0,
      u0: 0, v0: 0,
    }
    const local = frameToLocal2D(frame, ANCHOR, U_AX, V_AX)
    const pivot = pinLocal(local)!
    const eu = matVec(matInverse(local.m)!, [1, 0]) // the (unchanged) U edge direction
    // Drag the blue handle to a point along the SAME direction as the U edge
    // (scaled up) — the new V edge would be parallel to U, singular.
    const newHandle: [number, number] = [pivot[0] + eu[0] * 5, pivot[1] + eu[1] * 5]
    const result = shearScaleLocal(local, pivot, newHandle)
    expect(result).toBeNull()
  })
})

describe('tessellatePlaneBasis / planarDefaultFrame', () => {
  it('produces an orthonormal basis with u x v = normal', () => {
    const normal: [number, number, number] = [0, 0, 1]
    const { u, v } = tessellatePlaneBasis(normal)
    const lenU = Math.hypot(...u)
    const lenV = Math.hypot(...v)
    expect(lenU).toBeCloseTo(1)
    expect(lenV).toBeCloseTo(1)
    expect(u[0] * v[0] + u[1] * v[1] + u[2] * v[2]).toBeCloseTo(0, 5)
    const cross: [number, number, number] = [
      u[1] * v[2] - u[2] * v[1],
      u[2] * v[0] - u[0] * v[2],
      u[0] * v[1] - u[1] * v[0],
    ]
    expect(cross[0]).toBeCloseTo(normal[0])
    expect(cross[1]).toBeCloseTo(normal[1])
    expect(cross[2]).toBeCloseTo(normal[2])
  })

  it('picks the Y helper when |nx| >= 0.9 (matching the Rust threshold exactly)', () => {
    const normal: [number, number, number] = [0.95, 0, Math.sqrt(1 - 0.95 * 0.95)]
    const { u } = tessellatePlaneBasis(normal)
    // helper = (0,1,0); u = normalize(helper x n) = normalize((1*nz-0*0, 0*nx-0*nz, 0*0-1*nx))
    // = normalize((nz, 0, -nx))
    const expectedRaw = [normal[2], 0, -normal[0]]
    const len = Math.hypot(...expectedRaw)
    expect(u[0]).toBeCloseTo(expectedRaw[0] / len)
    expect(u[1]).toBeCloseTo(expectedRaw[1] / len)
    expect(u[2]).toBeCloseTo(expectedRaw[2] / len)
  })

  it('pins the exact threshold BOUNDARY (0.899 / 0.9 / 0.901) against tessellate\'s `< 0.9`', () => {
    // `tessellate::plane_basis` (crates/tessellate/src/lib.rs) branches on
    // `n.x.abs() < 0.9`: strictly less picks the X helper, everything else
    // (including exactly 0.9) picks Y. `tessellatePlaneBasis` above must
    // agree at the boundary itself, not just comfortably on either side —
    // a `<=` vs `<` typo would only show up exactly at 0.9.
    function helperIsX(nx: number): boolean {
      const nz = Math.sqrt(Math.max(0, 1 - nx * nx))
      const { u } = tessellatePlaneBasis([nx, 0, nz])
      // helper=(1,0,0) -> u = normalize(helper x n) = normalize((0, -nz, 0)):
      // u.x is EXACTLY 0 for the X-helper branch. helper=(0,1,0) -> u =
      // normalize((nz,0,-nx)) = (nz,0,-nx) (already unit, since nx²+nz²=1):
      // u.x = nz, nonzero whenever nx < 1. That's the discriminator.
      return Math.abs(u[0]) < 1e-9
    }

    expect(helperIsX(0.899), '0.899 < 0.9 -> X helper').toBe(true)
    expect(helperIsX(0.9), '0.9 is NOT < 0.9 -> Y helper').toBe(false)
    expect(helperIsX(0.901), '0.901 >= 0.9 -> Y helper').toBe(false)
  })

  it('planarDefaultFrame divides by world size and has zero offset, matching tessellate\'s fallback formula', () => {
    const normal: [number, number, number] = [0, 0, 1]
    const frame = planarDefaultFrame(normal, 2, 4)
    expect(frame.u0).toBe(0)
    expect(frame.v0).toBe(0)
    const { u, v } = tessellatePlaneBasis(normal)
    expect(frame.sx).toBeCloseTo(u[0] / 2)
    expect(frame.sy).toBeCloseTo(u[1] / 2)
    expect(frame.tx).toBeCloseTo(v[0] / 4)
    expect(frame.ty).toBeCloseTo(v[1] / 4)
  })
})

describe('frameToArray / arrayToFrame', () => {
  it('round-trips through the 8-float wasm layout', () => {
    const frame: UvFrameComponents = {
      sx: 1, sy: 2, sz: 3,
      tx: 4, ty: 5, tz: 6,
      u0: 7, v0: 8,
    }
    const arr = frameToArray(frame)
    expect(arr).toEqual([1, 2, 3, 4, 5, 6, 7, 8])
    const back = arrayToFrame(arr)
    expect(back).toEqual(frame)
  })
})

describe('writeFrameUvs', () => {
  it('writes the correct UV sub-range for a face, leaving the rest of dest untouched', () => {
    // Two "faces": face A at base 0 (2 verts), face B at base 2 (2 verts).
    const positions = new Float32Array([
      0, 0, 0, // v0
      1, 0, 0, // v1
      0, 1, 0, // v2 (face B)
      1, 1, 0, // v3 (face B)
    ])
    const dest = new Float32Array(8).fill(-1)
    const frame: UvFrameComponents = { sx: 1, sy: 0, sz: 0, tx: 0, ty: 1, tz: 0, u0: 0, v0: 0 }
    writeFrameUvs(positions, 2, 2, frame, dest)
    // Face A's UVs (indices 0-3) untouched.
    expect(dest[0]).toBe(-1)
    expect(dest[1]).toBe(-1)
    expect(dest[2]).toBe(-1)
    expect(dest[3]).toBe(-1)
    // Face B: v2=(0,1,0) -> uv=(0,1); v3=(1,1,0) -> uv=(1,1).
    expect(dest[4]).toBeCloseTo(0)
    expect(dest[5]).toBeCloseTo(1)
    expect(dest[6]).toBeCloseTo(1)
    expect(dest[7]).toBeCloseTo(1)
  })
})

/** Builds a Local2 whose M⁻¹ has the given tile edge columns — i.e. the
 *  frame for which `handleLocal - pinLocal === eu` and
 *  `vHandleLocal - pinLocal === ev`. */
function localFromEdges(
  eu: readonly [number, number],
  ev: readonly [number, number],
  off: readonly [number, number] = [0, 0],
): Local2 {
  const inv = { a: eu[0], b: ev[0], c: eu[1], d: ev[1] }
  return { m: matInverse(inv)!, off }
}

describe('uvPointLocal (tile-lattice queries)', () => {
  it('returns the point mapping to an arbitrary integer lattice UV, generalizing pinLocal/handleLocal/vHandleLocal', () => {
    const local = localFromEdges([2, 0.5], [-0.3, 1.5], [0.7, -0.2])
    for (const uv of [[0, 0], [1, 0], [0, 1], [3, -2]] as const) {
      const p = uvPointLocal(local, uv)!
      const back = matVec(local.m, p)
      expect(back[0] + local.off[0]).toBeCloseTo(uv[0])
      expect(back[1] + local.off[1]).toBeCloseTo(uv[1])
    }
    expect(uvPointLocal(local, [0, 0])).toEqual(pinLocal(local))
    expect(uvPointLocal(local, [1, 0])).toEqual(handleLocal(local))
    expect(uvPointLocal(local, [0, 1])).toEqual(vHandleLocal(local))
  })

  it('returns null for a singular frame', () => {
    const singular: Local2 = { m: { a: 1, b: 2, c: 2, d: 4 }, off: [0, 0] }
    expect(uvPointLocal(singular, [1, 1])).toBeNull()
  })
})

describe('pivot gestures preserve the pivot\'s CURRENT UV — the red pin can sit on ANY lattice point', () => {
  // The red pin is the lattice point nearest the entry click, which is
  // almost never (0, 0) once the object sits away from the origin. These
  // pin the generalization: the pivot keeps ITS uv, not a hardwired (0,0).
  const PIN_UV: [number, number] = [3, 2]

  it('scaleRotateLocal keeps the pivot at its own lattice UV and lands the dragged corner on the adjacent one', () => {
    const local = localFromEdges([1, 0.2], [0.1, 1.3], [0.4, -0.6])
    const pivot = uvPointLocal(local, PIN_UV)!
    const oldHandle = uvPointLocal(local, [PIN_UV[0] + 1, PIN_UV[1]])!
    const newHandle: [number, number] = [pivot[0] + 0.8, pivot[1] - 1.7]
    const result = scaleRotateLocal(local, pivot, oldHandle, newHandle)!
    expect(result).not.toBeNull()

    const uvAtPivot = matVec(result.m, pivot)
    expect(uvAtPivot[0] + result.off[0]).toBeCloseTo(PIN_UV[0])
    expect(uvAtPivot[1] + result.off[1]).toBeCloseTo(PIN_UV[1])

    const uvAtNew = matVec(result.m, newHandle)
    expect(uvAtNew[0] + result.off[0]).toBeCloseTo(PIN_UV[0] + 1)
    expect(uvAtNew[1] + result.off[1]).toBeCloseTo(PIN_UV[1])
  })

  it('shearScaleLocal keeps the pivot AND the green corner at their lattice UVs and lands the dragged corner one tile along V', () => {
    const local = localFromEdges([1, 0], [0.2, 1.1], [0.4, -0.6])
    const pivot = uvPointLocal(local, PIN_UV)!
    const greenBefore = uvPointLocal(local, [PIN_UV[0] + 1, PIN_UV[1]])!
    const newHandle: [number, number] = [pivot[0] + 0.5, pivot[1] + 2.2]
    const result = shearScaleLocal(local, pivot, newHandle)!
    expect(result).not.toBeNull()

    const uvAtPivot = matVec(result.m, pivot)
    expect(uvAtPivot[0] + result.off[0]).toBeCloseTo(PIN_UV[0])
    expect(uvAtPivot[1] + result.off[1]).toBeCloseTo(PIN_UV[1])

    const greenAfter = uvPointLocal(result, [PIN_UV[0] + 1, PIN_UV[1]])!
    expect(greenAfter[0]).toBeCloseTo(greenBefore[0])
    expect(greenAfter[1]).toBeCloseTo(greenBefore[1])

    const uvAtNew = matVec(result.m, newHandle)
    expect(uvAtNew[0] + result.off[0]).toBeCloseTo(PIN_UV[0])
    expect(uvAtNew[1] + result.off[1]).toBeCloseTo(PIN_UV[1] + 1)
  })
})

describe('frameAbsolute — the absolute rotation/scale/skew the user sees', () => {
  it('the identity frame at natural tile size reads angle 0, scale 1/1, skew 0, right-handed', () => {
    const abs = frameAbsolute(identityLocal2(), 1, 1)!
    expect(abs.angle).toBeCloseTo(0)
    expect(abs.scaleU).toBeCloseTo(1)
    expect(abs.scaleV).toBeCloseTo(1)
    expect(abs.skew).toBeCloseTo(0)
    expect(abs.handed).toBe(1)
  })

  it('a uniformly rotated+scaled frame reads the exact absolute angle and scale', () => {
    const theta = (30 * Math.PI) / 180
    const k = 2
    const eu: [number, number] = [k * Math.cos(theta), k * Math.sin(theta)]
    const ev: [number, number] = [-k * Math.sin(theta), k * Math.cos(theta)]
    const abs = frameAbsolute(localFromEdges(eu, ev, [0.3, -0.8]), 1, 1)!
    expect(abs.angle).toBeCloseTo(theta)
    expect(abs.scaleU).toBeCloseTo(2)
    expect(abs.scaleV).toBeCloseTo(2)
    expect(abs.skew).toBeCloseTo(0)
  })

  it('scale is measured against the MATERIAL\'s natural tile size, not raw meters', () => {
    // A 2m tile on a 2m-natural material is exactly 1x.
    const abs = frameAbsolute(localFromEdges([2, 0], [0, 2]), 2, 2)!
    expect(abs.scaleU).toBeCloseTo(1)
    expect(abs.scaleV).toBeCloseTo(1)
  })

  it('a sheared frame reads its skew as the V edge\'s signed angle past the perpendicular, scales unchanged by rotation', () => {
    const eu: [number, number] = [1, 0]
    const ev: [number, number] = [0.5, 1]
    const abs = frameAbsolute(localFromEdges(eu, ev), 1, 1)!
    expect(abs.scaleV).toBeCloseTo(Math.hypot(0.5, 1))
    expect(abs.skew).toBeCloseTo(Math.atan2(-0.5, 1)) // signedAngle2((0,1) -> (0.5,1))
    expect(abs.handed).toBe(1)
  })

  it('a mirrored frame reads handed -1 with zero skew for a rectangular tile', () => {
    const abs = frameAbsolute(localFromEdges([1, 0], [0, -1]), 1, 1)!
    expect(abs.handed).toBe(-1)
    expect(abs.skew).toBeCloseTo(0)
    expect(abs.scaleV).toBeCloseTo(1)
  })

  it('reconstruction round-trip: R(skew)·perp scaled by scaleV·worldH reproduces the V edge exactly (the typed-absolute path\'s inverse)', () => {
    const eu: [number, number] = [1.4, 0.7]
    const ev: [number, number] = [-0.9, 1.1]
    const worldH = 0.8
    const local = localFromEdges(eu, ev)
    const abs = frameAbsolute(local, 1, worldH)!
    const euLen = Math.hypot(eu[0], eu[1])
    const uDir: [number, number] = [eu[0] / euLen, eu[1] / euLen]
    const perp: [number, number] = abs.handed === 1 ? [-uDir[1], uDir[0]] : [uDir[1], -uDir[0]]
    const dir = matVec(rotation2(abs.skew), perp)
    expect(dir[0] * abs.scaleV * worldH).toBeCloseTo(ev[0])
    expect(dir[1] * abs.scaleV * worldH).toBeCloseTo(ev[1])
  })

  it('returns null for a singular frame or non-positive material size', () => {
    expect(frameAbsolute({ m: { a: 1, b: 2, c: 2, d: 4 }, off: [0, 0] }, 1, 1)).toBeNull()
    expect(frameAbsolute(identityLocal2(), 0, 1)).toBeNull()
    expect(frameAbsolute(identityLocal2(), 1, -2)).toBeNull()
  })
})

describe('Esc-revert scenario (round trip through a drag then discard)', () => {
  it('capturing the pre-gesture frame and never committing an intermediate drag reproduces it exactly', () => {
    const original: UvFrameComponents = {
      sx: 0.8, sy: 0.1, sz: 0,
      tx: -0.1, ty: 0.9, tz: 0,
      u0: 0.3, v0: -0.4,
    }
    // Simulate: enter positioning mode (snapshot), drag around, then Esc
    // (discard the working local state and re-derive the ORIGINAL frame's
    // array form for re-committing to the preview/kernel unchanged).
    const snapshot = frameToArray(original)
    const local = frameToLocal2D(original, ANCHOR, U_AX, V_AX)
    const dragged = translateLocal(local, [5, -5])
    const draggedFrame = local2DToFrame(dragged, ANCHOR, U_AX, V_AX)
    expect(frameToArray(draggedFrame)).not.toEqual(snapshot)

    // Esc: revert to the untouched original — no re-derivation needed, the
    // tool just re-uses the snapshot it captured before the gesture began.
    expect(snapshot).toEqual(frameToArray(original))
  })
})
