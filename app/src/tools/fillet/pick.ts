/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/** Pick stage: hover, collecting, and arming a target. */

import { type Ray } from '../../viewport/math'
import { kernelErrorMessage } from '../../kernelErrors'
import { typedReadout } from '../../settings/units'
import { makeFatSegments } from '../../viewport/fatLine'
import { applyAffine3x4, applyAffine3x4Linear, normalizeV3, type Affine3x4 } from '../../viewport/geoHelpers'
import { editCurveBuffer, isCurveInputKey } from '../curveInput'
import { getFilletSettings, setFilletCornerOnly, setFilletProfile, subscribeFilletSettings } from '../filletSettings'
import { MIN_FILLET_SIZE, dihedralFromNormals, type PlaneSide, type V3 } from '../filletMath'
import { cornerAngle, cornerDirections, cornerMaxSetback, roundedCornerPolyline } from '../filletCorner2d'
import { chainPolyline, emptyGripState, polylineLength } from '../filletGrips'
import { frameOf, resolveHover, toWorldPoint, type ChainInfo, type CornerTarget, type Frame, type Hover } from '../filletPick'
import type { HandleFrame, Anchor, SolidTarget, ArmTarget, Pick, ArmedStage } from './types'
import { HIGHLIGHT_STYLE } from './constants'
import { seamHint, SMOOTH_HINT } from './hints'
import { cornerGeomOf, nearestEdge } from './helpers'
import { cornerSizeLimit } from './spec'
import type { FilletContext } from './context'

/** The radius handle's frame for the first target, world: off the anchor
 *  edge at the arming click (its midpoint without one), along the
 *  bisector of the two faces; off a drawn corner along its in-plane
 *  bisector, away from the arc. */
function handleOf(t: ArmTarget): HandleFrame | null {
  if (t.kind === 'corner') {
    const d = cornerDirections(t.geom)
    if (d === null) return null
    const away: V3 = [-(d.dp[0] + d.dq[0]), -(d.dp[1] + d.dq[1]), -(d.dp[2] + d.dq[2])]
    const axis = normalizeV3(away)
    return axis === null ? null : { base: t.geom.c, axis }
  }
  const { p0, dir, a, b } = t.anchor
  const l2 = dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]
  let u = 0.5
  if (t.pickPoint !== null && l2 > 0) {
    const local = t.frame.inverse === null ? t.pickPoint : applyAffine3x4(t.frame.inverse as Affine3x4, t.pickPoint)
    u = Math.max(0, Math.min(1, ((local[0] - p0[0]) * dir[0] + (local[1] - p0[1]) * dir[1] + (local[2] - p0[2]) * dir[2]) / l2))
  }
  const baseLocal: V3 = [p0[0] + dir[0] * u, p0[1] + dir[1] * u, p0[2] + dir[2] * u]
  const bisLocal: V3 = [a.normal[0] + b.normal[0], a.normal[1] + b.normal[1], a.normal[2] + b.normal[2]]
  const axis = normalizeV3(t.frame.pose === null ? bisLocal : applyAffine3x4Linear(t.frame.pose, bisLocal))
  if (axis === null) return null
  return { base: toWorldPoint(t.frame, baseLocal), axis }
}

function gripChainOf(t: SolidTarget): ArmedStage['gripChain'] {
  const chain = t.chains[0]
  if (chain.closed || chain.edges.length === 0) return null
  const pts = chainPolyline(chain.segments)
  if (pts.length < 2) return null
  const length = polylineLength(pts)
  const bis = (face: PlaneSide, other: PlaneSide): V3 => {
    void face
    const v: V3 = [face.normal[0] + other.normal[0], face.normal[1] + other.normal[1], face.normal[2] + other.normal[2]]
    const l = Math.hypot(v[0], v[1], v[2])
    return l > 0 ? [v[0] / l, v[1] / l, v[2] / l] : [0, 0, 1]
  }
  const b = bis(t.anchor.a, t.anchor.b)
  return { pts, length, bisectors: [b, b] }
}

export class PickStage {
  constructor(private readonly c: FilletContext) {}

