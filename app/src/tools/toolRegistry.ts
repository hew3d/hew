/**
 * toolRegistry — single source of truth for tool metadata: which tools
 * exist, their tool-rail grouping (Draw/Modify/Inspect, `03_tool_rail.md`),
 * and the command id each one is selected by.
 *
 * Before this was split across App.tsx's `TOOLS`/`TOOL_KEYS` consts and
 * three separate menus in MenuBar.tsx (Draw/Tools/Camera), all sharing one
 * flat `activeTool` string but with no single source of truth for shortcut
 * display. This registry is that source; `ToolRail.tsx` and `MenuBar.tsx`
 * both read it.
 *
 * Keyboard shortcuts live in the keymap (`keymap/`), keyed by each tool's
 * `actionId`: the defaults follow SketchUp's bare-letter scheme
 * (`keymap/defaults.ts`) and the person can change them in Settings ▸
 * Controls. Display code reads the effective binding with `shortcutLabel`.
 */

import { i18n } from '@lingui/core'
import { msg } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'
import type { ToolName } from './toolIcons'

export type { ToolName }

export type ToolGroup = 'Draw' | 'Modify' | 'Inspect'

export interface ToolSpec {
  /** The tool ID: compared with === across the app, persisted, and used as
   * `activeTool`. Never displayed — show `label` instead. */
  name: ToolName
  /** Display name. Resolve with `i18n._(spec.label)` at render time. */
  label: MessageDescriptor
  /** Tool-rail group. Undefined = no permanent rail slot (still reachable
   * via the Tools/Camera menus, and — once lands — the command
   * palette); matches `03_tool_rail.md`'s curated Draw/Modify/Inspect set,
   * which omits Protractor/Slice/Edit Vertex/the camera tools. */
  group?: ToolGroup
  /** The command id that selects this tool — the `menuActionRef` payload
   *  and the keymap's id for it (`keymap/defaults.ts`). */
  actionId: string
}

// oxlint-disable lingui/no-unlocalized-strings -- tool IDs, rail group IDs, and shortcut keys are identifiers, not copy; every label is a msg descriptor
export const TOOL_REGISTRY: readonly ToolSpec[] = [
  // ---- Draw ----
  { name: 'Select', label: msg({ context: 'tool', message: 'Select' }), group: 'Draw', actionId: 'tool-select' },
  { name: 'Line', label: msg({ context: 'tool', message: 'Line' }), group: 'Draw', actionId: 'tool-line' },
  { name: 'Rectangle', label: msg`Rectangle`, group: 'Draw', actionId: 'tool-rectangle' },
  { name: 'Circle', label: msg`Circle`, group: 'Draw', actionId: 'tool-circle' },
  { name: 'Polygon', label: msg`Polygon`, group: 'Draw', actionId: 'tool-polygon' },
  { name: 'Arc', label: msg`Arc`, group: 'Draw', actionId: 'tool-arc' },
  // ---- Modify ----
  { name: 'Push/Pull', label: msg`Push/Pull`, group: 'Modify', actionId: 'tool-pushpull' },
  { name: 'Follow Me', label: msg`Follow Me`, group: 'Modify', actionId: 'tool-follow-me' },
  { name: 'Offset', label: msg({ context: 'tool', message: 'Offset' }), group: 'Modify', actionId: 'tool-offset' },
  { name: 'Move', label: msg({ context: 'tool', message: 'Move' }), group: 'Modify', actionId: 'tool-move' },
  { name: 'Rotate', label: msg({ context: 'tool', message: 'Rotate' }), group: 'Modify', actionId: 'tool-rotate' },
  { name: 'Scale', label: msg({ context: 'tool', message: 'Scale' }), group: 'Modify', actionId: 'tool-scale' },
  // ---- Inspect ----
  { name: 'Tape Measure', label: msg`Tape Measure`, group: 'Inspect', actionId: 'tool-tape-measure' },
  { name: 'Dimension', label: msg({ context: 'tool', message: 'Dimension' }), group: 'Inspect', actionId: 'tool-dimension' },
  { name: 'Paint', label: msg({ context: 'tool', message: 'Paint' }), group: 'Inspect', actionId: 'tool-paint' },
  { name: 'Section Plane', label: msg`Section Plane`, group: 'Inspect', actionId: 'tool-section-plane' },
  // ---- Menu/palette-only (no rail slot; see ToolSpec.group doc above) ----
  { name: 'Protractor', label: msg`Protractor`, actionId: 'tool-protractor' },
  { name: 'Slice', label: msg({ context: 'tool', message: 'Slice' }), actionId: 'tool-slice' },
  { name: 'Edit Vertex', label: msg`Edit Vertex`, actionId: 'tool-edit-vertex' },
  { name: 'Drawing Axes', label: msg`Drawing Axes`, actionId: 'tool-axes' },
  // Text: leader-text annotations (docs/design/dimensions-text.md) — menu/
  // palette-only, like Protractor/Slice/Edit Vertex above.
  { name: 'Text', label: msg({ context: 'tool', message: 'Text' }), actionId: 'tool-text' },
  // Position Texture: reached via the contextual dock on a textured face
  // (paint-tool design §3), not the rail.
  { name: 'Position Texture', label: msg`Position Texture`, actionId: 'tool-position-texture' },
  // ---- Camera ----
  { name: 'Orbit', label: msg`Orbit`, actionId: 'tool-orbit' },
  { name: 'Pan', label: msg({ context: 'tool', message: 'Pan' }), actionId: 'tool-pan' },
  { name: 'Zoom', label: msg({ context: 'tool', message: 'Zoom' }), actionId: 'tool-zoom' },
  // Zoom Window (docs/design/camera.md §3): menu + command palette only.
  { name: 'Zoom Window', label: msg`Zoom Window`, actionId: 'tool-zoom-window' },
  // Position Camera / Look Around / Walk (docs/design/camera.md §4): menu +
  // command palette only.
  { name: 'Position Camera', label: msg`Position Camera`, actionId: 'tool-position-camera' },
  { name: 'Look Around', label: msg`Look Around`, actionId: 'tool-look-around' },
  { name: 'Walk', label: msg({ context: 'tool', message: 'Walk' }), actionId: 'tool-walk' },
]
// oxlint-enable lingui/no-unlocalized-strings

