/**
 * Lock — the app's read of the kernel's lock model (OW-1) and the copy it
 * speaks with.
 *
 * The kernel owns the truth (`node_locked`, `node_frozen`, `set_nodes_locked`)
 * and refuses any edit of a frozen node. The app's job is to say so BEFORE
 * the kernel has to: the hover line beside the cursor, the toast with its
 * Unlock / Make Unique buttons, and the "left behind" rule for commands that
 * act on a selection. Everything here is pure over a narrow scene shape, so
 * it is tested without a kernel.
 *
 * Words: a node is LOCKED when it carries the lock (a standalone Object or
 * group's own flag, or for a placement its shape's, which every copy
 * shares). It is FROZEN when it is locked or sits inside something locked.
 * A shape used inside a locked group is frozen even from its outside
 * copies' point of view (the kernel's "via shared shape" refusal): the
 * outside copy itself still moves, but editing the shape through it is
 * refused, so the way out is Make Unique.
 */

import { plural, t } from '@lingui/core/macro'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeKind, NodeRef } from '../panels/treeModel'
import { isDraftKind, nodeEq, nodeKindToNumber } from '../panels/treeModel'
import { resolveDraft, type PlaneUnitsScene } from '../tools/planeUnits'

/** The scene reads the lock model needs. */
export type LockScene = Pick<
  WasmScene,
  | 'node_locked'
  | 'node_frozen'
  | 'node_parent'
  | 'owner_definition'
  | 'group_members'
  | 'instance_def'
  | 'instances_of'
  | 'definition_usage'
  | 'group_ids'
  | 'instance_ids'
  | 'object_ids'
>

/** A node kind the kernel can lock (object, group, instance). */
export function isLockableKind(kind: NodeKind): kind is 'object' | 'group' | 'instance' {
  return kind === 'object' || kind === 'group' || kind === 'instance'
}

/** Whether the node is locked (its own or its shape's lock). `false` for a
 *  kind that cannot be locked or a handle the kernel no longer knows. */
export function isLocked(scene: Pick<LockScene, 'node_locked'>, node: NodeRef): boolean {
  if (!isLockableKind(node.kind)) return false
  try {
    return scene.node_locked(nodeKindToNumber(node.kind), node.id)
  } catch {
    return false
  }
}

/** A drawing that belongs to a frozen Object is frozen with it (a free
 *  drawing belongs to nothing and is never frozen): the cause of the first
 *  edge whose owner is frozen. */
function draftFrozenCause(scene: Pick<LockScene, 'node_frozen'>, node: NodeRef): NodeRef | null {
  try {
    const resolved = resolveDraft(scene as unknown as PlaneUnitsScene, node)
    if (resolved === null) return null
    for (const edge of resolved.edges) {
      if (edge.store.kind === 'free') continue
      const cause = frozenCause(scene, { kind: 'object', id: edge.store.object })
      if (cause !== null) return cause
    }
  } catch {
    // A listing the kernel cannot give: nothing to say.
  }
  return null
}

/** The locked node that keeps `node` from changing, or null when nothing does. */
export function frozenCause(scene: Pick<LockScene, 'node_frozen'>, node: NodeRef): NodeRef | null {
  if (isDraftKind(node.kind)) return draftFrozenCause(scene, node)
  if (!isLockableKind(node.kind)) return null
  try {
    const cause = scene.node_frozen(nodeKindToNumber(node.kind), node.id)
    return cause === undefined ? null : { kind: cause.kind as NodeKind, id: cause.id }
  } catch {
    return null
  }
}

/** Why a node refuses an edit, as the app words it. */
export interface LockBlock {
  /** The locked node to unlock (the node itself, or the locked group above
   *  it); null only for the indirect case when no cause can be named. */
  cause: NodeRef | null
  /** The shape is also used inside a locked group: Make Unique comes first. */
  indirect: boolean
  /** Other placements sharing the locked shape (0 when it stands alone). */
  copies: number
  /** The placement Make Unique would split off, when there is one to offer. */
  uniqueTarget: NodeRef | null
}