  updateHover(ray: Ray): void {
    this.c.hover = resolveHover(this.c.wasmScene, ray, this.c.pickOptions())
    this.c.statusLine = null
    const h = this.c.hover
    // A blend a later edit has changed cannot be resized or removed: say
    // so on hover, not after the click. Read once per blend hovered.
    if (h.kind === 'blend') {
      const key = `${h.object}:${h.blendId}`
      if (key !== this.c.hoverBlendKey) {
        this.c.hoverBlendKey = key
        this.c.hoverBlendIntact = this.blendIsIntact(h.object, h.blendId)
      }
    } else {
      this.c.hoverBlendKey = null
    }
    this.redrawHover()
  }

  blendIsIntact(object: bigint, blendId: number): boolean {
    try {
      const rows = JSON.parse(this.c.wasmScene.blend_features_json(object)) as Array<{ id: number; intact: boolean }>
      return rows.find((r) => r.id === blendId)?.intact ?? true
    } catch {
      return true
    }
  }

  /** A pick on an object with gaps is refused at the pick, not after a size
   *  was typed: the kernel would refuse the same at the commit. */
  refusesNonSolid(object: bigint, frame: Frame): boolean {
    if (frame.instance !== null || this.c.wasmScene.object_solid(object)) return false
    this.c.hooks.onToast(kernelErrorMessage('BlendNeedsSolid', ''), 'BlendNeedsSolid')
    return true
  }

  redrawHover(): void {
    this.c.preview.clearHighlight()
    const lines: number[] = []
    const push = (frame: Frame, local: ArrayLike<number>): void => {
      for (let i = 0; i + 2 < local.length; i += 3) {
        const p = toWorldPoint(frame, [local[i], local[i + 1], local[i + 2]])
        lines.push(p[0], p[1], p[2])
      }
    }
    for (const c of this.c.collected) push(c.frame, c.chain.segments)
    const h = this.c.hover
    if (h.kind === 'edge' || h.kind === 'face') push(h.frame, h.chain.segments)
    if (h.kind === 'blend') {
      for (const f of h.faces) {
        let ring: Float32Array
        try {
          ring = this.c.wasmScene.face_boundary(h.object, f)
        } catch {
          continue
        }
        const n = Math.floor(ring.length / 3)
        for (let i = 0; i < n; i++) {
          const a = i * 3
          const b = ((i + 1) % n) * 3
          push(h.frame, [ring[a], ring[a + 1], ring[a + 2], ring[b], ring[b + 1], ring[b + 2]])
        }
      }
    }
    if (h.kind === 'corner') {
      const g = cornerGeomOf(h.corner)
      const theta = cornerAngle(g)
      if (theta !== null) {
        const r = 0.15 * Math.min(Math.hypot(g.p[0] - g.c[0], g.p[1] - g.c[1], g.p[2] - g.c[2]), Math.hypot(g.q[0] - g.c[0], g.q[1] - g.c[1], g.q[2] - g.c[2])) * Math.tan(theta / 2)
        const poly = roundedCornerPolyline(g, r, 6)
        if (poly !== null) {
          for (let i = 0; i + 1 < poly.length; i++) lines.push(...poly[i], ...poly[i + 1])
        }
      }
    }
    if (lines.length >= 6) {
      const line = makeFatSegments(lines, HIGHLIGHT_STYLE)
      line.name = 'FilletHighlight'
      this.c.root.add(line)
    }
    // The status line for hover refusals.
    this.c.statusLine = h.kind === 'seam' ? seamHint(h.names, h.group) : h.kind === 'smooth' ? SMOOTH_HINT : null
    this.syncCornerProfile()
  }

