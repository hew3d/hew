/**
 * toolRegistry — single source of truth for tool metadata: which tools
 * exist, their tool-rail grouping (Draw/Modify/Inspect, `03_tool_rail.md`),
 * and per-platform keyboard shortcuts.
 *
 * Before this was split across App.tsx's `TOOLS`/`TOOL_KEYS` consts and
 * three separate menus in MenuBar.tsx (Draw/Tools/Camera), all sharing one
 * flat `activeTool` string but with no single source of truth for shortcut
 * display. This registry is that source; `ToolRail.tsx` and `MenuBar.tsx`
 * both read it.
 *
 * Keyboard shortcuts: every platform, macOS included, dispatches the
 * SketchUp bare-letter scheme through the JS keydown handler in `App.tsx`
 * (the macOS playtest flagged that the rail displayed keys that only worked
 * elsewhere — the handler now runs the bare-letter block on macOS too, so
 * display and dispatch agree everywhere). The pre-existing macOS Cmd-combo
 * accelerators remain wired to the native Tauri menu
 * (`shells/tauri/src-tauri/src/main.rs`) as secondary shortcuts — the menu
 * advertises those, the rail advertises these. The scheme is
 * SketchUp-for-Windows' real bare letters: the 10 tools `03_tool_rail.md`
 * covers plus the camera tools' O / H / Z (verified against the official
 * SketchUp 2024 Windows Quick Reference Card). Protractor / Slice /
 * Section Plane / Edit Vertex stay shortcut-less: SketchUp defines no
 * default key for them either.
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
  /** Shortcut shown (and, for the rail, active) on macOS. */
  macKey: string
  /** Shortcut shown (and active) on Windows, Linux, and the web build. */
  winKey: string
}

// oxlint-disable lingui/no-unlocalized-strings -- tool IDs, rail group IDs, and shortcut keys are identifiers, not copy; every label is a msg descriptor
export const TOOL_REGISTRY: readonly ToolSpec[] = [
  // ---- Draw ----
  { name: 'Select', label: msg({ context: 'tool', message: 'Select' }), group: 'Draw', macKey: 'Spc', winKey: 'Spc' },
  { name: 'Line', label: msg({ context: 'tool', message: 'Line' }), group: 'Draw', macKey: 'L', winKey: 'L' },
  { name: 'Rectangle', label: msg`Rectangle`, group: 'Draw', macKey: 'R', winKey: 'R' },
  { name: 'Circle', label: msg`Circle`, group: 'Draw', macKey: 'C', winKey: 'C' },
  // Polygon: SketchUp assigns no default key (it lives in the shapes
  // flyout) — reachable from the rail and the command palette only.
  { name: 'Polygon', label: msg`Polygon`, group: 'Draw', macKey: '', winKey: '' },
  // Arc: 'A' is SketchUp's real arc key (the arc FAMILY key there — it
  // cycles 2-point/3-point/pie modes; Hew's Arc is the simpler 2-point
  // gesture, but the same key keeps muscle memory intact).
  { name: 'Arc', label: msg`Arc`, group: 'Draw', macKey: 'A', winKey: 'A' },
  // ---- Modify ----
  { name: 'Push/Pull', label: msg`Push/Pull`, group: 'Modify', macKey: 'P', winKey: 'P' },
  // Follow Me: SketchUp defines no default key for it either.
  { name: 'Follow Me', label: msg`Follow Me`, group: 'Modify', macKey: '', winKey: '' },
  // Offset: 'F' is SketchUp's real offset key.
  { name: 'Offset', label: msg({ context: 'tool', message: 'Offset' }), group: 'Modify', macKey: 'F', winKey: 'F' },
  { name: 'Move', label: msg({ context: 'tool', message: 'Move' }), group: 'Modify', macKey: 'M', winKey: 'M' },
  { name: 'Rotate', label: msg({ context: 'tool', message: 'Rotate' }), group: 'Modify', macKey: 'Q', winKey: 'Q' },
  { name: 'Scale', label: msg({ context: 'tool', message: 'Scale' }), group: 'Modify', macKey: 'S', winKey: 'S' },
  // Flip: SketchUp has no Flip tool key (Flip Along lives on the context
  // menu), so none here either.
  { name: 'Flip', label: msg({ context: 'tool', message: 'Flip' }), group: 'Modify', macKey: '', winKey: '' },
  // ---- Inspect ----
  { name: 'Tape Measure', label: msg`Tape Measure`, group: 'Inspect', macKey: 'T', winKey: 'T' },
  // Dimension: SketchUp's own Dimension tool has no default bare-letter key
  // either (it lives on the Tools/Construction toolbar, not the drawing
  // rail) — rail + palette per the design doc, no shortcut.
  { name: 'Dimension', label: msg({ context: 'tool', message: 'Dimension' }), group: 'Inspect', macKey: '', winKey: '' },
  { name: 'Paint', label: msg({ context: 'tool', message: 'Paint' }), group: 'Inspect', macKey: 'B', winKey: 'B' },
  // Section Plane: SketchUp assigns no default key either.
  { name: 'Section Plane', label: msg`Section Plane`, group: 'Inspect', macKey: '', winKey: '' },
  // ---- Menu/palette-only (no rail slot; see ToolSpec.group doc above) ----
  { name: 'Protractor', label: msg`Protractor`, macKey: '', winKey: '' },
  { name: 'Slice', label: msg({ context: 'tool', message: 'Slice' }), macKey: '', winKey: '' },
  { name: 'Edit Vertex', label: msg`Edit Vertex`, macKey: '', winKey: '' },
  // Drawing Axes: SketchUp assigns no default key either.
  { name: 'Drawing Axes', label: msg`Drawing Axes`, macKey: '', winKey: '' },
  // Text: leader-text annotations (docs/design/dimensions-text.md) — menu/
  // palette-only, like Protractor/Slice/Edit Vertex above.
  { name: 'Text', label: msg({ context: 'tool', message: 'Text' }), macKey: '', winKey: '' },
  // Position Texture: reached via the contextual dock on a textured face
  // (paint-tool design §3), not the rail — SketchUp assigns no default key.
  { name: 'Position Texture', label: msg`Position Texture`, macKey: '', winKey: '' },
  // Camera tools: SketchUp's real O / H / Z everywhere.
  { name: 'Orbit', label: msg`Orbit`, macKey: 'O', winKey: 'O' },
  { name: 'Pan', label: msg({ context: 'tool', message: 'Pan' }), macKey: 'H', winKey: 'H' },
  { name: 'Zoom', label: msg({ context: 'tool', message: 'Zoom' }), macKey: 'Z', winKey: 'Z' },
  // Zoom Window (docs/design/camera.md §3): no default bare-letter shortcut
  // in v1 — SketchUp ships none for it either (menu + command palette only).
  { name: 'Zoom Window', label: msg`Zoom Window`, macKey: '', winKey: '' },
  // Position Camera / Look Around / Walk (docs/design/camera.md §4): no
  // default bare-letter shortcuts in v1 — SketchUp ships none for these
  // either (menu + command palette only).
  { name: 'Position Camera', label: msg`Position Camera`, macKey: '', winKey: '' },
  { name: 'Look Around', label: msg`Look Around`, macKey: '', winKey: '' },
  { name: 'Walk', label: msg({ context: 'tool', message: 'Walk' }), macKey: '', winKey: '' },
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

/** The shortcut string to display (and, for the rail/keydown handler, to
 * treat as active) for `name` on the current platform. */
export function shortcutFor(name: ToolName, isMac: boolean): string {
  const spec = toolSpec(name)
  return isMac ? spec.macKey : spec.winKey
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
