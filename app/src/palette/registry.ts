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
 * Two sections for now: Tools (from `tools/toolRegistry.ts`, ALL tools —
 * not just the rail's curated subset, since the palette's whole point is
 * finding things that aren't visible elsewhere) and Actions (hand-authored,
 * covering File/Edit/View/Camera/Window/Help). The spec's Components and
 * Learn sections are deferred: Components needs the tray's material/outliner
 * data to exist first; Learn needs real help content Hew doesn't have yet —
 * both are net additions to this registry's shape later, not a rework.
 *
 * Completeness is enforced: registry.test.ts diffs this registry against the
 * `case '…':` ids of App.tsx's menuActionRef switch (the canonical action
 * id space), so a new action can't silently skip the palette — it must be
 * registered here or explicitly excused in PALETTE_EXCLUDED_ACTION_IDS.
 *
 * Selection-gated entries (booleans, group/component verbs) carry a `gate`
 * key naming the App-computed enablement flag; the palette shows them
 * disabled rather than hiding them, matching how the menus present the same
 * commands (visible, greyed out when the selection doesn't qualify).
 */

import { TOOL_REGISTRY, shortcutFor, type ToolName } from '../tools/toolRegistry'

/** 'Model' entries are dynamic (built per-document by App.tsx from the
 * scene's object/group/component/tag names) and passed to the palette as
 * `extraEntries` — this module only defines the static Tools/Actions sets.
 * 'Library' entries are also dynamic (App.tsx's `libraryIndex`, desktop
 * only) — the static `open-library` action below is the one entry this
 * module registers for that group; the per-item `insert-library:<file>`
 * entries live entirely in App.tsx, the same way `jump-node:`/`jump-tag:`
 * do for 'Model'. */
export type PaletteGroup = 'Tools' | 'Actions' | 'Model' | 'Library'

/** The selection-gated enablement flags App.tsx computes (`menuGates` — the
 * same flags that grey out the Edit menu items and drive the native menu's
 * sync_menu_state). `selection` is the plain "something is selected" gate
 * (Edit ▸ Delete). */
export type PaletteGate =
  | 'selection'
  | 'canGroup'
  | 'canUngroup'
  | 'canMakeComponent'
  | 'canPlaceCopy'
  | 'canExplode'
  | 'canMakeUnique'
  | 'canBoolean'
  | 'canImport'
  | 'canDrawText'
  /** A Scene is currently active (docs/design/scenes.md §5) — gates
   *  "Update Scene", which has nothing to re-capture with none active. */
  | 'sceneActive'
  /** At least one Scene exists — gates "Next Scene"/"Previous Scene". */
  | 'scenesAny'

export interface PaletteEntry {
  /** Matches a `menuActionRef.current(id)` payload string in App.tsx. */
  id: string
  label: string
  description: string
  group: PaletteGroup
  /** Extra terms that should also match this entry (e.g. "extrude" -> Push/Pull). */
  synonyms?: string[]
  /** Selection gate: when the named flag is false the entry renders
   * disabled (still listed — discoverable, like a greyed menu item). */
  gate?: PaletteGate
}

const TOOL_ACTION_ID: Record<ToolName, string> = {
  'Select': 'tool-select',
  'Line': 'tool-line',
  'Rectangle': 'tool-rectangle',
  'Circle': 'tool-circle',
  'Polygon': 'tool-polygon',
  'Arc': 'tool-arc',
  'Push/Pull': 'tool-pushpull',
  'Follow Me': 'tool-follow-me',
  'Offset': 'tool-offset',
  'Paint': 'tool-paint',
  'Position Texture': 'tool-position-texture',
  'Move': 'tool-move',
  'Rotate': 'tool-rotate',
  'Scale': 'tool-scale',
  'Tape Measure': 'tool-tape-measure',
  'Dimension': 'tool-dimension',
  'Protractor': 'tool-protractor',
  'Slice': 'tool-slice',
  'Section Plane': 'tool-section-plane',
  'Edit Vertex': 'tool-edit-vertex',
  'Axes': 'tool-axes',
  'Text': 'tool-text',
  'Orbit': 'tool-orbit',
  'Pan': 'tool-pan',
  'Zoom': 'tool-zoom',
  'Position Camera': 'tool-position-camera',
  'Walk': 'tool-walk',
  'Look Around': 'tool-look-around',
  'Zoom Window': 'tool-zoom-window',
}

const TOOL_DESCRIPTIONS: Record<ToolName, string> = {
  'Select': 'Pick faces, edges, and objects in the model.',
  'Line': 'Draw a straight edge between two points.',
  'Rectangle': 'Draw a rectangular face on the ground or a face.',
  'Circle': 'Draw a faceted circular face.',
  'Polygon': 'Draw a regular N-sided polygon from a center point.',
  'Arc': 'Click two endpoints, then pull out the bulge. Alt cycles open · pie · segment.',
  'Push/Pull': 'Extrude a face into a solid, or reshape one.',
  'Follow Me': 'Sweep a profile along a path into a solid.',
  'Offset': "Copy a face or profile's boundary inward or outward at a set distance.",
  'Paint': 'Apply the current material to a face or object.',
  'Position Texture': "Move, scale, and rotate a textured face's texture about a fixed pin.",
  'Move': 'Translate the selection, with inference snapping.',
  'Rotate': 'Rotate the selection around an inferred axis.',
  'Scale': 'Resize the selection uniformly about its center.',
  'Tape Measure': 'Measure a distance, or create a parallel guide line.',
  'Dimension': 'Place a linear or radius/diameter dimension between snap points.',
  'Protractor': 'Measure an angle, or create an angular guide line.',
  'Slice': 'Cut a solid into two separate watertight Objects.',
  'Section Plane': 'Look inside the model with a non-destructive clipping plane.',
  'Edit Vertex': 'Drag an individual vertex to reshape a face.',
  'Axes': 'Reposition the drawing axes: click an origin, then the red and green directions.',
  'Text': 'Place a leader-text annotation from a face, edge, or point.',
  'Orbit': 'Rotate the camera around the model.',
  'Pan': 'Slide the camera parallel to the view plane.',
  'Zoom': 'Move the camera closer to or farther from the model.',
  'Position Camera': 'Click to stand at a point facing your view, or drag to look toward where you release.',
  'Walk': 'Drag to walk forward/back and turn; hold Shift to strafe and change height.',
  'Look Around': 'Drag to look around from a fixed eye position.',
  'Zoom Window': 'Drag a rectangle to zoom the camera onto it, then return to Select.',
}

/** One-line instructor hint for a tool, for the status bar's active-tool line
 * (`02_app_shell.md`). Reuses the palette's own tool descriptions so the two
 * surfaces never drift. Empty string for an unknown tool name. */
export function toolHint(name: string): string {
  return TOOL_DESCRIPTIONS[name as ToolName] ?? ''
}

/** The `menuActionRef` dispatch id for a tool name (e.g. `'Arc'` ->
 * `'tool-arc'`) — the same id space `dockLogic.ts`'s verbs and this
 * registry's own tool entries use. `undefined` for an unknown tool name
 * (the contextual dock uses this to match its active-tool highlight). */
export function toolActionId(name: string): string | undefined {
  return TOOL_ACTION_ID[name as ToolName]
}

const TOOL_SYNONYMS: Partial<Record<ToolName, string[]>> = {
  'Push/Pull': ['extrude', 'pull', 'push'],
  'Follow Me': ['sweep', 'pipe', 'molding', 'lathe', 'revolve'],
  'Offset': ['inset', 'outset', 'concentric'],
  'Rectangle': ['rect', 'box'],
  'Polygon': ['polygon', 'hexagon', 'hex', 'pentagon', 'n-gon', 'nut', 'bolt'],
  'Tape Measure': ['measure', 'distance', 'guide'],
  'Dimension': ['dimension', 'measure', 'annotate', 'radius', 'diameter'],
  'Protractor': ['angle'],
  'Section Plane': ['section', 'cut', 'cross-section', 'clip', 'inside', 'wall thickness'],
  'Edit Vertex': ['vertex', 'reshape'],
  'Position Camera': ['stand', 'eye height', 'walkthrough', 'first person'],
  'Walk': ['walkthrough', 'first person', 'fps'],
  'Look Around': ['mouselook', 'walkthrough', 'first person'],
  'Text': ['leader', 'label', 'note', 'annotate', 'annotation'],
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
  { id: 'import', label: 'Import…', description: 'Import a COLLADA (.dae), SketchUp (.skp), glTF, or STL model.', group: 'Actions', gate: 'canImport' },
  { id: 'export', label: 'Export…', description: 'Export the model as glTF, STL, 3MF, or USDZ — format chosen in the dialog.', group: 'Actions', synonyms: ['stl', 'glb', 'gltf', '3mf', 'usdz', 'usd', 'ar', 'quick look', '3d print', 'print', 'slicer'] },
  { id: 'draw-3d-text', label: '3D Text…', description: 'Place extruded, watertight text on a face or the ground.', group: 'Actions', synonyms: ['text', 'font', 'letters', 'lettering', 'engrave', 'extrude text'], gate: 'canDrawText' },
  { id: 'save', label: 'Save', description: 'Save the current document.', group: 'Actions' },
  { id: 'save-as', label: 'Save As…', description: 'Save the current document under a new name.', group: 'Actions' },
  { id: 'undo', label: 'Undo', description: 'Undo the last change.', group: 'Actions' },
  { id: 'redo', label: 'Redo', description: 'Redo the last undone change.', group: 'Actions' },
  { id: 'edit-select-all', label: 'Select All', description: 'Select every visible object, group, component, and sketch.', group: 'Actions', synonyms: ['select everything'] },
  { id: 'edit-delete', label: 'Delete', description: 'Delete the current selection.', group: 'Actions', synonyms: ['remove', 'erase'], gate: 'selection' },
  { id: 'edit-delete-guides', label: 'Delete Guide Lines', description: 'Remove every construction guide.', group: 'Actions' },
  { id: 'edit-group', label: 'Group', description: 'Group the selected objects so they move together.', group: 'Actions', synonyms: ['make group'], gate: 'canGroup' },
  { id: 'edit-ungroup', label: 'Ungroup', description: 'Dissolve the selected group back into its members.', group: 'Actions', synonyms: ['dissolve group'], gate: 'canUngroup' },
  { id: 'edit-make-component', label: 'Make Component', description: 'Turn the selection into a reusable component definition.', group: 'Actions', synonyms: ['create component', 'component'], gate: 'canMakeComponent' },
  { id: 'edit-place-copy', label: 'Place Copy', description: 'Place another instance of the selected component.', group: 'Actions', synonyms: ['duplicate', 'instance', 'copy component'], gate: 'canPlaceCopy' },
  { id: 'edit-explode', label: 'Explode', description: 'Break the selected component instance into plain objects.', group: 'Actions', synonyms: ['explode instance', 'break component'], gate: 'canExplode' },
  { id: 'edit-make-unique', label: 'Make Unique', description: 'Give the selected instance its own definition, detached from its siblings.', group: 'Actions', synonyms: ['unique component'], gate: 'canMakeUnique' },
  { id: 'edit-union', label: 'Union', description: 'Merge two selected solids into one (boolean add).', group: 'Actions', synonyms: ['boolean', 'merge', 'combine', 'add', 'join'], gate: 'canBoolean' },
  { id: 'edit-subtract', label: 'Subtract', description: 'Cut the second selected solid out of the first (boolean difference).', group: 'Actions', synonyms: ['boolean', 'difference', 'cut', 'carve'], gate: 'canBoolean' },
  { id: 'edit-intersect', label: 'Intersect', description: 'Keep only the volume the two selected solids share (boolean intersection).', group: 'Actions', synonyms: ['boolean', 'common', 'overlap'], gate: 'canBoolean' },
  { id: 'toggle-axes', label: 'Toggle Axes', description: 'Show or hide the world axes.', group: 'Actions' },
  { id: 'reset-axes', label: 'Reset Axes', description: 'Reset the movable drawing axes back to world identity.', group: 'Actions', synonyms: ['drawing axes', 'origin'] },
  { id: 'toggle-grid', label: 'Toggle Grid', description: 'Show or hide the ground grid.', group: 'Actions' },
  { id: 'toggle-guides', label: 'Toggle Guides', description: 'Show or hide construction guides.', group: 'Actions' },
  { id: 'toggle-section-active', label: 'Section Plane', description: 'Turn the placed section plane\'s clip on or off without removing it.', group: 'Actions', synonyms: ['section', 'cut', 'cross-section', 'active cut', 'toggle section active', 'toggle section'] },
  { id: 'zoom-extents', label: 'Zoom Extents', description: 'Fit the camera to all scene geometry.', group: 'Actions', synonyms: ['zoom to fit'] },
  { id: 'toggle-parallel-projection', label: 'Parallel Projection', description: 'Toggle between perspective and parallel (orthographic) projection.', group: 'Actions', synonyms: ['orthographic', 'perspective', 'ortho'] },
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
  { id: 'toggle-scenes', label: 'Toggle Scenes', description: 'Show or hide the Scenes panel.', group: 'Actions' },
  { id: 'scenes-add', label: 'Add Scene', description: 'Save the current camera, visibility, and section cut as a new Scene.', group: 'Actions', synonyms: ['scene', 'save view', 'named view'] },
  { id: 'scenes-update', label: 'Update Scene', description: 'Re-capture the active Scene’s checked properties.', group: 'Actions', synonyms: ['scene', 'refresh scene'], gate: 'sceneActive' },
  { id: 'scenes-next', label: 'Next Scene', description: 'Activate the next Scene in the list.', group: 'Actions', synonyms: ['scene'], gate: 'scenesAny' },
  { id: 'scenes-previous', label: 'Previous Scene', description: 'Activate the previous Scene in the list.', group: 'Actions', synonyms: ['scene'], gate: 'scenesAny' },
  { id: 'scenes-transitions', label: 'Scene Transitions', description: 'Toggle the 0.6s camera tween when activating a Scene.', group: 'Actions', synonyms: ['scene', 'animation', 'tween'] },
  { id: 'toggle-object-info', label: 'Toggle Object Info', description: 'Show or hide the Object Info panel.', group: 'Actions' },
  { id: 'toggle-debug-log', label: 'Toggle Debug Log', description: 'Show or hide the debug log panel.', group: 'Actions' },
  { id: 'open-library', label: 'Library', description: 'Browse and insert saved components, materials, and models', group: 'Actions', synonyms: ['library', 'components', 'insert'] },
  { id: 'save-to-library-doc', label: 'Save to Library…', description: 'Save the whole document as a library item.', group: 'Actions', synonyms: ['library', 'component', 'model'] },
  { id: 'open-settings', label: 'Settings…', description: 'Open Hew Settings.', group: 'Actions', synonyms: ['preferences'] },
  { id: 'report-bug', label: 'Report Bug…', description: 'Assemble and save a bug-report bundle.', group: 'Actions' },
]

/**
 * Action ids the menuActionRef switch handles that are DELIBERATELY not in
 * the palette, each with its reason. registry.test.ts enforces that every
 * switch case is either registered above or excused here — additions to the
 * switch fail the test until they pick a side, so the palette can't silently
 * drift out of date again.
 */
export const PALETTE_EXCLUDED_ACTION_IDS: Record<string, string> = {
  'open-palette': 'self-referential — the palette cannot usefully open itself',
  'close': 'window lifecycle, desktop shells only — not a model action',
  'open-on-phone':
    'desktop shells only — the LAN server lives in the Tauri shell and the palette has no platform gate; the web build hides its menu item the same way (App.tsx passes onOpenOnPhone only under isTauri)',
  'enter-context': 'contextual-dock alias; needs a picked node, not a bare trigger',
  'ungroup': 'contextual-dock alias of edit-ungroup',
  'make-unique': 'contextual-dock alias of edit-make-unique',
  'explode-instance': 'contextual-dock alias of edit-explode',
  'save-to-library': 'contextual-dock only — needs a single selected object/group/instance, not a bare trigger (the whole-document save is separately registered as save-to-library-doc)',
  'toggle-library': 'keyboard/native-menu/dock-icon toggle alias of open-library — the palette exposes the single "open" verb, not a checkbox toggle',
}

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
