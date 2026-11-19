import { t } from '@lingui/core/macro'
/**
 * Selection path — the pure crumb builder behind the viewport strip, the
 * Object Info "in A › B › C" line, and the hover tip line
 * (docs/design/selection-path.md, "Decisions"). Answers "what is this thing,
 * and where is it in the Outliner?" from the open session frames, the one
 * selected node, and the hit leaf of the click that selected it.
 *
 * Pure: every scene read arrives as a callback, so the whole contract is
 * unit-testable without wasm.
 */
import { nodeEq, nodeKey, isDraftKind, type NodeKind, type NodeRef } from './treeModel'

/** What a crumb stands for. `member` is a definition member inside a
 *  component instance that is not open: it has no world id and cannot be
 *  selected, only named. `divider` separates the open frames from a selection
 *  that is not inside them: two chains, never a false nesting. */
export type PathCrumbKind = NodeKind | 'model' | 'member' | 'divider'

/** `above`: ancestors and open frames (accent). `selected`: the one selected
 *  node. `below`: the crumbs down to the hit leaf (secondary). `member`: the
 *  final, non-clickable definition-member crumb. */
export type PathCrumbState = 'above' | 'selected' | 'below' | 'member'

export interface PathCrumb {
  /** Stable React key. */
  key: string
  label: string
  kind: PathCrumbKind
  state: PathCrumbState
  /** The node a click selects; `null` for `Model` and the member crumb. */
  node: NodeRef | null
  /** Index on the combined open path (session frames, then the object
   *  context) when this crumb is an open frame — it carries the `editing`
   *  chip and a click closes down to it. `null` otherwise. */
  frameDepth: number | null
  /** What a double-click enters: the node itself, the instance for a member
   *  crumb, nothing for `Model`/sketch/drawing crumbs. */
  enterNode: NodeRef | null
}

/** The leaf a viewport click hit, as the select snap names it. */
export interface HitLeaf {
  object?: bigint
  instance?: bigint
}

export interface SelectionPathInput {
  sessionStack: readonly { node: NodeRef; label: string }[]
  activeContext: readonly NodeRef[]
  selected: readonly NodeRef[]
  /** Set only for a selection made by a viewport click. */
  hitLeaf: HitLeaf | null
  /** The containing group of an object/group/instance, if any. Open session
   *  frames erase their direct members' links, so this may stop short. */
  parentOf: (node: NodeRef) => NodeRef | undefined
  /** Kernel truth for the innermost open frame's direct members. */
  sessionMembers: readonly NodeRef[] | null
  labelFor: (node: NodeRef) => string
  /** A definition member's own name, if it has one. */
  memberName: (objectId: bigint) => string | undefined
  /** The object or free sketch a drawn selection lives on, if any. */
  draftOwner: (ref: NodeRef) => NodeRef | null
  /** True for a linked copy: it reads as the Object or Group it is, so its
   *  shared shape is never named as a part of a component. */
  isLinkedCopy?: (instanceId: bigint) => boolean
}

const WALK_LIMIT = 256

/** True for the node kinds that take part in the containment tree. */
function hasTreeParent(kind: NodeKind): boolean {
  return kind === 'object' || kind === 'group' || kind === 'instance'
}

/**
 * The containers above `node`, outermost first (not including `node`).
 * `underFrames` is true when the walk ended at a direct member of the
 * innermost open session frame: the frame erased that member's parent link,
 * so the open frames are the real continuation of the chain — the same case
 * `enterNode` splices the open stack back in for.
 */
export function ancestorsOf(
  node: NodeRef,
  parentOf: (n: NodeRef) => NodeRef | undefined,
  sessionStackLen: number,
  sessionMembers: readonly NodeRef[] | null,
): { ancestors: NodeRef[]; underFrames: boolean } {
  const up: NodeRef[] = []
  let cur: NodeRef = node
  for (let i = 0; i < WALK_LIMIT && hasTreeParent(cur.kind); i++) {
    const p = parentOf(cur)
    if (p === undefined) break
    up.push(p)
    cur = p
  }
  const root = cur
  const underFrames =
    sessionStackLen > 0 && sessionMembers !== null && sessionMembers.some((m) => nodeEq(m, root))
  return { ancestors: up.reverse(), underFrames }
}

/**
 * The full path above `node` from Model, open frames included when the node
 * sits under them. This is what Object Info's "in …" line prints.
 */
