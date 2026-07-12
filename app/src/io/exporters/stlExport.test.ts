/**
 *  — binary STL writer tests.
 *
 * The writer is pure (triangle soup in → bytes out), so every byte-layout
 * requirement is asserted by parsing the writer's own output back with an
 * independent little-endian reader implemented here.
 */

import { describe, expect, it } from 'vitest'
import {
  STL_HEADER_BYTES,
  STL_TRIANGLE_BYTES,
  writeBinaryStl,
  collectKernelTriangles,
  collectNonSolidObjects,
  type SolidQueryScene,
  type StlExportScene,
} from './stlExport'

// ---------------------------------------------------------------------------
// Independent binary-STL reader (little-endian throughout).
// ---------------------------------------------------------------------------

interface ParsedTriangle {
  normal: [number, number, number]
  verts: [number, number, number][]
  attr: number
}

function parseStl(bytes: Uint8Array): {
  header: string
  count: number
  tris: ParsedTriangle[]
} {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  const header = new TextDecoder('ascii')
    .decode(bytes.subarray(0, STL_HEADER_BYTES))
    .replace(/\0+$/, '')
  const count = view.getUint32(STL_HEADER_BYTES, true)
  const tris: ParsedTriangle[] = []
  let off = STL_HEADER_BYTES + 4
  for (let t = 0; t < count; t++) {
    const normal: [number, number, number] = [
      view.getFloat32(off, true),
      view.getFloat32(off + 4, true),
      view.getFloat32(off + 8, true),
    ]
    off += 12
    const verts: [number, number, number][] = []
    for (let v = 0; v < 3; v++) {
      verts.push([
        view.getFloat32(off, true),
        view.getFloat32(off + 4, true),
        view.getFloat32(off + 8, true),
      ])
      off += 12
    }
    const attr = view.getUint16(off, true)
    off += 2
    tris.push({ normal, verts, attr })
  }
  return { header, count, tris }
}

/** One unit right triangle in the XY plane at z=0, CCW (normal +Z), meters. */
const TRI_XY = [0, 0, 0, 1, 0, 0, 0, 1, 0]

describe('writeBinaryStl — byte layout', () => {
  it('empty input → header + zero count, no triangle records', () => {
    const { bytes, triangleCount, skippedDegenerate } = writeBinaryStl([])
    expect(bytes.byteLength).toBe(STL_HEADER_BYTES + 4)
    expect(triangleCount).toBe(0)
    expect(skippedDegenerate).toBe(0)
    expect(parseStl(bytes).count).toBe(0)
  })

  it('header is 80 bytes, names Hew + millimeters, and never starts with "solid"', () => {
    const { bytes } = writeBinaryStl(TRI_XY, '1.2.3')
    const { header } = parseStl(bytes)
    expect(header).toBe('Hew 1.2.3 binary STL, millimeters')
    expect(header.startsWith('solid')).toBe(false)
    // Padding after the text is NUL, not garbage.
    for (let i = header.length; i < STL_HEADER_BYTES; i++) {
      expect(bytes[i]).toBe(0)
    }
  })

  it('uses a 50-byte stride: total = 84 + 50 × count', () => {
    const two = [...TRI_XY, ...TRI_XY.map((v) => v + 5)]
    const { bytes, triangleCount } = writeBinaryStl(two)
    expect(triangleCount).toBe(2)
    expect(bytes.byteLength).toBe(STL_HEADER_BYTES + 4 + 2 * STL_TRIANGLE_BYTES)
  })

  it('writes the triangle count as little-endian u32', () => {
    const { bytes } = writeBinaryStl(TRI_XY)
    // LE: low byte first.
    expect(bytes[STL_HEADER_BYTES]).toBe(1)
    expect(bytes[STL_HEADER_BYTES + 1]).toBe(0)
    expect(bytes[STL_HEADER_BYTES + 2]).toBe(0)
    expect(bytes[STL_HEADER_BYTES + 3]).toBe(0)
  })

  it('writes vertex floats little-endian (1000mm = 0x447A0000 stored low-byte first)', () => {
    const { bytes } = writeBinaryStl(TRI_XY)
    // First vertex starts after normal (12 bytes): x of v1 = 1 m = 1000 mm.
    const off = STL_HEADER_BYTES + 4 + 12 + 12 // second vertex (v1) x
    // f32 1000.0 = 0x447A0000 big-endian → LE bytes 00 00 7A 44.
    expect([...bytes.subarray(off, off + 4)]).toEqual([0x00, 0x00, 0x7a, 0x44])
  })

  it('sets every attribute byte count to 0', () => {
    const two = [...TRI_XY, ...TRI_XY.map((v) => v + 5)]
    const { bytes } = writeBinaryStl(two)
    for (const tri of parseStl(bytes).tris) {
      expect(tri.attr).toBe(0)
    }
  })
})