  /** A drawn corner takes only Round and Chamfer, so while the pointer is on
   *  one (or a corner is armed or clicked) the chip disables the rest and any
   *  other profile gives way to Round. A corner merely hovered while solid
   *  picks are collected leaves the profile alone: a typed size goes to
   *  those picks. */
  syncCornerProfile(clickedCorner = false): void {
    const s = this.c.stage
    const onCorner =
      clickedCorner ||
      (this.c.hover.kind === 'corner' && this.c.collected.length === 0) ||
      (s.kind === 'armed' && s.targets[0]?.kind === 'corner')
    setFilletCornerOnly(onCorner)
    const profile = getFilletSettings().profile
    if (onCorner) {
      if (profile.kind !== 'round' && profile.kind !== 'chamfer') {
        this.c.displacedProfile = profile
        this.c.forcedRound = { kind: 'round' }
        setFilletProfile(this.c.forcedRound)
      }
    } else if (this.c.displacedProfile !== null) {
      if (profile === this.c.forcedRound) setFilletProfile(this.c.displacedProfile)
      this.c.displacedProfile = null
      this.c.forcedRound = null
    }
  }

  toggleCollected(h: Hover): void {
    if (h.kind !== 'edge' && h.kind !== 'face') return
    if (this.refusesNonSolid(h.object, h.frame)) return
    const mine = new Set(h.chain.edges.map(String))
    const at = this.c.collected.findIndex(
      (c) => c.object === h.object && c.frame.instance === h.frame.instance && c.chain.edges.some((e) => mine.has(String(e))),
    )
    if (at >= 0) {
      this.c.collected.splice(at, 1)
    } else {
      this.c.collected.push({
        object: h.object,
        frame: h.frame,
        chain: h.chain,
        hitFace: h.kind === 'edge' ? h.hitFace : h.face,
        point: h.point,
      })
    }
    this.redrawHover()
  }

  typeIntoCollected(key: string): void {
    if (key === 'Enter') {
      const token = this.c.size.parseTyped(this.c.retypeTyped)
      this.c.retypeTyped = ''
      this.c.measure('')
      if (token === null) return
      const targets = this.buildGroupTargets(this.c.collected)
      if (targets === null) return
      const profile = getFilletSettings().profile
      const picks = this.c.collected
      this.c.collected = []
      this.c.preview.clearHighlight()
      const before = this.c.wasmScene.undo_depth()
      this.c.commit.commitTargets(targets, profile, { src: 'typed', token })
      // A refused commit changed nothing: keep the picks so the user can
      // retype a size instead of picking everything again.
      if (this.c.wasmScene.undo_depth() === before && this.c.collected.length === 0) {
        this.c.collected = picks
        this.redrawHover()
      }
      return
    }
    if (!isCurveInputKey(key)) return
    this.c.retypeTyped = editCurveBuffer(this.c.retypeTyped, key)
    this.c.measure(this.c.retypeTyped === '' ? '' : typedReadout(this.c.retypeTyped))
  }

  armFromHover(h: Extract<Hover, { kind: 'edge' | 'face' }>, held = false): void {
    if (this.refusesNonSolid(h.object, h.frame)) return
    const picks: Pick[] = [
      ...this.c.collected,
      {
        object: h.object,
        frame: h.frame,
        chain: h.chain,
        hitFace: h.kind === 'edge' ? h.hitFace : h.face,
        point: h.point,
      },
    ]
    const anchorEdge = h.kind === 'edge' ? h.edge : undefined
    const targets = this.buildGroupTargets(picks, anchorEdge)
    if (targets === null) return
    this.c.collected = []
    this.beginArmed(targets, held)
  }

  armObjects(objects: bigint[]): void {
    const targets: SolidTarget[] = []
    for (const object of objects) {
      let edges: BigUint64Array
      try {
        edges = this.c.wasmScene.object_sharp_edges(object)
      } catch {
        continue
      }
      if (edges.length === 0) continue
      const frame = frameOf(this.c.wasmScene, null)
      if (frame === null) continue
      const chains: ChainInfo[] = []
      for (const e of edges) {
        const c = this.c.wasmScene.logical_edge(object, e, false)
        try {
          chains.push({
            edges: Array.from(c.edges()),
            segments: c.segments(),
            dihedral: c.dihedral(),
            convexity: c.convexity(),
            closed: c.closed(),
          })
        } finally {
          c.free()
        }
      }
      const t = this.buildSolidTarget(
        chains.map((chain) => ({ object, frame, chain, hitFace: null, point: [0, 0, 0] as V3 })),
      )
      if (t !== null) targets.push(t)
    }
    if (targets.length > 0) this.beginArmed(targets)
  }

