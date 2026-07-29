/**
 * uvFrameMath — pure math for the Position Texture tool (paint-tool design
 * §3: SketchUp's fixed-pin move/scale/rotate subset over `kernel::UvFrame`).
 *
 * No three.js or DOM imports — fully testable in Node/vitest, same posture
 * as transformMath.ts.
 *
 * ## The frame and its 2D local representation
 *
 * A `kernel::UvFrame` maps a 3D world position to UV: `uv = (s·p + u0, t·p +
 * v0)`, `s`/`t` world-space gradient vectors (see the Rust doc comment on
 * `UvFrame`). Editing gestures (translate, pin-anchored scale/rotate) are far
 * simpler to express as a 2D affine map in the face's OWN plane, so every
 * gesture here:
 *
 *   1. Projects the frame into a 2D "local" representation `{ M, c }` —
 *      `uv = M · (a, b) + c`, where `(a, b)` are the face-plane-local
 *      coordinates of a world point `p = anchor + a·uAx + b·vAx` (`anchor`
 *      any fixed point on the face plane, `uAx`/`vAx` any orthonormal basis
 *      spanning it — the gestures below are invariant to both choices, so
 *      callers may pick anything consistent for one editing session, e.g.
 *      `planeBasis()` from transformMath.ts).
 *   2. Edits `{ M, c }` with ordinary 2×2 linear algebra.
 *   3. Projects back to `{ sx, sy, sz, tx, ty, tz, u0, v0 }` for the kernel.
 *
 * This round-trip is basis/anchor-invariant: only genuine geometric
 * quantities (world drag vectors, angles, ratios) drive the edits, so any
 * consistent choice of anchor/basis within ONE gesture reproduces the same
 * `UvFrameComponents` result.
 *
 * ## Pin convention (SketchUp's fixed-pin tile)
 *
 * Pins sit at the CORNERS of one texture tile laid on the face — the UV
 * lattice. The RED origin pin is the world point mapping to an integer UV
 * lattice point `(m, n)` (chosen once per session as the lattice point
 * nearest the entry click — see {@link uvPointLocal}); the GREEN pin is the
 * adjacent corner one tile along U, mapping to `(m+1, n)`; the BLUE pin is
 * one tile along V, mapping to `(m, n+1)`. Because the pins are corners of
 * the visible tile, "drag the corner" reads directly: the tile's edge
 * follows the cursor.
 *
 * Dragging GREEN scales+rotates the texture as a rigid decal in world space,
 * about the FIXED red pin — a similarity applied to BOTH `s`/`t` gradients
 * together, so it preserves whatever shear/aspect the frame already had
 * ({@link scaleRotateLocal}). Dragging BLUE is SketchUp's distort pin
 * restricted to the affine-expressible sense: it moves the V edge vector
 * directly to the dragged-to point and leaves the U edge vector (and hence
 * the red/green pins) untouched — shear and non-uniform scale, no rotation
 * component of its own ({@link shearScaleLocal}). Dragging anywhere else on
 * the face (or the red pin itself) translates the decal rigidly; all three
 * gestures keep the OTHER two pins exactly fixed, which is what "fixed-pin
 * mode" means throughout this file. Both pivot gestures preserve the
 * pivot's CURRENT UV value — whatever lattice point the red pin sits on —
 * not a hardwired `(0, 0)`.
 *
 * ## Absolute frame readout ({@link frameAbsolute})
 *
 * The user-facing rotation and scale are ABSOLUTE, not relative to a drag's
 * start: rotation is the angle of the tile's U edge measured against the
 * face's own planar axes (the axes an untextured face's default projection
 * aligns to — callers pass {@link tessellatePlaneBasis} as the session
 * basis so local +a IS that reference), and scale is the tile edge length
 * over the material's natural world size (`1` = the texture renders at its
 * natural size). Typing `45` or `2x` targets these absolute quantities.
 *
 * `kernel::UvFrame` cannot express SketchUp's fourth (yellow, perspective)
 * pin — there is no `w` row/divide in `UvFrame::apply` — so it is
 * deliberately not implemented here; see paint-playtest2.md §1.
 */

