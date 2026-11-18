import { plural } from '@lingui/core/macro'
import { isNodeItem, type SelItem } from './treeModel'

/** The noun a selection entry is counted by. A drawn unit on a plane counts
 *  as a sketch, as the transform labels count it. */
export type SelNoun = 'object' | 'group' | 'instance' | 'sketch' | 'guide' | 'annotation'

/** The order a per-kind count lists its nouns in. */
const NOUN_ORDER: readonly SelNoun[] = ['object', 'group', 'instance', 'sketch', 'guide', 'annotation']

export function selNoun(s: SelItem): SelNoun {
  if (!isNodeItem(s)) return s.kind
  switch (s.kind) {
    case 'object':
    case 'group':
    case 'instance':
      return s.kind
    default:
      return 'sketch'
  }
}

/** "# objects" and so on, in the user's language. */
function nounCount(noun: SelNoun, n: number): string {
  switch (noun) {
    case 'object':
      return plural(n, { one: '# object', other: '# objects' })
    case 'group':
      return plural(n, { one: '# group', other: '# groups' })
    case 'instance':
      return plural(n, { one: '# instance', other: '# instances' })
    case 'sketch':
      return plural(n, { one: '# sketch', other: '# sketches' })
    case 'guide':
      return plural(n, { one: '# guide', other: '# guides' })
    case 'annotation':
      return plural(n, { one: '# annotation', other: '# annotations' })
  }
}

/** A selection counted per kind, in a fixed order: "2 objects · 3 guides". */
export function selectionCountLine(items: readonly SelItem[]): string {
  const counts = new Map<SelNoun, number>()
  for (const s of items) {
    const noun = selNoun(s)
    counts.set(noun, (counts.get(noun) ?? 0) + 1)
  }
  return NOUN_ORDER.filter((noun) => counts.has(noun))
    .map((noun) => nounCount(noun, counts.get(noun)!))
    .join(' · ')
}