  armCorner(corner: CornerTarget, held = false): void {
    this.syncCornerProfile(true)
    const geom = cornerGeomOf(corner)
    const theta = cornerAngle(geom)
    if (theta === null) return
    const profile = getFilletSettings().profile
    if (profile.kind !== 'round' && profile.kind !== 'chamfer') {
      this.c.hooks.onToast('Drawn corners can be rounded or chamfered.')
      return
    }
    this.c.collected = []
    this.beginArmed([{ kind: 'corner', corner, geom, theta }], held)
  }

  /** Arms `targets` with the preview at the initial size and the gizmo up.
   *  `held`: the arming click still holds the radius handle, so a drag
   *  before its release sizes in one gesture. */
  beginArmed(targets: ArmTarget[], held = false): void {
    this.c.hover = { kind: 'none' }
    this.c.preview.clearHighlight()
    const profile = getFilletSettings().profile
    let gripChain: ArmedStage['gripChain'] = null
    const first = targets[0]
    if (targets.length === 1 && first.kind === 'solid' && first.frame.instance === null && first.chains.length === 1) {
      gripChain = gripChainOf(first)
    }
    this.c.unsubSettings?.()
    this.c.unsubSettings = subscribeFilletSettings(() => {
      const st = this.c.stage
      if (st.kind !== 'armed') return
      st.profile = getFilletSettings().profile
      this.c.preview.drawArmedPreview(true)
      this.c.hooks.requestRender()
    })
    const handle = handleOf(targets[0])
    const stage: ArmedStage = {
      kind: 'armed',
      targets,
      profile,
      amount: handle === null ? null : { src: 'handle', size: this.initialSize(targets[0]) },
      typed: '',
      armedScreen: this.c.screen,
      grips: emptyGripState(),
      gripChain,
      grab: null,
      grabScreen: null,
      grabRay: null,
      gripTyped: '',
      gripHover: null,
      handle,
      active: { kind: 'radius' },
      pressArm: false,
      limits: new Map(),
      gripLimits: new Map(),
      held: null,
    }
    this.c.stage = stage
    this.c.statusLine = null
    this.syncCornerProfile()
    this.c.preview.drawArmedPreview()
    this.c.size.refreshGripMarkers()
    if (held && handle !== null) {
      stage.grab = { kind: 'radius' }
      stage.grabScreen = this.c.screen
      stage.grabRay = this.c.lastRay
      stage.pressArm = true
    }
  }

  /** The size a fresh arm previews at: the last committed one, else a
   *  tenth of the picked chain (a quarter of a corner's shorter leg). */
  initialSize(t: ArmTarget): number {
    if (this.c.lastToken !== null) {
      if (this.c.lastToken.kind === 'length') return Math.max(MIN_FILLET_SIZE, this.c.lastToken.meters)
      if (this.c.lastToken.kind === 'pair') return Math.max(MIN_FILLET_SIZE, this.c.lastToken.a)
    }
    if (t.kind === 'corner') {
      const leg = cornerMaxSetback(t.geom)
      const profile = getFilletSettings().profile
      const limit = cornerSizeLimit(t, profile.kind === 'chamfer' ? 'chamfer' : 'round')
      return Math.max(MIN_FILLET_SIZE, Math.min(leg / 4, 0.9 * limit))
    }
    const chain = t.chains[0]
    const length = chain === undefined ? 0 : polylineLength(chainPolyline(chain.segments)) * t.frame.scale
    return Math.max(MIN_FILLET_SIZE, length > 0 ? length / 10 : 0.01)
  }

