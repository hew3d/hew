/**
 * Shared test double for the wasm `draft_*` surface (unified drafting). Spread
 * `draftMock().members` into a tool test's mock scene; assert through the
 * `vi.fn`s (`draft_add_segment`, `draft_begin_face`, ...) or the `log`.
 *
 * Behaves like the real engine at the seams the tools see: every begin
 * returns a fresh handle and remembers its kind, `draft_end` answers a
 * report JSON naming what a draft of that kind changes (a face draft
 * changes its object's topology; a plane/sketch draft touches a sketch),
 * and `draft_cancel` closes the open draft. Only one draft may be open —
 * a second begin throws `DraftAlreadyOpen`, like the kernel. It also
 * stubs the sketch-region fill pick as a miss (see the members' tail).
 */
import { vi } from 'vitest'

export interface DraftLog {
  /** Every begin call, in order, with its kind and raw args. */
  begins: Array<{ kind: 'plane' | 'plane_in_instance' | 'face' | 'face_in_instance' | 'sketch' | 'ctx' | 'detached'; args: unknown[] }>
  /** Every segment: [ax, ay, az, bx, by, bz]. */
  segments: Array<[number, number, number, number, number, number]>
  /** Curve brackets, in order: 'circle' | 'polygon' (begin) and 'end'. */
  curves: Array<{ kind: 'circle' | 'polygon' | 'curve' | 'end'; center?: number[]; radius?: number }>
  /** Every `draft_offset_region`: [regionKey, distance]. */
  offsets: Array<[bigint, number]>
  /** Every `draft_move_vertex`: [at, to] world positions. */
  moves: Array<[[number, number, number], [number, number, number]]>
  ended: number
  cancelled: number
}

export interface DraftMockOpts {
  /** `regions_created` for the nth `draft_add_segment` (0-based, across the mock's life). */
  regionsCreated?: (n: number) => bigint[]
  /** Throw from the nth `draft_add_segment` (0-based) with this error. */
  failSegmentAt?: (n: number) => Error | null
  /** Throw from `draft_end`. */
  failEnd?: () => Error | null
  /** Objects a `draft_begin_ctx` draft reports as changed (a face context). */
  ctxObjects?: bigint[]
  /** Plane-membership check: throw `PointOffPlane` for off-plane segments. */
  enforcePlane?: boolean
  /** Extra report fields (`faces_closed`, `cells`) merged into the nth `draft_end` report. */
  reportExtras?: (n: number) => Record<string, unknown>
}

