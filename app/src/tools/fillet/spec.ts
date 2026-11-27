/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/** Pure size and kernel-spec arithmetic for the Fillet tool: an amount into a size, a size into the kernel BlendSpec JSON, and the drawn-corner limits. No state. */

import { formatLengthLimit, getLengthUnit } from '../../settings/units'
import { arcSegments, segmentsPerTurn } from '../arcMath'
import { parseCurveInput } from '../curveInput'
import { getFilletSettings, type FilletProfile } from '../filletSettings'
import { chamferFromAngle, clampBlendSegments, customArcSegments, roundRadiusFromSetback, type ProfileShapeJson } from '../filletMath'
import { cornerMaxSetback, cornerRadiusFromSetback } from '../filletCorner2d'
import { gripStateIsActive, handlesForSpec, type GripState } from '../filletGrips'
import type { Amount, SolidTarget, CornerArm, ArmTarget, Sized, Resolved, ArmedStage, EditStage } from './types'
import { sizeOf } from './helpers'

/** Turns an amount into a size for the profile, or says why not. */
export function resolve(primary: ArmTarget, amountIn: Amount, profile: FilletProfile): Resolved | { error: string } {
  // The handle names the headline size directly, as a typed length does.
  const amount: Amount =
    amountIn.src === 'handle' ? { src: 'typed', token: { kind: 'length', meters: amountIn.size } } : amountIn
  const theta = primary.kind === 'solid' ? primary.anchor.theta : primary.theta
  const typed = amount.src === 'typed' ? amount.token : null
  if (typed !== null && typed.kind !== 'length' && typed.kind !== 'pair' && typed.kind !== 'lengthAngle') {
    return { error: 'Type a size, like 6, or two sizes, like 6,3 for a chamfer.' }
  }
  switch (profile.kind) {
    case 'round':
    case 'cove': {
      let r: number
      if (amount.src === 'pointer') {
        r = profile.kind === 'round' ? roundRadiusFromSetback(amount.d, theta) : amount.d
      } else if (typed !== null && typed.kind === 'length') {
        r = typed.meters
      } else {
        return { error: 'Type one length for a fillet, like 6.' }
      }
      return { sized: { kind: profile.kind, radius: r }, headline: r, label: 'Radius' }
    }
    case 'chamfer': {
      if (amount.src === 'pointer') {
        return { sized: { kind: 'chamfer', a: amount.d, b: amount.d, asymmetric: false }, headline: amount.d, label: 'Distance' }
      }
      if (typed !== null && typed.kind === 'length') {
        return { sized: { kind: 'chamfer', a: typed.meters, b: typed.meters, asymmetric: false }, headline: typed.meters, label: 'Distance' }
      }
      if (typed !== null && typed.kind === 'pair') {
        return { sized: { kind: 'chamfer', a: typed.a, b: typed.b, asymmetric: true }, headline: typed.a, label: 'Distance' }
      }
      if (typed !== null && typed.kind === 'lengthAngle') {
        const r = chamferFromAngle(typed.meters, typed.radians, theta)
        if (!r.ok) return { error: "That angle doesn't fit this edge." }
        return { sized: { kind: 'chamfer', a: typed.meters, b: r.setbackB, asymmetric: true }, headline: typed.meters, label: 'Distance' }
      }
      return { error: 'Type a distance.' }
    }
    case 'bullnose':
      return { sized: { kind: 'bullnose' }, headline: null, label: 'Radius' }
    case 'shape': {
      const shape = profile.shape as ProfileShapeJson
      let width: number
      if (amount.src === 'pointer') width = amount.side === 'A' ? amount.d : amount.d / shape.b
      else if (typed !== null && typed.kind === 'length') width = typed.meters
      else return { error: 'Type one width.' }
      return { sized: { kind: 'shape', width }, headline: width, label: 'Width' }
    }
  }
}

/** What a size limit depends on besides the size: the profile and its
 *  proportions, the corners, the facet count override. */
export function limitKey(t: SolidTarget, sized: Sized, profile: FilletProfile): string {
  const settings = getFilletSettings()
  const ratio = sized.kind === 'chamfer' ? `${sized.b / sized.a}:${sized.asymmetric ? String(t.primaryFace ?? t.anchor.faceA) : ''}` : ''
  const shape = profile.kind === 'shape' ? profile.id : ''
  return [sized.kind, ratio, shape, settings.corners, settings.segmentsOverride ?? ''].join('|')
}

/** The kernel `BlendSpec` JSON for `target` at `sized`, with the size
 *  multiplied by `k` (1/scale for a preview inside an instance) and the
 *  grips converted to the same units. The automatic facet count follows
 *  `facetsFor` (default `sized`): a preview held at the size limit keeps
 *  the asked size's count, as the kernel's own clamped preview does. */
