/**
 * documentLoad — the minimal, UI-free slice of App.tsx's `.hew`-bytes →
 * scene-apply path that a second shell (Shop Mode, `shop/ShopApp.tsx`) also
 * needs: parsing loaded bytes into a live `Scene`, recognizing a kernel
 * panic, reconstructing the session-only hidden-node/hidden-tag sets a
 * freshly loaded document's registries carry (an imported `.skp`'s hidden
 * layers, or a re-opened `.hew` with nodes previously eye-toggled), and
 * (`unionHiddenLeafIds`) walking those sets out to the concrete leaf
 * object/instance ids a renderer actually hides — the same union
 * App.tsx's `pushUnionHidden` computes, factored out so Shop Mode's
 * boot-time seed reproduces it exactly rather than approximating it.
 *
 * Deliberately NOT the whole of App.tsx's `applyLoadedBytes`: that function
 * also resets app-shell-only state (the welcome screen, `docSession`'s
 * dirty/currentRef bookkeeping, the autosave-suppression ref, the recovery
 * snapshot) that has no Shop Mode equivalent — Shop Mode never dirties a
 * document (docs/dev/DEVELOPMENT.md's kernel-mutation rules don't apply to a
 * shell that issues none) and tracks no save state at all. Pulling only
 * this much out — rather than the whole function — keeps App.tsx's own
 * behavior untouched (it still owns every one of those app-shell resets;
 * this module just supplies the piece both shells share) instead of forcing
 * a shared shape neither shell fully wants.
 */

import type { Scene } from '../wasm/loader'
import { collectLeafIds, nodeKey, nodeKindToNumber, nodeRefFromJs, type NodeRef } from '../panels/treeModel'
import { isPathUnder, tagPathKey } from '../panels/tagModel'

/** Strings that signal the Scene borrow-lock after a Rust panic (mirrors
 *  the check `loader.ts`'s global reproducer-dump hook uses). */
const PANIC_SIGNATURES = ['recursive use of an object', 'unreachable']

/** True when `message` (an error's `.message`, lower-cased by the caller or
 *  not — this lower-cases itself) reads like the Scene borrow-lock left
 *  behind by a Rust panic, rather than an ordinary typed refusal. */
export function isPanicError(message: string): boolean {
  const lower = message.toLowerCase()
  return PANIC_SIGNATURES.some((sig) => lower.includes(sig))
}

/** Outcome of `loadHewBytes`. */
export type LoadHewBytesResult =
  | { ok: true }
  | {
      ok: false
      /** The raw error/exception `scene.load` threw — pass to
       *  `friendlyErrorText`/`kernelErrorMessage` (kernelErrors.ts) to
       *  render it; kept raw rather than pre-formatted so callers stay free
       *  to format it however their chrome does. */
      error: unknown
      /** True when `error` looks like a kernel panic (`isPanicError`) —
       *  callers that surface a "kernel panicked" banner should latch it
       *  from this rather than re-deriving it. */
      panicked: boolean
    }

/**
 * Parse `bytes` as a `.hew` document into `scene`, in place — the one
 * fallible step at the center of every "open a file" flow (File ▸ Open,
 * File ▸ New's blank bytes, crash recovery, a QR/LAN handoff open, …).
 * Never throws; a parse failure comes back as `{ ok: false, error,
 * panicked }` instead, so a caller can toast it and leave the previous
 * document (and its own state) untouched, exactly like App.tsx's
 * `applyLoadedBytes` already did inline.
 */
export function loadHewBytes(scene: Scene, bytes: Uint8Array): LoadHewBytesResult {
  try {
    scene.load(bytes)
    return { ok: true }
  } catch (error) {
    const raw = error instanceof Error ? error.message : String(error)
    return { ok: false, error, panicked: isPanicError(raw) }
  }
}

/** True when the document holds no entities at all (a pristine "Untitled").
 *  Used by File ▸ New (blank documents are reused instead of spawning a
 *  second empty window) and by the open-time Zoom Extents (nothing to
 *  frame) — both App.tsx and Shop Mode's document-open path need the
 *  latter. */
export function isSceneEmpty(scene: Scene): boolean {
  return (
    scene.object_ids().length === 0 &&
    scene.group_ids().length === 0 &&
    scene.instance_ids().length === 0 &&
    scene.sketch_ids().length === 0
  )
}

/**
 * Build the set of tag-path keys marked hidden-by-default in the document's
 * tag metadata registry (`scene.tag_meta_paths()`/`tag_meta_hidden()`) —
 * this covers tags no node carries (e.g. an imported `.skp` layer list,
 * empty layers included), so a hidden empty layer still comes up hidden.
 * Pure function of `scene`; a freshly loaded document's registries are the
 * only state it reads.
 */
export function seedHiddenTagPathsFromRegistry(scene: Scene): Set<string> {
  const paths = scene.tag_meta_paths()
  const hidden = scene.tag_meta_hidden()
  const seeded = new Set<string>()
  for (let i = 0; i < paths.length; i++) {
    if (hidden[i] !== 1) continue
    const path = paths[i].split('/').map((s) => s.trim()).filter((s) => s.length > 0)
    if (path.length > 0) seeded.add(tagPathKey(path))
  }
  return seeded
}

