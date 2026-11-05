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

import { i18n, type MessageDescriptor } from '@lingui/core'
import { msg, t } from '@lingui/core/macro'
import { MENU } from '../health/healthCopy'
import { TOOL_REGISTRY, type ToolName } from '../tools/toolRegistry'
import { shortcutLabel } from '../keymap/keymap'

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
  /** One object, group, or instance selected, with a library backend. */
  | 'canSaveSelectionToLibrary'
  | 'canImport'
  /** The document has a file to return to and unsaved changes. */
  | 'canRevertToSaved'
  | 'canDrawText'
  /** A Scene is currently active (docs/design/scenes.md §5) — gates
   *  "Update Scene", which has nothing to re-capture with none active. */
  | 'sceneActive'
  /** At least one Scene exists — gates "Next Scene"/"Previous Scene". */
  | 'scenesAny'
  /** Cut/Copy (Lane D): at least one structural node is selected. */
  | 'hasStructuralSelection'
  /** Paste/Paste In Place (Lane D): the clipboard has content. */
  | 'clipboardHasContent'

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
  /** Set on Tools entries: the tool's ID (its `ToolName`), so callers can
   * pick the icon without matching on the translated `label`. */
  toolName?: ToolName
}

const TOOL_ACTION_ID = Object.fromEntries(TOOL_REGISTRY.map((t) => [t.name, t.actionId])) as Record<ToolName, string>

const TOOL_DESCRIPTIONS: Record<ToolName, MessageDescriptor> = {
  'Select': msg`Pick faces, edges, and objects in the model.`,
  'Line': msg`Draw a straight edge between two points. Ctrl detaches the shape from the face.`,
  'Rectangle': msg`Draw a rectangular face on the ground or a face. Ctrl detaches the shape from the face.`,
  'Circle': msg`Draw a faceted circular face. Ctrl detaches the shape from the face.`,
  'Polygon': msg`Draw a regular N-sided polygon from a center point. Ctrl detaches the shape from the face.`,
  'Arc': msg`Click two endpoints, then pull out the bulge. Alt cycles open · pie · segment. Ctrl detaches the shape from the face.`,
  'Push/Pull': msg`Extrude a face into a solid, or reshape one.`,
  'Follow Me': msg`Sweep a profile along a path into a solid.`,
  'Offset': msg`Copy a face or profile's boundary inward or outward at a set distance.`,
  'Paint': msg`Apply the current material to a face or object.`,
  'Position Texture': msg`Move, scale, and rotate a textured face's texture about a fixed pin.`,
  'Move': msg`Translate the selection, with inference snapping.`,
  'Rotate': msg`Rotate the selection around an inferred axis.`,
  'Scale': msg`Resize the selection uniformly about its center.`,
  'Tape Measure': msg`Measure a distance, or create a parallel guide line.`,
  'Dimension': msg`Place a linear or radius/diameter dimension between snap points.`,
  'Protractor': msg`Measure an angle, or create an angular guide line.`,
  'Slice': msg`Cut a solid into two separate watertight Objects.`,
  'Section Plane': msg`Look inside the model with a non-destructive clipping plane.`,
  'Edit Vertex': msg`Drag an individual vertex to reshape a face.`,
  'Drawing Axes': msg`Reposition the drawing axes: click an origin, then the red and green directions.`,
  'Text': msg`Place a leader-text annotation from a face, edge, or point.`,
  'Orbit': msg`Rotate the camera around the model.`,
  'Pan': msg`Slide the camera parallel to the view plane.`,
  'Zoom': msg`Move the camera closer to or farther from the model.`,
  'Position Camera': msg`Click to stand at a point facing your view, or drag to look toward where you release.`,
  'Walk': msg`Drag to walk forward/back and turn; hold Shift to strafe and change height.`,
  'Look Around': msg`Drag to look around from a fixed eye position.`,
  'Zoom Window': msg`Drag a rectangle to zoom the camera onto it, then return to Select.`,
}

/** One-line instructor hint for a tool, for the status bar's active-tool line
 * (`02_app_shell.md`). Reuses the palette's own tool descriptions so the two
 * surfaces never drift. Empty string for an unknown tool name. */
export function toolHint(name: string): string {
  const description = TOOL_DESCRIPTIONS[name as ToolName]
  return description === undefined ? '' : i18n._(description)
}

/** The `menuActionRef` dispatch id for a tool name (e.g. `'Arc'` ->
 * `'tool-arc'`) — the same id space `dockLogic.ts`'s verbs and this
 * registry's own tool entries use. `undefined` for an unknown tool name
 * (the contextual dock uses this to match its active-tool highlight). */
