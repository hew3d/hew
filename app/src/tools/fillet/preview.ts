/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/** Preview stage: asking the previewer and drawing its answer. */

import * as THREE from 'three'
import { parseKernelErrorCode, kernelErrorMessage } from '../../kernelErrors'
import { formatLength, formatLengthLimit, typedReadout } from '../../settings/units'
import { makeFatSegments, disposeFatSegments, PREVIEW_LINE_STYLE } from '../../viewport/fatLine'
import { getFilletSettings, setFilletThreeWayCorners } from '../filletSettings'
import { clampCornerSegments, limitedSentence, parseProfileWidth, parseRadiusLimit, profileTooWideSentence, type V3 } from '../filletMath'
import { chamferedCornerPolyline, cornerDefaultSegments, roundedCornerPolyline } from '../filletCorner2d'
import { gripStateIsActive, type GripId, type GripState } from '../filletGrips'
import { toWorldPoint, type Frame } from '../filletPick'
import type { SolidTarget, CornerArm, Sized, Resolved, ArmedStage, SizeLimit } from './types'
import { whenReady, type BlendPreviewResult, type Deferred } from './previewer'
import { FACE_MASK_MAX_TRIANGLES, PATCH_COLOR, RAIL_COLOR, CLAMPED_RAIL_COLOR, HIGHLIGHT_STYLE, SLOW_PREVIEW_MS, SEARCHING_REFUSALS, SLOW_PREVIEW_HINT } from './constants'
import { sizeOf, scaledSized } from './helpers'
import { resolve, limitKey, specJson, currentAmount, cornerClamp, cornerLimitText } from './spec'
import type { FilletContext } from './context'

/** How often a drag of one grip searches the limit again after its first
 *  search: each time it moves into a new `heldBucket`, up to this many. */
const HELD_RESEARCHES = 3

/** A coarse bucket of the held grip's value: an eighth of the chain for a
 *  stop or a radius point's position, a factor of 1.25 for a radius. */
function heldBucket(g: GripState, held: GripId, length: number): string {
  const step = length / 8
  const ratio = (r: number): string => String(Math.floor(Math.log(Math.max(r, 1e-12)) / Math.log(1.25)))
  switch (held.kind) {
    case 'stop':
      return String(Math.floor(g.stops[held.end] / step))
    case 'handlePos': {
      const h = g.handles[held.index]
      return h === undefined ? '' : String(Math.floor(h.at / step))
    }
    case 'handleRadius': {
      const h = g.handles[held.index]
      return h === undefined ? '' : ratio(h.radius)
    }
    case 'endRadius': {
      const at = held.end === 'start' ? 0 : length
      const h = g.handles.find((x) => Math.abs(x.at - at) < 1e-9)
      return h === undefined ? '' : ratio(h.radius)
    }
    case 'radius':
      return ''
  }
}

/** `g` as a cache key, the grip `held` (being dragged) left out; `length`
 *  is the chain's, where an end's radius handle sits. */
function gripSignature(g: GripState, held: GripId | null, length: number): string {
  const stop = (end: 'start' | 'end'): number | string =>
    held !== null && held.kind === 'stop' && held.end === end ? 'held' : g.stops[end]
  const handles = g.handles.map((h, i) => {
    const isHeld =
      held !== null &&
      (((held.kind === 'handlePos' || held.kind === 'handleRadius') && held.index === i) ||
        (held.kind === 'endRadius' && Math.abs(h.at - (held.end === 'start' ? 0 : length)) < 1e-9))
    return isHeld ? 'held' : `${h.at}:${h.radius}`
  })
  return JSON.stringify([stop('start'), stop('end'), handles.filter((h) => h !== 'held')])
}

export class PreviewStage {
  constructor(private readonly c: FilletContext) {}

