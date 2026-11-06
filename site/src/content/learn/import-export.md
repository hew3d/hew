---
title: "Import and export"
description: "Hew reads and writes SketchUp files, reads COLLADA, glTF, and STL, and writes manifold STL and 3MF for printing, glTF for everything else, USDZ for AR, and true-size SVG line drawings."
order: 19
---

| Format | Import | Export |
|---|---|---|
| `.hew` (native) | ✓ | ✓ |
| glTF / GLB (`.gltf`, `.glb`) | ✓ | ✓ (`.glb`) |
| COLLADA (`.dae`) | ✓ | — |
| SketchUp (`.skp`, 2026 format and releases 3 to 8 and 2013 to 2020) | ✓ | ✓ |
| STL (`.stl`) | ✓ | ✓ |
| 3MF (`.3mf`) | — | ✓ |
| USDZ (`.usdz`) | — | ✓ |
| SVG line drawing (`.svg`) | — | ✓ |
| Pictures (`.png`, `.jpg`) | ✓ (placed as an image) | — |

## Importing

**File ▸ Import…** accepts SketchUp, COLLADA, glTF, and STL files; the format is detected from the file you pick. (To work on a `.skp` file itself, open it with File ▸ Open instead; see [Files, saving, and recovery](/learn/files-and-saving/#sketchup-files).) Imported geometry is rebuilt into real, editable Hew Objects that you can keep modeling, and Hew checks on the way in which ones are solid.

After every import, an **Import Complete** report says what came in. For example: "Imported 66 objects. 27 are solid. The other 39 are panels, sheets, and open shapes, like glass, terrain, and pipes, which SketchUp models often have. They work as they are." It also lists any meshes that had to be skipped and why, and any texture images the file referenced but Hew couldn't find. Nothing is repaired or dropped behind your back.

**OK** closes the report and leaves everything as it came in. **Make Solid…** opens [Make Solid](#making-imported-shapes-solid) on what you imported. Opening a `.skp` with File ▸ Open shows the same report, titled **Open Complete**, and its summary starts with "Opened".

### SketchUp (`.skp`)

Hew reads `.skp` files directly; no SketchUp installation required. A file saved by any current SketchUp, the free web app included, is in the 2026 format, and that is what Hew reads first; it also reads every older release from 3 to 8 and 2013 to 2020, the 2017 one being the last free desktop version's that many existing models are still in. Either way the same model comes across: names, materials, components, tags, guides, and scenes all arrive natively. A SketchUp group becomes a Hew group and a SketchUp component becomes a shared Hew component, nested assemblies keeping their hierarchy and their sharing intact; a group that exists only to wrap a single solid collapses to that solid, since Hew doesn't need a wrapper to isolate geometry the way SketchUp does. Each SketchUp scene becomes a Hew Scene, keeping its camera (a parallel view keeps its zoom, and the camera's target is set on the model so panning and zooming work from where you are looking), the objects and tags it hides, its active section cut, and the backdrop and axes of its saved style; a cut made outside any scene comes across as Hew's section plane too. Glass and other transparent materials stay see-through, a face painted differently on each side keeps both paints, and parts a component hides stay hidden in every copy of it. SketchUp's dimensions and leader texts arrive as Hew dimensions and text on their tags, so they show and hide with them, inside groups and components too; only screen texts, which SketchUp pins to the view rather than the model, aren't read, and a save back keeps them in the file as they are. Lines that bound no face come across as drafting lines: a group of lines, like a section outline or a construction sketch, becomes a group holding its drawing, which shows and hides with the group, and a component of lines alone, like a wire-frame box or a light rig, becomes a component of its drawing. A closed loop of such lines stays a loop with no face, as it is in SketchUp; draw over one of its lines with the Line tool to give it a face. Guides come across on their layers' tags, so they show and hide with the tags the way SketchUp shows them scene by scene, and a guide that starts at a point or runs between two keeps its length. Hidden lines are left out, since Hew has no hidden drawing, and a save back keeps them. Photos placed as images come across, inside groups and components too. Importing into an empty document also adopts the view the file was saved with, so the model first appears the way it last did in SketchUp, including its backdrop: a file whose style turns the sky and ground off, say, opens on a plain background with the axes as the file shows them. Importing into a document that already has content leaves your view alone. (The reader is [Skpwright](https://github.com/hew3d/skpwright), a clean-room implementation with no Trimble SDK code in its lineage.) If a file has damaged sections, the importer recovers what it can and lists warnings in the import report. A file that was cut short, by a download or copy that didn't finish, is refused when nothing of it can be read; when some of it can, it opens with a notice that parts of the model may be missing. Either way, a fresh copy of the file is the fix. A file whose release is not on the list is read as far as it can be, with a warning; if nothing of it can be read it is refused with its version, and opening it in any current SketchUp and saving it again produces a file Hew reads.

### COLLADA (`.dae`)

COLLADA import covers SketchUp's own export path, including a healing pass that repairs the specific non-manifold artifacts SketchUp's exporter is known to produce. COLLADA stores textures as separate image files: the desktop app scans the folders next to the file automatically, and the web app lets you point at the folder containing the images.

### glTF / GLB

Both `.gltf` and `.glb` import, with embedded textures and the full node hierarchy.

### STL

STL is where downloaded prints live — Printables, Thingiverse, anywhere makers share models — and Hew reads both the binary and text (ASCII) flavors, auto-detected from the file itself. STL is the crudest format Hew imports: no object grouping, no names, no materials, no units, and its per-triangle normals are unreliable enough that Hew ignores them and works out which way is "outward" from the geometry itself, the same way every other importer does.

Because STL carries no unit information at all, Hew asks once per import: millimeters (the default — the near-universal maker convention), centimeters, inches, or meters. Picking wrong is the single most common STL headache ("why is my model 1000× too big"), so there's no silent guess. Your last choice is remembered for the rest of the session.

A single-part file becomes one Object. A multi-part plate — several disconnected shapes saved into one `.stl`, common for a print batch — comes back as one Object per part, each independently assessed for watertightness. Objects are named from the file: `bunny.stl` gives you "bunny", or "bunny", "bunny (2)", "bunny (3)" for a multi-part file. A part with a gap in it imports anyway as an open shape, exactly like a COLLADA or glTF mesh with a gap: never refused, never patched shut behind your back.

A hollow model — one whose outer wall encloses a separate inner wall (vase-mode or explicit wall-thickness prints) — reconstructs into one watertight Object with the inner wall as a cavity, a real void in the material, exactly how a hollow is meant to be. A solid piece floating inside that cavity comes in as its own separate Object.

### Pictures (`.png`, `.jpg`)

To trace over a floor plan, a photo of a wall, or a sketch, place it in the model as a picture: **File ▸ Import Image…**, or drop a PNG or JPEG onto the view. Unlike File ▸ Import…, this adds to the model you have open.

The picture follows the cursor, 1 m wide, on the face under it or on the ground. Click where one corner goes, then move to size it: it keeps its proportions, and it grows toward whichever side of that corner you move to. Click again to place it, or type the width: the picture takes that width at once, and Enter or a click places it. Esc after the first click starts over; Esc before it cancels. To place a picture on a plane of its own, press an arrow key before the first click, the same way you lock a plane for the drawing tools: right for red, left for green, up for blue. With a plane locked, the first Esc unlocks it and leaves the picture ready to place; the next Esc cancels.

A placed picture is a component named after the file, holding one face painted with the picture, the way a SketchUp image arrives. It stands upright on a wall and faces you. Placing it is one undo step, and it is selected afterward. To size it to a known length, double-click it to open it, measure an edge whose real length you know with the Tape Measure, type that length, and choose to resize: the picture stays stretched over the face. Hew keeps the picture file exactly as it is, so a large photo makes a large model file; pictures over 64 megapixels are refused, so save a smaller copy first. A picture can also be placed too small or too big: each side has to be at least 0.001 mm and at most 1,000,000 m, and Hew tells you which when you drag or type a size outside that.

## Making imported shapes solid

Everything you model in Hew is solid. An imported model usually isn't, all the way through, and that's fine: SketchUp lets you draw glass as one face, a duct as an open pipe, and a box without its lid. Those work in Hew as they are. You need a solid only to push or pull it, combine it with another, slice it, or 3D print it, and **Make Solid** closes what can be closed.

Open it with **Object ▸ Make Solid…** (or the command palette), or with **Make Solid…** in the import report. With something selected it works on the selection; otherwise on the whole model. The window opens beside the model, so you can still orbit and zoom; drag its left edge to make it wider.

The first page sums up what Hew found and what it recommends. **Use recommended choices** does that in one step. **Go step by step** shows one kind of shape per page:

- **Openings.** Shapes with open sides or gaps. Most are open on purpose, like a duct or a box without a lid, so on the whole model Hew leaves them open; choose **Close the openings** to add a flat face across each one. On a selection, closing is the recommended choice, since you picked those shapes to make them solid.
- **Rejoin split shapes.** SketchUp drew each of these as one shape with a wall inside it, and they came in as separate pieces. Hew closes the pieces where they meet and removes the wall between them, or keeps the wall if you choose. Openings the shape was drawn with stay open.
- **Flat panels.** Single flat faces such as glass, signs, or a drawn floor. Hew leaves them as they are; choose **Add thickness** to make them solid (in front, behind, or centered).
- **Curved sheets.** Single curved faces such as terrain or fabric. Hew can't add thickness to curved sheets yet.
- **Can't be closed.** Shapes whose open edges don't lie flat, or branch, so no flat face can close them. These are usually open on purpose too: a pipe fitting with three open ends, an electrical box with an open front.

**Choose for each item** under a list gives every item its own checkbox, so you can close one duct and leave the rest open.

Click any item in a list to see it: Hew hides everything else, frames it, and draws its open edges, and if the item is part of a group or component, the rest of that group or component stays visible but faded. For a shape that can't be closed, a sentence under the list says why. **Show everything** at the top of the view, or Escape, brings the rest back.

The last page lists what will happen; **Make Solid** does it as one step, so **Edit ▸ Undo** puts everything back. Anything Hew couldn't close is listed afterward with the reason.

To work through objects one at a time instead, choose **Object ▸ Model Health…**. It opens a window over the view that lists every object that isn't solid, grouped the same way, with **Close**, **Add thickness…**, and **Keep as is** (which hides it from the list) on each one. Drag the window by its title to move it out of the way, or by its bottom-right corner to resize it; Hew opens it there next time. When it frames an object, the object lands in the part of the view the window doesn't cover. Object Info has the same buttons for the selected object. Pushing or pulling a flat panel with the [Push/Pull](/learn/push-pull/) tool also turns it into a solid. Scripts can do the same through the Hew API; see [the API guide](https://github.com/hew3d/hew/blob/main/docs/API_GUIDE.md).

## Exporting

**File ▸ Export…** shows the system save dialog with a **Format** list in it; a format with options of its own (STL's curve resolution, SVG's view and scale) shows them under the list while it's the one chosen. The next export starts on the format you used last. In the browser, whose save dialog has no format list, Hew shows its own Export dialog first, with the same formats and options:

![The browser's Export dialog with STL selected, showing the format and curve-resolution selectors](/docs/export-dialog.webp)

### SketchUp (`.skp`)

The format list offers **SketchUp (.skp)**, the 2026 format current SketchUp saves, and **SketchUp 2020 (.skp)** down to **SketchUp 3 (.skp)**, the formats of those releases (3 to 8 and 2013 to 2020; each opens in its own release and later). Export writes a copy and leaves the document where it is; to keep working in a `.skp` file, use Save As instead. Names, materials, textures, tags, guides, groups, components, and scenes are written. Where a SketchUp file cannot hold something exactly, Hew writes the closest equivalent and shows a notice listing what was simplified, for example curved surfaces written as smooth facets. Unlike the notice for a Save, it appears on every export.

### STL — for 3D printing

Binary STL, in **millimeters**, Z-up, ready for any slicer. Hew models are watertight solids, so the exported STL is manifold: no repair step, no "fix errors?" prompt in your slicer.

STL has a **Curve resolution** option. Because drawn circles and arcs remember their exact geometry, Hew can rebuild curved walls at export time at any smoothness — the facets you modeled with are the floor, not the ceiling. "As modeled" writes the stored facets verbatim; Draft through Ultra re-facet every eligible curved wall at 24 to 192 segments per turn, and the mesh stays manifold at every setting. A curved wall that later operations have made irregular (a boss on the wall, a boolean seam through it) keeps its stored facets rather than being approximated.

The export is **gated on solidity**. If any object isn't solid, Hew shows *Export STL Anyway?* with those objects listed by name. **Make Solid…** there opens Make Solid on just those objects; **Export Anyway** exports them as they are. STL contains geometry only (no names, colors, or units metadata) and merges everything into one mesh.

### 3MF — for multi-part printing

3MF is the modern print format: **explicit millimeter units**, Z-up, and — unlike STL — real structure. Every object and component instance exports as its own named part with its face colors, so a multi-part print arrives in your slicer as separate, recognizable pieces instead of one anonymous blob. The same solidity gate as STL applies.

### glTF (GLB) — for everything else

A single `.glb` file in the industry-standard format: **meters, Y-up**, with your object hierarchy, per-instance transforms, names, colors, and embedded textures. Use it for Blender, game engines, web viewers, or any modern 3D pipeline; the round trip through Blender is tested for fidelity. Only solid geometry exports; sketches and guides stay home.

### USDZ — for AR Quick Look and USD pipelines

A single `.usdz` file — **meters, Y-up**, one named part per object or component instance, with face colors carried over as materials. This is the format iOS understands natively: AirDrop it, attach it to a Message, or open it from Files, and it launches straight into AR Quick Look, dropped into the room in front of you at true scale, no app required. It also opens in any USD-aware tool (Pixar's usdview, Blender's USD importer, Apple's Reality Composer). Textured materials export as their tint color rather than the image itself.

On an iPhone or iPad, [Shop Mode](/learn/hew-on-your-phone/#view-in-ar-ios-only) skips this dialog entirely — a **View in AR** button exports and launches Quick Look directly from the model open in your hand.

### SVG line drawing — for laser, CNC, and vector tools

A true-size hidden-line drawing of the model, written as an `.svg` with a millimeter `viewBox` so laser cutters, CNC software, and any other vector tool reads it at its real dimensions. Choose a **View** (Current or one of the seven standard views), a **Scale** (1:1 by default — the ladder is the same one [Printing](/learn/printing/#scaled) uses), whether **Hidden lines** draw dashed, and whether **Dimensions & text** are included. Hidden surfaces are removed by the same engine that draws vector Line art on a printed page — see [Printing](/learn/printing/) for paper and PDF output of the same drawing.

### What about STEP?

Hew doesn't import or export STEP/IGES. For those jobs: STL or 3MF to print, glTF to interchange, USDZ for AR, `.hew` or `.skp` to keep working.