export function specJson(
  target: SolidTarget,
  sized: Sized,
  profile: FilletProfile,
  k: number,
  grips: GripState | null,
  facetsFor: Sized = sized,
  handleScale = 1,
): string {
  const settings = getFilletSettings()
  const theta = target.anchor.theta
  const seg = (r: number): number =>
    clampBlendSegments(settings.segmentsOverride ?? arcSegments(r, Math.PI - theta))
  const facetSize = sizeOf(facetsFor) ?? 0
  let primary: bigint | null = null
  let profileJson: Record<string, unknown>
  switch (sized.kind) {
    case 'round':
      profileJson = { kind: 'round', radius: sized.radius * k, segments: seg(facetSize) }
      break
    case 'cove':
      profileJson = { kind: 'cove', radius: sized.radius * k, segments: seg(facetSize) }
      break
    case 'chamfer':
      profileJson = { kind: 'chamfer', setback_a: sized.a * k, setback_b: sized.b * k }
      if (sized.asymmetric) primary = target.primaryFace ?? target.anchor.faceA
      break
    case 'bullnose':
      profileJson = { kind: 'bullnose', segments: clampBlendSegments(settings.segmentsOverride ?? 12) }
      break
    case 'shape': {
      const shape = (profile as Extract<FilletProfile, { kind: 'shape' }>).shape as ProfileShapeJson
      profileJson = {
        kind: 'custom',
        shape,
        width: sized.width * k,
        arc_segments: clampBlendSegments(
          settings.segmentsOverride ?? customArcSegments(shape, facetSize, segmentsPerTurn),
        ),
      }
      break
    }
  }
  const spec: Record<string, unknown> = {
    edges: target.edges.map(String),
    profile: profileJson,
    corners: settings.corners,
  }
  if (primary !== null) spec.primary = String(primary)
  if (grips !== null && gripStateIsActive(grips)) {
    // Grips are kept in the object's own frame, the sizes in world units:
    // `k` turns world units into the call's, so object units take
    // `k * scale` (1 for a preview on the definition object; the scale for
    // a commit through an instance, which divides every length by it).
    const g = k * target.frame.scale
    if (grips.stops.start > 0 || grips.stops.end > 0) {
      spec.stops = { start: grips.stops.start * g, end: grips.stops.end * g }
    }
    // Handle radii scale with a clamped size as the kernel's
    // `BlendSpec::at_size` scales them, so a held or committed size is the
    // shape the clamped preview showed.
    const handles = handlesForSpec(grips)
    if (handles !== undefined) spec.handles = handles.map((h) => ({ ...h, at: h.at * g, radius: h.radius * g * handleScale }))
  }
  return JSON.stringify(spec)
}

/** The resize spec at `size`; the automatic facet count follows
 *  `facetSize` (see `specJson`). */
export function editProfileJson(s: EditStage, size: number, k: number, facetSize = size): string | null {
  const settings = getFilletSettings()
  const kind =
    s.kindOfBlend === 'chamfer' ? 'chamfer' : s.kindOfBlend === 'profile' ? 'shape' : 'round'
  if (kind === 'chamfer') {
    return JSON.stringify({ kind: 'chamfer', setback_a: size * k, setback_b: size * k })
  }
  if (kind === 'round') {
    return JSON.stringify({
      kind: 'round',
      radius: size * k,
      segments: clampBlendSegments(settings.segmentsOverride ?? arcSegments(facetSize, Math.PI / 2)),
    })
  }
  const p = settings.profile
  if (p.kind !== 'shape') return null
  const shape = p.shape as ProfileShapeJson
  return JSON.stringify({
    kind: 'custom',
    shape,
    width: size * k,
    arc_segments: clampBlendSegments(customArcSegments(shape, facetSize, segmentsPerTurn)),
  })
}

export function currentAmount(s: ArmedStage): Amount | null {
  if (s.typed !== '') {
    const parsed = parseCurveInput(s.typed, getLengthUnit())
    if (!('error' in parsed) && parsed.kind !== 'segments') return { src: 'typed', token: parsed }
  }
  return s.amount
}

export function editSize(s: EditStage): number | null {
  if (s.typed !== '') {
    const t = parseCurveInput(s.typed, getLengthUnit())
    if (!('error' in t) && t.kind === 'length') return t.meters
    return null
  }
  return s.amount
}

/** The largest size a drawn corner takes: the shorter leg's setback, as
 *  a radius for a round (`r = s tan(theta/2)`) and a distance for a chamfer. */
export function cornerSizeLimit(t: CornerArm, kind: 'round' | 'chamfer'): number {
  // A hair inside the leg: the kernel refuses a tangent point on the
  // leg's far end itself.
  const s = cornerMaxSetback(t.geom) * (1 - 1e-4)
  return kind === 'round' ? cornerRadiusFromSetback(s, t.theta) : s
}

/** `sized` held at the corner's limit when it asks for more, with the
 *  limit and the size's name; null when it fits. */
export function cornerClamp(t: CornerArm, sized: Sized): { sized: Sized; limit: number; label: string } | null {
  if (sized.kind === 'round') {
    const limit = cornerSizeLimit(t, 'round')
    return sized.radius > limit ? { sized: { kind: 'round', radius: limit }, limit, label: 'Radius' } : null
  }
  if (sized.kind === 'chamfer') {
    const limit = cornerSizeLimit(t, 'chamfer')
    const over = Math.max(sized.a, sized.b)
    if (over <= limit) return null
    const k = limit / over
    return { sized: { ...sized, a: sized.a * k, b: sized.b * k }, limit, label: 'Distance' }
  }
  return null
}

export function cornerLimitText(t: CornerArm, kind: 'round' | 'chamfer'): string {
  return `${kind === 'round' ? 'Radius' : 'Distance'} limited to ${formatLengthLimit(cornerSizeLimit(t, kind))} by the shorter side.`
}