  /** Redraws the armed preview. `typedRedraw`: the redraw answers typing
   *  (a key in the size field, Backspace) or a settings or profile change,
   *  not a pointer move; only then, with a typed size and no drag under
   *  way, does it take the typed preview (once per typed size, as the
   *  kernel asks). A pointer or grip-drag redraw takes the quick preview
   *  even with a typed size in the field. */
  drawArmedPreview(typedRedraw = false): void {
    const s = this.c.stage
    if (s.kind !== 'armed') return
    const round = this.startRound()
    const amount = currentAmount(s)
    this.c.clampedHeadline = null
    if (amount === null) {
      this.clearPreviewDrawing()
      this.c.measure(s.typed === '' ? '' : typedReadout(s.typed))
      this.drawHighlightOnly(s)
      return
    }
    const resolved = resolve(s.targets[0], amount, s.profile)
    if ('error' in resolved) {
      this.clearPreviewDrawing()
      this.c.statusLine = resolved.error
      return
    }
    this.clearPatch()
    this.c.statusLine = null
    this.c.lastLabel = resolved.label
    if (s.typed === '') {
      this.c.measure(resolved.headline === null ? '' : formatLength(resolved.headline))
    } else {
      this.c.measure(typedReadout(s.typed))
    }
    this.drawHighlightOnly(s)
    let corners = 0
    this.c.slowSkipped = false
    this.c.clampedHeadline = null
    const exact = typedRedraw && amount.src === 'typed' && s.grab === null
    // A synchronous previewer answers each target inside the loop; an
    // asynchronous one answers later, and the round finishes once every
    // target has.
    const pending: Array<Promise<number | 'cancelled'>> = []
    for (const t of s.targets) {
      if (t.kind === 'corner') {
        this.drawCornerPreview(t, resolved.sized)
        continue
      }
      const drawn = this.drawSolidPreview(t, resolved, s, exact, round)
      if (typeof drawn === 'number') corners = Math.max(corners, drawn)
      else pending.push(drawn)
    }
    if (pending.length === 0) {
      this.finishArmedRound(s, corners)
      return
    }
    void Promise.all(pending).then((counts) => {
      if (round !== this.c.epoch) return
      let max = corners
      for (const n of counts) {
        if (n === 'cancelled') return
        max = Math.max(max, n)
      }
      this.finishArmedRound(s, max)
      this.c.hooks.requestRender()
    })
  }

  /** What follows once every target's preview has been drawn. */
  private finishArmedRound(s: ArmedStage, corners: number): void {
    // The readout shows the size that will be made: at a limit, the limit
    // (spelled as the status line spells it), not how far the pointer went.
    if (s.typed === '' && (s.grab === null || s.grab.kind === 'radius') && this.c.clampedHeadline !== null) {
      this.c.measure(formatLengthLimit(this.c.clampedHeadline))
    }
    // A grip being typed for keeps its own value in the box, whatever the
    // preview did to the radius.
    if (s.active.kind !== 'radius') this.c.measure(s.gripTyped === '' ? this.c.size.gripReadout(s) : typedReadout(s.gripTyped))
    if (this.c.slowSkipped && this.c.statusLine === null) this.c.statusLine = SLOW_PREVIEW_HINT
    setFilletThreeWayCorners(corners)
  }

  /** Starts a preview round: whatever an earlier round still has in flight
   *  is dropped, and its answer will not be applied. Returns the round's id. */
  startRound(): number {
    this.c.previewer.cancel()
    return ++this.c.epoch
  }

  /** A target held at `max`: the tightest clamp among the targets is the
   *  size the handle shows and the commit makes. */
  noteClamp(max: number): void {
    this.c.clampedHeadline = this.c.clampedHeadline === null ? max : Math.min(this.c.clampedHeadline, max)
  }

  drawHighlightOnly(s: ArmedStage): void {
    this.clearHighlight()
    const lines: number[] = []
    for (const t of s.targets) {
      if (t.kind !== 'solid') continue
      for (const c of t.chains) {
        for (let i = 0; i + 2 < c.segments.length; i += 3) {
          const p = toWorldPoint(t.frame, [c.segments[i], c.segments[i + 1], c.segments[i + 2]])
          lines.push(p[0], p[1], p[2])
        }
      }
    }
    if (lines.length >= 6) {
      const line = makeFatSegments(lines, HIGHLIGHT_STYLE)
      line.name = 'FilletHighlight'
      this.c.root.add(line)
    }
  }

