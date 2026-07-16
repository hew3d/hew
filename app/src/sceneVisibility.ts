/**
 * sceneVisibility.ts — visible-emptiness queries for the first-sketch
 * auto-zoom edge (App.handleDocumentChanged).
 *
 * "Empty" for framing purposes means VISIBLY empty: the kernel's entity
 * counts minus session-hidden and tag-hidden content. A document whose only
 * solid is hidden renders a blank viewport, and the first rectangle drawn
 * into that blank view must reframe exactly like one drawn into a truly
 * empty document — otherwise the tiny shape sits unframed at the default
 * meter-scale distance, which is precisely the failure the auto-zoom
 * feature was built to prevent.
 *
 * Pure and UI-free: the scene is consumed through a minimal structural view
 * (satisfied by the wasm `Scene`), and hidden state is the leaf id union the
 * app already derives in `pushUnionHidden`. Sketches cannot be hidden, so
 * they always count as visible content.
 */

/** Leaf object/instance ids currently hidden (eye hides ∪ tag hides) — the
 *  same union `App.pushUnionHidden` pushes to the renderer and kernel. */
export interface HiddenLeafIds {
  objects: ReadonlySet<bigint>
  instances: ReadonlySet<bigint>
}

/** The slice of the wasm `Scene` these queries need. `group_members` returns
 *  anything with `kind`/`id` properties (the wasm `NodeJs` getters qualify). */
export interface VisibilitySceneView {
  object_ids(): ArrayLike<bigint>
  group_ids(): ArrayLike<bigint>
  instance_ids(): ArrayLike<bigint>
  sketch_ids(): ArrayLike<bigint>
  group_members(group: bigint): ArrayLike<{ kind: string; id: bigint }>
}

/** True when any solid content (object or instance, top-level or nested in a
 *  group) is visible after subtracting the hidden leaf sets. */
function hasVisibleSolidContent(scene: VisibilitySceneView, hidden: HiddenLeafIds): boolean {
  const objects = scene.object_ids()
  for (let i = 0; i < objects.length; i++) {
    if (!hidden.objects.has(objects[i])) return true
  }
  const instances = scene.instance_ids()
  for (let i = 0; i < instances.length; i++) {
    if (!hidden.instances.has(instances[i])) return true
  }
  // Grouped leaves are not in object_ids() (world objects only), so groups
  // are walked through their members; group_ids() may list nested groups
  // too, which just re-checks a subtree and stays correct.
  const groups = scene.group_ids()
  for (let i = 0; i < groups.length; i++) {
    if (groupHasVisibleLeaf(scene, groups[i], hidden)) return true
  }
  return false
}

function groupHasVisibleLeaf(
  scene: VisibilitySceneView,
  group: bigint,
  hidden: HiddenLeafIds,
): boolean {
  const members = scene.group_members(group)
  for (let i = 0; i < members.length; i++) {
    const m = members[i]
    if (m.kind === 'object' && !hidden.objects.has(m.id)) return true
    if (m.kind === 'instance' && !hidden.instances.has(m.id)) return true
    if (m.kind === 'group' && groupHasVisibleLeaf(scene, m.id, hidden)) return true
  }
  return false
}

/** True when the viewport shows nothing: no visible solid content and no
 *  sketches. The pre-state of the first-sketch auto-zoom edge. */
export function isSceneVisiblyEmpty(
  scene: VisibilitySceneView,
  hidden: HiddenLeafIds,
): boolean {
  return scene.sketch_ids().length === 0 && !hasVisibleSolidContent(scene, hidden)
}

/** True when EXACTLY one sketch — and nothing else — is visible: the
 *  post-state that, together with a visibly-empty pre-state, triggers the
 *  first-sketch reframe. */
export function isLoneVisibleSketchScene(
  scene: VisibilitySceneView,
  hidden: HiddenLeafIds,
): boolean {
  return scene.sketch_ids().length === 1 && !hasVisibleSolidContent(scene, hidden)
}
