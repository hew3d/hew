/**
 * Localized undo/redo labels from the kernel's structured history kinds
 * (docs/LOCALIZATION.md). `history_entries_json` gives each entry a `kind`:
 * `{ id, args }`, where `id` is a stable kebab-case name and `args` holds
 * the names and counts the label needs (nested kinds for `repeat`/
 * `bundle`). The kernel renders its own English `label` from the same
 * data (`render_history_label` in crates/kernel/src/document.rs); this
 * table is that renderer's localized twin, one message per id.
 *
 * A kind this table does not know (a newer kernel than app, or a typo)
 * falls back to the kernel's English `label`, never to a blank or an id.
 */

import { i18n, type MessageDescriptor } from '@lingui/core'
import { msg, plural, t } from '@lingui/core/macro'

export interface HistoryKind {
  id: string
  args: Record<string, string | number | HistoryKind>
}

export function isHistoryKind(v: unknown): v is HistoryKind {
  return (
    typeof v === 'object' &&
    v !== null &&
    typeof (v as { id?: unknown }).id === 'string' &&
    typeof (v as { args?: unknown }).args === 'object' &&
    (v as { args?: unknown }).args !== null
  )
}

const text = (k: HistoryKind, key: string): string => {
  const v = k.args[key]
  return typeof v === 'string' ? v : ''
}
const count = (k: HistoryKind, key: string): number => {
  const v = k.args[key]
  return typeof v === 'number' ? v : 0
}
const inner = (k: HistoryKind, key: string): HistoryKind | null => {
  const v = k.args[key]
  return isHistoryKind(v) ? v : null
}

/** Fixed-text kinds. */
const SIMPLE: Record<string, MessageDescriptor> = {
  edit: msg`Edit`,
  'text-3d': msg`3D Text`,
  'follow-me': msg`Follow Me`,
  'push-pull': msg`Push/Pull`,
  'draw-on-face': msg`Draw on face`,
  'delete-imprint': msg`Delete imprint`,
  'move-imprint': msg`Move imprint`,
  'edit-face': msg`Edit face`,
  union: msg`Union`,
  subtract: msg`Subtract`,
  intersect: msg`Intersect`,
  slice: msg`Slice`,
  'push-through': msg`Push/Pull through`,
  'add-thickness': msg`Add thickness`,
  'detach-sketch': msg`Move sketch off its plane`,
  'copy-sketch': msg`Copy sketch`,
  'attach-sketch': msg`Attach sketch`,
  'group-sketch': msg`Group sketch`,
  'move-axes': msg`Move drawing axes`,
  'move-sketch-point': msg`Move sketch point`,
  draw: msg`Draw`,
  group: msg`Group`,
  ungroup: msg`Ungroup`,
  'place-instance': msg`Place instance`,
  copy: msg`Copy`,
  guide: msg`Guide`,
  'delete-guide': msg`Delete guide`,
  'move-guide': msg`Move guide`,
  'delete-sketch': msg`Delete sketch`,
  explode: msg`Explode`,
  'make-unique': msg`Make unique`,
  'link-identical-copies': msg`Link identical copies`,
  'edit-group': msg`Edit group`,
  'finish-editing-group': msg`Finish editing group`,
  'paint-face': msg`Paint face`,
  'paint-object': msg`Paint object`,
  'paint-instance': msg`Paint instance`,
  'position-texture': msg`Position texture`,
  'material-opacity': msg`Change material opacity`,
  'rename-material': msg`Rename material`,
  'delete-material': msg`Delete material`,
  'delete-component': msg`Delete component`,
  'move-into-group': msg`Move into group`,
  'move-out-of-group': msg`Move out of group`,
  tag: msg`Tag`,
  untag: msg`Untag`,
  'change-tags': msg`Change tags`,
  'rename-tag': msg`Rename tag`,
  'delete-component-member': msg`Delete component member`,
}

/** "{verb} {count} {noun}" — the transform family's verb and noun words. */
function verbWord(verb: string): string {
  switch (verb) {
    case 'move':
      return t({ context: 'undo verb', message: 'Move' })
    case 'rotate':
      return t({ context: 'undo verb', message: 'Rotate' })
    case 'scale':
      return t({ context: 'undo verb', message: 'Scale' })
    default:
      return t({ context: 'undo verb', message: 'Transform' })
  }
}

function annotationWord(what: string): string {
  switch (what) {
    case 'dimension':
      return t`Dimension`
    case 'text':
      return t({ context: 'annotation', message: 'Text' })
    default:
      return t`Annotation`
  }
}

/** `'Walls'` — a user-typed name quoted for a label. Built outside the
 * message because ICU MessageFormat reads an apostrophe next to a brace as
 * an escape, so `'{name}'` inside a `t` call would print the braces. */
function quoted(name: string): string {
  return `'${name}'`
}

function withName(base: string, name: string | undefined): string {
  return name === undefined ? base : `${base} ${quoted(name)}`
}

/**
 * The localized label for `kind`, or `null` for an id this table does not
 * know (the caller shows the kernel's English `label` instead).
 */
