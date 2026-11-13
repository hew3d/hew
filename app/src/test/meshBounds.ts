/**
 * meshBounds — a test-only `object_bounds` built from a mocked
 * `object_mesh`: the kernel bounds an object's vertices under a pose, and a
 * mock that already carries positions answers the same way through
 * `meshWorldBounds`, so a scene mock keeps describing parts by their
 * corners.
 */

import { meshWorldBounds } from '../panels/objectBounds'

export function objectBoundsFromMesh(
  objectMesh: (id: bigint) => { positions(): Float32Array; free(): void },
): (id: bigint, pose?: Float64Array) => Float64Array {
  return (id, pose) => {
    const mesh = objectMesh(id)
    try {
      const b = meshWorldBounds(mesh.positions(), pose)
      return b === null ? new Float64Array() : Float64Array.from([...b.min, ...b.max])
    } finally {
      mesh.free()
    }
  }
}

type MeshFn = (id: bigint) => { positions(): Float32Array; free(): void }

/**
 * A test-only `part_frame_json` for scenes of axis-aligned boxes: the
 * kernel measures an object or group along its largest face, which for a
 * box reads longest first and thickness last; an instance reads its
 * definition's box scaled by its pose's column lengths, in the
 * definition's own axis order (`Scene::part_frame_json`).
 */
export function partFrameFromMesh(
  objectMesh: MeshFn,
  groupMembers: (id: bigint) => { kind: string; id: bigint }[] = () => [],
  instanceFrame: (id: bigint) => { local: readonly [number, number, number]; pose: ArrayLike<number> } | undefined = () => undefined,
): (kind: number, id: bigint) => string {
  const leafBounds = (kind: string, id: bigint, acc: { min: number[]; max: number[] } | null): { min: number[]; max: number[] } | null => {
    if (kind === 'object') {
      const mesh = objectMesh(id)
      try {
        const b = meshWorldBounds(mesh.positions())
        if (b === null) return acc
        if (acc === null) return { min: [...b.min], max: [...b.max] }
        return { min: acc.min.map((v, i) => Math.min(v, b.min[i])), max: acc.max.map((v, i) => Math.max(v, b.max[i])) }
      } finally {
        mesh.free()
      }
    }
    if (kind === 'group') {
      let out = acc
      for (const m of groupMembers(id)) out = leafBounds(m.kind, m.id, out)
      return out
    }
    return acc
  }
  return (kind, id) => {
    if (kind === 2) {
      const f = instanceFrame(id)
      if (f === undefined) throw new Error('NoPartFrame')
      const p = f.pose
      const col = (j: number) => Math.hypot(p[j], p[4 + j], p[8 + j])
      return JSON.stringify({ extents: [f.local[0] * col(0), f.local[1] * col(1), f.local[2] * col(2)], source: 'definition' })
    }
    const b = leafBounds(kind === 0 ? 'object' : 'group', id, null)
    if (b === null) throw new Error('NoPartFrame')
    const ext = b.max.map((v, i) => v - b.min[i]).sort((x, y) => y - x)
    return JSON.stringify({ extents: ext, source: 'face' })
  }
}