/** Every tool name, in registry order — replaces the old App.tsx `TOOLS` const. */
export const TOOLS: readonly ToolName[] = TOOL_REGISTRY.map((t) => t.name)

const BY_NAME: ReadonlyMap<ToolName, ToolSpec> = new Map(TOOL_REGISTRY.map((t) => [t.name, t]))

/** Look up a tool's registry entry. Throws if the registry is missing an
 * entry for `name` — a coding error, not a runtime condition to recover from. */
export function toolSpec(name: ToolName): ToolSpec {
  const spec = BY_NAME.get(name)
  if (spec === undefined) throw new Error(`toolRegistry: no entry for "${name}"`)
  return spec
}

const ACTION_IDS: ReadonlySet<string> = new Set(TOOL_REGISTRY.map((t) => t.actionId))

/** Whether command `id` selects a tool. */
export function isToolAction(id: string): boolean {
  return ACTION_IDS.has(id)
}

/** Tools grouped for the rail, in `03_tool_rail.md`'s Draw/Modify/Inspect order. */
// oxlint-disable-next-line lingui/no-unlocalized-strings -- rail group IDs (the heading text is resolved in ToolRail)
export const RAIL_GROUPS: readonly ToolGroup[] = ['Draw', 'Modify', 'Inspect']

export function toolsInGroup(group: ToolGroup): ToolSpec[] {
  return TOOL_REGISTRY.filter((t) => t.group === group)
}

/** Translated display name for a tool ID, resolved at call time. Falls back
 * to the ID itself for a name the registry doesn't know (the viewport can
 * report non-registry names). Use this wherever a tool name is SHOWN; keep
 * comparing and storing `name`. */
export function toolLabel(name: string): string {
  const spec = BY_NAME.get(name as ToolName)
  return spec === undefined ? name : i18n._(spec.label)
}

/** Rail group headings, keyed by the group ID. Resolve with `i18n._()`. */
export const TOOL_GROUP_LABELS: Record<ToolGroup, MessageDescriptor> = {
  Draw: msg`Draw`,
  Modify: msg`Modify`,
  Inspect: msg`Inspect`,
}
