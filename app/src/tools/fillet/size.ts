/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/** Size stage: handle and grip drags, typed entry, readouts. */

import type { Snap } from '../types'
import { sameRay, type Ray } from '../../viewport/math'
import { kernelErrorMessage } from '../../kernelErrors'
import { formatLength, getLengthUnit, typedReadout } from '../../settings/units'
import { parseCurveInput, type CurveToken } from '../curveInput'
import { setFilletSegmentsOverride } from '../filletSettings'
import { MIN_FILLET_SIZE, clampBlendSegments, clampCornerSegments, type V3 } from '../filletMath'
import { closestOnPolyline, lineParamNearRay, movedHandle, pointAtLength, sameGrip, withHandle, withStop, type GripId, type GripMarker } from '../filletGrips'
import { type FilletLabel } from '../filletLabels'
import { rayToLocal, toWorldPoint } from '../filletPick'
import type { Amount, ArmedStage } from './types'
import { HANDLE_PICK_PX, HANDLE_PICK_FALLBACK_M } from './constants'
import { offsetBy } from './helpers'
import { resolve, currentAmount } from './spec'
import type { FilletContext } from './context'

export class SizeStage {
  constructor(private readonly c: FilletContext) {}

  /** The radius handle follows the pointer along its axis: the size is how
   *  far along the axis the pointer ray passes nearest. */
  sizeFromHandle(s: ArmedStage, ray: Ray): void {
    if (s.handle === null) return
    const along = lineParamNearRay(s.handle.base, s.handle.axis, ray.origin, ray.direction)
    if (along === null) return
    s.amount = { src: 'handle', size: Math.max(MIN_FILLET_SIZE, along) }
    this.c.preview.drawArmedPreview()
    this.refreshGripMarkers()
  }

  /** Reads the typed buffer's token or toasts why not. */
  parseTyped(text: string): CurveToken | null {
    const parsed = parseCurveInput(text, getLengthUnit())
    if ('error' in parsed) {
      this.c.hooks.onToast(parsed.error)
      return null
    }
    return parsed
  }

  applyTypedToPreview(s: ArmedStage): void {
    void s
    this.c.preview.drawArmedPreview(true)
    // The radius grips sit at the current radius: a typed one moves them.
    this.refreshGripMarkers()
    this.c.hooks.requestRender()
  }

  pointerDownArmed(s: ArmedStage, snap: Snap | null, ray: Ray): void {
    const t = s.targets[0]
    if (s.grab !== null) {
      if (s.grab.kind === 'radius') {
        // A press while the handle is held (no release was seen): a move
        // since the pick-up sized it; in place it is the arming click's
        // own release, or a click on the handle, which finishes.
        this.releaseHandle(s, ray)
        return
      }
      // Second click of a grip move: place it.
      this.putGripDown(s)
      return
    }
    const grip = this.gripUnder(s, ray)
    if (grip !== null) {
      this.pickUpGrip(s, grip, ray)
      return
    }
    if (s.handle !== null && this.withinPickAperture(ray, s.handle.base)) {
      // On the edge where the handle leaves it: as good as the handle.
      this.finishArmed(s)
      return
    }
    if (t.kind === 'solid' && s.gripChain !== null) {
      const local = rayToLocal(t.frame, ray)
      const hit = closestOnPolyline(s.gripChain.pts, local.origin, local.direction)
      if (hit !== null && this.withinPickAperture(ray, toWorldPoint(t.frame, hit.point))) {
        this.addHandleAt(s, local)
        return
      }
    }
    // Anywhere else finishes the pick. Another edge, a drawn corner or a
    // band then gets the click too (it arms at the size just made, or opens
    // its edit); a face or empty space only finishes.
    const changed = new Set(s.targets.flatMap((t) => (t.kind === 'solid' ? [t.object] : [])))
    if (!this.finishArmed(s)) return
    if (this.c.stage.kind === 'idle' && this.c.pick.clickWantsNext(ray, changed)) this.c.dispatchPointerDown(snap, ray)
  }

  /** Commits the armed pick as it stands (typed or sized). False when
   *  nothing could be committed (no size yet, a bad typed value). */
  finishArmed(s: ArmedStage): boolean {
    if (s.active.kind !== 'radius' && s.gripTyped !== '') {
      this.enterOnArmed(s)
      return false
    }
    if (s.typed !== '') {
      this.enterOnArmed(s)
      return this.c.stage !== s
    }
    if (s.amount === null) return false
    this.c.commit.commitArmed(this.armedCommitAmount(s))
    return this.c.stage !== s
  }

