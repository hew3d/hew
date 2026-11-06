# Changelog

Installers for macOS, Windows, and Linux and the self-hosting tarball
are on each version's [GitHub release page](https://github.com/hew3d/hew/releases).
The same build runs in the browser at [app.hew3d.com](https://app.hew3d.com).

A newer Hew always opens documents saved by an older one. The reverse
is not guaranteed: releases that change the `.hew` format say so under
**Changed**.

## [Unreleased]

### Added

- **SketchUp files:** Hew opens and saves every SketchUp release from 3 to 8 and from 2013 to 2020 (the stream container, each saved in its own release's layout) as well as the 2021-and-later container. Save writes back in the format a file was opened in; Save As and Export offer the 2021-and-later format and an "Older SketchUp versions" group. A file is read by its content, not its header's version label. What an older release cannot hold is reported in a short list after the save.
- **SketchUp files:** A 2021-and-later save is written by content at revision 17 or 18. The "Avoid features older SketchUp versions cannot open" option (`compat` on `hew.doc.save` and `hew.doc.export`) writes revision 17, leaving out layer folders, hatch patterns, PBR materials, environments, snaps, procedures, page extras and the coordinate system. Revision, not the header's version, decides what SketchUp opens: revision 18 and below open in SketchUp 2027, and SketchUp 2027 desktop refuses a hand-edited file of revision 19 or above (the web app accepted it; no real file above 18 exists). A `compat` save to a target other than the 2021-and-later container is refused with `format_mismatch`. SketchUp 2021 to 2025 are supported: they use the same container, and the files to hand them are the revision 17 ones.
- **SketchUp files:** Layer folders map to tags and back. A scene's timing, a guide's dash and the section plane's look are kept.

- **Drawing:** Tap Ctrl (⌘ on a Mac) on any draw tool to detach the shape from the face or drawing under it. Push/Pull turns a detached shape into a new object instead of changing the face. The cursor shows a small + while Detach is on, and the outline being drawn is orange when the shape will join the face and blue when it stays detached.
- **Drawing:** Drawing now has two colors. Blue drawing belongs to no solid and pushes into a new object; orange drawing is part of a solid's face and pushes the solid itself. The outline you draw on the ground or in empty space is blue, and one you draw on a face is orange until you detach it. Push/Pull ghosts follow the same rule: orange where the solid grows, red where it loses material, and blue where a new object appears, including when you push a free shape inward. Offset shows its preview in the color of the shape it starts from.

- **SketchUp files:** Guide lines keep their dash pattern (dashed, dotted, long dashes, dash-dot or any other), draw with it, and save back with it. A camera set to a photo (Match Photo) keeps its image width and ratio through a save back until you move the view. A file's section-cut outline color and width are drawn, and its section fill, active section color, guide color and guide visibility settings are kept. Hew's own format is now manifest format 24.

- **SketchUp files:** A model's geo-location (its coordinate reference system: name, datum, projection, unit, and the origin's easting, northing and height) now opens with the file, shows at the top of Model Info, and is written back to a SketchUp 2021-and-later file with every field the file held. It is also stored in Hew's own format (manifest format 25) and can be read and set through `hew.geo.get` and `hew.geo.set`, one undo step each. Older SketchUp formats have no such record and leave it out.

- **SketchUp files:** Physically based materials open with the file: a material's metallic and roughness settings are drawn in the viewport (approximated with a Phong shading model), along with its metallic, roughness, normal and occlusion maps, and a save back writes them with everything else the file held about the material (workflow, enabled maps, normal-map style). The material palette has Metallic and Roughness sliders, and `hew.material.set_pbr` sets them, one undo step each. They are also stored in Hew's own format (manifest format 26).

- **SketchUp files:** Opening a file that holds things Hew keeps through a save back but cannot show or edit (hatch patterns, environments, snaps, procedures, layer line styles, the current style record, layer page behaviours) now says so in one line of the open report, with a count for each kind.

### Changed

- **Format:** The `.hew` format moves from manifest version 23 to 26 (guide dash and section look, geo-location, physically based materials). Hew opens every older file, but a file saved with these features needs this release or later.

### Fixed

- **Push/Pull:** The hint for the Ctrl/⌘ new-object toggle now refreshes the moment you tap the key.

## [1.3.1] (2026-10-26) - The "Safe to Reopen" Release

A fix for several ways Hew could save a file it then could not open, or stop
with an error when it saved. If you wrapped part of a group in a component
while editing that group, closed the group, and saved, the file could come
back with "This file is damaged and can't be opened." Ungrouping inside a
component you were editing could do the same, or crash Hew at the next
autosave. Nested editing, where one group or component is open inside another,
had a few more ways to get there. All are fixed.

### Fixed

- **Components:** Making a component out of something inside a group you are editing, then closing the group, left that item listed in both places. The file saved, and then would not open. Closing the group now leaves the item in its new component.
- **Components:** Deleting every part of a component while editing it used to delete the component, even when another component still held a copy of it. That left the copy pointing at nothing, and the saved file would not open. Such a component now stays, empty, until you delete the one that holds it.
- **Components:** Ungrouping a group inside a component you were editing took the group's contents out of the component's list, so Hew could stop with an error the next time it saved, autosave included, or save a file that would not open. The contents now stay in the component.
- **Groups:** Grouping some items inside a group you were editing, then ungrouping them again, listed those items twice when you finished editing, and the saved file would not open. They now stay as they were.
- **Components:** Placing a component while a group inside that same component was open could put the component inside itself, and a long enough chain of components inside components could go past the depth Hew allows. Both saved a file that would not open. Hew now refuses the placement instead.

## [1.3.0] (2026-10-24) - The "Second Nature" Release

More tools now do what a SketchUp hand expects without thinking about it.  Every
two-point tool takes click-drag-release as well as click-click, a typed length
goes straight up the blue axis, Standard views keep your zoom, an arrow key
after the first click still locks the plane, and two objects' edges snap where
they cross. And a model can start from a picture: drop a floor plan or a photo
on the viewport, size it by drag, by a typed width, or with the Tape Measure,
and trace over it.

Also new: tools for examining a model. X-ray and Wireframe lets you see inside a
model and a new selection path "breadcrumb" shows you exactly what nested object
you are interacting with.

And more: Revert to Saved, a Precision setting that marks a rounded readout, a
View pane in Settings, sorted and filtered panels, and an interface that is
ready to be translated. Eight user-reported bugs are fixed, along with three
kernel faults found by fuzzing.

### Added

- **Import Image:** File ▸ Import Image…, or a PNG or JPEG dropped on the viewport, places a reference picture on the ground or on any face. A click sets one corner, and dragging or typing a width sets its size with the proportions kept; the Tape Measure resizes it afterward like any part.
- **Selection:** A strip at the top of the viewport reads the path from Model down to what the click landed on (Model › Kitchen › Base Cabinets › Door). Click a crumb to select that level, double-click to open it. With the Select tool idle, the hover hint names the object under the cursor and its parent, and Object Info shows the same path.
- **Draw:** Click-drag-release works for every tool with a start point and an end point, alongside click-click. A press and a release on the same spot is still a click. [#52]
- **View:** X-ray and Wireframe, on the View menu and in the command palette. X-ray draws every face see-through, and everything behind a face can be snapped to and selected; Wireframe draws edges only. The mode is per window and not saved in the file, as in SketchUp. [#51]
- **Snapping:** Where one object's edge crosses another's, the crossing is a snap point. Line starts from it, and the Tape Measure measures from it instead of starting a guide. [#51]
- **Units:** A Precision choice, kept per display format. Meters, centimeters, millimeters, and decimal inches offer zero to six decimals; fractional and architectural inches offer 1' down to 1/64"; every format also offers Exact. A readout that was rounded carries a leading `~`, and one that was not shows no mark. The defaults are the rounding each format always had.
- **Settings:** A View pane: zoom wheel speed, smooth camera moves on or off, the inference hint and the tool pointer at the cursor, and the Measurements box at the top or the bottom of the viewport. [#26]
- **Settings:** A Language setting. Every string in the interface, the desktop menus included, now comes from a translation catalog, so Hew can be translated without touching code. English is the only language offered so far; a Japanese catalog is in the repository waiting for a translator, and `docs/LOCALIZATION.md` says how to add one. [#54]
- **File:** Revert to Saved, after Save As, re-reads the saved file into the same window after one confirmation. The API gains `hew.doc.revert`.
- **Panels:** The Materials palette and the Components panel list by name. The Outliner has a sort control (document order, name, kind) next to its filter, and the Tags pane gets the same filter box the other panels have.
- **Object Info:** Clicking a tag chip selects that tag in the Tags pane. A new Materials row lists every material on the selection, a group's or component's contents included; clicking one selects it in the Materials pane.
- **Push/Pull:** The preview is tinted by mode: orange for a plain pull, blue for extrude-as-new, red for a cut. Toggling the mode mid-gesture changes the color. [#41]
- **Keyboard:** Tool shortcuts follow the letter a key produces, so AZERTY, Dvorak, and other layouts keep their key caps; a key that produces no letter falls back to its physical position.

### Changed

- **Camera:** Top, Bottom, Front, Back, Left, and Right keep the current zoom and orbit target, so switching views no longer changes the on-screen scale. Iso still frames the whole model. [#52]
- **Draw:** A circle touching a face's edge at exactly one point now makes a face, like a circle that crosses the edge. The face keeps its hole where the circle touches. [#43]
- **Line:** Clicking the last placed point again finishes the chain whatever the pace. It used to be refused as the same point unless the click came fast enough to count as a double-click. [#36]
- **Dimensions:** Arrowheads are a fixed size on screen, tied to the label text, and never cover more than about 60 percent of the line. A 2 cm dimension used to carry two heads that crossed over each other. Label ink is the color the theme intends; the light theme's text had read mid-gray. [#36]

### Fixed

- **Draw:** A typed length with the cursor straight above or below the first point did nothing, for Line and for the Arc chord, and a click there was refused or ignored. Both now go up the blue axis. [#51]
- **Draw:** An arrow key pressed after the first click on a corner where faces meet was dropped, so a shape on a box at the origin could not be moved to the ground. It now moves the shape onto the locked plane, as pressing the arrow before the click does, and holding an arrow no longer toggles the lock on every repeat. [#51]
- **Library:** Typing a name with digits in Save to Library retyped the last rectangle. Keys that land in a text field or a dialog no longer reach the active tool. [#36]
- **Groups:** Opening any unnamed group showed "Editing Group 1". The label now matches the Outliner's numbering, and the selection path follows it.
- **Command palette:** After a click on the window's title bar or a scrollbar, the arrow keys and Enter went dead and bare-letter tool shortcuts fired instead. [#38]
- **Undo:** Saving after undoing and redoing the deletion of a component crashed when a sketch had been drawn into the component between the two. A definition's sketches now follow the definition's own hidden flag.
- **Undo:** Undoing a paint after a split and its undo left the paint in place; redoing a paint on half of a split after both were undone left that half bare; and a Position Texture, Replace Material, or Delete Material whose face had been rebuilt refused every undo. Each recorded face is now found again by its geometry.
- **Kernel:** A cut through a face whose hole touches the outer edge at one point was accepted or refused depending on which edge the hole touched and which way the cut was drawn, and a cut that went through could fail to redo. A hole's side is now decided from its own points.
- **Export:** A cylinder band beside a tangent circle exported to STL at its stored facets instead of re-faceted, and a loose line lying along an edge of a face was freed by a boolean or left behind by a push on two of the face's four sides.
- **Materials:** An imported textured face slid its texture when it was moved, rotated, or opened, and a resized picture tiled its image instead of stretching it.
- **Desktop:** On Linux, the first click after moving, resizing, or refocusing the window by its title bar was ignored.

### Security

- **Desktop:** Updated rustls-webpki to 0.103.16 and webpki-root-certs to 1.0.10 for RUSTSEC-2026-0382 and -0383, and replaced the yanked clipboard-win 5.4.1 with 5.4.3.
- **All:** Updated tokio to 1.53.3 for RUSTSEC-2026-0386, Vite to 8.2.4, and Astro to 7.3.3.

## [1.2.3] (2026-10-19) - The "Room to Spare" Release

Big SketchUp files now open. v1.2.2 made Hew say clearly when a model ran it
out of memory; this release makes that far less likely. Opening a `.skp` takes
a fraction of the memory it did, so the furnished house that v1.2.2 could only
refuse now opens with room to spare. Also fixed: double-clicking a `.hew` file
on macOS while Hew wasn't running.

### Fixed

- **Desktop:** Double-clicking a `.hew` file on macOS while Hew was not running started Hew but never opened the file. The open arrived before the app had finished starting and was dropped; it now waits for the app to be ready.
- **Open:** A large SketchUp model takes far less memory to open, so models that ran Hew out of memory now open. A furnished house of 1.8 million faces (a 443 MB `.skp`) needed more than the 4 GB Hew can use; it now opens with room to spare. Four things cost that memory and no longer do: a model's loose geometry came in as thousands of pieces, each carrying the whole model's list of soft edges; the snapping index copied every component's geometry at open, where it now reads a component the first time a snap or pick reaches one of its copies; the kernel kept a second copy of every render mesh the view already held; and the document's element storage was sized by doubling. Every edit on such a model also serialized the whole document for a diagnostic digest, seconds of work and a second copy of the model each time; a document of more than 50,000 faces now skips the digest. The log line for an open records the kernel's memory.

## [1.2.2] (2026-10-18) - The "Big Files Too" Release

A bug report about a SketchUp file that wouldn't open specifically because it
was too big would never arrive, since there was an arbitrary 90 MB limit. A file
that big typically means Hew ran out of memory opening it, and Hew didn't make
that clear either. It does now.

### Fixed

- **Open:** A model that needs more memory than Hew can use (4 GB) used to end on an internal-error screen quoting "recursive use of an object". Now the screen says the model is too large to open, how big the file was, and what may bring it down to size in SketchUp. The same goes for an import.
- **Report Bug:** A report can now carry up to 1 GB compressed, up from 90 MB, so the SketchUp files large enough to run Hew out of memory arrive with the report about them. A report that still has to leave out the file that didn't open now says so, and names the file, its size, and the error it failed with. Such a report used to arrive with no sign that anything had been left out.

## [1.2.1] (2026-10-17) - The "File That Didn't Open" Release

A bug report about a SketchUp file that wouldn't open arrived without the file,
and with no way to ask for it. Now a file that won't open goes with the report,
and a file that only half-downloaded says so instead of failing in confusing
ways.

### Added

- **Report Bug:** When a file won't open, Help ▸ Report Bug lists it as **File that didn't open** and sends it byte for byte unless you untick it. That covers a `.skp`, an imported file, and your own `.hew`. After an internal error, the banner offers Report Bug next to Reload, since reloading forgets the file.

### Fixed

- **Open:** A file cut short by a download or copy that didn't finish says so and asks for a fresh copy. A cut-short 2017 `.skp` used to open as an empty model. Now it's refused when none of it can be read, and opens with a notice that parts may be missing when some of it can. A cut-short 2026 `.skp` or `.hew` no longer says it isn't a SketchUp file or a Hew document.
- **Format:** A saved `.hew` records the Hew release that saved it. Every release through 1.2.0 wrote 0.1.0. The format is otherwise unchanged, so 1.2.0 opens files saved by 1.2.1.

### Security

- **All:** Updated the zip crate to 9 for RUSTSEC-2026-0367, -0368, and -0369. The desktop updater no longer carries the older zip at all.
- **Desktop:** Updated rustls to 0.23.46 for RUSTSEC-2026-0346 through -0349.

## [1.2.0] (2026-10-12) - The "Going Native" Release

SketchUp files become a *working format* in this release and no longer just a
format that can be imported. Open a 2017 or 2026 `.skp` file as a document, see
that it looks the way it does in SketchUp, then save it back in the format it
came in. This does mean both read and write for both formats (imported 3D
Warehouse components now work) and features not yet supported in Hew are
persisted as-is, so those bits aren't lost. You can essentially treat Hew as an
app for editing SketchUp files natively.

Also, drawing on a solid's face now works the way drawing on the ground does,
and you can fly through a model with classic WASD controls.

### Added

- **SketchUp files:** A `.skp` opens as a document and Save writes it back in the format it came in: the 2026 format every current SketchUp saves, or the 2017 one. Save As and Export write either. Open Recent and drag-and-drop take `.skp` too, and on macOS Hew appears under Open With.
- **SketchUp files:** An opened model looks as it does in SketchUp, scene by scene: cameras, hidden objects and tags, section cuts, the style's backdrop and axes, guides, dimensions and leader text, images, paint on a face's back, and lines that bound no face.
- **SketchUp files:** The first save to `.skp` that has to simplify something lists what changed, once per document, with a button that saves a `.hew` copy.
- **SketchUp files:** Large models open in steps behind a progress bar, and much faster than before.
- **Make Solid:** Object ▸ Make Solid… and the Model Health window show which imported objects aren't solid and why, then close openings, rejoin split shapes, and thicken panels, one kind at a time.
- **Fly:** Hold the right mouse button to fly: the mouse looks, W, A, S, and D move, E and Q climb and descend, Shift sprints, and the wheel sets the speed. Camera ▸ Fly keeps flying without the button. A tool in the middle of a shape keeps going underneath.
- **Draw:** A solid's face works like the ground. Shapes drawn there can touch, cross, or overhang its edges and split the face into regions, and a shape hanging over the edge becomes a part that pushes or pulls the solid plus a part that makes a new object.
- **Layout:** The tool rail shrinks to icons, the tray hides, and the contextual dock can be turned off, from the View menu or the command palette. [#22]
- **API:** The live API reaches every open window: list, open, close, focus, and attach documents by id, and address one call with `--document` in hew-cli. `hew.doc.import` works live.
- **API:** Create textured materials, position a texture on a face, replace one material with another, and save materials to the Library and insert them.
- **API:** `hew.doc.health` and `hew.solid.cap`, `thicken`, `delete_faces`, and `mark_surface` do what Make Solid does.

### Changed

- **Format:** The `.hew` format moves from manifest version 17 to 22 and from geometry version 7 to 8. Hew 1.2 opens every older file, but a file saved by 1.2 needs Hew 1.2 or later.
- **Camera:** Right-drag flies instead of panning. Pan with Shift and a middle drag, or with the Pan tool.
- **SketchUp files:** A `.skp` from a release the reader can't decode yet (anything but 2017 or 2026) is refused with its version and both ways to re-save it: in any current SketchUp, or as SketchUp 2017.

### Fixed

- **Rectangle:** On a face or a wall, the W,D readout and a typed pair read width across the surface, then height up it, in the order typing them back builds. A rectangle typed on a wall grows right and up as the wall reads on screen. [#20]
- **Object Info:** Changing a circle's Segments repaints at once, and an object given a hidden tag hides at once instead of after the next unrelated change. [#31]
- **Command palette:** Enter runs the highlighted row, hovering no longer steals the selection after a re-rank, and clicking empty space in the dialog keeps the keyboard in the search field. [#38]
- **Draw:** A value clicked on a face is exactly the value the measurements box showed.
- **API:** `hew.meta.documents` listed no documents.

## [1.1.0] (2026-09-16) - The "User Feedback" Release

Hew had no users before 1.0 and so this is the first one incorporating
user feedback... and there was a lot of that! As a result, this release
has the most changes of any Hew release to date.

Most of these changes are "muscle memory" changes for SketchUp users: type a
size after a click, Shift to pan mid-orbit, *all* selection modifiers, and
Cut and Paste In Place (which also works in groups).

Plus, shapes drawn on solids are now editable, history has names and is
visible, materials and components can be tidied, and there are even more
ways to install on Linux and macOS (and Windows per-machine installs
work, too).

### Added

- **Draw:** Shapes drawn on a solid's face are editable before they are pushed: select, move, rotate, scale, offset, or delete them on the face. [#9]
- **Typed sizes:** Every tool accepts a typed measurement after its committing click too, and the shape redraws in place as one undo step.
- **Draw:** Hold Shift to pin the drawing plane under the cursor until Shift or Escape releases it (like the docs already claimed), so a shape can stand in empty space on that plane. [#14]
- **Edit:** Cut, Copy, Paste, and Paste In Place for the selection, with the clipboard shared across desktop windows. [#18]
- **Groups:** Move parts into or out of a group by dragging in the Outliner, or by Cut, open the group, Paste In Place.
- **Selection:** Shift toggles, Ctrl/Cmd/Option adds, Shift with either subtracts; a triple click selects a connected shape; Select None and Invert Selection.
- **Camera:** Shift during an orbit drag switches to pan. Hold Ctrl/Cmd for a precise camera with no inertia; pressing it also stops a coasting view. [#5]
- **Undo:** Undo and Redo name what they will do, a Changes panel shows the session's timeline, and the unsaved-changes prompt lists what would be lost.
- **Draw:** Circles and arcs on a face stay true circles, with center, quadrant, and tangent snaps and smooth walls when pushed.
- **Outliner:** A filter field, a Model row that hides or shows everything, and show-all/hide-all on every group.
- **Materials:** Rename and delete materials from the Materials panel.
- **Components:** A Components panel lists every definition with a thumbnail and instance count, and can rename or delete one.
- **Purge Unused:** Remove unused materials and component definitions in one undoable step.
- **Report Bug:** Help ▸ Report Bug previews what a report contains, lets you untick parts, scrubs home paths, and sends or saves it. The crash screen can save a reproducer.
- **macOS:** A Homebrew tap: `brew install --cask hew3d/tap/hew` installs the app and puts `hew-cli` on the PATH. [#15]
- **Linux:** A signed pacman repository for Arch, a Gentoo overlay, and a signed Flatpak repository with a bundle on each release.
- **Windows:** The installer offers a per-machine install into Program Files as well as the per-user default.
- **API:** `hew.library` commands list, insert, save, and manage Library items from hew-cli and the MCP server. Groups get `hew.group.reparent`.
- **Docs:** Every Learn chapter opens with a captured demo clip, and a new section covers drawing on planes other than the ground.

### Changed

- **Push/Pull:** Hover picks the face first, so the cue near a corner no longer reads "Endpoint" and the drag anchors on the face being pushed.
- **Move and Rotate:** The copy toggle is Option on macOS and Ctrl on Windows and Linux. It was Option everywhere.
- **Camera:** Orbit and zoom pivot on the geometry under the cursor, so zoom no longer stops short of a detail on a large model.
- **Viewport:** Renders only when something changes. Idle CPU dropped about threefold in the foreground and tenfold in the background.
- **Saving:** Undo back to the saved state reads clean, so a change followed by its undo no longer prompts to save.
- **Follow Me:** Sweeps exactly the selected path; a single selected segment is no longer expanded to its connected run.
- **Linux:** The `.deb` carries a description, a desktop category, and the `.hew` file association; the association also applies on macOS and Windows.

### Fixed

- **Undo:** Undoing a vertex drag after undoing the extrusion that followed it crashed the kernel. [#12]
- **Draw:** A closed Line chain drawn on a face landed on the ground instead. Interior chains now imprint a sub-face, and chains that touch the boundary cut it per stretch. [#8]
- **Push/Pull:** A drawn shape standing in front of a solid's face lost the pick to the face behind it, with no way to choose. What is in front now wins. [#13]
- **Snapping:** On a face, the point snapped is the one the inference chip names, and an axis crossing an edge snaps as an intersection, so a corner-to-midpoint shape closes without a gap. [#11]
- **Push/Pull:** Pushing past a nearer wall facing the same way, or pulling out past a wall ahead, was refused. Both now carve or merge.
- **Draw:** A shape with one side along the face's own edge was refused; it now splits the face along that chord.
- **Draw:** A Line started on an edge or corner fell to the ground plane instead of the face the edge belongs to.
- **Snapping:** A ranking panic on dense floor plans broke every later snap until reload. Stacked corners in a plan view resolve to the nearest.
- **Dimensions:** In a Top view a dimension could land one endpoint on the floor. The first click now freezes the plane.
- **Units:** Imperial lengths with no integer part (`.75"`, `-.5`) were refused.
- **macOS:** Cmd+C, Cmd+V, and Cmd+A did not work in rename fields or in the Settings and Library windows.
- **Materials:** Double-clicking a material that was not yet selected did not open the rename editor.

### Security

- **Desktop:** Updated rustls to 0.23.45 for RUSTSEC-2026-0285.
- **All:** An audit of the Report Bug intake and everything new since 1.0. An upload is bounded in size, shape, and time, a guessed report id costs the service nothing, the log scrub covers any home folder, and a malformed arc claim is refused.

## [1.0.0] (2026-08-24) - The "Rock Solid" Release

Hew is now a use-it-every-day modeler: free, open source, and the same full app
on macOS, Windows, Linux, and in any modern browser with no account and no
reduced web tier.

The Library now works in the browser, the menus get a proper layout, and a
full-stack audit closes out the 0.x series.

### Added

- **Library:** Works in the browser, stored in the browser's own storage with nothing to set up.
- **Library:** On Chrome, Edge, and Brave, bind the Library to a folder on disk and let Dropbox, Nextcloud, or iCloud Drive carry it between machines.
- **Help:** Help ▸ Search finds any command by name.

### Changed

- **Menus:** One HIG-shaped layout on every platform. Object commands move from Edit to a new Object menu, panels move to View, and Draw and Tools are grouped by task.
- **Docs:** The website and user guide are rewritten for SketchUp users, with a feature-by-feature [comparison](https://hew3d.com/compare/sketchup/).
- **Tools:** The Axes tool is now Drawing Axes, so it no longer collides with View ▸ Axes.

### Fixed

- **Groups:** A group holding a component instance now moves as a whole.
- **Booleans:** Booleans on imported geometry no longer refuse themselves.
- **Web:** Two browser tabs can no longer overwrite each other's crash-recovery snapshot.
- **Rendering:** A sliver triangle no longer produces a bad face.

### Security

- **All:** An audit across the kernel, importers, relay, desktop shell, and browser build. A malformed model can no longer overflow an importer or pass a bad number into the kernel.

## [0.10.0] (2026-08-19) - The "Put It On Paper" Release

Hew can print. No, not 3D printing - paper, or a PDF, at an exact drawing scale.
Scaled floor plans, and 1:1 templates tiled across pages for gluing to a board
as a cut guide. SketchUp Make never had printing this good!

### Added

- **Print:** File ▸ Print Layout, with a Standard mode that prints what you see and a Scaled mode at any drawing scale (1:10, 1:1, 1" = 1').
- **Print:** A template larger than the page tiles across sheets with trim marks and an overlap margin for gluing.
- **Print:** Save PDF writes the PDF itself, so vectors stay vectors on every platform. Printing goes through the OS dialog at 100% / Actual Size.
- **Print:** Line art (white faces, black edges) alongside the full-color view; in Scaled mode it is true vector with hidden lines removed.
- **Export:** SVG line drawings, for laser cutters and CNC.
- **Print:** A cut list page lists every part with quantity and L × W × H.
- **Print:** Print every Scene as one drawing set, and a print sheet in Shop Mode.
- **API:** Printing and PDF export are available to hew-cli and agents.

### Security

- **All:** Updated the h2 crate to 0.4.16 for RUSTSEC-2026-0258.

## [0.9.0] (2026-08-17) - The "Making the Scene" Release

Scenes save a named view of a model. This includes the camera, what is hidden,
and the section plane. Set up the views you need before heading to the shop and
switch between them on the phone.

Also, self-hosting now covers Open on Phone, so a transfer can stay entirely on
your own network.

### Added

- **Scenes:** Named snapshots of the camera, hidden objects and tags, the section plane, and the grid, axes, and guides. Usable from Shop Mode and the API.
- **Self-hosting:** `hew-relay`, a static Linux binary that serves Open on Phone from your own origin, with a container image and a Proxmox LXC installer.
- **Desktop:** A setting points the Open on Phone QR code at your own server.
- **Tags:** Select everything a tag carries, rename a tag in place, and tag several objects at once.
- **Shop Mode:** The document menu lists recent models.

### Changed

- **File format:** Bumped to version 16 for Scenes and the section plane. Documents saved by 0.9.0 do not open in 0.8.0.
- **Desktop:** Opening a file already open in another window switches to that window.

## [0.8.0] (2026-08-15) - The "Hew on your Phone" Release

How can Hew be genuinely useful on a small screen? Enter "Shop Mode," a
dedicated mode, laser-focused on smartphones and touch screens. Tap a part for
its size, check the parts list, or see it in AR (iPhone/iPad only). Get the
file to the phone through any synced folder, or scan a QR code from the desktop
app for an end-to-end encrypted handoff.

Self-hosting the web app is more accessible, with a solid guide and a Proxmox
LXC script.

### Added

- **Shop Mode:** A read-only, touch-first viewer. Tap a part for its dimensions, browse a live parts sheet, measure with a loupe, isolate a part, and keep recent models offline.
- **Desktop:** File ▸ Open on Phone shows a QR code that loads the model in Shop Mode. The key travels only in the QR code; the relay sees ciphertext.
- **Export:** USDZ, and View in AR in Shop Mode on an iPhone or iPad.
- **Self-hosting:** A guide and a Proxmox LXC script for running the web app on your own machine.

### Fixed

- **Web:** Muted cross-origin errors no longer produce useless reproducer dumps on iOS home-screen installs.

## [0.7.2] (2026-08-11) - The "Update the code to match the docs" Release

Rewriting the Getting Started tutorial exposed a shortcut that differed between
platforms.

### Changed

- **Shortcuts:** The command palette is `Cmd+/` on macOS and `Ctrl+/` everywhere else.
- **Docs:** The [Getting Started](https://hew3d.com/learn/getting-started/) tutorial is rewritten to be easier to follow.

## [0.7.1] (2026-08-09) - First Bug Fix Release

### Fixed

- **Snapping:** Projected inference points were ignored under an axis lock, a regression from 0.4.0.

## [0.7.0] (2026-08-08) - Shared Library and Nested Components

Some parts are too useful to live in one model. The Library keeps components,
materials, and whole models for reuse, and components can finally contain other
components.

SketchUp imports honor groups and components properly.

### Added

- **Library:** Save components, materials, and models as plain `.hew` files in a folder you choose, then insert them into any document or open them to edit.
- **Components:** Definitions nest. A component can hold other components and groups, and all of them edit in place through any instance.

### Changed

- **File format:** Bumped to version 15 for nested definitions. Documents saved by 0.7.0 do not open in 0.6.0.
- **Import:** A plain SketchUp group imports as an Object, a nested group as a Group, and a component as a Component. Groups used to arrive as plain objects and components as componentized plain objects.
- **Build:** Every dependency is pinned and audited daily, so security fixes reach releases promptly.

## [0.6.0] (2026-08-05) - Public API and MCP

Hew can be driven from a shell, a script, or an AI agent. A new `hew-cli` builds
models headless or writes into the desktop document you already have open.

### Added

- **API:** The Hew API and `hew-cli`, which builds and edits models with no UI. `--live` drives the open desktop document instead.
- **MCP:** An MCP server in `hew-cli`, so an AI agent can model in Hew. See [docs/API_GUIDE.md](docs/API_GUIDE.md).
- **Desktop:** `hew-cli` ships inside the app on every platform. On Linux, prefer the `.deb` or `.rpm` over the AppImage to get it on the PATH.
- **Data:** Every entity has a stable ID that survives undo and save, and can carry attribute dictionaries for scripts.

### Changed

- **File format:** Bumped to version 14 for stable IDs and attributes. Documents saved by 0.6.0 do not open in 0.5.0.
- **Export:** glTF, STL, and 3MF share one writer and all keep colors.

## [0.5.0] (2026-08-01)

Groups and components used to support tools one at a time, so some tools did not
work inside them and others worked differently. Now they become plain geometry
for the length of an edit and every tool works.

### Added

- **Groups and components:** Every tool works inside an edit. A group's copy is a fresh copy; a component's copy is another instance.
- **Tape Measure:** Works on any plane and honors axis locks, modeled on SketchUp.
- **Tape Measure:** Type a new length inside a group or component edit to rescale just that part instead of the whole model.

## [0.4.0] (2026-07-30)

3D text, dimensions, a full camera system, texture positioning, and modeling
inside component definitions. One step closer to SketchUp parity.

### Added

- **3D Text:** Extruded solid text on any face or plane, with five bundled open-license fonts plus your system fonts.
- **Dimensions:** Linear and radial dimensions and leader text, saved with the document and legible at any zoom.
- **Camera:** Parallel Projection, typed Field of View, Zoom Window, and Position Camera, Look Around, and Walk.
- **Components:** Sketching, extrusion, Follow Me, booleans, slice, and transforms all work inside a definition and apply to every instance.
- **Textures:** Position Texture drags corner pins to move, rotate, scale, and shear a texture on a face, or type an exact angle or scale.
- **Paint:** Alt-click samples a face's material; Shift-click replaces that material everywhere, or within the clicked object with Ctrl/Cmd.
- **Rotate:** Rotational arrays, matching the linear arrays in Move.
- **Push/Pull:** Double-click a face to repeat the last distance; Ctrl/Cmd extrudes a new solid instead of stretching the existing one.
- **Tape Measure:** Measure between two points and type the length it should be to rescale the model.
- **Axes:** Place and orient the drawing origin anywhere on the model.
- **Axis locks:** An arrow key sets the lock at any point up to a shape's second click.
- **Materials:** A searchable palette, with the add-color and add-texture panels collapsed until needed.
- **Snapping:** The label says "projected" when a point is used as an alignment reference rather than as the point itself.

### Changed

- **File format:** Bumped to version 13 for annotations, the camera, and drawing axes. Documents saved by 0.4.0 do not open in 0.3.0.

### Fixed

- **Input:** Double-click did not register in Chromium-based browsers or the Windows desktop app.

## [0.3.0] (2026-07-22)

Sketches leave the ground, STL comes in, Follow Me grows up, and the desktop app
opens more than one window.

### Added

- **Draw:** Draw on any plane. Extend any sketch in place, or lock a drawing plane with the arrow keys.
- **Import:** STL, binary and ASCII, with healing, hollow-part reconstruction, and a units chooser.
- **Polygon:** Regular N-gons with a live typed side count, plus selectable polygon and circle centers.
- **Scale:** Non-uniform scale through a grip gizmo: stretch one axis, two, or all three.
- **Section Plane:** A non-destructive clipping plane for looking inside a model, with an offset-sweep widget.
- **Desktop:** Multiple windows, one Open dialog for every format, and opens that go to a new window instead of replacing unsaved work.
- **Object Info:** Shows bounding-box dimensions, so you can tell whether a part fits a 3D printer's print bed.
- **Circle:** Editable segment counts.

### Changed

- **File format:** Bumped to version 12 for polygons and other curve kinds. Documents saved by 0.3.0 do not open in 0.2.0.
- **Follow Me:** Profiles auto-orient to the path, corner starts and partial sweeps work, a face can be the profile, and spheres and tori render smooth.
- **Snapping:** Circle and polygon centers and quadrants pull harder than facet noise; Ctrl+Alt flattens the pull for the exact point.
- **Rotate:** Axis-locked Rotate is constrained to the pivot plane.

### Fixed

- **Viewport:** The origin axes no longer paint over linework.
- **Undo:** Undoing a sketch island move after other edits could resolve the wrong island.
- **Selection:** A drawn curve could not be selected from its own snap points.

## [0.2.0] (2026-07-18)

The first feature release after the MVP: two new tools and a round of polish.

### Added

- **Follow Me:** Sweep a profile along a path for moldings and pipes, or around an axis as a lathe for spheres, goblets, and cones.
- **Offset:** Inset or outset a profile, region, or face by a set distance.
- **Move:** Copy an object a set distance, then type `5x` to repeat it.
- **Groups:** Union, Subtract, and Intersect work on groups, and a group duplicates in one step.
- **Components:** Definition and instance names, and clear marking of which instances belong to which component.
- **Sketches:** Rotate a sketch out of plane, or copy an island off-plane onto its own sketch.
- **Tape Measure:** Lays down guides.
- **Viewport:** Guidance when the GPU is unavailable, with a software-rendering fallback.

### Changed

- **Selection:** A tool click on an object selects it and acts; there is no separate select step.
- **Move:** Dragging objects works as expected.
- **View:** Zoom Extents frames only what is visible.
- **Welcome:** Choose your units on the welcome screen instead of in Settings.

### Fixed

- **Push/Pull:** The direction no longer flips under drag noise, and a typed negative distance is honored.
- **Viewport:** Edges shimmered after orbiting.
- **Follow Me:** The path on a solid's face is directly pickable.
- **Snapping:** Circle centers snap reliably.
- **Tags:** Deleting a tag works and is undoable.

## [0.1.1] (2026-07-15)

### Fixed

- **Linux:** The AppImage failed to start on Ubuntu and other recent distributions because its bundled libwayland conflicted with the system's. No other changes from 0.1.0.

## [0.1.0] (2026-07-15)

The first public release: the minimum viable Hew, with the basic
modeling tools working and builds for every platform.

### Added

- **Draw:** Line, Rectangle, Circle, and Arc, on the ground or on any face, with inference snapping.
- **Push/Pull:** With a live preview. Every extrusion is a discrete, watertight solid, and objects never merge on their own.
- **Solids:** Union, Subtract, and Intersect, run only when you ask, and Slice to cut a solid along a plane.
- **Transform:** Move with copy, Rotate, and Scale.
- **Groups and components:** Non-destructive, with components that edit once and update everywhere.
- **Materials:** A palette, per-face and per-object paint, image textures, and per-material opacity.
- **Organization:** Tags with per-tag visibility, an Outliner, and construction guides.
- **Measure:** Tape Measure and Protractor, which measure or drop a guide.
- **Curves:** A drawn circle stays a circle underneath its facets, so exports can re-facet it at any smoothness.
- **Undo:** Grouped by gesture, on a deterministic kernel so the same steps always give the same result.
- **Import:** SketchUp 2017 `.skp` (clean-room, no SketchUp SDK), COLLADA `.dae`, and glTF.
- **Export:** glTF, STL, and 3MF.
- **Files:** The native `.hew` format, an open and documented container.
- **Units:** Metric, decimal inches, or feet-and-inches, with typed lengths in every tool.
- **Welcome:** A welcome screen with bundled sample models.
- **Refusals:** Anything Hew will not do, it explains and leaves the geometry untouched.
- **Platforms:** Native apps for macOS (signed and notarized), Windows, and Linux with a built-in updater, and the same app in the browser with offline support.

### Known issues

- **Linux:** The AppImage can fail to load on newer distributions. Fixed in 0.1.1.

[1.2.1]: https://github.com/hew3d/hew/compare/v1.2.0...v1.2.1
[1.2.0]: https://github.com/hew3d/hew/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/hew3d/hew/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/hew3d/hew/compare/v0.10.0...v1.0.0
[0.10.0]: https://github.com/hew3d/hew/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/hew3d/hew/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/hew3d/hew/compare/v0.7.2...v0.8.0
[0.7.2]: https://github.com/hew3d/hew/compare/v0.7.1...v0.7.2
[0.7.1]: https://github.com/hew3d/hew/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/hew3d/hew/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/hew3d/hew/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/hew3d/hew/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/hew3d/hew/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/hew3d/hew/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/hew3d/hew/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/hew3d/hew/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/hew3d/hew/releases/tag/v0.1.0

[#5]: https://github.com/hew3d/hew/issues/5
[#8]: https://github.com/hew3d/hew/issues/8
[#9]: https://github.com/hew3d/hew/issues/9
[#11]: https://github.com/hew3d/hew/issues/11
[#12]: https://github.com/hew3d/hew/issues/12
[#13]: https://github.com/hew3d/hew/issues/13
[#14]: https://github.com/hew3d/hew/issues/14
[#15]: https://github.com/hew3d/hew/issues/15
[#18]: https://github.com/hew3d/hew/issues/18
[#20]: https://github.com/hew3d/hew/pull/20
[#22]: https://github.com/hew3d/hew/issues/22
[#31]: https://github.com/hew3d/hew/pull/31
[#38]: https://github.com/hew3d/hew/issues/38