  /** Previews one solid target; returns its three-way corner count (or a
   *  promise of it, when the previewer answers later).
   *  `typed`: a typed redraw of a typed size (see `drawArmedPreview`),
   *  which previews as its commit makes it (a swept
   *  edge takes the commit's rounds); any other redraw takes the quick
   *  preview, which is asked again on every move. */
  drawSolidPreview(t: SolidTarget, resolved: Resolved, s: ArmedStage, typed: boolean, round: number): Deferred<number> {
    const k = 1 / t.frame.scale
    const asked = sizeOf(resolved.sized)
    // A limit found without grips still bounds the size with them (a stop or
    // a radius point never lets the base size grow), so the pointer's raw
    // size does not reach the kernel beside a stop and refuse it as out of
    // range. A preview with grips set records its limit apart, under them.
    // A typed preview's limit is the commit's (its search takes the
    // commit's rounds); a quick one's may be lower. Neither stands in for
    // the other.
    const key = `${limitKey(t, resolved.sized, s.profile)}|${typed ? 'typed' : 'quick'}`
    // With grips set, the limit found with these grips (the held one left
    // out: a drag holds the limit it found and the kernel clamps further
    // where it must, rather than searching on every move); else the
    // grip-free one, which bounds them too.
    const length = s.gripChain?.length ?? 0
    // The held grip by its bucket: a new bucket searches again, a few times
    // a drag at most (after that the drag holds the limit it last found).
    const heldId = s.grab === null || s.grab.kind === 'radius' ? null : JSON.stringify(s.grab)
    if (heldId === null) s.held = null
    else if (s.grab !== null) {
      const bucket = heldBucket(s.grips, s.grab, length)
      if (s.held === null || s.held.id !== heldId) s.held = { id: heldId, bucket, searches: 0 }
      else if (bucket !== s.held.bucket && s.held.searches < HELD_RESEARCHES) {
        s.held.bucket = bucket
        s.held.searches++
      }
    }
    const gripKey = gripStateIsActive(s.grips)
      ? `${key}|${gripSignature(s.grips, s.grab, length)}|${s.held?.bucket ?? ''}`
      : null
    const withGrips = gripKey === null ? undefined : s.gripLimits.get(t)
    const plain = s.limits.get(t)
    const limit =
      withGrips !== undefined && withGrips.key === gripKey ? withGrips : plain !== undefined && plain.key === key ? plain : undefined
    const heldLimit: SizeLimit | null = limit !== undefined && asked !== null && asked * k > limit.max ? limit : null
    const limitScale = heldLimit !== null && asked !== null ? heldLimit.max / (asked * k) : 1
    const sized = heldLimit !== null ? scaledSized(resolved.sized, limitScale) : resolved.sized
    if (this.c.slowObjects.has(t.object)) {
      this.releaseMask(t.object)
      this.c.slowSkipped = true
      return 0
    }
    const json = specJson(t, sized, s.profile, k, s.grips, resolved.sized, limitScale)
    return whenReady(
      this.c.previewer.request({ kind: 'create', object: t.object, spec: json, typed }),
      () => this.c.epoch === round,
      (r) => this.applySolidPreview(t, resolved, s, typed, round, r, { key, gripKey, heldLimit }),
    )
  }