/**
 * Build the set of hidden-node keys (`nodeKey` strings) from the document's
 * persisted USER-hidden registry (`scene.user_hidden_kinds()`/
 * `user_hidden_ids()`, manifest v6) — this is how imported `.skp` hidden
 * groups/components/instances (and a re-opened `.hew` with nodes previously
 * eye-toggled) arrive, since the hidden-node set itself is session-only
 * view state that gets reset on every load.
 */
export function seedHiddenKeysFromRegistry(scene: Scene): Set<string> {
  const kinds = scene.user_hidden_kinds()
  const ids = scene.user_hidden_ids()
  const kindNames: NodeRef['kind'][] = ['object', 'group', 'instance']
  const seeded = new Set<string>()
  for (let i = 0; i < kinds.length; i++) {
    const kind = kindNames[kinds[i]]
    if (kind === undefined) continue
    seeded.add(nodeKey({ kind, id: ids[i] }))
  }
  return seeded
}

/**
 * Resolve the full renderer-hidden leaf sets (deduplicated object/instance
 * ids) implied by a hidden-node-key set plus a hidden-tag-path set — the
 * union walk `App.tsx`'s `pushUnionHidden` runs on every hide-toggle, and
 * (this refactor's reason for existing here) the SAME walk a shell's
 * boot-time seed needs to reproduce the editor's exact initial hidden set
 * from `seedHiddenKeysFromRegistry`/`seedHiddenTagPathsFromRegistry`'s
 * output, without duplicating the walk per shell.
 *
 * Pure function of `scene` plus the two hidden-key sets: no renderer/kernel
 * side effects. `hiddenKeys` entries are `nodeKey` strings for the
 * manually-hidden nodes (always `object`/`group`/`instance` — the only
 * kinds a hide toggle ever targets, so the plain `kind:id` split below never
 * meets a sketch-scoped `kind:sketch:id` key); `hiddenTagPaths` entries are
 * `tagPathKey`-encoded path arrays. A hidden GROUP's/tag's coverage expands
 * through `collectLeafIds`/`isPathUnder` exactly as the live editor's own
 * eye toggles do. The caller decides what to do with the result — App.tsx
 * additionally pushes it into the kernel's inference-hide state via
 * `scene.set_hidden` (so snap/pick skip hidden geometry), a real kernel call
 * Shop Mode's view-state-only posture never makes; Shop Mode instead feeds
 * the result straight to `ViewportApi.setHidden`.
 */
export function unionHiddenLeafIds(
  scene: Scene,
  hiddenKeys: ReadonlySet<string>,
  hiddenTagPaths: ReadonlySet<string>,
): { objectIds: bigint[]; instanceIds: bigint[] } {
  const getGroupMembers = (groupId: bigint): NodeRef[] =>
    scene.group_members(groupId).map((m) => nodeRefFromJs(m as { kind: string; id: bigint }))

  const hiddenObjectIds: bigint[] = []
  const hiddenInstanceIds: bigint[] = []

  // --- (a) manual per-node hides ---
  for (const k of hiddenKeys) {
    const colonIdx = k.indexOf(':')
    const kind = k.slice(0, colonIdx) as NodeRef['kind']
    const id = BigInt(k.slice(colonIdx + 1))
    const { objectIds, instanceIds } = collectLeafIds({ kind, id }, getGroupMembers)
    hiddenObjectIds.push(...objectIds)
    hiddenInstanceIds.push(...instanceIds)
  }

  // --- (b) tag-path hides ---
  if (hiddenTagPaths.size > 0) {
    // Build the current tag list from the scene using first-class tag data.
    const allNodes = [
      ...Array.from(scene.object_ids()).map((id) => ({ kind: 'object' as const, id })),
      ...Array.from(scene.group_ids()).map((id) => ({ kind: 'group' as const, id })),
      ...Array.from(scene.instance_ids()).map((id) => ({ kind: 'instance' as const, id })),
    ]
    const tagged: { node: NodeRef; path: string[] }[] = []
    for (const raw of allNodes) {
      const node: NodeRef = raw as NodeRef
      const kindNum = nodeKindToNumber(node.kind)
      const rawTags = kindNum >= 0 ? scene.node_tags(kindNum, node.id) : []
      for (const rawTag of rawTags) {
        const path = rawTag.split('/').map((s) => s.trim()).filter((s) => s.length > 0)
        if (path.length > 0) {
          tagged.push({ node, path })
        }
      }
    }

    // For each hidden tag path, collect all nodes whose tag path is at or
    // under it — one pass over all tagged nodes.
    const hiddenAnchorPaths: string[][] = []
    for (const key of hiddenTagPaths) {
      try {
        const parsed = JSON.parse(key)
        if (Array.isArray(parsed)) hiddenAnchorPaths.push(parsed as string[])
      } catch { /* invalid key — skip */ }
    }

    // A node is covered if its tag path is at or under any hidden anchor path.
    for (const { node, path } of tagged) {
      const covered = hiddenAnchorPaths.some((anchor) => isPathUnder(path, anchor))
      if (!covered) continue
      const { objectIds, instanceIds } = collectLeafIds(node, getGroupMembers)
      hiddenObjectIds.push(...objectIds)
      hiddenInstanceIds.push(...instanceIds)
    }
  }

  // Deduplicate (a leaf may be covered by multiple hidden paths/tags).
  return {
    objectIds: [...new Set(hiddenObjectIds)],
    instanceIds: [...new Set(hiddenInstanceIds)],
  }
}