export function toolActionId(name: string): string | undefined {
  return TOOL_ACTION_ID[name as ToolName]
}

function toolSynonyms(): Partial<Record<ToolName, string[]>> {
  return {
    'Push/Pull': [t`extrude`, t`pull`, t`push`],
    'Follow Me': [t`sweep`, t`pipe`, t`molding`, t`lathe`, t`revolve`],
    'Offset': [t`inset`, t`outset`, t`concentric`],
    'Rectangle': [t`rect`, t`box`],
    'Polygon': [t`polygon`, t`hexagon`, t`hex`, t`pentagon`, t`n-gon`, t`nut`, t`bolt`],
    'Tape Measure': [t`measure`, t`distance`, t`guide`],
    'Dimension': [t`dimension`, t`measure`, t`annotate`, t`radius`, t`diameter`],
    'Protractor': [t`angle`],
    'Section Plane': [t`section`, t`cut`, t`cross-section`, t`clip`, t`inside`, t`wall thickness`],
    'Edit Vertex': [t`vertex`, t`reshape`],
    'Position Camera': [t`stand`, t`eye height`, t`walkthrough`, t`first person`],
    'Walk': [t`walkthrough`, t`first person`, t`fps`],
    'Look Around': [t`mouselook`, t`walkthrough`, t`first person`],
    'Text': [t`leader`, t`label`, t`note`, t`annotate`, t`annotation`],
    'Drawing Axes': [t`axes`, t`axis`, t`origin`, t`reorient`, t`custom axes`],
  }
}

function toolEntries(): PaletteEntry[] {
  const synonyms = toolSynonyms()
  return TOOL_REGISTRY.map((spec) => ({
    id: TOOL_ACTION_ID[spec.name],
    toolName: spec.name,
    label: i18n._(spec.label),
    description: i18n._(TOOL_DESCRIPTIONS[spec.name]),
    // oxlint-disable-next-line lingui/no-unlocalized-strings -- group ID
    group: 'Tools',
    synonyms: synonyms[spec.name],
  }))
}

/** Hand-authored: everything reachable via menuActionRef that isn't a tool.
 * A function, not a constant: `t` resolves at call time so a locale switch
 * is picked up the next time the palette builds its entries. */