  /** What a finish commits: a typed size as typed (past the limit it is
   *  refused with the limit named); a handle or pointer size held at a limit
   *  commits the limit the preview showed, never the raw ask. */
  armedCommitAmount(s: ArmedStage): Amount {
    const amount = currentAmount(s) ?? s.amount
    if (amount === null) return { src: 'handle', size: MIN_FILLET_SIZE }
    if (amount.src === 'typed' || this.c.clampedHeadline === null) return amount
    // The radius handles were asked at the raw size and clamp with it: they
    // commit scaled by clamped/asked, as the kernel's clamped preview did.
    const asked = s.targets.length > 0 ? resolve(s.targets[0], amount, s.profile) : null
    const askedSize = asked !== null && !('error' in asked) ? asked.headline : null
    const handleScale =
      askedSize !== null && askedSize > this.c.clampedHeadline ? this.c.clampedHeadline / askedSize : 1
    return { src: 'handle', size: this.c.clampedHeadline, held: { asked: amount, handleScale } }
  }

  /** Picks a grip up: it becomes the active field and follows the pointer
   *  until it is put down (a drag) or clicked again. */
  pickUpGrip(s: ArmedStage, grip: GripId, ray: Ray): void {
    s.grab = grip
    s.grabScreen = this.c.screen
    s.grabRay = ray
    s.gripTyped = ''
    s.gripHover = null
    s.active = grip
    s.pressArm = false
    this.c.statusLine = null
    this.c.measure(this.gripReadout(s))
    this.refreshGripMarkers()
  }

  /** The radius handle let go: sized by a drag it stays at the size; a
   *  click in place finishes, unless it is the arming press's own release. */
  releaseHandle(s: ArmedStage, ray: Ray): void {
    const moved = s.grabScreen === null ? !sameRay(s.grabRay, ray) : this.c.movedEnough(s.grabScreen)
    const pressArm = s.pressArm
    s.pressArm = false
    if (moved) {
      this.c.pendingMove = null
      if (s.typed === '') this.sizeFromHandle(s, ray)
      this.putGripDown(s)
      return
    }
    // In place: nothing moved, so the preview stands as drawn.
    this.putGripDown(s, false)
    if (!pressArm) this.finishArmed(s)
  }

  /** Whether `point` (world) is within the pick aperture of `ray`. */
  withinPickAperture(ray: Ray, point: V3): boolean {
    const v: V3 = [point[0] - ray.origin[0], point[1] - ray.origin[1], point[2] - ray.origin[2]]
    const dl = Math.hypot(ray.direction[0], ray.direction[1], ray.direction[2])
    if (!(dl > 0)) return false
    const d: V3 = [ray.direction[0] / dl, ray.direction[1] / dl, ray.direction[2] / dl]
    const t = v[0] * d[0] + v[1] * d[1] + v[2] * d[2]
    if (t < 0) return false
    const off = Math.hypot(v[0] - d[0] * t, v[1] - d[1] * t, v[2] - d[2] * t)
    const wpp = this.c.worldPerPixel === null ? null : this.c.worldPerPixel(t)
    const aperture = wpp === null ? HANDLE_PICK_FALLBACK_M : wpp * HANDLE_PICK_PX
    return off <= aperture
  }

  enterOnArmed(s: ArmedStage): void {
    if (s.grab !== null && s.grab.kind !== 'radius' && s.gripTyped === '') {
      // Enter with a grip held and nothing typed puts it down where it is.
      this.putGripDown(s)
      this.c.hooks.requestRender()
      return
    }
    if (s.active.kind !== 'radius' && s.gripTyped !== '') {
      // A typed length sets the active grip; the pick stays armed.
      const token = this.parseTyped(s.gripTyped)
      if (token === null || token.kind !== 'length') {
        this.c.hooks.onToast('Type one length for the grip, like 150.')
        return
      }
      this.applyTypedToGrip(s, token.meters)
      return
    }
    if (s.grab !== null && s.grab.kind === 'radius') this.putGripDown(s)
    if (s.typed === '') {
      if (s.amount !== null) this.c.commit.commitArmed(this.armedCommitAmount(s))
      return
    }
    const token = this.parseTyped(s.typed)
    if (token === null) return
    if (token.kind === 'segments') {
      this.applySegmentsOverride(token.n)
      s.typed = ''
      this.c.preview.drawArmedPreview()
      this.c.hooks.requestRender()
      return
    }
    this.c.commit.commitArmed({ src: 'typed', token })
  }