/** A `kernel::UvFrame`'s flat components — the exact 8-float layout
 *  `scene.set_face_uv_frame`/`face_uv_frame` use on the wasm boundary. */
export interface UvFrameComponents {
  sx: number
  sy: number
  sz: number
  tx: number
  ty: number
  tz: number
  u0: number
  v0: number
}

/** Row-major 2×2 matrix `[[a, b], [c, d]]`. */
export interface Mat2 {
  a: number
  b: number
  c: number
  d: number
}

/** The 2D local representation of a `UvFrame`: `uv = M · (a, b) + off`. */
export interface Local2 {
  readonly m: Mat2
  readonly off: readonly [number, number]
}

const IDENTITY_MAT2: Mat2 = { a: 1, b: 0, c: 0, d: 1 }

// ─────────────────────────────────────────────────────────── 2×2 algebra

export function matVec(m: Mat2, v: readonly [number, number]): [number, number] {
  return [m.a * v[0] + m.b * v[1], m.c * v[0] + m.d * v[1]]
}

export function matMul(x: Mat2, y: Mat2): Mat2 {
  return {
    a: x.a * y.a + x.b * y.c,
    b: x.a * y.b + x.b * y.d,
    c: x.c * y.a + x.d * y.c,
    d: x.c * y.b + x.d * y.d,
  }
}

/** `M^-1`, or `null` if `M` is singular (`|det| < 1e-12`) — a degenerate
 *  frame (e.g. `s`/`t` collapsed to parallel vectors) has no invertible local
 *  map; callers treat that as "can't derive pin/handle", not a crash. */
export function matInverse(m: Mat2): Mat2 | null {
  const det = m.a * m.d - m.b * m.c
  if (Math.abs(det) < 1e-12) return null
  const inv = 1 / det
  return { a: m.d * inv, b: -m.b * inv, c: -m.c * inv, d: m.a * inv }
}

/** Standard counter-clockwise 2D rotation matrix by `theta` radians. */
export function rotation2(theta: number): Mat2 {
  const cs = Math.cos(theta)
  const sn = Math.sin(theta)
  return { a: cs, b: -sn, c: sn, d: cs }
}

function scaleMat2(m: Mat2, k: number): Mat2 {
  return { a: m.a * k, b: m.b * k, c: m.c * k, d: m.d * k }
}

function sub2(x: readonly [number, number], y: readonly [number, number]): [number, number] {
  return [x[0] - y[0], x[1] - y[1]]
}

function len2(v: readonly [number, number]): number {
  return Math.sqrt(v[0] * v[0] + v[1] * v[1])
}

/** 2D cross product (the scalar z-component) — its magnitude relative to the
 *  two vectors' lengths is `sin` of the angle between them, used by
 *  {@link shearScaleLocal}'s near-parallel degenerate guard. */
function cross2(a: readonly [number, number], b: readonly [number, number]): number {
  return a[0] * b[1] - a[1] * b[0]
}

/** Signed angle (radians, atan2) from vector `f` to vector `t` in the 2D
 *  plane. Returns 0 if either is ~zero length (no reference direction). */
export function signedAngle2(f: readonly [number, number], t: readonly [number, number]): number {
  const fLen = len2(f)
  const tLen = len2(t)
  if (fLen < 1e-9 || tLen < 1e-9) return 0
  const cross = f[0] * t[1] - f[1] * t[0]
  const dot = f[0] * t[0] + f[1] * t[1]
  return Math.atan2(cross, dot)
}

// ───────────────────────────────────────────────────────── 3-vector helpers

type Vec3Tuple = readonly [number, number, number]

function dot3(a: Vec3Tuple, b: Vec3Tuple): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

function addScaled3(base: Vec3Tuple, a: Vec3Tuple, fa: number, b: Vec3Tuple, fb: number): [number, number, number] {
  return [base[0] + a[0] * fa + b[0] * fb, base[1] + a[1] * fa + b[1] * fb, base[2] + a[2] * fa + b[2] * fb]
}

