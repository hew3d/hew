import { plural, t } from '@lingui/core/macro'
import { selNoun } from '../panels/selectionCounts'
import type { SelItem } from '../panels/treeModel'

/**
 * The Undo step's name for a Delete of `items`, in the user's language. One
 * kind is counted by its own noun ("Delete 2 objects", "Delete 3 guides");
 * a mix of kinds by items ("Delete 4 items"), the rule the transform labels
 * follow. A single entry keeps the plain "Delete": a lone node, guide or
 * annotation is named by the kernel's own step, and only a drawn unit or a
 * sketch, whose delete needs a bracket, reaches here alone.
 */
export function deleteStepLabel(items: readonly SelItem[]): string {
  const n = items.length
  if (n === 1) return t`Delete`
  const nouns = new Set(items.map(selNoun))
  const noun = nouns.size === 1 ? [...nouns][0] : 'item'
  switch (noun) {
    case 'object':
      return plural(n, { one: 'Delete # object', other: 'Delete # objects' })
    case 'group':
      return plural(n, { one: 'Delete # group', other: 'Delete # groups' })
    case 'instance':
      return plural(n, { one: 'Delete # instance', other: 'Delete # instances' })
    case 'sketch':
      return plural(n, { one: 'Delete # sketch', other: 'Delete # sketches' })
    case 'guide':
      return plural(n, { one: 'Delete # guide', other: 'Delete # guides' })
    case 'annotation':
      return plural(n, { one: 'Delete # annotation', other: 'Delete # annotations' })
    default:
      return plural(n, { one: 'Delete # item', other: 'Delete # items' })
  }
}