export function pathAbove(
  node: NodeRef,
  input: Pick<SelectionPathInput, 'sessionStack' | 'parentOf' | 'sessionMembers'>,
): NodeRef[] {
  const { ancestors, underFrames } = ancestorsOf(
    node, input.parentOf, input.sessionStack.length, input.sessionMembers,
  )
  return underFrames ? [...input.sessionStack.map((f) => f.node), ...ancestors] : ancestors
}

/** Build the strip's crumbs. Empty means "show no strip". */
export function buildSelectionPath(input: SelectionPathInput): PathCrumb[] {
  const { sessionStack, activeContext, selected, labelFor } = input
  const stackLen = sessionStack.length

  const frames: { node: NodeRef; label: string; depth: number }[] = [
    ...sessionStack.map((f, i) => ({ node: f.node, label: f.label, depth: i })),
    ...activeContext.map((n, j) => ({ node: n, label: labelFor(n), depth: stackLen + j })),
  ]
  const frameOf = (n: NodeRef) => frames.find((f) => nodeEq(f.node, n))

  const crumbOf = (node: NodeRef, state: PathCrumbState): PathCrumb => {
    const frame = frameOf(node)
    const enterable = node.kind === 'group' || node.kind === 'instance' || node.kind === 'object'
    return {
      key: nodeKey(node),
      label: frame?.label ?? labelFor(node),
      kind: node.kind,
      state,
      node,
      frameDepth: frame?.depth ?? null,
      enterNode: enterable ? node : null,
    }
  }

  const crumbs: PathCrumb[] = [
    { key: 'model', label: t({ context: 'root', comment: 'Selection path strip: the top of the document', message: 'Model' }), kind: 'model', state: 'above', node: null, frameDepth: null, enterNode: null },
  ]

  let sel: NodeRef | null = null
  let isDraft = false
  let anchor: NodeRef | null = null
  let ancestors: NodeRef[] = []
  let framesKept = frames.length
  let divider = false
  if (selected.length === 1) {
    sel = selected[0]
    isDraft = isDraftKind(sel.kind)
    // A drawn selection hangs off its owner's chain; the drawing itself is
    // the selected crumb. Everything else is its own chain's last link.
    anchor = isDraft ? input.draftOwner(sel) : sel
    if (anchor !== null) {
      const up = ancestorsOf(anchor, input.parentOf, stackLen, input.sessionMembers)
      ancestors = up.ancestors
      if (!up.underFrames) {
        // The chain reaches the open frames only through a frame it names.
        // Frames deeper than the deepest one on the chain are not its
        // ancestors; with none on the chain, the selection is outside them all.
        let deepest = -1
        for (const n of [...ancestors, anchor]) {
          const f = frameOf(n)
          if (f !== undefined) deepest = Math.max(deepest, f.depth)
        }
        divider = deepest < 0 && frames.length > 0
        if (deepest >= 0) framesKept = deepest + 1
      }
    }
  }
  for (const f of frames.slice(0, framesKept)) crumbs.push(crumbOf(f.node, 'above'))
  if (divider) {
    crumbs.push({ key: 'divider', label: '', kind: 'divider', state: 'above', node: null, frameDepth: null, enterNode: null })
  }

  if (sel !== null) {
    if (anchor !== null) {
      for (const a of ancestors) if (frameOf(a) === undefined) crumbs.push(crumbOf(a, 'above'))
      if (isDraft) {
        if (frameOf(anchor) === undefined) crumbs.push(crumbOf(anchor, 'above'))
      } else if (frameOf(anchor) === undefined) {
        crumbs.push(crumbOf(anchor, 'selected'))
      }
    }
    if (frameOf(sel) !== undefined) {
      // The selection is itself an open frame: that crumb carries the wash.
      crumbs.forEach((c) => { if (c.node !== null && nodeEq(c.node, sel)) c.state = 'selected' })
    } else if (isDraft) {
      crumbs.push({
        key: nodeKey(sel), label: labelFor(sel), kind: sel.kind, state: 'selected',
        node: sel, frameDepth: null, enterNode: null,
      })
    }
    if (!isDraft && input.hitLeaf !== null) crumbs.push(...belowCrumbs(input, sel, frameOf, crumbOf))
  }

  // The two-crumb rule: a lone top-level object says nothing Object Info
  // does not. An open session always shows (it replaces the "Editing X" pill).
  if (stackLen === 0 && crumbs.filter((c) => c.kind !== 'divider').length - 1 < 2) return []
  return crumbs
}

/** Display name for a part of a closed component that has no name of its own. */
function partFallback(): string {
  return t({ context: 'unnamed item', comment: 'Name shown for a part inside a component when it has no name', message: 'Part' })
}