export function historyKindLabel(k: HistoryKind): string | null {
  const simple = SIMPLE[k.id]
  if (simple !== undefined) return i18n._(simple)
  const name = typeof k.args.name === 'string' ? k.args.name : undefined
  switch (k.id) {
    case 'labeled':
      return text(k, 'label')
    case 'repeat': {
      const first = inner(k, 'inner')
      if (first === null) return null
      const what = historyLabel(first, '')
      const n = count(k, 'count')
      return t`${what} ×${n}`
    }
    case 'bundle': {
      const first = inner(k, 'inner')
      if (first === null) return null
      const what = historyLabel(first, '')
      const more = count(k, 'more')
      return t`${what} (+${more} more)`
    }
    case 'set-attribute': {
      const namespace = text(k, 'namespace')
      const key = text(k, 'key')
      return t`Set attribute ${namespace}.${key}`
    }
    case 'delete-attribute': {
      const namespace = text(k, 'namespace')
      return t`Delete attribute ${namespace}`
    }
    case 'close-openings': {
      const n = count(k, 'count')
      return plural(n, { one: 'Close an opening', other: 'Close # openings' })
    }
    case 'delete-faces': {
      const n = count(k, 'count')
      return plural(n, { one: 'Delete # face', other: 'Delete # faces' })
    }
    case 'transform': {
      const verb = verbWord(text(k, 'verb'))
      const n = count(k, 'count')
      switch (text(k, 'noun')) {
        case 'sketch':
          return plural(n, { one: `${verb} # sketch`, other: `${verb} # sketches` })
        case 'instance':
          return plural(n, { one: `${verb} # instance`, other: `${verb} # instances` })
        case 'item':
          return plural(n, { one: `${verb} # item`, other: `${verb} # items` })
        default:
          return plural(n, { one: `${verb} # object`, other: `${verb} # objects` })
      }
    }
    case 'transform-sketch': {
      const verb = verbWord(text(k, 'verb'))
      return t`${verb} sketch`
    }
    case 'transform-instance': {
      const verb = verbWord(text(k, 'verb'))
      return t`${verb} instance`
    }
    case 'rescale': {
      const factor = text(k, 'factor')
      return t`Resize model ×${factor}`
    }
    case 'delete-node': {
      const node = text(k, 'node')
      if (name !== undefined) {
        const q = quoted(name)
        return node === 'group' ? t`Delete group ${q}` : t`Delete ${q}`
      }
      switch (node) {
        case 'group':
          return t`Delete group`
        case 'instance':
          return t`Delete instance`
        default:
          return t`Delete object`
      }
    }
    case 'make-component':
      return withName(t`Make component`, name)
    case 'change-component-axes':
      return withName(t`Change component axes`, name)
    case 'copy-array': {
      const n = count(k, 'count')
      return t`Copy ×${n}`
    }
    case 'delete-guides': {
      const n = count(k, 'count')
      return plural(n, { one: 'Delete # guide', other: 'Delete # guides' })
    }
    case 'copy-guides': {
      const n = count(k, 'count')
      return plural(n, { one: 'Copy guide', other: 'Copy guide ×#' })
    }
    case 'add-annotation':
      return annotationWord(text(k, 'what'))
    case 'edit-annotation': {
      const what = annotationWord(text(k, 'what')).toLocaleLowerCase()
      return t`Edit ${what}`
    }
    case 'delete-annotation': {
      const what = annotationWord(text(k, 'what')).toLocaleLowerCase()
      return t`Delete ${what}`
    }
    case 'edit-component':
      return withName(t`Edit component`, name)
    case 'finish-editing-component':
      return withName(t`Finish editing component`, name)
    case 'replace-material': {
      const n = count(k, 'count')
      return plural(n, { one: 'Replace material on # face', other: 'Replace material on # faces' })
    }
    case 'rename': {
      const from = typeof k.args.from === 'string' ? k.args.from : undefined
      const to = typeof k.args.to === 'string' ? k.args.to : undefined
      if (from !== undefined && to !== undefined) {
        const a = quoted(from)
        const b = quoted(to)
        return t`Rename ${a} → ${b}`
      }
      if (to !== undefined) {
        const b = quoted(to)
        return t`Rename to ${b}`
      }
      return t`Rename`
    }
    case 'clear-name': {
      const from = quoted(text(k, 'from'))
      return t`Clear name ${from}`
    }
    case 'rename-component': {
      const from = typeof k.args.from === 'string' ? k.args.from : undefined
      const to = typeof k.args.to === 'string' ? k.args.to : undefined
      if (from !== undefined && to !== undefined) {
        const a = quoted(from)
        const b = quoted(to)
        return t`Rename component ${a} → ${b}`
      }
      if (to !== undefined) {
        const b = quoted(to)
        return t`Rename component to ${b}`
      }
      return t`Rename component`
    }
    case 'delete-tags': {
      const n = count(k, 'count')
      return plural(n, { one: 'Delete # tag', other: 'Delete # tags' })
    }
    case 'import': {
      const n = count(k, 'count')
      return plural(n, { one: 'Import # item', other: 'Import # items' })
    }
    default:
      return null
  }
}

/** The label to show for a history entry: the localized rendering of its
 * kind, else the kernel's English `label`. */
export function historyLabel(kind: HistoryKind | undefined | null, label: string): string {
  if (kind === undefined || kind === null) return label
  return historyKindLabel(kind) ?? label
}