/** Other placements of the shape `instance` is a copy of. */
export function otherCopies(scene: Pick<LockScene, 'instance_def' | 'definition_usage'>, instance: bigint): number {
  try {
    const def = scene.instance_def(instance)
    if (def === undefined) return 0
    return Math.max(0, scene.definition_usage(def) - 1)
  } catch {
    return 0
  }
}

/**
 * The placement of `instance`'s shape that is frozen while `instance` itself
 * is not: the shape is also used inside a locked group (or component), so
 * editing it through this copy would change that locked thing. Returns the
 * frozen placement's cause (the locked group to offer Unlock on), or null
 * when the shape is used nowhere locked.
 */
export function sharedShapeCause(
  scene: Pick<LockScene, 'instance_def' | 'instances_of' | 'node_frozen'>,
  instance: bigint,
): NodeRef | null {
  try {
    const def = scene.instance_def(instance)
    if (def === undefined) return null
    for (const other of scene.instances_of(def)) {
      if (other === instance) continue
      const cause = frozenCause(scene, { kind: 'instance', id: other })
      if (cause !== null) return cause
    }
  } catch {
    // A handle the kernel no longer knows: nothing to say.
  }
  return null
}

/**
 * Whether editing `node` is blocked, and how to get out. Null when it is
 * free. `node` is the thing under the cursor or in the selection.
 *
 * `shape` is set by an act that edits the SHAPE a placement shows (opening
 * it for editing, Push/Pull, Offset, Follow Me, Eraser on its faces): that
 * is also refused when the shape is used inside a locked group, even though
 * this copy is free (the indirect case). Moving, deleting or grouping the
 * free copy is not an edit of the shape.
 */
export function lockBlockFor(scene: LockScene, node: NodeRef, opts: { shape?: boolean } = {}): LockBlock | null {
  const cause = frozenCause(scene, node)
  if (cause === null) {
    if (opts.shape === true && node.kind === 'instance') {
      const shared = sharedShapeCause(scene, node.id)
      if (shared !== null) return { cause: shared, indirect: true, copies: 0, uniqueTarget: node }
    }
    return null
  }
  return blockForCause(scene, cause)
}

/** The block for a known `cause`. The set of copies is the cause's own when
 *  it is a placement (every copy shares its shape's lock); a locked group
 *  above is just one thing. */
export function blockForCause(scene: LockScene, cause: NodeRef): LockBlock {
  const placement = cause.kind === 'instance' ? cause : null
  const copies = placement !== null ? otherCopies(scene, placement.id) : 0
  const uniqueTarget = placement !== null && copies > 0 && isLocked(scene, placement) ? placement : null
  return { cause, indirect: false, copies, uniqueTarget }
}

/** The "Locked" line for a block, in plain words. */
export function lockedMessage(block: Pick<LockBlock, 'copies' | 'indirect'>): string {
  if (block.indirect) return t`Also used in a locked group`
  if (block.copies === 1) return t`Both copies are locked`
  if (block.copies > 1) {
    const total = block.copies + 1
    return t`All ${total} copies are locked`
  }
  return t`Locked`
}

/** "1 locked item was left alone." / "N locked items were left alone." */
export function leftAloneMessage(count: number): string {
  return plural(count, { one: '# locked item was left alone.', other: '# locked items were left alone.' })
}

/** What a command that ran on the rest says about the nodes it left behind: a
 *  node that is itself locked counts as a locked item; an unlocked group that
 *  merely HOLDS something locked is said plainly, not called locked. */
export function leftBehindMessage(scene: LockScene, left: readonly NodeRef[]): string {
  const locked = left.filter((n) => frozenCause(scene, n) !== null).length
  const holding = left.length - locked
  const parts: string[] = []
  if (locked > 0) parts.push(leftAloneMessage(locked))
  if (holding > 0) {
    parts.push(
      plural(holding, {
        one: '# group that holds something locked was left alone.',
        other: '# groups that hold something locked were left alone.',
      }),
    )
  }
  return parts.join(' ')
}