  /** Applies a solid target's preview answer: caches the limit it found,
   *  words the clamp, and draws the patch and rails. Returns the target's
   *  three-way corner count. */
  private applySolidPreview(
    t: SolidTarget,
    resolved: Resolved,
    s: ArmedStage,
    typed: boolean,
    round: number,
    r: BlendPreviewResult,
    held: { key: string; gripKey: string | null; heldLimit: SizeLimit | null },
  ): Deferred<number> {
    const { key, gripKey, heldLimit } = held
    if (r.kind === 'refused') {
      // A refusal that searched for no limit takes as long as a preview
      // (a typed one's time says nothing about following the pointer).
      if (!typed && !SEARCHING_REFUSALS.has(r.code ?? '') && r.elapsedMs >= SLOW_PREVIEW_MS) {
        this.c.slowObjects.add(t.object)
      }
      if (heldLimit !== null) {
        // The size the limit was found at no longer previews (its facet
        // count differs): ask the kernel afresh.
        s.limits.delete(t)
        s.gripLimits.delete(t)
        return this.drawSolidPreview(t, resolved, s, typed, round)
      }
      // The quick preview cannot build every corner the commit can (a loop
      // that rolls a ball around a junction, a seam through a curve): its
      // "can't blend this corner" is not the commit's answer, so it is not
      // said. A typed preview or a commit that refuses still says it.
      const quickOnly = !typed && r.code === 'UnsupportedVertex'
      if (!quickOnly) this.c.statusLine = this.refusalText(r.message)
      this.releaseMask(t.object)
      return 0
    }
    const clamped = heldLimit !== null || r.clamped
    // Only a pointer preview that did not search tells how long following
    // the pointer takes (one held at a known limit did not search).
    if (!typed && !r.clamped && r.elapsedMs >= SLOW_PREVIEW_MS) this.c.slowObjects.add(t.object)
    if (heldLimit !== null) {
      // Held at a cached limit; with grips the kernel may clamp further
      // (a stop shortens the run), and then its own size is the truth.
      const max = r.clamped ? Math.min(heldLimit.max, r.maxSize) : heldLimit.max
      const by = r.clamped && r.maxSize < heldLimit.max ? r.limitedBy : heldLimit.limitedBy
      if (gripKey !== null && max < heldLimit.max) s.gripLimits.set(t, { key: gripKey, max, limitedBy: by })
      this.noteClamp(max * t.frame.scale)
      this.c.statusLine = limitedSentence(resolved.label, max * t.frame.scale, by, formatLengthLimit)
    } else if (clamped) {
      const found = { max: r.maxSize, limitedBy: r.limitedBy }
      if (gripKey === null) s.limits.set(t, { key, ...found })
      else s.gripLimits.set(t, { key: gripKey, ...found })
      this.noteClamp(r.maxSize * t.frame.scale)
      this.c.statusLine = limitedSentence(resolved.label, r.maxSize * t.frame.scale, r.limitedBy, formatLengthLimit)
    }
    this.drawResult(t.object, t.frame, r, clamped)
    return r.threeWayCorners
  }

  /** Draws an answer's patch and rails, hiding the faces the patch replaces
   *  when the object is small enough to mask. */
  drawResult(object: bigint, frame: Frame, r: Extract<BlendPreviewResult, { kind: 'ok' }>, clamped: boolean): void {
    const masked = frame.instance === null ? this.c.hooks.setFaceMask(object, r.replacedFaces, FACE_MASK_MAX_TRIANGLES) : false
    if (masked) this.c.maskedObjects.add(object)
    this.drawPatch(frame, r.positions, r.normals, r.indices, !masked)
    this.drawRails(frame, r.rails, clamped)
  }