  applySegmentsOverride(n: number): void {
    const s = this.c.stage
    const corner = s.kind === 'armed' && s.targets[0].kind === 'corner'
    const count = corner ? clampCornerSegments(n) : clampBlendSegments(n)
    setFilletSegmentsOverride(count)
    this.c.measure('')
    this.c.note = `${corner ? 'Segments' : 'Facets'} set to ${count}.`
    this.c.noteRay = this.c.pendingMove?.ray ?? this.c.lastRay
  }

  refreshGripMarkers(): void {
    const s = this.c.stage
    if (s.kind === 'edit') {
      this.c.edit.refreshEditMarkers()
      return
    }
    if (s.kind !== 'armed') {
      this.c.gripsView.clear()
      this.c.labels.clear()
      return
    }
    const markers: GripMarker[] = []
    const labels: FilletLabel[] = []
    const isActive = (id: GripId): boolean => sameGrip(id, s.active) || sameGrip(id, s.gripHover)
    const headline = this.currentHeadline(s)
    if (s.handle !== null && headline !== null) {
      const id: GripId = { kind: 'radius' }
      const pos = offsetBy(s.handle.base, s.handle.axis, headline)
      markers.push({ id, pos, from: s.handle.base, active: isActive(id) })
      labels.push({ key: 'radius', pos, dir: s.handle.axis, text: this.radiusReadout(s), active: s.active.kind === 'radius' })
    }
    const t = s.targets[0]
    if (s.gripChain !== null && t.kind === 'solid') {
      const { pts, length, bisectors } = s.gripChain
      const first = pts[0]
      const last = pts[pts.length - 1]
      const scale = t.frame.scale
      const baseRadius = this.currentRadiusGuess(s)
      const endR = (at: number): number => s.grips.handles.find((h) => Math.abs(h.at - at) < 1e-9)?.radius ?? baseRadius
      // A grip's value is written beside it only while it is the active
      // field or under the pointer: six values at once crowd a short edge.
      const add = (id: GripId, pos: V3, dir: V3, value: number): void => {
        markers.push({ id, pos, active: isActive(id) })
        const active = sameGrip(id, s.active)
        if (!active && !sameGrip(id, s.gripHover)) return
        const text = active && s.gripTyped !== '' ? typedReadout(s.gripTyped) : formatLength(value * scale)
        labels.push({ key: `${id.kind}:${'end' in id ? id.end : 'index' in id ? id.index : ''}`, pos, dir, text, active })
      }
      const away = (b: V3): V3 => [-b[0], -b[1], -b[2]]
      add({ kind: 'stop', end: 'start' }, pointAtLength(pts, s.grips.stops.start), away(bisectors[0]), s.grips.stops.start)
      add({ kind: 'stop', end: 'end' }, pointAtLength(pts, length - s.grips.stops.end), away(bisectors[1]), s.grips.stops.end)
      add({ kind: 'endRadius', end: 'start' }, offsetBy(first, bisectors[0], endR(0)), bisectors[0], endR(0))
      add({ kind: 'endRadius', end: 'end' }, offsetBy(last, bisectors[1], endR(length)), bisectors[1], endR(length))
      s.grips.handles.forEach((h, i) => {
        if (h.at <= 1e-9 || h.at >= length - 1e-9) return
        const p = pointAtLength(pts, h.at)
        add({ kind: 'handlePos', index: i }, p, away(bisectors[0]), h.at)
        add({ kind: 'handleRadius', index: i }, offsetBy(p, bisectors[0], h.radius), bisectors[0], h.radius)
      })
    }
    this.c.gripsView.setMarkers(markers)
    this.c.labels.set(labels)
    this.c.publishDimension()
  }

  currentRadiusGuess(s: ArmedStage): number {
    const amount = currentAmount(s)
    if (amount === null) return 0.01
    const r = resolve(s.targets[0], amount, s.profile)
    if ('error' in r || r.headline === null) return 0.01
    return r.sized.kind === 'round' || r.sized.kind === 'cove' ? r.sized.radius : r.headline
  }

  /** The grip under `ray` (world), or null. The chain grips exist only on
   *  world-frame objects, whose local frame is the world; the radius
   *  handle is kept in world space for every target. */
  gripUnder(s: ArmedStage, ray: Ray): GripId | null {
    void s
    return this.c.gripsView.pick(ray.origin, ray.direction)?.id ?? null
  }

