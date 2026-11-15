/**
 * toolActions — every command id that selects a tool, and what selecting it
 * does. This is the one list behind App.tsx's `tool-*` dispatch, so a tool's
 * commands can be listed, tested, or withheld in one place.
 *
 * A tool's own command (`TOOL_REGISTRY[...].actionId`) is derived from the
 * registry. The few commands that are NOT a tool's own id are listed here:
 * Chamfer (Fillet in its bevel mode), Skin (Loft under SketchUp's name), and
 * Deform's Bend / Taper / Bend to Path (Twist is Deform's own command). Each
 * may set a mode before the tool opens.
 *
 * App.tsx keeps a literal `case '<id>':` label for each id (the palette and
 * keymap completeness tests read the switch's labels) and routes the body
 * through `runToolAction`.
 */

import { TOOL_REGISTRY, type ToolName } from './toolRegistry'
import { setDeformMode } from './deformMode'
import { setFilletProfile } from './filletSettings'

export interface ToolAction {
  /** The `menuActionRef` payload. */
  id: string
  /** The tool it opens. */
  tool: ToolName
  /** Runs first: set the mode the tool opens in. */
  prepare?: () => void
}

/* oxlint-disable lingui/no-unlocalized-strings -- command ids and tool IDs, never shown */
/** Setup a tool's own command runs before the tool opens, keyed by action id. */
const OWN_PREPARE: Readonly<Record<string, () => void>> = {
  'tool-fillet': () => { setFilletProfile({ kind: 'round' }) },
  'tool-deform-twist': () => { setDeformMode('twist') },
}

/** Commands that open a tool under another name or in another mode. */
const ALIAS_ACTIONS: readonly ToolAction[] = [
  { id: 'tool-chamfer', tool: 'Fillet', prepare: () => { setFilletProfile({ kind: 'chamfer' }) } },
  { id: 'tool-skin', tool: 'Loft' },
  { id: 'tool-deform-bend', tool: 'Deform', prepare: () => { setDeformMode('bend') } },
  { id: 'tool-deform-taper', tool: 'Deform', prepare: () => { setDeformMode('taper') } },
  { id: 'tool-deform-bend-path', tool: 'Deform', prepare: () => { setDeformMode('bend-to-path') } },
]
/* oxlint-enable lingui/no-unlocalized-strings */

export const TOOL_ACTIONS: readonly ToolAction[] = [
  ...TOOL_REGISTRY.map((spec): ToolAction => ({ id: spec.actionId, tool: spec.name, prepare: OWN_PREPARE[spec.actionId] })),
  ...ALIAS_ACTIONS,
]

const BY_ID: ReadonlyMap<string, ToolAction> = new Map(TOOL_ACTIONS.map((a) => [a.id, a]))

/** The tool action behind command `id`, or undefined when it selects no tool. */
export function toolActionFor(id: string): ToolAction | undefined {
  return BY_ID.get(id)
}

/** Run command `id` if it selects a tool: set its mode, then open it with
 *  `activate`. Returns whether `id` was a tool command. */
export function runToolAction(id: string, activate: (tool: ToolName) => void): boolean {
  const action = BY_ID.get(id)
  if (action === undefined) return false
  action.prepare?.()
  activate(action.tool)
  return true
}