// ───────────────────────────────────────────────────── frame <-> local2D

/** Projects `frame` into its 2D local representation relative to `anchor`
 *  (any point on the face plane) and the orthonormal in-plane basis
 *  `uAx`/`vAx` (any consistent choice — see the module doc comment). */
export function frameToLocal2D(
  frame: UvFrameComponents,
  anchor: Vec3Tuple,
  uAx: Vec3Tuple,
  vAx: Vec3Tuple,
): Local2 {
  const s: Vec3Tuple = [frame.sx, frame.sy, frame.sz]
  const t: Vec3Tuple = [frame.tx, frame.ty, frame.tz]
  const m: Mat2 = {
    a: dot3(s, uAx),
    b: dot3(s, vAx),
    c: dot3(t, uAx),
    d: dot3(t, vAx),
  }
  const off: [number, number] = [frame.u0 + dot3(s, anchor), frame.v0 + dot3(t, anchor)]
  return { m, off }
}

/**
 * Inverse of {@link frameToLocal2D}: recovers the 3D `UvFrameComponents`
 * from a local 2D representation and the SAME anchor/basis used to build it
 * (a different anchor/basis reproduces the same frame, since the round trip
 * is invariant — see the module doc comment).
 *
 * INTENDED canonicalization: `s`/`t` come back as PURE linear combinations
 * of `uAx`/`vAx` — i.e. entirely IN the face's own plane — even if the
 * frame this session started from had a component along the plane's
 * NORMAL (`frameToLocal2D` drops it; there is no 2D local coordinate for an
 * out-of-plane direction to survive in). This is a no-op on the actual
 * rendered result: every face vertex `p` this frame is ever applied to
 * lies exactly in the plane, so `s`'s normal-axis component is always
 * dotted against a zero in-plane-relative coordinate and contributes
 * nothing to `apply(p)` either way (see the "round-trips functionally, not
 * byte-for-byte" test in uvFrameMath.test.ts for the worked example). It
 * is also not a live concern in practice: the only producer of a frame
 * with genuine gradient data (`mesh_heal::uv::fit_uv_frame`, used by the
 * DAE/glTF/SKP importers alike) always lifts its fit strictly back through
 * the face's own in-plane basis (`s = α·e1 + β·e2`) and never introduces a
 * normal-axis component in the first place — so Position Texture, run on
 * an imported face, canonicalizes a coefficient nothing currently produces
 * and nothing currently needs.
 */
export function local2DToFrame(
  local: Local2,
  anchor: Vec3Tuple,
  uAx: Vec3Tuple,
  vAx: Vec3Tuple,
): UvFrameComponents {
  const s = addScaled3([0, 0, 0], uAx, local.m.a, vAx, local.m.b)
  const t = addScaled3([0, 0, 0], uAx, local.m.c, vAx, local.m.d)
  return {
    sx: s[0],
    sy: s[1],
    sz: s[2],
    tx: t[0],
    ty: t[1],
    tz: t[2],
    u0: local.off[0] - dot3(s, anchor),
    v0: local.off[1] - dot3(t, anchor),
  }
}

/** World (or, equivalently, local-plane) point for local coordinates
 *  `(a, b)`: `anchor + a·uAx + b·vAx`. */
export function local2ToWorld(
  anchor: Vec3Tuple,
  uAx: Vec3Tuple,
  vAx: Vec3Tuple,
  ab: readonly [number, number],
): [number, number, number] {
  return addScaled3(anchor, uAx, ab[0], vAx, ab[1])
}

/** Local `(a, b)` coordinates of world point `p`, relative to `anchor`/basis. */
export function worldToLocal2(
  anchor: Vec3Tuple,
  uAx: Vec3Tuple,
  vAx: Vec3Tuple,
  p: Vec3Tuple,
): [number, number] {
  const d: Vec3Tuple = [p[0] - anchor[0], p[1] - anchor[1], p[2] - anchor[2]]
  return [dot3(d, uAx), dot3(d, vAx)]
}

// ───────────────────────────────────────────────────────── pin / handle

