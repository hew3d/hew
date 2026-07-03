/**
 * palette/registry — the searchable action list for the command palette
 * (`04_command_palette.md`).
 *
 * Reuses `App.tsx`'s existing `menuActionRef.current(payload)` dispatch (the
 * same string-payload switch the native menu's `menu-action` Tauri event and
 * every menu click already route through) as the execution path — a
 * selected palette result just calls `menuActionRef.current(entry.id)`, so
 * this module holds no handler logic of its own, only searchable metadata.
 *
 * Two sections for now: Tools (from `tools/toolRegistry.ts`, ALL 16 tools —
 * not just the rail's curated 10, since the palette's whole point is finding
 * things that aren't visible elsewhere) and Actions (hand-authored, covering
 * File/Edit/View/Camera/Window/Help). The spec's Components and Learn
 * sections are deferred: Components needs the tray's material/outliner
 * data to exist first; Learn needs real help content Hew doesn't have yet —
 * both are net additions to this registry's shape later, not a rework.
 */

import { TOOL_REGISTRY, shortcutFor, type ToolName } from '../tools/toolRegistry'

export type PaletteGroup = 'Tools' | 'Actions'

export interface PaletteEntry {
  /** Matches a `menuActionRef.current(id)` payload string in App.tsx. */
  id: string
  label: string
  description: string
  group: PaletteGroup
  /** Extra terms that should also match this entry (e.g. "extrude" -> Push/Pull). */
  synonyms?: string[]
}

const TOOL_ACTION_ID: Record<ToolName, string> = {
  'Select': 'tool-select',
  'Line': 'tool-line',
  'Rectangle': 'tool-rectangle',
  'Circle': 'tool-circle',
  'Arc': 'tool-arc',
  'Push/Pull': 'tool-pushpull',
  'Paint': 'tool-paint',
  'Move': 'tool-move',
  'Rotate': 'tool-rotate',
  'Scale': 'tool-scale',
  'Tape Measure': 'tool-tape-measure',
  'Protractor': 'tool-protractor',
  'Slice': 'tool-slice',
  'Edit Vertex': 'tool-edit-vertex',
  'Orbit': 'tool-orbit',
  'Pan': 'tool-pan',
  'Zoom': 'tool-zoom',
}

const TOOL_DESCRIPTIONS: Record<ToolName, string> = {
  'Select': 'Pick faces, edges, and objects in the model.',
  'Line': 'Draw a straight edge between two points.',
  'Rectangle': 'Draw a rectangular face on the ground or a face.',
  'Circle': 'Draw a faceted circular face.',
  'Arc': 'Click two endpoints, then pull out the bulge.',
  'Push/Pull': 'Extrude a face into a solid, or reshape one.',
  'Paint': 'Apply the current material to a face or object.',
  'Move': 'Translate the selection, with inference snapping.',
  'Rotate': 'Rotate the selection around an inferred axis.',
  'Scale': 'Resize the selection from a corner or edge handle.',
  'Tape Measure': 'Measure a distance, or create a parallel guide line.',
  'Protractor': 'Measure an angle, or create an angular guide line.',
  'Slice': 'Cut a solid into two separate watertight Objects.',
  'Edit Vertex': 'Drag an individual vertex to reshape a face.',
  'Orbit': 'Rotate the camera around the model.',
  'Pan': 'Slide the camera parallel to the view plane.',
  'Zoom': 'Move the camera closer to or farther from the model.',
}

/** One-line instructor hint for a tool, for the status bar's active-tool line
 * (`02_app_shell.md`). Reuses the palette's own tool descriptions so the two
 * surfaces never drift. Empty string for an unknown tool name. */
export function toolHint(name: string): string {
  return TOOL_DESCRIPTIONS[name as ToolName] ?? ''
}

const TOOL_SYNONYMS: Partial<Record<ToolName, string[]>> = {
  'Push/Pull': ['extrude', 'pull', 'push'],
  'Rectangle': ['rect', 'box'],
  'Tape Measure': ['measure', 'distance', 'guide'],
  'Protractor': ['angle'],
  'Edit Vertex': ['vertex', 'reshape'],
}

function toolEntries(): PaletteEntry[] {
  return TOOL_REGISTRY.map((t) => ({
    id: TOOL_ACTION_ID[t.name],
    label: t.name,
    description: TOOL_DESCRIPTIONS[t.name],
    group: 'Tools',
    synonyms: TOOL_SYNONYMS[t.name],
  }))
}

