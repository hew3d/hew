/**
 * Native menu labels — the one source for every label the desktop shell's
 * menu bar shows (docs/LOCALIZATION.md "Native menus").
 *
 * The shell (`shells/tauri/src-tauri/src/main.rs`) builds its menu from
 * ids and looks each label up in `shells/tauri/src-tauri/locales/
 * menu.<locale>.json`, which `scripts/i18n-menus.mjs` generates from this
 * table and the gettext catalogs — so a translator fills in one `.po` file
 * and the native menu follows. `nativeMenuParity.test.ts` checks that this
 * table and the Rust source name exactly the same ids, and that the
 * generated JSON is current.
 *
 * Keys: a menu item's id as `main.rs` registers it; `submenu:<name>` for a
 * submenu title; `predefined:<name>` for the macOS app-menu items Tauri
 * supplies (About, Hide, Quit, …) whose text the shell overrides so they
 * translate with the rest.
 *
 * The generator reads this file by its shape (one `'key': msg\`Label\`` or
 * `'key': msg({ context: '…', message: '…' })` per line), so keep entries on
 * one line each and keep the labels literal. The HTML menu bar
 * (`panels/MenuBar.tsx`) shows the same words, so these share catalog
 * entries with it rather than carrying a context of their own.
 */

import { msg } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'