/** The local position of the point currently mapping to UV `uv` — the
 *  general tile-lattice query the pins are placed with (red at `(m, n)`,
 *  green at `(m+1, n)`, blue at `(m, n+1)` — see the module doc comment).
 *  `null` if `local.m` is singular (degenerate frame). */
export function uvPointLocal(local: Local2, uv: readonly [number, number]): [number, number] | null {
  const inv = matInverse(local.m)
  if (inv === null) return null
  return matVec(inv, [uv[0] - local.off[0], uv[1] - local.off[1]])
}

/** The point currently mapping to UV `(0, 0)` — {@link uvPointLocal} at the
 *  zero lattice point. */
export function pinLocal(local: Local2): [number, number] | null {
  return uvPointLocal(local, [0, 0])
}

/** The point currently mapping to UV `(1, 0)` — one tile-width along U. */
export function handleLocal(local: Local2): [number, number] | null {
  return uvPointLocal(local, [1, 0])
}

/** The point currently mapping to UV `(0, 1)` — one tile-height along V. */
export function vHandleLocal(local: Local2): [number, number] | null {
  return uvPointLocal(local, [0, 1])
}

// ─────────────────────────────────────────────────── absolute decomposition

/**
 * The ABSOLUTE, user-facing decomposition of a frame (see the module doc
 * comment): everything is measured against fixed references the user can
 * see — the local basis axes (callers pass {@link tessellatePlaneBasis}, the
 * axes a default/untextured projection aligns to) and the material's natural
 * tile size — never against a drag's own starting state.
 */
export interface FrameAbsolute {
  /** Angle (radians, CCW in the local basis) of the tile's U edge measured
   *  from the local `+a` axis. `0` = the texture sits exactly as the
   *  planar default would. */
  angle: number
  /** Tile width over the material's natural world width — `1` means the
   *  texture renders at natural size along U. */
  scaleU: number
  /** Tile height over the material's natural world height. */
  scaleV: number
  /** Signed angle (radians) of the V edge past the handedness-consistent
   *  perpendicular of the U edge — `0` = a rectangular (unsheared) tile. */
  skew: number
  /** `1` for a right-handed `(E_u, E_v)` pair in the local basis, `-1` for
   *  a mirrored frame. */
  handed: 1 | -1
}

/**
 * Decomposes `local` into {@link FrameAbsolute}. The tile edge vectors
 * `E_u`/`E_v` are the columns of `M⁻¹` (`M⁻¹·(1,0)` is the local
 * displacement per unit U — exactly {@link handleLocal} minus
 * {@link pinLocal}). `null` when the frame is degenerate (singular `M`, a
 * collapsed edge) or the material sizes are non-positive.
 */
export function frameAbsolute(local: Local2, worldW: number, worldH: number): FrameAbsolute | null {
  if (!(worldW > 0) || !(worldH > 0)) return null
  const inv = matInverse(local.m)
  if (inv === null) return null
  const eu: [number, number] = [inv.a, inv.c]
  const ev: [number, number] = [inv.b, inv.d]
  const euLen = len2(eu)
  const evLen = len2(ev)
  if (euLen < 1e-12 || evLen < 1e-12) return null
  const handed: 1 | -1 = cross2(eu, ev) >= 0 ? 1 : -1
  // The perpendicular of E_u on E_v's own side — so skew is 0 for any
  // rectangular tile, mirrored or not.
  const perp: [number, number] = handed === 1 ? [-eu[1], eu[0]] : [eu[1], -eu[0]]
  return {
    angle: Math.atan2(eu[1], eu[0]),
    scaleU: euLen / worldW,
    scaleV: evLen / worldH,
    skew: signedAngle2(perp, ev),
    handed,
  }
}

// ───────────────────────────────────────────────────────────── gestures

/** Drag-anywhere / red-handle translate: rigidly shifts the decal by the
 *  local delta `(da, db)` — `M` (scale/rotate/shear) is untouched, only the
 *  offset changes (`off' = off - M · delta`). Grabbing the decal and moving
 *  the cursor by `delta` moves the pattern by exactly `delta` — a natural
 *  1:1 drag. */