  /** Lets go of the held grip where it is; typing goes to the radius again
   *  (Tab reaches a grip's field without picking it up). */
  putGripDown(s: ArmedStage, redraw = true, typedRedraw = false): void {
    s.grab = null
    s.gripTyped = ''
    s.active = { kind: 'radius' }
    s.gripHover = this.c.lastRay === null ? null : this.gripUnder(s, this.c.lastRay)
    if (redraw) this.c.preview.drawArmedPreview(typedRedraw)
    this.refreshGripMarkers()
  }

  /** Tab: the next field takes typing (radius, start stop, end stop, then
   *  each radius point's position and radius). */
  cycleActive(s: ArmedStage): void {
    const order = this.c.gripsView.all.map((m) => m.id)
    if (order.length === 0) return
    const at = order.findIndex((id) => sameGrip(id, s.active))
    const next = order[(at + 1) % order.length]
    if (s.grab !== null) this.putGripDown(s, s.grab.kind !== 'radius')
    s.active = next
    s.gripTyped = ''
    this.c.statusLine = null
    this.c.measure(next.kind === 'radius' ? this.radiusReadout(s) : this.gripReadout(s))
    this.refreshGripMarkers()
    this.c.hooks.requestRender()
  }

  /** Delete on an active radius point takes it out; the radius is active
   *  again. False when the active field is not a radius point. */
  deleteActivePoint(s: ArmedStage): boolean {
    const g = s.active
    if (g.kind !== 'handlePos' && g.kind !== 'handleRadius') return false
    const h = s.grips.handles[g.index]
    if (h === undefined) return false
    s.grips = { ...s.grips, handles: s.grips.handles.filter((_, i) => i !== g.index) }
    s.grab = null
    s.gripTyped = ''
    s.active = { kind: 'radius' }
    this.c.preview.drawArmedPreview()
    this.refreshGripMarkers()
    this.c.hooks.requestRender()
    return true
  }

  radiusReadout(s: ArmedStage): string {
    if (s.typed !== '') return typedReadout(s.typed)
    const h = this.currentHeadline(s)
    return h === null ? '' : formatLength(h)
  }

  /** The headline size the preview shows: the clamp when the kernel
   *  clamped, else what the amount resolves to. */
  currentHeadline(s: ArmedStage): number | null {
    if (this.c.clampedHeadline !== null) return this.c.clampedHeadline
    const amount = currentAmount(s)
    if (amount === null) return null
    const r = resolve(s.targets[0], amount, s.profile)
    return 'error' in r ? null : r.headline
  }

  /** The active field in world meters, shared by readouts and external inputs. */
  gripValue(s: ArmedStage): number | null {
    const g = s.active
    if (g.kind === 'radius') return this.currentHeadline(s)
    const chain = s.gripChain
    const t = s.targets[0]
    if (chain === null || t.kind !== 'solid') return null
    const scale = t.frame.scale
    switch (g.kind) {
      case 'stop': return s.grips.stops[g.end] * scale
      case 'endRadius': {
        const at = g.end === 'start' ? 0 : chain.length
        return (s.grips.handles.find(h => Math.abs(h.at - at) < 1e-9)?.radius ?? this.currentRadiusGuess(s)) * scale
      }
      case 'handlePos': return (s.grips.handles[g.index]?.at ?? 0) * scale
      case 'handleRadius': return (s.grips.handles[g.index]?.radius ?? 0) * scale
    }
  }

  /** The active grip's own value, for the measurement box. */
  gripReadout(s: ArmedStage): string {
    if (s.active.kind === 'radius') return this.radiusReadout(s)
    const value = this.gripValue(s)
    return value === null ? '' : formatLength(value)
  }