// oxlint-disable lingui/no-unlocalized-strings -- group and gate values are identifiers; every label, description, and synonym is wrapped in t``
function actionEntries(): PaletteEntry[] {
  return [
    { id: 'new', label: t`New`, description: t`Start a new, blank document.`, group: 'Actions' },
    { id: 'open', label: t`Open…`, description: t`Open a .hew file from disk.`, group: 'Actions' },
    { id: 'import', label: t`Import…`, description: t`Import a COLLADA (.dae), SketchUp (.skp), glTF, or STL model.`, group: 'Actions', gate: 'canImport' },
    { id: 'import-image', label: t`Import Image…`, description: t`Place a PNG or JPEG picture on a face or the ground.`, group: 'Actions', gate: 'canImport', synonyms: [t`picture`, t`photo`, t`image`, t`reference`, t`place image`, t`png`, t`jpeg`, t`jpg`] },
    { id: 'export', label: t`Export…`, description: t`Export the model as glTF, STL, 3MF, USDZ, or an SVG line drawing — format chosen in the dialog.`, group: 'Actions', synonyms: [t`stl`, t`glb`, t`gltf`, t`3mf`, t`usdz`, t`usd`, t`ar`, t`quick look`, t`3d print`, t`slicer`, t`svg`, t`line drawing`, t`laser`, t`cnc`] },
    { id: 'print', label: t`Print…`, description: t`Print the current view to paper or PDF — standard, or to an exact scale across pages.`, group: 'Actions', synonyms: [t`pdf`, t`paper`, t`plot`, t`scale`, t`template`, t`to scale`, t`page`] },
    { id: 'draw-3d-text', label: t`3D Text…`, description: t`Place extruded, watertight text on a face or the ground.`, group: 'Actions', synonyms: [t`text`, t`font`, t`letters`, t`lettering`, t`engrave`, t`extrude text`], gate: 'canDrawText' },
    { id: 'save', label: t`Save`, description: t`Save the current document.`, group: 'Actions' },
    { id: 'save-as', label: t`Save As…`, description: t`Save the current document under a new name.`, group: 'Actions' },
    { id: 'revert-to-saved', label: t`Revert to Saved`, description: t`Throw away every change since the last save and read the file back in.`, group: 'Actions', gate: 'canRevertToSaved', synonyms: [t`discard changes`, t`reload`, t`revert`] },
    { id: 'undo', label: t`Undo`, description: t`Undo the last change.`, group: 'Actions' },
    { id: 'redo', label: t`Redo`, description: t`Redo the last undone change.`, group: 'Actions' },
    { id: 'edit-select-all', label: t`Select All`, description: t`Select every visible object, group, component, and sketch.`, group: 'Actions', synonyms: [t`select everything`] },
    { id: 'edit-select-none', label: t`Select None`, description: t`Deselect everything.`, group: 'Actions', synonyms: [t`deselect all`, t`clear selection`] },
    { id: 'edit-invert-selection', label: t`Invert Selection`, description: t`Select everything that is not selected, and deselect what is.`, group: 'Actions', synonyms: [t`inverse selection`, t`swap selection`] },
    { id: 'edit-cut', label: t`Cut`, description: t`Copy the selection to the clipboard, then delete it.`, group: 'Actions', gate: 'hasStructuralSelection' },
    { id: 'edit-copy', label: t`Copy`, description: t`Copy the selected objects, groups, and components to the clipboard.`, group: 'Actions', gate: 'hasStructuralSelection' },
    { id: 'edit-paste', label: t`Paste`, description: t`Place a copy of the clipboard at the cursor.`, group: 'Actions', gate: 'clipboardHasContent' },
    { id: 'edit-paste-in-place', label: t`Paste In Place`, description: t`Insert the clipboard at its original position.`, group: 'Actions', synonyms: [t`paste in-place`], gate: 'clipboardHasContent' },
    { id: 'edit-delete', label: t`Delete`, description: t`Delete the current selection.`, group: 'Actions', synonyms: [t`remove`, t`erase`], gate: 'selection' },
    { id: 'edit-delete-guides', label: t`Delete Guide Lines`, description: t`Remove every construction guide.`, group: 'Actions' },
    { id: 'edit-group', label: t`Group`, description: t`Group the selected objects so they move together.`, group: 'Actions', synonyms: [t`make group`], gate: 'canGroup' },
    { id: 'edit-ungroup', label: t`Ungroup`, description: t`Dissolve the selected group back into its members.`, group: 'Actions', synonyms: [t`dissolve group`], gate: 'canUngroup' },
    { id: 'edit-make-component', label: t`Make Component`, description: t`Turn the selection into a reusable component definition.`, group: 'Actions', synonyms: [t`create component`, t`component`], gate: 'canMakeComponent' },
    { id: 'edit-place-copy', label: t`Place Copy`, description: t`Place another instance of the selected component.`, group: 'Actions', synonyms: [t`duplicate`, t`instance`, t`copy component`], gate: 'canPlaceCopy' },
    { id: 'edit-explode', label: t`Explode`, description: t`Break the selected component instance into plain objects.`, group: 'Actions', synonyms: [t`explode instance`, t`break component`], gate: 'canExplode' },
    { id: 'edit-make-unique', label: t`Make Unique`, description: t`Give the selected instance its own definition, detached from its siblings.`, group: 'Actions', synonyms: [t`unique component`], gate: 'canMakeUnique' },
    { id: 'edit-union', label: t`Union`, description: t`Merge two selected solids into one (boolean add).`, group: 'Actions', synonyms: [t`boolean`, t`merge`, t`combine`, t`add`, t`join`], gate: 'canBoolean' },
    { id: 'edit-subtract', label: t`Subtract`, description: t`Cut the second selected solid out of the first (boolean difference).`, group: 'Actions', synonyms: [t`boolean`, t`difference`, t`cut`, t`carve`], gate: 'canBoolean' },
    { id: 'edit-intersect', label: t`Intersect`, description: t`Keep only the volume the two selected solids share (boolean intersection).`, group: 'Actions', synonyms: [t`boolean`, t`common`, t`overlap`], gate: 'canBoolean' },
    { id: 'toggle-axes', label: t`Toggle Axes`, description: t`Show or hide the world axes.`, group: 'Actions' },
    { id: 'reset-axes', label: t`Reset Drawing Axes`, description: t`Reset the movable drawing axes back to world identity.`, group: 'Actions', synonyms: [t`drawing axes`, t`origin`, t`reset axes`] },
    { id: 'toggle-grid', label: t`Toggle Grid`, description: t`Show or hide the ground grid.`, group: 'Actions' },
    { id: 'toggle-guides', label: t`Toggle Guides`, description: t`Show or hide construction guides.`, group: 'Actions' },
    { id: 'toggle-section-active', label: t`Section Cut`, description: t`Turn the placed section plane's clip on or off without removing it.`, group: 'Actions', synonyms: [t`section`, t`cut`, t`section plane`, t`cross-section`, t`active cut`, t`toggle section active`, t`toggle section`] },
    { id: 'toggle-xray', label: t`X-ray`, description: t`See through every face, so hidden edges show and can be snapped to. Turns Wireframe off.`, group: 'Actions', synonyms: [t`x ray`, t`transparent`, t`see through`, t`translucent`] },
    { id: 'toggle-wireframe', label: t`Wireframe`, description: t`Show only the edges. Faces are not drawn and cannot be clicked. Turns X-ray off.`, group: 'Actions', synonyms: [t`edges only`, t`lines only`, t`hidden line`] },
    { id: 'zoom-extents', label: t`Zoom Extents`, description: t`Fit the camera to all scene geometry.`, group: 'Actions', synonyms: [t`zoom to fit`] },
    { id: 'toggle-parallel-projection', label: t`Parallel Projection`, description: t`Toggle between perspective and parallel (orthographic) projection.`, group: 'Actions', synonyms: [t`orthographic`, t`perspective`, t`ortho`] },
    { id: 'toggle-fly', label: t`Fly`, description: t`Fly through the model with no button held: the mouse or a trackpad finger looks, W A S D move, E and Q climb and descend, the wheel or a pinch sets the speed, Shift goes faster. Esc exits.`, group: 'Actions', synonyms: [t`fly`, t`wasd`, t`first person`, t`free camera`, t`noclip`, t`spectator`, t`flythrough`, t`fly through`] },
    { id: 'view-top', label: t`Standard View: Top`, description: t`Look straight down at the model.`, group: 'Actions' },
    { id: 'view-bottom', label: t`Standard View: Bottom`, description: t`Look straight up at the model.`, group: 'Actions' },
    { id: 'view-front', label: t`Standard View: Front`, description: t`Look at the model from the front.`, group: 'Actions' },
    { id: 'view-back', label: t`Standard View: Back`, description: t`Look at the model from the back.`, group: 'Actions' },
    { id: 'view-left', label: t`Standard View: Left`, description: t`Look at the model from the left.`, group: 'Actions' },
    { id: 'view-right', label: t`Standard View: Right`, description: t`Look at the model from the right.`, group: 'Actions' },
    { id: 'view-iso', label: t`Standard View: Iso`, description: t`Switch to the standard isometric view.`, group: 'Actions' },
    { id: 'toggle-tray', label: t`Toggle Tray`, description: t`Show or hide the right-hand tray and all its panels.`, group: 'Actions', synonyms: [t`hide tray`, t`show tray`, t`panels`, t`sidebar`, t`inspector`, t`more room`, t`hide panels`] },
    { id: 'toggle-dock', label: t`Toggle Contextual Dock`, description: t`Show or hide the floating action dock at the bottom of the viewport.`, group: 'Actions', synonyms: [t`dock`, t`action bar`, t`floating toolbar`, t`hide dock`, t`show dock`] },
    { id: 'toggle-compact-rail', label: t`Compact Tool Rail`, description: t`Shrink the tool rail to icons only, or bring the tool names back.`, group: 'Actions', synonyms: [t`tool rail`, t`toolbar`, t`icons only`, t`tool names`, t`collapse`, t`expand`, t`sidebar`] },
    { id: 'toggle-model-info', label: t`Toggle Model Info`, description: t`Show or hide the Model Info panel.`, group: 'Actions' },
    { id: 'toggle-materials', label: t`Toggle Materials`, description: t`Show or hide the Materials panel.`, group: 'Actions' },
    { id: 'toggle-components', label: t`Toggle Components`, description: t`Show or hide the Components panel.`, group: 'Actions' },
    { id: 'toggle-tags', label: t`Toggle Tags`, description: t`Show or hide the Tags panel.`, group: 'Actions' },
    { id: 'toggle-scenes', label: t`Toggle Scenes`, description: t`Show or hide the Scenes panel.`, group: 'Actions' },
    { id: 'scenes-add', label: t`Add Scene`, description: t`Save the current camera, visibility, and section cut as a new Scene.`, group: 'Actions', synonyms: [t`scene`, t`save view`, t`named view`] },
    { id: 'scenes-update', label: t`Update Scene`, description: t`Re-capture the active Scene’s checked properties.`, group: 'Actions', synonyms: [t`scene`, t`refresh scene`], gate: 'sceneActive' },
    { id: 'scenes-next', label: t`Next Scene`, description: t`Activate the next Scene in the list.`, group: 'Actions', synonyms: [t`scene`], gate: 'scenesAny' },
    { id: 'scenes-previous', label: t`Previous Scene`, description: t`Activate the previous Scene in the list.`, group: 'Actions', synonyms: [t`scene`], gate: 'scenesAny' },
    { id: 'scenes-transitions', label: t`Scene Transitions`, description: t`Toggle the 0.6s camera tween when activating a Scene.`, group: 'Actions', synonyms: [t`scene`, t`animation`, t`tween`] },
    { id: 'toggle-object-info', label: t`Toggle Object Info`, description: t`Show or hide the Object Info panel.`, group: 'Actions' },
    { id: 'toggle-changes', label: t`Toggle Changes`, description: t`Show or hide the Changes panel — this session’s undo/redo entries since the last save.`, group: 'Actions', synonyms: [t`history`, t`undo`, t`redo`] },
    { id: 'toggle-debug-log', label: t`Toggle Debug Log`, description: t`Show or hide the debug log panel.`, group: 'Actions' },
    { id: 'open-library', label: t`Library`, description: t`Browse and insert saved components, materials, and models`, group: 'Actions', synonyms: [t`library`, t`components`, t`insert`] },
    { id: 'save-to-library', label: t`Save Selection to Library…`, description: t`Save the selected object, group, or component as a library item.`, group: 'Actions', synonyms: [t`library`, t`component`, t`save selection`], gate: 'canSaveSelectionToLibrary' },
    { id: 'save-to-library-doc', label: t`Save to Library…`, description: t`Save the whole document as a library item.`, group: 'Actions', synonyms: [t`library`, t`component`, t`model`] },
    { id: 'make-solid', label: MENU.makeSolid, description: MENU.makeSolidHelp, group: 'Actions', synonyms: [t`health`, t`repair`, t`fix`, t`make solid`, t`close openings`, t`solid`, t`watertight`, t`check`, t`import`, t`thickness`] },
    { id: 'model-health', label: MENU.modelHealthItem, description: MENU.modelHealthHelp, group: 'Actions', synonyms: [t`health`, t`openings`, t`panels`, t`solid`, t`check`, t`import`] },
    { id: 'purge-unused', label: t`Purge Unused…`, description: t`Delete every unused palette material and component definition.`, group: 'Actions', synonyms: [t`cleanup`, t`clean up`, t`materials`, t`components`] },
    { id: 'open-settings', label: t`Settings…`, description: t`Open Hew Settings.`, group: 'Actions', synonyms: [t`preferences`] },
    { id: 'report-bug', label: t`Report Bug…`, description: t`Review and send a bug report, or save it to a file.`, group: 'Actions' },
    { id: 'open-guide', label: t`Hew Help`, description: t`Open the online user guide in your browser.`, group: 'Actions', synonyms: [t`help`, t`guide`, t`documentation`, t`docs`, t`manual`, t`learn`, t`user guide`] },
  ]
}
// oxlint-enable lingui/no-unlocalized-strings