export function translateLocal(local: Local2, delta: readonly [number, number]): Local2 {
  const shift = matVec(local.m, delta)
  return { m: local.m, off: [local.off[0] - shift[0], local.off[1] - shift[1]] }
}

/**
 * Green-handle scale+rotate about the FIXED pin: the decal is scaled by
 * `k = |newHandle - pivot| / |oldHandle - pivot|` and rotated by the signed
 * angle from `(oldHandle - pivot)` to `(newHandle - pivot)`, rigidly, about
 * `pivot` — preserving whatever shear/aspect `M` already had (composed as a
 * RIGHT-multiplication, `M' = (1/k) · M · R(-theta)`, so it acts on the
 * decal's own axes, not the abstract UV space; see the module doc comment
 * for the full derivation). The offset is chosen so the pivot keeps the
 * EXACT UV value it had under the incoming frame — whatever lattice point
 * the red pin currently sits on, not a hardwired `(0, 0)`.
 *
 * Degenerate drags (old handle coincides with the pivot — `|oldHandle -
 * pivot| < 1e-9`) are refused (`null`): there is no reference direction/
 * length to scale or rotate from.
 */
export function scaleRotateLocal(
  local: Local2,
  pivot: readonly [number, number],
  oldHandle: readonly [number, number],
  newHandle: readonly [number, number],
): Local2 | null {
  const fromVec = sub2(oldHandle, pivot)
  const toVec = sub2(newHandle, pivot)
  const fromLen = len2(fromVec)
  if (fromLen < 1e-9) return null
  const k = len2(toVec) / fromLen
  if (!(k > 1e-9)) return null
  const theta = signedAngle2(fromVec, toVec)
  const mPrime = scaleMat2(matMul(local.m, rotation2(-theta)), 1 / k)
  return { m: mPrime, off: pivotPreservingOff(local, pivot, mPrime) }
}

/** The offset that keeps `pivot` mapped to the same UV value it has under
 *  the incoming `local`, once the linear part becomes `mPrime` — shared by
 *  both pivot gestures ({@link scaleRotateLocal}, {@link shearScaleLocal}). */
function pivotPreservingOff(local: Local2, pivot: readonly [number, number], mPrime: Mat2): [number, number] {
  const uvPivot = matVec(local.m, pivot)
  return [
    local.off[0] + uvPivot[0] - (mPrime.a * pivot[0] + mPrime.b * pivot[1]),
    local.off[1] + uvPivot[1] - (mPrime.c * pivot[0] + mPrime.d * pivot[1]),
  ]
}

/**
 * Blue-handle shear+scale about the FIXED pin (paint-playtest2 §1): moves
 * the V edge vector DIRECTLY to `newHandle - pivot` — unlike
 * {@link scaleRotateLocal}'s similarity (which scales/rotates BOTH edge
 * vectors together), this touches ONLY the V edge; the U edge (and hence the
 * red/green pins) is left exactly as it was. This is the affine-expressible
 * half of SketchUp's distort pin: shear and non-uniform scale, with no
 * rotational degree of freedom of its own (the U edge's own direction never
 * moves).
 *
 * Derivation: writing `M⁻¹`'s two columns as the local (a,b)-space vectors
 * `E_u`/`E_v` from the pin to the green/blue handles (`M⁻¹·(1,0) = E_u`,
 * `M⁻¹·(0,1) = E_v` — exactly {@link handleLocal}/{@link vHandleLocal} minus
 * the pin), the new frame's inverse has columns `[E_u, newHandle - pivot]`;
 * `M'` is that matrix's own inverse, and the offset keeps the pivot mapped
 * to the same UV value it had before — whatever lattice point the red pin
 * sits on, exactly as `scaleRotateLocal` treats it.
 *
 * Degenerate guards (`null`, frame left unchanged by the caller): `local.m`
 * itself singular (no `E_u` to preserve), the new V edge collapsed to
 * near-zero length, or the new V edge landing near-PARALLEL to the
 * (unchanged) U edge — either case makes the new `M⁻¹` singular, an
 * unrecoverable basis no further drag could fix.
 */
