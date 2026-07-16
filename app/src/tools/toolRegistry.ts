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
 * Edit Vertex stay shortcut-less: SketchUp defines no default key for them
 * either.
 */

import type { ToolName } from './toolIcons'

export type { ToolName }

export type ToolGroup = 'Draw' | 'Modify' | 'Inspect'

export interface ToolSpec {
  name: ToolName
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

export const TOOL_REGISTRY: readonly ToolSpec[] = [
  // ---- Draw ----
  { name: 'Select', group: 'Draw', macKey: 'Spc', winKey: 'Spc' },
  { name: 'Line', group: 'Draw', macKey: 'L', winKey: 'L' },
  { name: 'Rectangle', group: 'Draw', macKey: 'R', winKey: 'R' },
  { name: 'Circle', group: 'Draw', macKey: 'C', winKey: 'C' },
  // Arc: 'A' is SketchUp's real arc key (the arc FAMILY key there — it
  // cycles 2-point/3-point/pie modes; Hew's Arc is the simpler 2-point
  // gesture, but the same key keeps muscle memory intact).
  { name: 'Arc', group: 'Draw', macKey: 'A', winKey: 'A' },
  // ---- Modify ----
  { name: 'Push/Pull', group: 'Modify', macKey: 'P', winKey: 'P' },
  // Follow Me: SketchUp defines no default key for it either.
  { name: 'Follow Me', group: 'Modify', macKey: '', winKey: '' },
  { name: 'Move', group: 'Modify', macKey: 'M', winKey: 'M' },
  { name: 'Rotate', group: 'Modify', macKey: 'Q', winKey: 'Q' },
  { name: 'Scale', group: 'Modify', macKey: 'S', winKey: 'S' },
  // ---- Inspect ----
  { name: 'Tape Measure', group: 'Inspect', macKey: 'T', winKey: 'T' },
  { name: 'Paint', group: 'Inspect', macKey: 'B', winKey: 'B' },
  // ---- Menu/palette-only (no rail slot; see ToolSpec.group doc above) ----
  { name: 'Protractor', macKey: '', winKey: '' },
  { name: 'Slice', macKey: '', winKey: '' },
  { name: 'Edit Vertex', macKey: '', winKey: '' },
  // Camera tools: SketchUp's real O / H / Z everywhere.
  { name: 'Orbit', macKey: 'O', winKey: 'O' },
  { name: 'Pan', macKey: 'H', winKey: 'H' },
  { name: 'Zoom', macKey: 'Z', winKey: 'Z' },
]

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
export const RAIL_GROUPS: readonly ToolGroup[] = ['Draw', 'Modify', 'Inspect']

export function toolsInGroup(group: ToolGroup): ToolSpec[] {
  return TOOL_REGISTRY.filter((t) => t.group === group)
}
