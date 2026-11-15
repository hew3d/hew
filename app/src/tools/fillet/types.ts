/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/** Shared types of the Fillet tool and its stages. */

import type { BlendPreviewer } from './previewer'
import { type Ray } from '../../viewport/math'
import { type CurveToken } from '../curveInput'
import { type FilletProfile } from '../filletSettings'
import { type PlaneSide, type V3 } from '../filletMath'
import { type Corner2d } from '../filletCorner2d'
import { type GripId, type GripState } from '../filletGrips'
import { type ChainInfo, type CornerTarget, type Frame } from '../filletPick'

/** What a commit held at a limit remembers of the ask the preview drew: the
 *  amount asked (its facet count) and the factor variable-radius handles
 *  scale by. Carried with the commit, never kept on the tool. */
export interface HeldAsk {
  asked: Amount
  handleScale: number
}

export type Amount =
  | { src: 'pointer'; d: number; side: 'A' | 'B' }
  /** The gizmo's radius handle: the headline size itself (world meters). */
  | { src: 'handle'; size: number; held?: HeldAsk }
  | { src: 'typed'; token: CurveToken }

/** Where the radius handle leaves the edge and which way it points (world). */
export interface HandleFrame {
  base: V3
  axis: V3
}

export interface Anchor {
  p0: V3
  dir: V3
  a: PlaneSide
  b: PlaneSide
  faceA: bigint
  faceB: bigint
  theta: number
}

export interface SolidTarget {
  kind: 'solid'
  object: bigint
  frame: Frame
  edges: bigint[]
  chains: ChainInfo[]
  anchorEdge: bigint
  anchor: Anchor
  /** The face the pick named (a face pick, or the face under an edge click). */
  primaryFace: bigint | null
  /** Where the arming click landed (world), when it was a click. */
  pickPoint: V3 | null
}

export interface CornerArm {
  kind: 'corner'
  corner: CornerTarget
  geom: Corner2d
  theta: number
}

export type ArmTarget = SolidTarget | CornerArm

/** One pick kept by Shift-click. */
export interface Pick {
  object: bigint
  frame: Frame
  chain: ChainInfo
  hitFace: bigint | null
  point: V3
}

/** A size independent of which edge it is applied to. */
export type Sized =
  | { kind: 'round'; radius: number }
  | { kind: 'cove'; radius: number }
  | { kind: 'chamfer'; a: number; b: number; asymmetric: boolean }
  | { kind: 'bullnose' }
  | { kind: 'shape'; width: number }

/** A target's size limit as a clamped preview reported it, in the target's
 *  own units, valid while `key` (the spec apart from its size) matches. */
export interface SizeLimit {
  key: string
  max: number
  limitedBy: string
}

export interface Resolved {
  sized: Sized
  /** The number the readout and the next double-click repeat. */
  headline: number | null
  label: 'Radius' | 'Distance' | 'Width'
}

export type Stage =
  | { kind: 'idle' }
  | {
      kind: 'armed'
      /** The first target sizes; the rest (other selected objects) follow. */
      targets: ArmTarget[]
      profile: FilletProfile
      amount: Amount | null
      typed: string
      armedScreen: [number, number] | null
      grips: GripState
      /** The chain the grips sit on (world objects with one open chain). */
      gripChain: { pts: V3[]; length: number; bisectors: [V3, V3] } | null
      grab: GripId | null
      /** Where the pointer was when the grip was picked up: a release this
       *  far from it places the grip (a drag), a release in place does not.
       *  The ray is the fallback when no screen point has been seen yet. */
      grabScreen: [number, number] | null
      grabRay: Ray | null
      /** A length typed while a grip is held, for that grip; the blend's
       *  own typed size stays in `typed`. */
      gripTyped: string
      /** The grip under the pointer while none is held. */
      gripHover: GripId | null
      /** The radius handle: null only for a profile with no size (bullnose). */
      handle: HandleFrame | null
      /** The field a typed value goes to: the radius, a stop, a radius
       *  point's position or radius. Tab cycles it; a grip picked up takes
       *  it. */
      active: GripId
      /** The arming press still holds the radius handle: its release in
       *  place puts the handle down (it does not commit). */
      pressArm: boolean
      /** Each target's largest size, once a preview past it found it: a
       *  larger size previews at that size directly, without the kernel
       *  searching for the limit again on every pointer move. */
      limits: Map<SolidTarget, SizeLimit>
      /** Limits found with grips set, keyed by those grips (the held one
       *  by a coarse bucket of its value only, so a drag searches again
       *  when it has moved enough to change the limit, not on every move). */
      gripLimits: Map<SolidTarget, SizeLimit>
      /** The held grip's bucket in that key, and how often a drag of it
       *  moved to a new bucket (at most `HELD_RESEARCHES` re-searches). */
      held: { id: string; bucket: string; searches: number } | null
    }
  | {
      kind: 'edit'
      object: bigint
      frame: Frame
      blendId: number
      kindOfBlend: string
      size0: number
      typed: string
      amount: number | null
      anchorPoint: V3
      /** The radius handle at the band; `grab` while it is being dragged;
       *  `pressEnter` while the click that entered the edit still holds it. */
      handle: HandleFrame
      grab: boolean
      pressEnter: boolean
      grabScreen: [number, number] | null
      grabRay: Ray | null
      anchorNormal: V3
      anchorRay: Ray
      armedScreen: [number, number] | null
      /** The blend's largest size once a preview past it found it (see the
       *  armed stage's `limits`). */
      limit: SizeLimit | null
    }

/** What a successful commit leaves for the retype window. */
export interface RetypeSpec {
  targets: ArmTarget[]
  profile: FilletProfile
  token: CurveToken | null
  grips: GripState
  size: number | null
}

export type ArmedStage = Extract<Stage, { kind: 'armed' }>
export type EditStage = Extract<Stage, { kind: 'edit' }>

export interface FilletHooks {
  /** Solids changed: rebuild their meshes. */
  onCommit(objectIds: bigint[]): void
  /** A sketch or a face drawing changed (a 2D corner). */
  onSketchChange(): void
  onToast(message: string, code?: string): void
  onMeasurement(text: string): void
  /** The tool changed the preview outside a pointer event (frame coalescing). */
  requestRender(): void
  /** Run a long commit behind the busy card (shown and painted first).
   *  Without it the commit runs at once. */
  busy?(message: string, work: () => void): boolean | void
  /** Hide faces of an object while previewing; false = skipped (too big). */
  setFaceMask(objectId: bigint, faceIds: bigint[], maxTriangles: number): boolean
  clearFaceMask(objectId: bigint): void
}

export interface FilletOptions {
  /** Runs `cb` on the next animation frame (tests pass a synchronous one). */
  schedule?: (cb: () => void) => void
  /** Milliseconds, for timing previews (tests pass a fake clock). */
  now?: () => number
  /** Where preview answers come from (default: the kernel, on this thread). */
  previewer?: BlendPreviewer
}