export function shearScaleLocal(
  local: Local2,
  pivot: readonly [number, number],
  newHandle: readonly [number, number],
): Local2 | null {
  const inv = matInverse(local.m)
  if (inv === null) return null
  const eu = matVec(inv, [1, 0])
  const evNew = sub2(newHandle, pivot)
  const euLen = len2(eu)
  const evLen = len2(evNew)
  if (euLen < 1e-9 || evLen < 1e-9) return null
  const sinAngle = Math.abs(cross2(eu, evNew)) / (euLen * evLen)
  if (sinAngle < 1e-6) return null // near-parallel: an unrecoverable basis

  const invNew: Mat2 = { a: eu[0], b: evNew[0], c: eu[1], d: evNew[1] }
  const mPrime = matInverse(invNew)
  if (mPrime === null) return null // belt-and-braces; the checks above already rule this out
  return { m: mPrime, off: pivotPreservingOff(local, pivot, mPrime) }
}

// ────────────────────────────────────────────────────── planar default

/**
 * Builds the orthonormal in-plane basis `tessellate::plane_basis` uses for
 * an UNTEXTURED-frame face's planar UV fallback — the EXACT Rust algorithm
 * (helper axis = X unless `|nx| >= 0.9`, then Y), so
 * {@link planarDefaultFrame} reproduces byte-identical UVs to what's already
 * on screen when entering positioning mode on a face with no explicit frame
 * (no visual jump at gesture start). This is DELIBERATELY not the same
 * "least-parallel-axis" convention as `planeBasis` in transformMath.ts —
 * that one is fine for gesture-internal math (basis-invariant, see the
 * module doc comment) but would NOT match tessellate's actual rendered UVs.
 */
export function tessellatePlaneBasis(normal: Vec3Tuple): { u: [number, number, number]; v: [number, number, number] } {
  const [nx, ny, nz] = normal
  const helper: Vec3Tuple = Math.abs(nx) < 0.9 ? [1, 0, 0] : [0, 1, 0]
  // u = normalize(helper × n)
  const cx = helper[1] * nz - helper[2] * ny
  const cy = helper[2] * nx - helper[0] * nz
  const cz = helper[0] * ny - helper[1] * nx
  const len = Math.sqrt(cx * cx + cy * cy + cz * cz)
  const u: [number, number, number] = len > 1e-12 ? [cx / len, cy / len, cz / len] : [1, 0, 0]
  // v = n × u
  const v: [number, number, number] = [
    ny * u[2] - nz * u[1],
    nz * u[0] - nx * u[2],
    nx * u[1] - ny * u[0],
  ]
  return { u, v }
}

/**
 * The explicit `UvFrameComponents` equivalent to a face with NO explicit
 * `UvFrame` (the planar-projection default `tessellate` applies): `s =
 * uAx/worldW`, `t = vAx/worldH`, `u0 = v0 = 0`, where `uAx`/`vAx` come from
 * {@link tessellatePlaneBasis}. Seeding a positioning session with this
 * (rather than starting from `M = I`) means the live preview starts
 * byte-identical to what's already rendered.
 */
export function planarDefaultFrame(normal: Vec3Tuple, worldW: number, worldH: number): UvFrameComponents {
  const { u, v } = tessellatePlaneBasis(normal)
  return {
    sx: u[0] / worldW,
    sy: u[1] / worldW,
    sz: u[2] / worldW,
    tx: v[0] / worldH,
    ty: v[1] / worldH,
    tz: v[2] / worldH,
    u0: 0,
    v0: 0,
  }
}

// ─────────────────────────────────────────────────── wasm array <-> frame

/** The 8-float layout `scene.set_face_uv_frame`/`face_uv_frame` use:
 *  `[sx, sy, sz, tx, ty, tz, u0, v0]`. */
export function frameToArray(f: UvFrameComponents): number[] {
  return [f.sx, f.sy, f.sz, f.tx, f.ty, f.tz, f.u0, f.v0]
}