function belowCrumbs(
  input: SelectionPathInput,
  sel: NodeRef,
  frameOf: (n: NodeRef) => unknown,
  crumbOf: (n: NodeRef, s: PathCrumbState) => PathCrumb,
): PathCrumb[] {
  const hit = input.hitLeaf
  if (hit === null) return []
  const leaf: NodeRef | null =
    hit.instance !== undefined
      ? { kind: 'instance', id: hit.instance }
      : hit.object !== undefined
        ? { kind: 'object', id: hit.object }
        : null
  if (leaf === null) return []

  // Leaf-to-root chain of the hit, then the stretch under the selection.
  const chain: NodeRef[] = [leaf]
  for (let i = 0; i < WALK_LIMIT; i++) {
    const last = chain[chain.length - 1]
    if (!hasTreeParent(last.kind)) break
    const p = input.parentOf(last)
    if (p === undefined) break
    chain.push(p)
  }
  const at = chain.findIndex((n) => nodeEq(n, sel))
  if (at < 0) return [] // stale hit: the selection moved on without us hearing
  const out = chain.slice(0, at).reverse().map((n) => crumbOf(n, 'below'))

  if (hit.instance !== undefined && hit.object !== undefined) {
    const instanceNode: NodeRef = { kind: 'instance', id: hit.instance }
    if (input.isLinkedCopy?.(hit.instance) === true) {
      // A linked copy is the leaf: nothing below it is named.
    } else if (frameOf(instanceNode) === undefined) {
      // The component is not open: the part is a definition member.
      out.push({
        key: `member:${hit.object}`,
        label: input.memberName(hit.object) ?? partFallback(),
        kind: 'member',
        state: 'member',
        node: null,
        frameDepth: null,
        enterNode: instanceNode,
      })
    } else if (!(sel.kind === 'object' && sel.id === hit.object)) {
      out.push(crumbOf({ kind: 'object', id: hit.object }, 'below'))
    }
  }
  return out
}

/**
 * The hover tip's second line: the hit label, then `· in <direct parent>`.
 * A definition member (the snap names an instance) reads
 * `<member> · in <instance label>`; when that instance is open the part
 * reads by its own label. An instance with no object (a line drawn inside
 * a component) reads as the instance. `null` when the hit names nothing.
 */
export function hoverLine(
  hit: HitLeaf,
  parentOf: (n: NodeRef) => NodeRef | undefined,
  labelFor: (n: NodeRef) => string,
  memberName: (objectId: bigint) => string | undefined,
  isOpenInstance: (instanceId: bigint) => boolean = () => false,
  isLinkedCopy: (instanceId: bigint) => boolean = () => false,
): string | null {
  // A linked copy reads as the Object or Group it is: the hit is the copy.
  if (hit.instance !== undefined && isLinkedCopy(hit.instance) && !isOpenInstance(hit.instance)) {
    hit = { instance: hit.instance }
  }
  if (hit.instance !== undefined && hit.object !== undefined) {
    const inst: NodeRef = { kind: 'instance', id: hit.instance }
    // Inside an open component the part is an ordinary object: name it by its
    // own label, not the "Part" fallback of a closed definition member.
    if (isOpenInstance(hit.instance)) {
      const partLabel = labelFor({ kind: 'object', id: hit.object })
      const parentLabel = labelFor(inst)
      return t({ comment: 'Hover tip: a thing and the container it sits in', message: `${partLabel} · in ${parentLabel}` })
    }
    const partLabel = memberName(hit.object) ?? partFallback()
    const parentLabel = labelFor(inst)
    return t({ comment: 'Hover tip: a thing and the container it sits in', message: `${partLabel} · in ${parentLabel}` })
  }
  // An instance alone: a line drawn inside a component is under the cursor,
  // and the component itself is what a click on it selects.
  const leaf: NodeRef | null = hit.object !== undefined
    ? { kind: 'object', id: hit.object }
    : hit.instance !== undefined ? { kind: 'instance', id: hit.instance } : null
  if (leaf === null) return null
  const parent = parentOf(leaf)
  const name = labelFor(leaf)
  if (parent === undefined) return name
  const parentLabel = labelFor(parent)
  return t({ comment: 'Hover tip: a thing and the container it sits in', message: `${name} · in ${parentLabel}` })
}

/**
 * What a click on an open-frame crumb does. `depth` is the crumb's index on
 * the combined open path; the innermost frame has nothing to close, so it is
 * a no-op (it must not fall through to a truncate that clears the selection).
 */
export function frameClickAction(depth: number, stackLen: number, ctxLen: number): 'noop' | 'close-to' {
  return depth + 1 === stackLen + ctxLen ? 'noop' : 'close-to'
}
