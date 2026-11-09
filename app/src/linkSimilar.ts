/**
 * Link identical copies — find loose Objects that are the same part (or its
 * mirror image) and make them one linked set.
 *
 * Lane A's `hew.query.signature { id } -> { shape, mirror, symmetric, frame }`
 * says which Objects match: equal `shape` is the same part, one Object's
 * `shape` equal to another's `mirror` is its twin. The first Object of a
 * group becomes the shared member (`wrap_linked`); each other Object is
 * replaced by a placement of that set whose pose maps the first's frame onto
 * its own (`frame_joiner ∘ frame_first⁻¹`, reflected for a twin), through
 * `join_linked_report`. The kernel re-checks every pose and refuses a wrong
 * one (`NotIdentical`) without touching the document, so a pose that is a
 * symmetry away from the right one is simply tried again.
 *
 * The whole action is one undo step and is never automatic.
 */

import { plural, t } from '@lingui/core/macro'
import type { Scene as WasmScene } from './wasm/loader'
import type { NodeRef } from './panels/treeModel'
import { inHistoryBracket } from './tools/transformSelection'
import { composeAffine } from './tools/transformMath'
import { invertAffine3x4 } from './viewport/geoHelpers'
import { parseDispatch } from './api/dispatchResult'
import { parseKernelErrorCode } from './kernelErrors'

type Affine12 = [number, number, number, number, number, number, number, number, number, number, number, number]
type Vec3 = [number, number, number]

export interface PartSignature {
  shape: string
  mirror: string
  symmetric: boolean
  /** The part's face frame: `origin` is its minimum corner, `extents` its size along x, y, z. */
  frame: { origin: Vec3; x: Vec3; y: Vec3; z: Vec3; extents: Vec3 }
}

/** One Object and what the signature said about it. */
export interface Signed {
  object: bigint
  signature: PartSignature
}

/** Objects that are the same part, first one (lowest id) first. */
export interface LookAlikeGroup {
  members: Signed[]
}

export interface LinkSimilarResult {
  /** Sets made: groups in which at least one copy joined. */
  sets: number
  /** Objects turned into placements (the first of each set is not counted). */
  linked: number
  /** Objects left alone: no pose the kernel accepted, stale, or every
   *  Object of a group that joined nothing (a refused signature is never
   *  grouped, so it is not counted). */
  skipped: number
  /** Joiners whose painted faces could not be kept. */
  droppedFacePaint: number
  /** Joiners whose own frame differed from the first's. */
  frameDiffered: number
}

type SignatureScene = Pick<
  WasmScene,
  'api_connection_open' | 'api_connection_close' | 'api_dispatch' | 'object_public_ids'
>

let frameId = 0

function isSignature(v: unknown): v is PartSignature {
  if (typeof v !== 'object' || v === null) return false
  const s = v as Partial<PartSignature>
  const vec = (a: unknown) => Array.isArray(a) && a.length === 3 && a.every((n) => typeof n === 'number' && Number.isFinite(n))
  return (
    typeof s.shape === 'string' &&
    typeof s.mirror === 'string' &&
    typeof s.frame === 'object' &&
    s.frame !== null &&
    vec(s.frame.origin) &&
    vec(s.frame.x) &&
    vec(s.frame.y) &&
    vec(s.frame.z) &&
    vec(s.frame.extents)
  )
}

/**
 * The signature of each Object, in order, or null for one the API refused
 * (`not_an_object`, `no_part_frame`, a stale id) or could not be asked.
 * Calls `hew.query.signature` once per Object over one connection.
 */