describe('writeBinaryStl — geometry', () => {
  it('scales meters to millimeters (×1000)', () => {
    const { bytes } = writeBinaryStl([0.001, 0, 0, 1, 2, 3, 0, 0.5, 0])
    const [tri] = parseStl(bytes).tris
    expect(tri.verts[0]).toEqual([1, 0, 0])
    expect(tri.verts[1]).toEqual([1000, 2000, 3000])
    expect(tri.verts[2]).toEqual([0, 500, 0])
  })

  it('computes the normal from CCW winding (right-hand rule)', () => {
    const { bytes } = writeBinaryStl(TRI_XY)
    const [tri] = parseStl(bytes).tris
    expect(tri.normal[0]).toBeCloseTo(0)
    expect(tri.normal[1]).toBeCloseTo(0)
    expect(tri.normal[2]).toBeCloseTo(1)
  })

  it('flips the normal when the winding reverses', () => {
    const reversed = [0, 0, 0, 0, 1, 0, 1, 0, 0]
    const { bytes } = writeBinaryStl(reversed)
    expect(parseStl(bytes).tris[0].normal[2]).toBeCloseTo(-1)
  })

  it('normals are unit length for a non-axis-aligned triangle', () => {
    const { bytes } = writeBinaryStl([0, 0, 0, 1, 0, 1, 0, 1, 1])
    const [nx, ny, nz] = parseStl(bytes).tris[0].normal
    expect(Math.hypot(nx, ny, nz)).toBeCloseTo(1)
  })

  it('skips zero-area (degenerate) triangles and counts them', () => {
    const collinear = [0, 0, 0, 1, 1, 1, 2, 2, 2]
    const zeroSpan = [3, 3, 3, 3, 3, 3, 3, 3, 3]
    const { bytes, triangleCount, skippedDegenerate } = writeBinaryStl([
      ...collinear,
      ...TRI_XY,
      ...zeroSpan,
    ])
    expect(triangleCount).toBe(1)
    expect(skippedDegenerate).toBe(2)
    const parsed = parseStl(bytes)
    expect(parsed.count).toBe(1)
    expect(parsed.tris[0].verts[1]).toEqual([1000, 0, 0])
  })
})