  /** A length typed while a grip is picked up sets that grip (WP10 §3.8):
   *  how far a stop or a radius point sits from its end, or the radius at a
   *  radius grip. The grip is put down; the next typed size is the blend's. */
  applyTypedToGrip(s: ArmedStage, meters: number, continuous = false, typedRedraw = false): void {
    const g = s.active
    const chain = s.gripChain
    const t = s.targets[0]
    if (g.kind === 'radius' || chain === null || t.kind !== 'solid') return
    const d = meters / t.frame.scale
    const { length } = chain
    const outOfRange = (): void => {
      this.c.hooks.onToast(kernelErrorMessage('StopOutOfRange', ''), 'StopOutOfRange')
      // The grip stays held; the readout goes back to its value.
      s.gripTyped = ''
      this.c.measure(this.gripReadout(s))
    }
    let note: string
    switch (g.kind) {
      case 'stop': {
        const other = g.end === 'start' ? s.grips.stops.end : s.grips.stops.start
        if ((continuous ? d < 0 : !(d > 0)) || d >= length - other) {
          outOfRange()
          return
        }
        s.grips = withStop(s.grips, g.end, g.end === 'start' ? d : length - d, length)
        note = `Stop set ${formatLength(meters)} from the ${g.end === 'start' ? 'start' : 'end'}.`
        break
      }
      case 'endRadius':
        if (!(d > MIN_FILLET_SIZE)) {
          this.c.hooks.onToast('Type a radius larger than zero.')
          s.gripTyped = ''
          this.c.measure(this.gripReadout(s))
          return
        }
        s.grips = withHandle(s.grips, g.end === 'start' ? 0 : length, d, 1e-9)
        note = `Radius at this end set to ${formatLength(meters)}.`
        break
      case 'handlePos':
        if (!(d > 0) || d >= length) {
          outOfRange()
          return
        }
        s.grips = movedHandle(s.grips, g.index, d, length)
        if (continuous) s.active = { kind: 'handlePos', index: s.grips.handles.findIndex(h => h.at === d) }
        note = `Radius point placed ${formatLength(meters)} along the edge.`
        break
      case 'handleRadius': {
        const h = s.grips.handles[g.index]
        if (h === undefined) return
        if (!(d > MIN_FILLET_SIZE)) {
          this.c.hooks.onToast('Type a radius larger than zero.')
          s.gripTyped = ''
          this.c.measure(this.gripReadout(s))
          return
        }
        s.grips = withHandle(s.grips, h.at, d, 1e-9)
        note = `Radius here set to ${formatLength(meters)}.`
        break
      }
    }
    // A typed grip value: the redraw answers typing.
    if (continuous) {
      s.gripTyped = ''
      this.c.preview.drawArmedPreview(typedRedraw)
      this.refreshGripMarkers()
      this.c.measure(this.gripReadout(s))
      return
    }
    this.putGripDown(s, true, true)
    // The confirmation outranks a limit sentence until the pointer moves.
    this.c.statusLine = null
    this.c.note = note
    this.c.noteRay = this.c.pendingMove?.ray ?? this.c.lastRay
    this.c.hooks.requestRender()
  }

  dragGrip(ray: Ray): void {
    const s = this.c.stage
    if (s.kind !== 'armed' || s.grab === null || s.gripChain === null) return
    const t = s.targets[0]
    if (t.kind !== 'solid') return
    const local = rayToLocal(t.frame, ray)
    const { pts, length, bisectors } = s.gripChain
    const g = s.grab
    if (g.kind === 'radius') return
    if (g.kind === 'stop' || g.kind === 'handlePos') {
      const hit = closestOnPolyline(pts, local.origin, local.direction)
      if (hit === null) return
      s.grips =
        g.kind === 'stop'
          ? withStop(s.grips, g.end, g.end === 'start' ? hit.s : hit.s, length)
          : movedHandle(s.grips, g.index, hit.s, length)
      if (g.kind === 'handlePos') {
        const idx = s.grips.handles.findIndex((h) => Math.abs(h.at - Math.max(0, Math.min(length, hit.s))) < 1e-9)
        if (idx >= 0) {
          s.grab = { kind: 'handlePos', index: idx }
          s.active = s.grab
        }
      }
    } else {
      const base =
        g.kind === 'endRadius'
          ? g.end === 'start'
            ? pts[0]
            : pts[pts.length - 1]
          : pointAtLength(pts, s.grips.handles[g.index]?.at ?? 0)
      const axis = g.kind === 'endRadius' ? (g.end === 'start' ? bisectors[0] : bisectors[1]) : bisectors[0]
      const along = lineParamNearRay(base, axis, local.origin, local.direction)
      if (along === null) return
      const radius = Math.max(MIN_FILLET_SIZE, along)
      const at = g.kind === 'endRadius' ? (g.end === 'start' ? 0 : length) : (s.grips.handles[g.index]?.at ?? 0)
      s.grips = withHandle(s.grips, at, radius, 1e-9)
    }
    this.refreshGripMarkers()
    this.c.preview.drawArmedPreview()
    this.c.measure(this.gripReadout(s))
  }

  addHandleAt(s: ArmedStage, local: Ray): void {
    if (s.gripChain === null) return
    const hit = closestOnPolyline(s.gripChain.pts, local.origin, local.direction)
    if (hit === null) return
    const radius = this.currentRadiusGuess(s)
    s.grips = withHandle(s.grips, hit.s, radius, 1e-9)
    this.refreshGripMarkers()
    this.c.preview.drawArmedPreview()
  }
}