export function readSignatures(scene: SignatureScene, objects: readonly bigint[]): (PartSignature | null)[] {
  if (objects.length === 0) return []
  const publicIds = scene.object_public_ids(BigUint64Array.from(objects))
  const conn = scene.api_connection_open()
  const send = (method: string, params: unknown): { result?: unknown; error?: unknown } | null => {
    const d = parseDispatch(scene.api_dispatch(conn, JSON.stringify({ jsonrpc: '2.0', id: ++frameId, method, params })))
    return d.kind === 'reply' ? (JSON.parse(d.frame) as { result?: unknown; error?: unknown }) : null
  }
  try {
    for (const [method, params] of [
      ['hew.meta.hello', { protocol: 1 }],
      ['hew.doc.attach', {}],
    ] as const) {
      if (send(method, params)?.error !== undefined) return objects.map(() => null)
    }
    return objects.map((_, i) => {
      const id = publicIds[i]
      if (!id) return null
      const reply = send('hew.query.signature', { id })
      return reply !== null && reply.error === undefined && isSignature(reply.result) ? reply.result : null
    })
  } finally {
    scene.api_connection_close(conn)
  }
}

/** Group Objects that are the same part or mirror twins; only groups of two or more. */
export function groupBySignature(signed: readonly Signed[]): LookAlikeGroup[] {
  const groups = new Map<string, Signed[]>()
  for (const s of signed) {
    // A part and its mirror image share a key: the smaller of the two hashes.
    const key = s.signature.shape <= s.signature.mirror ? s.signature.shape : s.signature.mirror
    const list = groups.get(key)
    if (list === undefined) groups.set(key, [s])
    else list.push(s)
  }
  return Array.from(groups.values())
    .filter((members) => members.length >= 2)
    .map((members) => ({ members }))
}

type LinkScene = SignatureScene &
  Pick<WasmScene, 'object_ids' | 'node_parent' | 'node_is_live'>

/** Loose world Objects in scope: the given selection's Objects, or the whole model. */
export function looseObjects(scene: LinkScene, selection: readonly NodeRef[] | null): bigint[] {
  const ids =
    selection === null
      ? Array.from(scene.object_ids())
      : selection.filter((n) => n.kind === 'object').map((n) => n.id)
  return ids.filter((id) => scene.node_is_live(0, id) && scene.node_parent(0, id) == null)
}

/** The look-alike groups in scope. A selection of fewer than two Objects means the whole model. */
export function findLookAlikes(scene: LinkScene, selection: readonly NodeRef[] | null): LookAlikeGroup[] {
  let ids = looseObjects(scene, selection)
  if (selection !== null && ids.length < 2) ids = looseObjects(scene, null)
  const sigs = readSignatures(scene, ids)
  const signed: Signed[] = []
  ids.forEach((object, i) => {
    const signature = sigs[i]
    if (signature !== null) signed.push({ object, signature })
  })
  return groupBySignature(signed)
}

/**
 * `groups` narrowed to the selected Objects when two or more are selected
 * (else unchanged), so the menu gate reuses one whole-model read.
 */
export function restrictToSelection(groups: readonly LookAlikeGroup[], selection: readonly NodeRef[]): LookAlikeGroup[] {
  const picked = new Set(selection.filter((n) => n.kind === 'object').map((n) => n.id))
  if (picked.size < 2) return [...groups]
  return groups
    .map((g) => ({ members: g.members.filter((m) => picked.has(m.object)) }))
    .filter((g) => g.members.length >= 2)
}

/**
 * What Link identical copies acts on: the Edit menu (and palette) act on the
 * selection (`findLookAlikes` widens fewer than two Objects to the model);
 * the Model Health button acts on the whole model, the scope its count is.
 */
export function linkSimilarScope(origin: 'menu' | 'model-health', selection: readonly NodeRef[]): readonly NodeRef[] | null {
  return origin === 'menu' ? selection : null
}

/** Whether Edit ▸ Link identical copies has anything to do. */
export function canLinkSimilar(scene: LinkScene, selection: readonly NodeRef[] | null): boolean {
  return findLookAlikes(scene, selection).length > 0
}