/**
 * Action ids the menuActionRef switch handles that are DELIBERATELY not in
 * the palette, each with its reason. registry.test.ts enforces that every
 * switch case is either registered above or excused here — additions to the
 * switch fail the test until they pick a side, so the palette can't silently
 * drift out of date again.
 */
// oxlint-disable lingui/no-unlocalized-strings -- developer-facing reasons read only by registry.test.ts, never shown
export const PALETTE_EXCLUDED_ACTION_IDS: Record<string, string> = {
  'open-palette': 'self-referential — the palette cannot usefully open itself',
  'close': 'window lifecycle, desktop shells only — not a model action',
  'open-on-phone':
    'desktop shells only — the LAN server lives in the Tauri shell and the palette has no platform gate; the web build hides its menu item the same way (App.tsx passes onOpenOnPhone only under isTauri)',
  'enter-context': 'contextual-dock alias; needs a picked node, not a bare trigger',
  'ungroup': 'contextual-dock alias of edit-ungroup',
  'make-unique': 'contextual-dock alias of edit-make-unique',
  'explode-instance': 'contextual-dock alias of edit-explode',
  'toggle-library': 'keyboard/native-menu/dock-icon toggle alias of open-library — the palette exposes the single "open" verb, not a checkbox toggle',
}
// oxlint-enable lingui/no-unlocalized-strings

/** The full palette registry — tools first (spec ranks Tools above Actions
 * absent other signals), then hand-authored actions. */
export function paletteEntries(): PaletteEntry[] {
  return [...toolEntries(), ...actionEntries()]
}

/** The shortcut to display next to a palette result — the first key of its
 * command in the keymap, for tools and actions alike; '' when it has none. */
export function paletteShortcut(entry: PaletteEntry): string {
  return shortcutLabel(entry.id)
}