export const NATIVE_MENU_LABELS: Record<string, MessageDescriptor> = {
  // Submenu titles
  'submenu:app': msg`Hew`,
  'submenu:open-recent': msg`Open Recent`,
  'submenu:file': msg`File`,
  'submenu:edit': msg`Edit`,
  'submenu:object': msg`Object`,
  'submenu:scenes': msg`Scenes`,
  'submenu:view': msg`View`,
  'submenu:draw': msg`Draw`,
  'submenu:tools': msg`Tools`,
  'submenu:object-smooth': msg`Smooth`,
  'submenu:shape': msg`Shape`,
  'submenu:deform': msg`Deform`,
  'submenu:standard-views': msg`Standard Views`,
  'submenu:camera': msg`Camera`,
  'submenu:window': msg`Window`,
  'submenu:help': msg`Help`,

  // macOS app menu (Tauri predefined items)
  'predefined:about': msg`About Hew`,
  'predefined:services': msg`Services`,
  'predefined:hide': msg`Hide Hew`,
  'predefined:hide-others': msg`Hide Others`,
  'predefined:show-all': msg`Show All`,
  'predefined:quit': msg`Quit Hew`,
  'predefined:minimize': msg`Minimize`,
  'predefined:maximize': msg`Zoom`,

  // App
  'app-settings': msg`Settings…`,
  'app-check-updates': msg`Check for Updates…`,

  // File
  'file-new': msg`New`,
  'file-open': msg`Open…`,
  'recent-clear': msg`Clear Recent`,
  'file-import': msg`Import…`,
  'file-import-image': msg`Import Image…`,
  'file-export': msg`Export…`,
  'file-print': msg`Print…`,
  'file-open-on-phone': msg`Open on Phone…`,
  'file-save': msg`Save`,
  'file-save-as': msg`Save As…`,
  'file-revert': msg`Revert to Saved`,
  'file-save-to-library': msg`Save to Library…`,
  'file-purge-unused': msg`Purge Unused…`,
  'file-close': msg`Close`,

  // Edit
  'edit-undo': msg`Undo`,
  'edit-redo': msg`Redo`,
  'edit-cut': msg`Cut`,
  'edit-copy': msg`Copy`,
  'edit-paste': msg`Paste`,
  'edit-paste-in-place': msg`Paste In Place`,
  'edit-paste-independent': msg`Paste independent`,
  'edit-paste-linked': msg`Paste linked`,
  'edit-delete': msg`Delete`,
  'edit-select-all': msg`Select All`,
  'edit-select-none': msg`Select None`,
  'edit-invert-selection': msg`Invert Selection`,
  'edit-delete-guides': msg`Delete Guide Lines`,
  'edit-group': msg`Group`,
  'edit-ungroup': msg`Ungroup`,
  'edit-make-component': msg`Make Component`,
  'edit-place-copy': msg`Place Copy`,
  'edit-explode': msg`Explode`,
  'edit-save-selection-to-library': msg`Save Selection to Library…`,
  'edit-make-unique': msg`Make Unique`,
  'edit-unlink': msg`Unlink`,
  'edit-unlink-all': msg`Unlink all`,
  'edit-link-identical-copies': msg`Link identical copies`,
  'edit-select-all-copies': msg`Select all copies`,
  'submenu:flip-along': msg`Flip Along`,
  'edit-flip-red': msg`Red`,
  'edit-flip-green': msg`Green`,
  'edit-flip-blue': msg`Blue`,
  'edit-union': msg`Union`,
  'edit-subtract': msg`Subtract`,
  'edit-intersect': msg`Intersect`,

  // Object
  'object-make-solid': msg`Make Solid…`,
  'object-model-health': msg`Model Health…`,
  'object-smooth-off': msg`Off`,
  'object-smooth-1': msg`1`,
  'object-smooth-2': msg`2`,
  'object-smooth-3': msg`3`,
  'object-bake-smooth': msg`Bake Smooth`,
  'object-soften-edges': msg`Soften Edges…`,

  // Scenes
  'scenes-add': msg`Add Scene`,
  'scenes-update': msg`Update Scene`,
  'scenes-next': msg`Next Scene`,
  'scenes-previous': msg`Previous Scene`,
  'scenes-transitions': msg`Scene Transitions`,

  // View
  'view-axes': msg`Axes`,
  'view-part-axes': msg`Part Axes`,
  'view-grid': msg`Grid`,
  'view-guides': msg`Guides`,
  'view-tangent-edges': msg`Tangent Edges`,
  'view-section-plane': msg`Section Cut`,
  'view-reset-axes': msg`Reset Drawing Axes`,
  'view-xray': msg`X-ray`,
  'view-wireframe': msg`Wireframe`,
  'view-compact-rail': msg`Compact Tool Rail`,
  'view-tray': msg`Tray`,
  'view-dock': msg`Contextual Dock`,
  'view-palette': msg`Command Palette…`,

  // Draw
  'draw-rectangle': msg`Rectangle`,
  'draw-circle': msg`Circle`,
  'draw-polygon': msg`Polygon`,
  'draw-arc': msg`Arc`,
  'draw-line': msg({ context: 'tool', message: 'Line' }),
  'draw-3d-text': msg`3D Text…`,

  // Tools
  'tool-select': msg`Select`,
  'tool-paint': msg`Paint`,
  'tool-position-texture': msg`Position Texture`,
  'tool-move': msg`Move`,
  'tool-rotate': msg`Rotate`,
  'tool-scale': msg({ context: 'tool', message: 'Scale' }),
  'tool-flip': msg({ context: 'tool', message: 'Flip' }),
  'tool-pushpull': msg`Push/Pull`,
  'tool-follow-me': msg`Follow Me`,
  'tool-eraser': msg`Eraser`,
  'tool-fillet': msg`Fillet`,
  'tool-chamfer': msg`Chamfer`,
  'tool-revolve': msg`Revolve`,
  'tool-loft': msg`Loft`,
  'tool-deform-twist': msg`Twist`,
  'tool-deform-bend': msg`Bend`,
  'tool-deform-taper': msg`Taper`,
  'tool-deform-bend-path': msg`Bend to Path`,
  'tool-path-array': msg`Array along Path`,
  'tool-shell': msg`Shell`,
  'tool-project': msg`Project`,
  'tool-offset': msg`Offset`,
  'tool-tape-measure': msg`Tape Measure`,
  'tool-dimension': msg`Dimension`,
  'tool-protractor': msg`Protractor`,
  'tool-slice': msg`Slice`,
  'tool-section-plane': msg`Section Plane`,
  'tool-edit-vertex': msg`Edit Vertex`,
  'tool-axes': msg`Drawing Axes`,
  'tool-text': msg({ context: 'tool', message: 'Text' }),

  // Camera
  'cam-orbit': msg`Orbit`,
  'cam-pan': msg`Pan`,
  'cam-zoom': msg`Zoom`,
  'cam-zoom-window': msg`Zoom Window`,
  'cam-zoom-extents': msg`Zoom Extents`,
  'cam-position-camera': msg`Position Camera`,
  'cam-walk': msg`Walk`,
  'cam-look-around': msg`Look Around`,
  'cam-fly': msg`Fly`,
  'cam-parallel-projection': msg`Parallel Projection`,
  'cam-view-top': msg`Top`,
  'cam-view-bottom': msg`Bottom`,
  'cam-view-front': msg`Front`,
  'cam-view-back': msg`Back`,
  'cam-view-left': msg`Left`,
  'cam-view-right': msg`Right`,
  'cam-view-iso': msg`Iso`,

  // Window
  'win-model-info': msg`Model Info`,
  'win-object-info': msg`Object Info`,
  'win-materials': msg`Materials`,
  'win-components': msg`Components`,
  'win-tags': msg`Tags`,
  'win-scenes': msg`Scenes Panel`,
  'win-changes': msg`Changes`,
  'win-library': msg`Library`,
  'win-debug-log': msg`Debug Log`,

  // Help
  'help-guide': msg`Hew Help`,
  'help-report-bug': msg`Report Bug…`,
  'help-sent-reports': msg`Sent Reports…`,
  'help-sent-reports-new': msg`Sent Reports ({n} new)…`,

  // Native window titles and dialogs the shell builds itself
  'window:settings': msg`Settings`,
  'dialog:choose-library-folder': msg`Choose Library Folder`,

  // Window list: a document window with no title yet
  'fallback:untitled': msg`Untitled`,

  // Updater: the window title and the two message boxes that stay native (an available update opens the update window, whose text is in the Lingui catalog); {error} is filled in by the shell
  'updater:title': msg`Update`,
  'updater:window-title': msg`Software Update`,
  'updater:in-progress-title': msg`Update in progress`,
  'updater:in-progress-body': msg`Hew is already checking for updates.`,
  'updater:up-to-date-title': msg`You’re up to date`,
  'updater:up-to-date-body': msg`Hew is running the latest version.`,
  'updater:start-failed-body': msg`Could not start the updater.\n\n{error}`,
  'updater:check-failed-body': msg`Could not check for updates.\n\n{error}`,
}