/** How many Objects sit in a group with a look-alike (the Model Health notice). */
export function countLookAlikes(groups: readonly LookAlikeGroup[]): number {
  return groups.reduce((n, g) => n + g.members.length, 0)
}

function frameMatrix(f: PartSignature['frame']): Affine12 {
  return [
    f.x[0], f.y[0], f.z[0], f.origin[0],
    f.x[1], f.y[1], f.z[1], f.origin[1],
    f.x[2], f.y[2], f.z[2], f.origin[2],
  ]
}

function diagonal(sx: number, sy: number, sz: number): Affine12 {
  return [sx, 0, 0, 0, 0, sy, 0, 0, 0, 0, sz, 0]
}

/** Sign patterns that map a part frame onto itself: proper ones for the same part, reflections for a twin. */
const PROPER = [diagonal(1, 1, 1), diagonal(1, -1, -1), diagonal(-1, 1, -1), diagonal(-1, -1, 1)]
const REFLECTING = [diagonal(-1, 1, 1), diagonal(1, -1, 1), diagonal(1, 1, -1), diagonal(-1, -1, -1)]

/** Translation by `v`. */
function translation(v: Vec3): Affine12 {
  return [1, 0, 0, v[0], 0, 1, 0, v[1], 0, 0, 1, v[2]]
}

/**
 * The poses to try for `joiner` against `first`, best guess first:
 * `frame_joiner ∘ C ∘ R ∘ C⁻¹ ∘ frame_first⁻¹` for each diagonal sign
 * symmetry `R` of the frame box, `C` the shift to the frame's center. A
 * twin (the joiner's shape is the first's mirror and not its own) needs a
 * reflection. The symmetries act about the frame's center, not its
 * origin: the origin is the part's minimum corner, so a reflection or a
 * half turn about it would put the body on the far side of the frame, off
 * by its own size. Only sign patterns are tried: two frames whose axis
 * labels differ by a permutation (possible when two extents tie and the
 * frame's tie-breaks fall differently on the twin) find no pose here and
 * the joiner is counted as skipped.
 */
export function candidatePoses(first: PartSignature, joiner: PartSignature): Affine12[] {
  const back = invertAffine3x4(frameMatrix(first.frame))
  if (back === null) return []
  const twin = joiner.shape !== first.shape
  const sign = twin ? REFLECTING : PROPER
  const half: Vec3 = [first.frame.extents[0] / 2, first.frame.extents[1] / 2, first.frame.extents[2] / 2]
  const toCenter = translation([-half[0], -half[1], -half[2]])
  const fromCenter = translation(half)
  const out: Affine12[] = []
  for (const r of sign) {
    const aboutCenter = composeAffine(composeAffine(toCenter, r), fromCenter) as Affine12
    const toFrame = composeAffine(back as Affine12, aboutCenter)
    out.push(composeAffine(toFrame, frameMatrix(joiner.frame)) as Affine12)
  }
  return out
}

/** A refusal `linkSimilar` skips past: the pose is wrong, or the Object went stale. */
function skippable(err: unknown): boolean {
  const code = parseKernelErrorCode(err)
  return code === 'NotIdentical' || code === 'UnknownObject'
}

function emptyResult(): LinkSimilarResult {
  return { sets: 0, linked: 0, skipped: 0, droppedFacePaint: 0, frameDiffered: 0 }
}

/** Thrown inside the bracket to take back a pass that left a group wrapped
 *  with nothing joined; `joined[i]` says whether group `i` joined any. */
class RollBack extends Error {
  constructor(readonly joined: boolean[]) {
    // oxlint-disable-next-line lingui/no-unlocalized-strings -- internal control-flow marker, never shown
    super('RollBack')
  }
}

/**
 * One pass over `groups`: wrap each first Object and join the others. Returns
 * whether each group joined at least one copy (a group that joined none still
 * has its first Object wrapped, unless the wrap itself was refused).
 */
