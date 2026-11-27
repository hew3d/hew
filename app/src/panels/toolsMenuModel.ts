/**
 * toolsMenuModel — the Tools and Draw menus as data.
 *
 * The web MenuBar renders these lists; it holds no per-tool JSX for them.
 * Order, separators, and submenus here ARE the menu's order (the native
 * menu in the Tauri shell mirrors it; `nativeMenuParity.test.ts` pins the
 * native side). Every entry that runs a command names its command id
 * (`menuEntryActionId`), so a menu can be pruned by command id in one call
 * (`pruneMenuEntries`).
 */

import { msg } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'
import { actionOffered } from '../curveActions'
import { ALL_TOOL_SPECS, type ToolName } from '../tools/toolRegistry'

export type MenuEntry =
  /** A check item that selects a registry tool and checks while it is active. */
  | { kind: 'tool'; tool: ToolName; /** Show the tool's keymap binding (default true). */ showKey?: boolean }
  /** A plain item that opens a tool by a name the registry does not have
   *  (Chamfer opens Fillet in its bevel mode). */
  | { kind: 'alias'; actionId: string; selectName: string; label: MessageDescriptor }
  /** One of Deform's modes: checks while Deform runs in that mode. */
  | { kind: 'deformMode'; mode: string; actionId: string; label: MessageDescriptor }
  | { kind: 'submenu'; id: string; label: MessageDescriptor; items: readonly MenuEntry[] }
  | { kind: 'separator' }

const SEP: MenuEntry = { kind: 'separator' }
const tool = (name: ToolName, showKey = true): MenuEntry => ({ kind: 'tool', tool: name, showKey })

/* oxlint-disable lingui/no-unlocalized-strings -- tool IDs, mode IDs, and submenu IDs are identifiers; every label is a msg descriptor */
/** Draw ▸ strokes (open paths), then closed shapes. (3D Text… follows in the
 *  MenuBar: it opens a dialog, it is not a tool.) */
const ALL_DRAW_MENU: readonly MenuEntry[] = [
  tool('Line'),
  tool('Arc'),
  SEP,
  tool('Rectangle'),
  tool('Circle'),
  tool('Polygon'),
]

/** Tools ▸ select and eraser, transform, the everyday shaping tools, the
 *  Shape and Deform submenus, then measure & annotate and model-space
 *  utilities (docs/design/true-curves-ui-organization.md section 4). */
const ALL_TOOLS_MENU: readonly MenuEntry[] = [
  tool('Select'),
  tool('Eraser'),
  tool('Paint'),
  tool('Position Texture'),
  SEP,
  tool('Move'),
  tool('Rotate'),
  tool('Scale'),
  tool('Flip', false),
  SEP,
  tool('Push/Pull'),
  tool('Follow Me'),
  tool('Offset'),
  tool('Fillet'),
  { kind: 'alias', actionId: 'tool-chamfer', selectName: 'Chamfer', label: msg`Chamfer` },
  SEP,
  {
    kind: 'submenu',
    id: 'shape',
    label: msg`Shape`,
    items: [
      tool('Revolve', false),
      tool('Loft', false),
      tool('Shell', false),
      tool('Project', false),
      SEP,
      tool('Array along Path', false),
    ],
  },
  {
    kind: 'submenu',
    id: 'deform',
    label: msg`Deform`,
    items: [
      { kind: 'deformMode', mode: 'twist', actionId: 'tool-deform-twist', label: msg`Twist` },
      { kind: 'deformMode', mode: 'bend', actionId: 'tool-deform-bend', label: msg`Bend` },
      { kind: 'deformMode', mode: 'taper', actionId: 'tool-deform-taper', label: msg`Taper` },
      { kind: 'deformMode', mode: 'bend-to-path', actionId: 'tool-deform-bend-path', label: msg`Bend to Path` },
    ],
  },
  SEP,
  tool('Slice'),
  tool('Edit Vertex'),
  SEP,
  tool('Tape Measure'),
  tool('Protractor'),
  tool('Dimension'),
  tool('Text'),
  SEP,
  tool('Drawing Axes'),
  tool('Section Plane'),
]
/* oxlint-enable lingui/no-unlocalized-strings */

/** Draw ▸ as this build offers it: the Preview wall prunes what it withholds. */
export const DRAW_MENU: readonly MenuEntry[] = pruneMenuEntries(ALL_DRAW_MENU, actionOffered)
/** Tools ▸ as this build offers it (empty submenus and stray separators go
 *  with the entries the Preview wall withholds). */
export const TOOLS_MENU: readonly MenuEntry[] = pruneMenuEntries(ALL_TOOLS_MENU, actionOffered)

/** The command id an entry runs; undefined for a separator or a submenu. */
export function menuEntryActionId(entry: MenuEntry): string | undefined {
  switch (entry.kind) {
    // The full list, not the effective registry: pruning asks about tools the
    // build may withhold.
    case 'tool': return ALL_TOOL_SPECS.find((t) => t.name === entry.tool)?.actionId
    case 'alias':
    case 'deformMode': return entry.actionId
    default: return undefined
  }
}

/** The entries whose command id `keep` accepts. A submenu left with nothing
 *  is dropped, and separators that would lead, trail, or double up are
 *  dropped with it. */
export function pruneMenuEntries(
  entries: readonly MenuEntry[],
  keep: (actionId: string) => boolean,
): MenuEntry[] {
  const kept: MenuEntry[] = []
  for (const entry of entries) {
    if (entry.kind === 'submenu') {
      const items = pruneMenuEntries(entry.items, keep)
      if (items.length > 0) kept.push({ ...entry, items })
      continue
    }
    const id = menuEntryActionId(entry)
    if (id === undefined || keep(id)) kept.push(entry)
  }
  const out: MenuEntry[] = []
  for (const entry of kept) {
    const last = out[out.length - 1]
    if (entry.kind === 'separator' && (last === undefined || last.kind === 'separator')) continue
    out.push(entry)
  }
  while (out.length > 0 && out[out.length - 1].kind === 'separator') out.pop()
  return out
}