export function arrayToFrame(a: ArrayLike<number>): UvFrameComponents {
  return {
    sx: a[0], sy: a[1], sz: a[2],
    tx: a[3], ty: a[4], tz: a[5],
    u0: a[6], v0: a[7],
  }
}

// ────────────────────────────────────────────────────────── preview patch

/**
 * Recomputes UVs for one face's vertex range under `frame` and writes them
 * into `dest` at `[base*2, (base+count)*2)` — the live-preview patch
 * `SceneRenderer.previewFaceUv` applies to a `uv` `BufferAttribute` in place
 * (no re-tessellation; paint-tool design §3). `positions` is the WHOLE
 * object's position buffer (xyz per vertex, `MeshJs.positions()`/
 * `RenderMesh::positions` layout); only `[base*3, (base+count)*3)` is read.
 *
 * Precision note (bounded, LIVE-PREVIEW-ONLY error): `positions` is the
 * already-uploaded, already-f32-truncated three.js `BufferAttribute` array
 * — the only position source available at this renderer-local call site,
 * with no re-tessellation. This differs from the COMMITTED render path
 * (`tessellate::plane_basis`'s caller, in `crates/tessellate/src/lib.rs`),
 * which evaluates `frame.apply(Point3)` against the kernel's full f64
 * vertex positions and only casts the resulting small UV value to f32 —
 * so the committed UVs stay precise (~1e-7 relative) at any distance from
 * the origin. Here, truncating the (potentially large) POSITION first
 * means the UV error scales with distance from the origin: for a gradient
 * component `s` (dimensionless UV/meter) and position `p`, the per-axis
 * error is bounded by `|s| · |p| · 2⁻²³` (f32's relative precision,
 * `2⁻²³ ≈ 1.19e-7`) — e.g. a 1 UV/m gradient at 1,000 m from the origin:
 * ~1.2e-4 UV units, invisible on any real texture; at 100,000 m: ~1.2e-2,
 * large enough to show as a fractional-tile jitter on a repeating pattern.
 * This is TRANSIENT (only visible while a drag is actually live) and
 * SELF-CORRECTING (the commit that ends the gesture re-tessellates through
 * the precise f64 path, so the final result is unaffected) — feeding this
 * function f64 positions instead would need a new wasm call to fetch them
 * per face on every `pointermove` during a drag, a real architecture/perf
 * change for a cosmetic, self-healing edge case well outside this app's
 * human/architectural-scale working distances. Documented here rather than
 * fixed, per that tradeoff.
 */
export function writeFrameUvs(
  positions: Float32Array,
  base: number,
  count: number,
  frame: UvFrameComponents,
  dest: Float32Array,
): void {
  for (let i = 0; i < count; i++) {
    const pi = (base + i) * 3
    const x = positions[pi]
    const y = positions[pi + 1]
    const z = positions[pi + 2]
    const ui = (base + i) * 2
    dest[ui] = frame.sx * x + frame.sy * y + frame.sz * z + frame.u0
    dest[ui + 1] = frame.tx * x + frame.ty * y + frame.tz * z + frame.v0
  }
}

// Ray/plane intersection and an in-plane orthonormal basis for GESTURE math
// (screen-drag -> local delta) are NOT duplicated here — the tool imports
// `rayPlaneIntersect`/`facePlaneBasis` from `../viewport/geoHelpers` (already
// used by RectangleTool/PushPullTool for the same purpose). Only
// `tessellatePlaneBasis` above is a deliberate near-duplicate: it must match
// `tessellate::plane_basis`'s EXACT helper-axis threshold byte-for-byte (see
// its doc comment), which is a different job than `facePlaneBasis`'s
// "any valid orthonormal basis" contract.

/** Re-exported for callers that want the neutral starting matrix (a fresh
 *  `Local2` with `M = I`, `off = [0, 0]`) — not currently needed by the tool
 *  (every session seeds from a real frame, explicit or planar-default), but
 *  useful for tests exercising the gesture math in isolation. */
export function identityLocal2(): Local2 {
  return { m: IDENTITY_MAT2, off: [0, 0] }
}