function linkPass(scene: WasmScene, groups: readonly LookAlikeGroup[], result: LinkSimilarResult): boolean[] {
  return groups.map((group) => {
    const [first, ...joiners] = group.members
    let set: bigint
    try {
      set = scene.wrap_linked(0, first.object)
    } catch (err) {
      if (!skippable(err)) throw err
      result.skipped += group.members.length
      return true // nothing was wrapped, so nothing to take back
    }
    let joinedAny = false
    for (const joiner of joiners) {
      let joined = false
      for (const pose of candidatePoses(first.signature, joiner.signature)) {
        try {
          const report = JSON.parse(scene.join_linked_report(set, joiner.object, Float64Array.from(pose))) as {
            dropped_face_paint?: boolean
            frame_differed?: boolean
          }
          result.linked += 1
          if (report.dropped_face_paint === true) result.droppedFacePaint += 1
          if (report.frame_differed === true) result.frameDiffered += 1
          joined = true
          break
        } catch (err) {
          if (!skippable(err)) throw err
          // A stale joiner is no closer at another pose.
          if (parseKernelErrorCode(err) === 'UnknownObject') break
        }
      }
      if (joined) joinedAny = true
      else result.skipped += 1
    }
    if (joinedAny) result.sets += 1
    return joinedAny
  })
}

/**
 * Make each group one linked set. One undo step ("Link identical copies").
 * A joiner whose every candidate pose is refused (`NotIdentical`) or that is
 * stale (`UnknownObject`) is skipped and counted, as is every Object of a
 * group whose first Object is stale. A group none of whose joiners joined is
 * left exactly as it was: the pass is taken back and run again without it
 * (the kernel's answers do not depend on the other groups), and its Objects
 * count as skipped. Any other failure aborts the whole step.
 */
export function linkSimilar(scene: WasmScene, groups: readonly LookAlikeGroup[]): LinkSimilarResult {
  if (groups.length === 0) return emptyResult()
  const label = t`Link identical copies`
  let joined: boolean[]
  try {
    return inHistoryBracket(scene, label, () => {
      const result = emptyResult()
      const ok = linkPass(scene, groups, result)
      if (ok.every(Boolean)) return result
      throw new RollBack(ok)
    })
  } catch (err) {
    if (!(err instanceof RollBack)) throw err
    joined = err.joined
  }
  const kept = groups.filter((_, i) => joined[i])
  const dropped = groups.filter((_, i) => !joined[i]).reduce((n, g) => n + g.members.length, 0)
  const result = emptyResult()
  if (kept.length > 0) inHistoryBracket(scene, label, () => linkPass(scene, kept, result))
  result.skipped += dropped
  return result
}

/** The status line after Link identical copies. */
export function linkSimilarMessage(r: LinkSimilarResult): string {
  if (r.linked === 0) {
    if (r.skipped === 0) return t`Nothing was linked: no two objects were the same shape.`
    return plural(r.skipped, {
      one: 'Nothing was linked: # object looked alike but was not the same shape.',
      other: 'Nothing was linked: # objects looked alike but were not the same shape.',
    })
  }
  const parts: string[] = [
    plural(r.linked + r.sets, { one: 'Linked # copy', other: 'Linked # copies' }),
  ]
  if (r.droppedFacePaint > 0) {
    parts.push(
      plural(r.droppedFacePaint, {
        one: 'paint on the faces of # copy was dropped',
        other: 'paint on the faces of # copies was dropped',
      }),
    )
  }
  if (r.frameDiffered > 0) {
    parts.push(
      plural(r.frameDiffered, {
        one: '# copy had its own axes; the first copy’s axes now apply',
        other: '# copies had their own axes; the first copy’s axes now apply',
      }),
    )
  }
  if (r.skipped > 0) {
    parts.push(plural(r.skipped, { one: '# object could not be linked', other: '# objects could not be linked' }))
  }
  return parts.join('; ')
}
