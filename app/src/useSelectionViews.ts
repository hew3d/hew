import { useRef } from 'react'
import { selectionAnnotations, selectionGuides, selectionNodes, type NodeRef, type SelItem } from './panels/treeModel'

/** The per-kind views of the app's one ordered selection (`SelItem`). */
export interface SelectionViews {
  /** The selected nodes, in pick order. */
  nodes: NodeRef[]
  /** The selected construction guides, in pick order. */
  guides: bigint[]
  /** The selected annotations, in pick order. */
  annotations: bigint[]
}

/** Keep `prev` while `next` holds the same entries, so a view's identity
 *  only changes when its own slice of the selection does. */
function stable<T>(prev: T[], next: T[]): T[] {
  if (prev.length === next.length && prev.every((v, i) => v === next[i])) return prev
  return next
}

/**
 * Derive the per-kind views of `selection`. Each view keeps its identity
 * while its own slice is unchanged: adding a guide to the selection leaves
 * `nodes` the same array, so effects and memos that read only the nodes do
 * not run again.
 */
export function useSelectionViews(selection: readonly SelItem[]): SelectionViews {
  const ref = useRef<SelectionViews>({ nodes: [], guides: [], annotations: [] })
  const prev = ref.current
  const nodes = stable(prev.nodes, selectionNodes(selection))
  const guides = stable(prev.guides, selectionGuides(selection))
  const annotations = stable(prev.annotations, selectionAnnotations(selection))
  if (nodes !== prev.nodes || guides !== prev.guides || annotations !== prev.annotations) {
    ref.current = { nodes, guides, annotations }
  }
  return ref.current
}