// ── The toast code: a toast names what it is about ─────────────────────────

/** Prefix of the toast code a locked hint carries. The code names the node
 *  to offer Unlock (and Make Unique) on, so the toast's buttons need no
 *  state beside the message: `LockedHint:<kind>:<id>`, optionally followed
 *  by `:indirect`. Not a kernel error code. */
// oxlint-disable-next-line lingui/no-unlocalized-strings -- a toast code, not copy
export const LOCKED_HINT_PREFIX = 'LockedHint:'

export interface LockedHintTarget {
  node: NodeRef
  indirect: boolean
}

export function lockedHintCode(node: NodeRef, indirect = false): string {
  return `${LOCKED_HINT_PREFIX}${node.kind}:${node.id}${indirect ? ':indirect' : ''}`
}

export function parseLockedHintCode(code: string | undefined): LockedHintTarget | null {
  if (code === undefined || !code.startsWith(LOCKED_HINT_PREFIX)) return null
  const [kind, id, flag] = code.slice(LOCKED_HINT_PREFIX.length).split(':')
  if (!isLockableKind(kind as NodeKind) || id === undefined || !/^\d+$/.test(id)) return null
  return { node: { kind: kind as NodeKind, id: BigInt(id) }, indirect: flag === 'indirect' }
}

/** Whether a toast code marks a lock toast: a locked hint or the kernel's own
 *  locked refusal. There is only ever one such toast on screen. */
export function isLockToastCode(code: string | null | undefined): boolean {
  return (code !== undefined && code !== null && code.startsWith(LOCKED_HINT_PREFIX)) || isLockRefusalCode(code)
}

/** The kernel's refusal codes that mean "locked". */
export function isLockRefusalCode(code: string | null | undefined): boolean {
  return code === 'LockedNode' || code === 'LockedNodeViaSharedShape'
}

// ── The toast's buttons ─────────────────────────────────────────────────────

/** One button on a locked hint's toast: what it does, and to which node. */
export interface LockToastStep {
  action: 'unlock' | 'make-unique'
  node: NodeRef
}

/**
 * The buttons a locked toast carries, in order, from its code (the node the
 * hint is about) or — for the kernel's own refusal, which names no node —
 * from the selection:
 *
 * - direct: Unlock; and, when the locked thing is one of a set of copies,
 *   Make Unique after it;
 * - indirect (the shape is also used inside a locked group): Make Unique
 *   first, then Unlock on the group that holds the shape.
 */
export function lockToastSteps(
  scene: LockScene,
  code: string | undefined,
  selection: readonly NodeRef[],
): LockToastStep[] | undefined {
  let target = parseLockedHintCode(code)
  if (target === null && isLockRefusalCode(code)) {
    for (const n of selection) {
      const cause = frozenCause(scene, n)
      if (cause !== null) {
        target = { node: cause, indirect: false }
        break
      }
      if (n.kind === 'instance' && sharedShapeCause(scene, n.id) !== null) {
        target = { node: n, indirect: true }
        break
      }
    }
  }
  if (target === null) return undefined
  if (target.indirect) {
    const holder = target.node.kind === 'instance' ? sharedShapeCause(scene, target.node.id) : null
    const unique: LockToastStep = { action: 'make-unique', node: target.node }
    return holder !== null ? [unique, { action: 'unlock', node: holder }] : [unique]
  }
  const unlock: LockToastStep = { action: 'unlock', node: target.node }
  const block = lockBlockFor(scene, target.node)
  return block?.uniqueTarget != null ? [unlock, { action: 'make-unique', node: block.uniqueTarget }] : [unlock]
}

// ── Hints (the line beside the cursor, and the click's toast) ─────────────

/** What a tool says about a locked thing under the cursor: the line, and the
 *  toast code that carries the target (the app builds the buttons from it). */
export interface LockHint {
  message: string
  code: string
}