  /** One solid target per object among `picks` (first-picked first; the
   *  last pick's object is the one `preferredEdge` anchors, and leads, so
   *  the handle sits where the pointer is). Null when a group has no anchor. */
  buildGroupTargets(picks: Pick[], preferredEdge?: bigint): SolidTarget[] | null {
    // Keyed on the definition object: two instances of one component share
    // its object and edge ids, so they merge into one target (the first
    // pick's instance frame).
    const keyOf = (p: Pick): string => String(p.object)
    const lead = keyOf(picks[picks.length - 1])
    const groups = new Map<string, Pick[]>()
    for (const p of picks) {
      const g = groups.get(keyOf(p))
      if (g === undefined) groups.set(keyOf(p), [p])
      else g.push(p)
    }
    const targets: SolidTarget[] = []
    for (const [key, group] of groups) {
      const t = this.buildSolidTarget(group, key === lead ? preferredEdge : undefined)
      if (t === null) return null
      if (key === lead) targets.unshift(t)
      else targets.push(t)
    }
    return targets
  }

  buildSolidTarget(picks: Pick[], preferredEdge?: bigint): SolidTarget | null {
    if (picks.length === 0) return null
    const { object, frame } = picks[0]
    const edges: bigint[] = []
    for (const p of picks) for (const e of p.chain.edges) if (!edges.includes(e)) edges.push(e)
    const last = picks[picks.length - 1]
    const anchorEdge = preferredEdge ?? nearestEdge(last)
    if (anchorEdge === null) return null
    const anchor = this.anchorOf(object, anchorEdge, last.hitFace)
    if (anchor === null) {
      this.c.hooks.onToast('That edge is not between two faces.')
      return null
    }
    return {
      kind: 'solid',
      object,
      frame,
      edges,
      chains: picks.map((p) => p.chain),
      anchorEdge,
      anchor,
      primaryFace: last.hitFace,
      pickPoint: preferredEdge === undefined && last.hitFace === null ? null : toWorldPoint(frame, last.point),
    }
  }

  anchorOf(object: bigint, edge: bigint, hitFace: bigint | null): Anchor | null {
    let faces: BigUint64Array
    try {
      faces = this.c.wasmScene.edge_faces(object, edge)
    } catch {
      return null
    }
    if (faces.length !== 2) return null
    let [fa, fb] = [faces[0], faces[1]]
    if (hitFace !== null && hitFace === fb) [fa, fb] = [fb, fa]
    const c = this.c.wasmScene.logical_edge(object, edge, false)
    let seg: Float32Array
    let at = 0
    try {
      seg = c.segments()
      // The logical edge is the whole run (a rim, a tangent chain): the
      // anchor is this kernel edge's own segment, where its two faces'
      // normals hold, not the run's first.
      at = Math.max(0, Array.from(c.edges()).indexOf(edge))
    } finally {
      c.free()
    }
    if (seg.length < at * 6 + 6) return null
    const p0: V3 = [seg[at * 6], seg[at * 6 + 1], seg[at * 6 + 2]]
    const dir: V3 = [seg[at * 6 + 3] - p0[0], seg[at * 6 + 4] - p0[1], seg[at * 6 + 5] - p0[2]]
    const side = (f: bigint): PlaneSide | null => {
      const n = this.c.wasmScene.face_normal(object, f)
      const ring = this.c.wasmScene.face_boundary(object, f)
      const count = Math.floor(ring.length / 3)
      if (count === 0) return null
      let cx = 0, cy = 0, cz = 0
      for (let i = 0; i < count; i++) {
        cx += ring[i * 3]
        cy += ring[i * 3 + 1]
        cz += ring[i * 3 + 2]
      }
      return { normal: [n[0], n[1], n[2]], inside: [cx / count, cy / count, cz / count] }
    }
    const a = side(fa)
    const b = side(fb)
    if (a === null || b === null) return null
    return { p0, dir, a, b, faceA: fa, faceB: fb, theta: dihedralFromNormals(a.normal, b.normal) }
  }

  /** Whether the click that just finished a pick is worth a second look:
   *  an edge or a drawn corner (it arms), or a band on an object the
   *  commit did not touch (it opens its edit). A band the commit itself
   *  made or rebuilt is not re-entered by the same click. */
  clickWantsNext(ray: Ray, changed: ReadonlySet<bigint>): boolean {
    const h = resolveHover(this.c.wasmScene, ray, this.c.pickOptions())
    if (h.kind === 'blend') return !changed.has(h.object)
    return h.kind === 'edge' || h.kind === 'corner'
  }
}
