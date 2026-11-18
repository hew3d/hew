/* oxlint-disable lingui/no-unlocalized-strings -- Preview-only UI copy */
/** A non-committing input adapter owned by one Fillet operation. */
import { getLengthUnit, typedReadout } from '../../settings/units'
import { parseCurveInput } from '../curveInput'
import { MIN_FILLET_SIZE } from '../filletMath'
import { getFilletSettings, type FilletProfile } from '../filletSettings'
import { sameGrip } from '../filletGrips'
import { currentAmount, limitKey, resolve } from './spec'
import { NO_DIMENSION, publishFilletDimension, type DimensionGesture, type DimensionSnapshot, type DimensionSource } from './dimensionStore'
import type { FilletContext } from './context'
import type { Stage } from './types'

let nextOperation = 0
export class FilletDimension implements DimensionSource {
  private stage: Stage | null = null
  private operation = 0
  private gesture = 0
  private inputIdentity: string | null = null
  private inputStage: Stage | null = null
  private inputProfile: FilletProfile | null = null
  private profile: FilletProfile | null = null
  private profileRevision = 0
  constructor(private readonly c: FilletContext) {}

  snapshot(): DimensionSnapshot {
    const s = this.c.stage
    if (s !== this.stage) { this.stage = s; this.operation = ++nextOperation }
    if (s.kind === 'idle') return NO_DIMENSION
    const profile = s.kind === 'armed' ? s.profile : getFilletSettings().profile
    const grip = s.kind === 'armed' ? s.active : { kind: 'radius' as const }
    if (profile !== this.profile) { this.profile = profile; this.profileRevision++ }
    const identity = this.c.dimensionEditing && this.inputStage === s && this.inputProfile === profile && this.inputIdentity !== null ? this.inputIdentity : `${this.operation}:${this.profileRevision}:${JSON.stringify(grip)}`
    const base: DimensionSnapshot = { ...NO_DIMENSION, identity, grip, profile, reason: null }
    if (s.kind === 'edit') {
      // Existing resize exposes one headline only; don't flatten stored variable radii.
      return { ...base, reason: 'Use the existing blend handle to resize this treatment.' }
    }
    if (profile.kind === 'bullnose') return { ...base, reason: 'Bullnose fits the edge automatically; it has no size input.' }
    const amount = currentAmount(s)
    if (amount === null) return { ...base, reason: 'Drag the handle to establish a size first.' }
    const resolved = resolve(s.targets[0], amount, profile)
    if ('error' in resolved) return { ...base, reason: resolved.error }
    if (resolved.sized.kind === 'chamfer' && resolved.sized.asymmetric) {
      return { ...base, reason: 'Use Measurements for a chamfer with two distances or an angle.' }
    }
    const draft = grip.kind === 'radius' ? s.typed : s.gripTyped
    if (draft !== '') {
      // An incomplete buffer must not silently fall back to the last valid amount.
      const token = parseCurveInput(draft, getLengthUnit())
      if ('error' in token || token.kind !== 'length') return { ...base, reason: 'Finish the dimension in Measurements first.' }
    }
    const requested = grip.kind === 'radius' ? resolved.headline : this.c.size.gripValue(s)
    const value = this.c.size.gripValue(s)
    const label = this.c.heldGripLabel() ?? resolved.label
    if (value === null || !Number.isFinite(value)) return { ...base, reason: 'This selection has no scalar size.' }
    let limit = grip.kind === 'radius' ? this.c.clampedHeadline : null
    const target = s.targets[0]
    if (grip.kind === 'radius' && limit === null) {
      for (const t of s.targets) {
        if (t.kind !== 'solid') continue
        const cached = s.limits.get(t)
        const key = limitKey(t, resolved.sized, profile)
        if (cached !== undefined && cached.key.startsWith(`${key}|`)) {
          const world = cached.max * t.frame.scale
          limit = limit === null ? world : Math.min(limit, world)
        }
      }
    }
    if (target.kind === 'solid' && s.gripChain !== null) {
      if (grip.kind === 'stop') limit = (s.gripChain.length - s.grips.stops[grip.end === 'start' ? 'end' : 'start']) * target.frame.scale
      if (grip.kind === 'handlePos') limit = s.gripChain.length * target.frame.scale
    }
    return { ...base, label, value, requested, limit, draft: draft === '' ? undefined : typedReadout(draft) }
  }

  publish(): void { publishFilletDimension(this) }

  begin(identity: string, input: 'slider' | 'typed' = 'slider'): DimensionGesture | null {
    const snapshot = this.snapshot()
    const s = this.c.stage
    if (snapshot.identity !== identity || snapshot.reason !== null || s.kind !== 'armed') return null
    if (this.c.dimensionEditing) return null
    this.inputIdentity = identity
    this.inputStage = s
    this.inputProfile = s.profile
    const generation = ++this.gesture
    const profile = s.profile
    let grip = s.active
    const saved = { amount: s.amount, typed: s.typed, gripTyped: s.gripTyped, grips: s.grips, active: s.active }
    let live = true
    let pending: number | null = null
    let pendingText: string | undefined
    let scheduled = false
    const valid = (): boolean => live && generation === this.gesture && this.c.stage === s && s.profile === profile && sameGrip(s.active, grip)
    this.c.pendingMove = null
    s.grab = null
    s.pressArm = false
    this.c.dimensionEditing = true
    const apply = (): void => {
      scheduled = false
      if (!valid() || pending === null) return
      const value = pending
      pending = null
      if (grip.kind === 'radius') {
        if (!(value > MIN_FILLET_SIZE)) return
        s.typed = input === 'typed' ? pendingText ?? '' : ''
        s.amount = input === 'typed' ? { src: 'typed', token: { kind: 'length', meters: value } } : { src: 'handle', size: value }
        s.grab = null
        this.c.preview.drawArmedPreview(input === 'typed')
        this.c.size.refreshGripMarkers()
      } else {
        this.c.size.applyTypedToGrip(s, value, true, input === 'typed')
        grip = s.active // Moving a point can change its sorted index.
      }
      this.c.hooks.requestRender()
      this.publish()
    }
    const close = (): void => {
      live = false
      if (generation === this.gesture) {
        this.c.dimensionEditing = false
        this.inputIdentity = null
        this.c.pendingMove = null
      }
    }
    return {
      update: (meters, text) => {
        if (!valid() || !Number.isFinite(meters) || meters < 0) return
        pending = meters
        pendingText = text
        if (scheduled) return
        scheduled = true
        this.c.schedule(apply)
      },
      finish: () => {
        if (valid()) {
          apply()
          s.grab = null
          s.pressArm = false
          this.c.measure(this.c.size.gripReadout(s))
        }
        close()
        this.publish()
      },
      cancel: () => {
        if (valid()) {
          pending = null
          Object.assign(s, saved)
          s.grab = null
          s.pressArm = false
          s.limits.clear()
          s.gripLimits.clear()
          s.held = null
          this.c.preview.drawArmedPreview(saved.typed !== '' || saved.amount?.src === 'typed')
          this.c.size.refreshGripMarkers()
          this.c.measure(saved.gripTyped !== '' ? typedReadout(saved.gripTyped) : this.c.size.gripReadout(s))
          this.c.hooks.requestRender()
        }
        close()
        this.publish()
      },
    }
  }
}