describe('collectKernelTriangles', () => {
  const IDENTITY_POSE = new Float64Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0])

  function mockExportScene(overrides: Partial<StlExportScene>): StlExportScene {
    return {
      object_ids: () => new BigUint64Array(),
      instance_ids: () => new BigUint64Array(),
      instance_def: () => undefined,
      component_member_objects: () => new BigUint64Array(),
      object_solid: () => true,
      object_name: () => undefined,
      instance_pose: () => IDENTITY_POSE,
      object_export_triangles: () => new Float32Array(TRI_XY),
      ...overrides,
    }
  }

  it('concatenates every top-level object soup verbatim (baked world coords)', () => {
    const scene = mockExportScene({
      object_ids: () => new BigUint64Array([1n, 2n]),
      object_export_triangles: (id) =>
        new Float32Array(TRI_XY.map((v) => v + Number(id) * 10)),
    })
    const tris = collectKernelTriangles(scene, 48)
    expect(tris.length).toBe(18)
    expect(tris.slice(0, 3)).toEqual([10, 10, 10])
    expect(tris.slice(9, 12)).toEqual([20, 20, 20])
  })

  it('passes the chosen resolution through to the kernel', () => {
    const seen: number[] = []
    const scene = mockExportScene({
      object_ids: () => new BigUint64Array([1n]),
      object_export_triangles: (_id, segments) => {
        seen.push(segments)
        return new Float32Array(TRI_XY)
      },
    })
    collectKernelTriangles(scene, 96)
    expect(seen).toEqual([96])
  })

  it('applies instance poses to definition members (translation)', () => {
    const scene = mockExportScene({
      instance_ids: () => new BigUint64Array([10n]),
      instance_def: () => 7n,
      component_member_objects: () => new BigUint64Array([42n]),
      instance_pose: () => new Float64Array([1, 0, 0, 10, 0, 1, 0, 0, 0, 0, 1, 0]),
    })
    const tris = collectKernelTriangles(scene, 0)
    expect(tris.slice(0, 9)).toEqual([10, 0, 0, 11, 0, 0, 10, 1, 0])
  })

  it('flips winding under a mirrored (negative-determinant) pose so normals stay outward', () => {
    const scene = mockExportScene({
      instance_ids: () => new BigUint64Array([10n]),
      instance_def: () => 7n,
      component_member_objects: () => new BigUint64Array([42n]),
      // Mirror across YZ: det < 0.
      instance_pose: () => new Float64Array([-1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0]),
    })
    const { bytes } = writeBinaryStl(collectKernelTriangles(scene, 0))
    // A +Z-facing triangle mirrored in X still faces +Z; without the winding
    // flip the emitted normal would be −Z (inward).
    expect(parseStl(bytes).tris[0].normal[2]).toBeCloseTo(1)
  })

  it('skips instances with stale defs or poses rather than guessing', () => {
    const scene = mockExportScene({
      instance_ids: () => new BigUint64Array([10n, 11n]),
      instance_def: (id) => (id === 10n ? 7n : undefined),
      instance_pose: (id) => (id === 10n ? undefined : IDENTITY_POSE),
      component_member_objects: () => new BigUint64Array([42n]),
    })
    expect(collectKernelTriangles(scene, 0).length).toBe(0)
  })
})

describe('collectNonSolidObjects', () => {
  function mockScene(overrides: Partial<SolidQueryScene>): SolidQueryScene {
    return {
      object_ids: () => new BigUint64Array(),
      instance_ids: () => new BigUint64Array(),
      instance_def: () => undefined,
      component_member_objects: () => new BigUint64Array(),
      object_solid: () => true,
      object_name: () => undefined,
      ...overrides,
    }
  }

  it('returns [] when every object is solid', () => {
    const scene = mockScene({ object_ids: () => new BigUint64Array([1n, 2n]) })
    expect(collectNonSolidObjects(scene)).toEqual([])
  })

  it('names non-solid top-level objects, with an id fallback for unnamed ones', () => {
    const scene = mockScene({
      object_ids: () => new BigUint64Array([1n, 2n, 3n]),
      object_solid: (id) => id === 2n,
      object_name: (id) => (id === 1n ? 'Roof' : undefined),
    })
    expect(collectNonSolidObjects(scene)).toEqual([
      { id: 1n, name: 'Roof' },
      { id: 3n, name: 'Object 3' },
    ])
  })

  it('includes instance definition members, deduplicated across instances', () => {
    const scene = mockScene({
      instance_ids: () => new BigUint64Array([10n, 11n]),
      instance_def: () => 7n, // both instances share one definition
      component_member_objects: () => new BigUint64Array([42n]),
      object_solid: (id) => id !== 42n,
      object_name: () => 'Leg',
    })
    expect(collectNonSolidObjects(scene)).toEqual([{ id: 42n, name: 'Leg' }])
  })
})