/** The hint for editing `node`, or null when it is free to edit. */
export function lockHintFor(scene: LockScene, node: NodeRef | null, opts: { shape?: boolean } = {}): LockHint | null {
  if (node === null) return null
  const block = lockBlockFor(scene, node, opts)
  if (block === null) return null
  // The toast's buttons act on the node the code names: the placement to
  // make unique (indirect), else the locked node itself.
  const target = block.indirect ? (block.uniqueTarget ?? node) : (block.cause ?? node)
  return { message: lockedMessage(block), code: lockedHintCode(target, block.indirect) }
}

/**
 * The hint for a face or edge the pointer is over, given the Object and the
 * placement the pick went through. A pick through a placement is that
 * placement's (the shape's lock is every copy's); a plain Object is its own,
 * and so is a member of a group (the group above may be what is locked,
 * which `node_frozen` answers for the member).
 */
export function lockHintForPick(
  scene: LockScene,
  object: bigint,
  instance: bigint | undefined,
  opts: { shape?: boolean } = {},
): LockHint | null {
  const node: NodeRef = instance !== undefined ? { kind: 'instance', id: instance } : { kind: 'object', id: object }
  return lockHintFor(scene, node, opts)
}

// ── The selection split: locked items are left behind ─────────────────────

/** Whether `node` is a group (or holds one) with a frozen member: the kernel
 *  refuses to move or delete such a group and names the member, so the app
 *  leaves it behind too. */
export function hasFrozenDescendant(scene: LockScene, node: NodeRef): boolean {
  if (node.kind !== 'group') return false
  const seen = new Set<bigint>()
  const walk = (group: bigint): boolean => {
    if (seen.has(group)) return false
    seen.add(group)
    let members
    try {
      members = scene.group_members(group)
    } catch {
      return false
    }
    for (const m of members) {
      const ref: NodeRef = { kind: m.kind as NodeKind, id: m.id }
      if (frozenCause(scene, ref) !== null) return true
      if (ref.kind === 'group' && walk(ref.id)) return true
    }
    return false
  }
  return walk(node.id)
}

export interface SelectionSplit {
  /** What the command runs on. */
  go: NodeRef[]
  /** What it leaves alone, in selection order. */
  left: NodeRef[]
}

/**
 * Splits a selection for a command that runs on what it can: a frozen node
 * is left behind, and so is a group holding a frozen member when
 * `descendants` is set (Move, Delete, Group). Drawing and any other kind the
 * kernel cannot lock stay in `go`; the kernel's own refusal is the backstop.
 */
export function splitLocked(
  scene: LockScene,
  nodes: readonly NodeRef[],
  opts: { descendants?: boolean } = {},
): SelectionSplit {
  const go: NodeRef[] = []
  const left: NodeRef[] = []
  for (const n of nodes) {
    const frozen = frozenCause(scene, n) !== null
    const blocked = frozen || (opts.descendants === true && hasFrozenDescendant(scene, n))
    ;(blocked ? left : go).push(n)
  }
  return { go, left }
}

/** Whether `node` may be COPIED: free, or locked only by its own lock (a
 *  locked standalone Object, group or placement). A node frozen by what holds
 *  it (a locked group above) is not: the copy would rewrite that container. */
export function copyAllowed(scene: LockScene, node: NodeRef): boolean {
  if (frozenCause(scene, node) === null) return true
  return isLocked(scene, node) && containerFreezeCause(scene, node) === null
}

/**
 * The locked node ABOVE `node` that freezes it, ignoring `node`'s own lock:
 * its parent group's cause (the group's own lock or what holds the group),
 * or for a node at the top of a definition, the first frozen placement of
 * that definition (its contents are frozen through the shared shape). Null
 * when nothing above holds it. The kernel's `node_frozen` answers with the
 * node itself when it is locked, which hides a locked group above it.
 */
export function containerFreezeCause(scene: LockScene, node: NodeRef): NodeRef | null {
  if (!isLockableKind(node.kind)) return null
  try {
    const kind = nodeKindToNumber(node.kind)
    const parent = scene.node_parent(kind, node.id)
    if (parent !== undefined && parent !== null) return frozenCause(scene, { kind: 'group', id: parent })
    const def = scene.owner_definition(kind, node.id)
    if (def === undefined || def === null) return null
    for (const placement of scene.instances_of(def)) {
      const cause = frozenCause(scene, { kind: 'instance', id: placement })
      if (cause !== null) return cause
    }
  } catch {
    // A handle the kernel no longer knows: nothing holds it.
  }
  return null
}

