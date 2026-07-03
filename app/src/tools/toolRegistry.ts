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
 * Keyboard-shortcut split: macOS keeps its pre-existing Cmd-combo
 * accelerators unchanged — they're wired to the native Tauri menu
 * (`shells/tauri/src-tauri/src/main.rs`), never routed through the JS
 * keydown handler, and explicitly preserves them as-is. Windows, Linux,
 * and the web build adopt SketchUp-for-Windows' real bare-letter scheme for
 * the 10 tools `03_tool_rail.md` covers (`winKey` below); the remaining six
 * Hew-only tools (camera navigation + inspect extras) keep their existing
 * Ctrl-combo shortcuts on those platforms too — the spec doesn't define bare
 * letters for them, and inventing one would contradict the "curated, not
 * exhaustive" rail philosophy the whole redesign follows. See the keydown
 * handler in `App.tsx` for where `winKey` actually gets wired to a key
 * event (this module only holds the data + display strings).
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
  { name: 'Line', group: 'Draw', macKey: '⌘L', winKey: 'L' },
  { name: 'Rectangle', group: 'Draw', macKey: '⌘K', winKey: 'R' },
  { name: 'Circle', group: 'Draw', macKey: 'C', winKey: 'C' },
  // Arc: 'A' is SketchUp-for-Windows' real arc key. The macOS
  // Cmd-scheme has no slot assigned yet — that's a product decision, so
  // macKey stays empty (Arc remains reachable on mac via the rail, the Draw
  // menu, and the command palette).
  { name: 'Arc', group: 'Draw', macKey: '', winKey: 'A' },
  // ---- Modify ----
  { name: 'Push/Pull', group: 'Modify', macKey: '⌘=', winKey: 'P' },
  { name: 'Move', group: 'Modify', macKey: '⌘0', winKey: 'M' },
  { name: 'Rotate', group: 'Modify', macKey: '⌘8', winKey: 'Q' },
  { name: 'Scale', group: 'Modify', macKey: '⌘9', winKey: 'S' },
  // ---- Inspect ----
  { name: 'Tape Measure', group: 'Inspect', macKey: '⌘D', winKey: 'T' },
  { name: 'Paint', group: 'Inspect', macKey: '4', winKey: 'B' },
  // ---- Menu/palette-only (no rail slot; see ToolSpec.group doc above) ----
  { name: 'Protractor', macKey: '', winKey: '' },
  { name: 'Slice', macKey: '', winKey: '' },
  { name: 'Edit Vertex', macKey: '', winKey: '' },
  { name: 'Orbit', macKey: '⌘B', winKey: 'Ctrl+B' },
  { name: 'Pan', macKey: '⌘R', winKey: 'Ctrl+R' },
  { name: 'Zoom', macKey: '⌘\\', winKey: 'Ctrl+\\' },
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