  drawPatch(frame: Frame, positions: Float32Array, normals: Float32Array, indices: Uint32Array, offset: boolean): void {
    if (indices.length === 0) return
    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(positions, 3))
    geo.setAttribute('normal', new THREE.BufferAttribute(normals, 3))
    geo.setIndex(new THREE.BufferAttribute(indices, 1))
    const mat = new THREE.MeshPhongMaterial({
      color: PATCH_COLOR,
      side: THREE.DoubleSide,
      polygonOffset: offset,
      polygonOffsetFactor: offset ? -1 : 0,
      polygonOffsetUnits: offset ? -1 : 0,
    })
    const mesh = new THREE.Mesh(geo, mat)
    mesh.name = 'FilletPatch'
    this.placeInFrame(mesh, frame)
    this.c.root.add(mesh)
  }

  drawRails(frame: Frame, rails: Float32Array, clamped: boolean): void {
    if (rails.length < 6) return
    const line = makeFatSegments(rails, {
      ...PREVIEW_LINE_STYLE,
      color: clamped ? CLAMPED_RAIL_COLOR : RAIL_COLOR,
      widthPx: 2.5,
    })
    line.name = 'FilletRail'
    this.placeInFrame(line, frame)
    this.c.root.add(line)
  }

  placeInFrame(obj: THREE.Object3D, frame: Frame): void {
    if (frame.pose === null) return
    const m = frame.pose
    obj.matrixAutoUpdate = false
    obj.matrix.set(m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8], m[9], m[10], m[11], 0, 0, 0, 1)
  }

  drawCornerPreview(t: CornerArm, sized: Sized): void {
    let poly: V3[] | null = null
    const settings = getFilletSettings()
    const held = cornerClamp(t, sized)
    if (held !== null) {
      sized = held.sized
      this.noteClamp(held.limit)
      this.c.statusLine = `${held.label} limited to ${formatLengthLimit(held.limit)} by the shorter side.`
    }
    if (sized.kind === 'round') {
      const segs = clampCornerSegments(settings.segmentsOverride ?? cornerDefaultSegments(sized.radius, t.theta))
      poly = roundedCornerPolyline(t.geom, sized.radius, segs)
      if (poly === null) this.c.statusLine = cornerLimitText(t, sized.kind)
    } else if (sized.kind === 'chamfer') {
      poly = chamferedCornerPolyline(t.geom, sized.a, sized.b, t.corner.normal)
      if (poly === null) this.c.statusLine = cornerLimitText(t, sized.kind)
    }
    if (poly === null || poly.length < 2) return
    const flat: number[] = []
    for (let i = 0; i + 1 < poly.length; i++) flat.push(...poly[i], ...poly[i + 1])
    const line = makeFatSegments(flat, { ...PREVIEW_LINE_STYLE, color: RAIL_COLOR, widthPx: 3 })
    line.name = 'FilletRail'
    this.c.root.add(line)
  }

  /** Removes the preview patch and rails but keeps any face mask: the mask
   *  is rebuilt only when the replaced set changes, not on every move. */
  clearPatch(): void {
    for (const child of [...this.c.root.children]) {
      if (child.name === 'FilletPatch') {
        const m = child as THREE.Mesh
        m.geometry.dispose()
        ;(m.material as THREE.Material).dispose()
        this.c.root.remove(child)
      } else if (child.name === 'FilletRail') {
        disposeFatSegments(child)
        this.c.root.remove(child)
      }
    }
  }

  /** Removes the whole preview, masks included. */
  clearPreviewDrawing(): void {
    this.startRound()
    this.clearPatch()
    for (const o of [...this.c.maskedObjects]) this.releaseMask(o)
  }

  releaseMask(object: bigint): void {
    if (this.c.maskedObjects.delete(object)) this.c.hooks.clearFaceMask(object)
  }

  /** The status text for a refused preview or commit. */
  refusalText(err: unknown): string {
    const raw = err instanceof Error ? err.message : String(err)
    const code = parseKernelErrorCode(err)
    const radius = parseRadiusLimit(raw)
    if (radius !== null) return limitedSentence(this.c.lastLabel, radius.max, radius.limitedBy, formatLengthLimit)
    const width = parseProfileWidth(raw)
    if (width !== null) return profileTooWideSentence(width.needed, width.available, formatLength)
    return kernelErrorMessage(code ?? 'Unknown', raw)
  }

  clearHighlight(): void {
    for (const child of [...this.c.root.children]) {
      if (child.name === 'FilletHighlight') {
        disposeFatSegments(child)
        this.c.root.remove(child)
      }
    }
  }
}
