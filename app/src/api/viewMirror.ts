/**
 * The app's view state as the bus reads it (docs/agents/HEW_API.md §7,
 * "Selection and view state"): the JSON `Scene.set_ui_selection` and
 * `Scene.set_ui_isolation` take. The bus runs inside wasm with no DOM, so
 * the app mirrors its selection and the viewport's isolation into the
 * `Scene` whenever they change; `hew.selection.get`, `selection_changed`,
 * and `hew.view.hidden`'s `isolated` read the mirror.
 *
 * Kernel handles travel as decimal strings, so a `bigint` past 2^53 keeps
 * every bit. The drafting kinds have no public id and are left out: a
 * drafting selection reads as empty on the bus.
 */

import { isDraftKind, isNodeItem, type NodeRef, type SelItem } from '../panels/treeModel'
import type { IsolationTarget } from '../viewport/isolation'

/** The selection mirror: the app's one ordered selection, in pick order
 * (the primary first), nodes, sketches, guides and annotations mixed. A
 * drawn unit on a plane has no kernel handle the bus can name and is left
 * out. */
export function uiSelectionJson(selection: readonly SelItem[]): string {
  const entries: { kind: string; id: string }[] = []
  for (const s of selection) {
    if (isNodeItem(s) && isDraftKind(s.kind)) continue
    entries.push({ kind: s.kind, id: s.id.toString() })
  }
  return JSON.stringify(entries)
}

/** The isolation mirror: the viewport's targets, or `null` when nothing is
 * isolated. */
export function uiIsolationJson(targets: readonly IsolationTarget[] | null): string {
  if (targets === null) return 'null'
  const entries: Record<string, string>[] = []
  for (const t of targets) {
    if (t.kind === 'member') {
      entries.push({ kind: 'member', object: t.object.toString(), instance: t.instance.toString() })
    } else if (t.kind === 'object' || t.kind === 'group' || t.kind === 'instance') {
      entries.push({ kind: t.kind, id: t.id.toString() })
    }
  }
  return JSON.stringify(entries)
}

/** The in-context mirror (`Scene.set_ui_context`): the entries the app keeps
 * past the kernel's session stack, outermost first. The fallback holds only
 * objects and instances (a group always opens a session), so any other kind
 * is left out. */
export function uiContextJson(entries: readonly NodeRef[]): string {
  const out: { kind: string; id: string }[] = []
  for (const n of entries) {
    if (n.kind === 'object' || n.kind === 'instance') out.push({ kind: n.kind, id: n.id.toString() })
  }
  return JSON.stringify(out)
}