export function draftMock(opts: DraftMockOpts = {}) {
  const log: DraftLog = { begins: [], segments: [], curves: [], offsets: [], moves: [], ended: 0, cancelled: 0 }
  let next = 500n
  let open: { handle: bigint; objects: bigint[]; sketches: bigint[]; plane: number[] | null } | null = null
  let segmentCount = 0
  let sketchCounter = 900n

  const begin = (
    kind: DraftLog['begins'][number]['kind'],
    args: unknown[],
    objects: bigint[],
    plane: number[] | null,
  ): bigint => {
    if (open !== null) throw new Error('DraftAlreadyOpen: finish or cancel the open draft first')
    log.begins.push({ kind, args })
    next += 1n
    sketchCounter += 1n
    open = { handle: next, objects, sketches: objects.length === 0 ? [sketchCounter] : [], plane }
    return next
  }

  const members = {
    draft_begin_plane: vi.fn((px: number, py: number, pz: number, nx: number, ny: number, nz: number, ...rest: unknown[]) =>
      begin('plane', [px, py, pz, nx, ny, nz, ...rest], [], [px, py, pz, nx, ny, nz])),
    draft_begin_plane_in_instance: vi.fn((inst: bigint, px: number, py: number, pz: number, nx: number, ny: number, nz: number, ...rest: unknown[]) =>
      begin('plane_in_instance', [inst, px, py, pz, nx, ny, nz, ...rest], [], [px, py, pz, nx, ny, nz])),
    draft_begin_face: vi.fn((object: bigint, face: bigint, ...rest: unknown[]) =>
      begin('face', [object, face, ...rest], [object], null)),
    draft_begin_face_in_instance: vi.fn((inst: bigint, object: bigint, face: bigint, ...rest: unknown[]) =>
      begin('face_in_instance', [inst, object, face, ...rest], [object], null)),
    draft_begin_sketch: vi.fn((sketch: bigint, ...rest: unknown[]) =>
      begin('sketch', [sketch, ...rest], [], null)),
    draft_begin_ctx: vi.fn((ctx: string, ...rest: unknown[]) =>
      begin('ctx', [ctx, ...rest], opts.ctxObjects ?? [], null)),
    draft_begin_detached: vi.fn((ctx: string, ...rest: unknown[]) =>
      begin('detached', [ctx, ...rest], [], null)),
    draft_offset_region: vi.fn((d: bigint, key: bigint, distance: number) => {
      if (open === null || open.handle !== d) throw new Error('StaleDraft: no such draft')
      log.offsets.push([key, distance])
    }),
    draft_move_vertex: vi.fn((d: bigint, x: number, y: number, z: number, tx: number, ty: number, tz: number) => {
      if (open === null || open.handle !== d) throw new Error('StaleDraft: no such draft')
      log.moves.push([[x, y, z], [tx, ty, tz]])
    }),
    draft_add_segment: vi.fn((d: bigint, ax: number, ay: number, az: number, bx: number, by: number, bz: number) => {
      if (open === null || open.handle !== d) throw new Error('StaleDraft: no such draft')
      const n = segmentCount++
      const fail = opts.failSegmentAt?.(n) ?? null
      if (fail !== null) throw fail
      if (opts.enforcePlane === true && open.plane !== null) {
        const [ox, oy, oz, nx, ny, nz] = open.plane
        for (const p of [[ax, ay, az], [bx, by, bz]]) {
          if (Math.abs(nx * (p[0] - ox) + ny * (p[1] - oy) + nz * (p[2] - oz)) > 1e-9) {
            throw new Error('PointOffPlane: point is off the draft plane')
          }
        }
      }
      log.segments.push([ax, ay, az, bx, by, bz])
      const regions = opts.regionsCreated?.(n) ?? []
      return {
        new_edges: () => new BigUint64Array([]),
        regions_created: () => new BigUint64Array(regions),
        regions_removed: () => new BigUint64Array([]),
        free: vi.fn(),
      }
    }),
    draft_begin_curve: vi.fn(() => {
      log.curves.push({ kind: 'curve' })
    }),
    draft_begin_curve_with: vi.fn((_d: bigint, center: Float64Array, radius: number) => {
      log.curves.push({ kind: 'circle', center: Array.from(center), radius })
    }),
    draft_begin_polygon_with: vi.fn((_d: bigint, center: Float64Array, radius: number) => {
      log.curves.push({ kind: 'polygon', center: Array.from(center), radius })
    }),
    draft_end_curve: vi.fn(() => {
      log.curves.push({ kind: 'end' })
    }),
    draft_end: vi.fn((d: bigint) => {
      if (open === null || open.handle !== d) throw new Error('StaleDraft: no such draft')
      const fail = opts.failEnd?.() ?? null
      if (fail !== null) throw fail
      const report = {
        objects: open.objects.map((o) => Number(o)),
        sketches: open.sketches.map((s) => Number(s)),
        kept_as_sketch: [],
        ...(opts.reportExtras?.(log.ended) ?? {}),
      }
      open = null
      log.ended += 1
      return JSON.stringify(report)
    }),
    draft_cancel: vi.fn((d: bigint) => {
      if (open === null || open.handle !== d) return false
      open = null
      log.cancelled += 1
      return true
    }),
    // Not part of the draft surface, but every draw tool's idle hover asks
    // it (`resolveIdleDrawTarget`'s fill pick, view-plane design §2 step
    // 4b): no filled region under the ray unless a test overrides these.
    pick_sketch_region: vi.fn(() => undefined),
    pick_sketch_region_in_instance: vi.fn(() => undefined),
  }
  return { members, log, isOpen: () => open !== null }
}