/** Hand-authored: everything reachable via menuActionRef that isn't a tool. */
const ACTION_ENTRIES: PaletteEntry[] = [
  { id: 'new', label: 'New', description: 'Start a new, blank document.', group: 'Actions' },
  { id: 'open', label: 'Open…', description: 'Open a .hew file from disk.', group: 'Actions' },
  { id: 'import', label: 'Import…', description: 'Import a COLLADA (.dae) or glTF model.', group: 'Actions' },
  { id: 'export', label: 'Export…', description: 'Export the model as glTF/GLB.', group: 'Actions' },
  { id: 'export-stl', label: 'Export STL…', description: 'Export the model as binary STL (millimeters) for 3D printing.', group: 'Actions', synonyms: ['print', 'slicer'] },
  { id: 'save', label: 'Save', description: 'Save the current document.', group: 'Actions' },
  { id: 'save-as', label: 'Save As…', description: 'Save the current document under a new name.', group: 'Actions' },
  { id: 'undo', label: 'Undo', description: 'Undo the last change.', group: 'Actions' },
  { id: 'redo', label: 'Redo', description: 'Redo the last undone change.', group: 'Actions' },
  { id: 'edit-delete', label: 'Delete', description: 'Delete the current selection.', group: 'Actions', synonyms: ['remove', 'erase'] },
  { id: 'edit-delete-guides', label: 'Delete Guide Lines', description: 'Remove every construction guide.', group: 'Actions' },
  { id: 'toggle-axes', label: 'Toggle Axes', description: 'Show or hide the world axes/grid.', group: 'Actions' },
  { id: 'toggle-guides', label: 'Toggle Guides', description: 'Show or hide construction guides.', group: 'Actions' },
  { id: 'zoom-extents', label: 'Zoom Extents', description: 'Fit the camera to all scene geometry.', group: 'Actions', synonyms: ['zoom to fit'] },
  { id: 'view-top', label: 'Standard View: Top', description: 'Look straight down at the model.', group: 'Actions' },
  { id: 'view-bottom', label: 'Standard View: Bottom', description: 'Look straight up at the model.', group: 'Actions' },
  { id: 'view-front', label: 'Standard View: Front', description: 'Look at the model from the front.', group: 'Actions' },
  { id: 'view-back', label: 'Standard View: Back', description: 'Look at the model from the back.', group: 'Actions' },
  { id: 'view-left', label: 'Standard View: Left', description: 'Look at the model from the left.', group: 'Actions' },
  { id: 'view-right', label: 'Standard View: Right', description: 'Look at the model from the right.', group: 'Actions' },
  { id: 'view-iso', label: 'Standard View: Iso', description: 'Switch to the standard isometric view.', group: 'Actions' },
  { id: 'toggle-model-info', label: 'Toggle Model Info', description: 'Show or hide the Model Info panel.', group: 'Actions' },
  { id: 'toggle-materials', label: 'Toggle Materials', description: 'Show or hide the Materials panel.', group: 'Actions' },
  { id: 'toggle-tags', label: 'Toggle Tags', description: 'Show or hide the Tags panel.', group: 'Actions' },
  { id: 'toggle-object-info', label: 'Toggle Object Info', description: 'Show or hide the Object Info panel.', group: 'Actions' },
  { id: 'toggle-debug-log', label: 'Toggle Debug Log', description: 'Show or hide the debug log panel.', group: 'Actions' },
  { id: 'open-settings', label: 'Settings…', description: 'Open Hew Settings.', group: 'Actions', synonyms: ['preferences'] },
  { id: 'report-bug', label: 'Report Bug…', description: 'Assemble and save a bug-report bundle.', group: 'Actions' },
]

/** The full palette registry — tools first (spec ranks Tools above Actions
 * absent other signals), then hand-authored actions. */
export function paletteEntries(): PaletteEntry[] {
  return [...toolEntries(), ...ACTION_ENTRIES]
}

/** Convenience for callers (e.g. the palette UI) that also want the
 * platform-correct shortcut string to display next to a tool result. */
export function paletteShortcut(entry: PaletteEntry, isMac: boolean): string {
  const tool = TOOL_REGISTRY.find((t) => TOOL_ACTION_ID[t.name] === entry.id)
  return tool === undefined ? '' : shortcutFor(tool.name, isMac)
}