/** The first frozen node (or group holding one) of a set, for the "Locked."
 *  toast of a command that needs all its operands. */
export function firstBlocked(
  scene: LockScene,
  nodes: readonly NodeRef[],
  opts: { descendants?: boolean } = {},
): { node: NodeRef; block: LockBlock } | null {
  for (const n of nodes) {
    const direct = lockBlockFor(scene, n)
    if (direct !== null) return { node: n, block: direct }
    if (opts.descendants === true && hasFrozenDescendant(scene, n)) {
      const member = firstFrozenMember(scene, n)
      if (member !== null) {
        const block = lockBlockFor(scene, member)
        if (block !== null) return { node: member, block }
      }
    }
  }
  return null
}

function firstFrozenMember(scene: LockScene, group: NodeRef): NodeRef | null {
  const seen = new Set<bigint>()
  const walk = (id: bigint): NodeRef | null => {
    if (seen.has(id)) return null
    seen.add(id)
    for (const m of scene.group_members(id)) {
      const ref: NodeRef = { kind: m.kind as NodeKind, id: m.id }
      if (frozenCause(scene, ref) !== null) return ref
      if (ref.kind === 'group') {
        const deeper = walk(ref.id)
        if (deeper !== null) return deeper
      }
    }
    return null
  }
  return walk(group.id)
}

/** What the app tells a person about a command that met locked items. */
export type GateOutcome =
  | { kind: 'clear'; go: NodeRef[] }
  | { kind: 'left-behind'; go: NodeRef[]; left: NodeRef[] }
  | { kind: 'refused'; node: NodeRef; block: LockBlock }

/**
 * The gate for a command that acts on a selection and runs on the rest:
 * 'clear' when nothing is locked, 'left-behind' when some are (the command
 * runs on `go`; say `leftBehindMessage(scene, left)` once), 'refused' when
 * everything is locked (do nothing; toast the "Locked." line).
 */
export function gateSelection(
  scene: LockScene,
  nodes: readonly NodeRef[],
  opts: { descendants?: boolean } = {},
): GateOutcome {
  const { go, left } = splitLocked(scene, nodes, opts)
  if (left.length === 0) return { kind: 'clear', go }
  if (go.length === 0) {
    const first = firstBlocked(scene, left, opts)
    if (first !== null) return { kind: 'refused', node: first.node, block: first.block }
    return { kind: 'clear', go: [...nodes] }
  }
  return { kind: 'left-behind', go, left }
}

/**
 * The gate for a command that needs ALL its operands (Union, Make Component,
 * Explode, …): the first blocked operand, or null when every one is free.
 */
export function gateAllOperands(
  scene: LockScene,
  nodes: readonly NodeRef[],
  opts: { descendants?: boolean } = {},
): { node: NodeRef; block: LockBlock } | null {
  return firstBlocked(scene, nodes, opts)
}

/** Whether two sets name the same nodes (for tests and de-duplication). */
export function sameNodes(a: readonly NodeRef[], b: readonly NodeRef[]): boolean {
  return a.length === b.length && a.every((n, i) => nodeEq(n, b[i]))
}

// ── Whole-document reads (the Unlock ▸ All gate) ───────────────────────────

/** Every lockable node: objects, groups and placements. */
export function allLockableNodes(scene: LockScene): NodeRef[] {
  const out: NodeRef[] = []
  for (const id of scene.object_ids()) out.push({ kind: 'object', id })
  for (const id of scene.group_ids()) out.push({ kind: 'group', id })
  for (const id of scene.instance_ids()) out.push({ kind: 'instance', id })
  return out
}

/** Whether anything in the document is locked. */
export function anyLocked(scene: LockScene): boolean {
  return allLockableNodes(scene).some((n) => isLocked(scene, n))
}
