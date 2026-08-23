---
title: "Import and export"
description: "Hew reads SketchUp, COLLADA, glTF, and STL, and writes manifold STL and 3MF for printing, glTF for everything else, USDZ for AR, and true-size SVG line drawings."
order: 19
---

| Format | Import | Export |
|---|---|---|
| `.hew` (native) | ✓ | ✓ |
| glTF / GLB (`.gltf`, `.glb`) | ✓ | ✓ (`.glb`) |
| COLLADA (`.dae`) | ✓ | — |
| SketchUp (`.skp`, 2017 format) | ✓ | — |
| STL (`.stl`) | ✓ | ✓ |
| 3MF (`.3mf`) | — | ✓ |
| USDZ (`.usdz`) | — | ✓ |
| SVG line drawing (`.svg`) | — | ✓ |

## Importing

**File ▸ Import…** accepts SketchUp, COLLADA, glTF, and STL files; the format is detected from the file you pick. Imported geometry is rebuilt into real, editable Hew Objects that you can keep modeling, and each object's watertightness is assessed on the way in.

After every import, an **Import Complete** report summarizes what happened: how many objects were created (and how many are solid vs. leaky), any meshes that had to be skipped and why, any texture images the file referenced but Hew couldn't find, and warnings — for instance a non-manifold mesh imported as separate open shells rather than one solid. Nothing is silently repaired or silently dropped.

### SketchUp (`.skp`)

Hew reads SketchUp 2017-format files directly; no SketchUp installation required. Names, materials, components, tags, and guides all come across natively. A SketchUp group becomes a Hew group and a SketchUp component becomes a shared Hew component, nested assemblies keeping their hierarchy and their sharing intact; a group that exists only to wrap a single solid collapses to that solid, since Hew doesn't need a wrapper to isolate geometry the way SketchUp does. (The reader is [OpenSKP](https://github.com/hew3d/openskp), a clean-room implementation with no Trimble SDK code in its lineage.) If a file has damaged sections, the importer recovers what it can and lists warnings in the import report. Newer `.skp` versions can usually be saved back to the 2017 format from SketchUp itself.

### COLLADA (`.dae`)

COLLADA import covers SketchUp's own export path, including a healing pass that repairs the specific non-manifold artifacts SketchUp's exporter is known to produce. COLLADA stores textures as separate image files: the desktop app scans the folders next to the file automatically, and the web app lets you point at the folder containing the images.

### glTF / GLB

Both `.gltf` and `.glb` import, with embedded textures and the full node hierarchy.

### STL

STL is where downloaded prints live — Printables, Thingiverse, anywhere makers share models — and Hew reads both the binary and text (ASCII) flavors, auto-detected from the file itself. STL is the crudest format Hew imports: no object grouping, no names, no materials, no units, and its per-triangle normals are unreliable enough that Hew ignores them and works out which way is "outward" from the geometry itself, the same way every other importer does.

Because STL carries no unit information at all, Hew asks once per import: millimeters (the default — the near-universal maker convention), centimeters, inches, or meters. Picking wrong is the single most common STL headache ("why is my model 1000× too big"), so there's no silent guess. Your last choice is remembered for the rest of the session.

A single-part file becomes one Object. A multi-part plate — several disconnected shapes saved into one `.stl`, common for a print batch — comes back as one Object per part, each independently assessed for watertightness. Objects are named from the file: `bunny.stl` gives you "bunny", or "bunny", "bunny (2)", "bunny (3)" for a multi-part file. A part with a genuine gap in its surface imports anyway, flagged leaky in the report, exactly like a leaky COLLADA or glTF mesh: never refused, never silently patched shut.

A hollow model — one whose outer wall encloses a separate inner wall (vase-mode or explicit wall-thickness prints) — reconstructs into one watertight Object with the inner wall as a cavity, a real void in the material, exactly how a hollow is meant to be. A solid piece floating inside that cavity comes in as its own separate Object.

## Exporting

**File ▸ Export…** opens one dialog with a format choice:

![The Export dialog with STL selected, showing the format and curve-resolution selectors](/docs/export-dialog.webp)

### STL — for 3D printing

Binary STL, in **millimeters**, Z-up, ready for any slicer. Hew models are watertight solids, so the exported STL is manifold: no repair step, no "fix errors?" prompt in your slicer.

The export dialog has a **Curve resolution** choice for STL. Because drawn circles and arcs remember their exact geometry, Hew can rebuild curved walls at export time at any smoothness — the facets you modeled with are the floor, not the ceiling. "As modeled" writes the stored facets verbatim; Draft through Ultra re-facet every eligible curved wall at 24 to 192 segments per turn, and the mesh stays manifold at every setting. A curved wall that later operations have made irregular (a boss on the wall, a boolean seam through it) keeps its stored facets rather than being approximated.

The export is **gated on solidity**. If any object is leaky, Hew shows *Export STL Anyway?* with the offending objects listed by name; you can export regardless, but you've been told exactly what's wrong and where. STL contains geometry only (no names, colors, or units metadata) and merges everything into one mesh.

### 3MF — for multi-part printing

3MF is the modern print format: **explicit millimeter units**, Z-up, and — unlike STL — real structure. Every object and component instance exports as its own named part with its face colors, so a multi-part print arrives in your slicer as separate, recognizable pieces instead of one anonymous blob. The same solidity gate as STL applies.

### glTF (GLB) — for everything else

A single `.glb` file in the industry-standard format: **meters, Y-up**, with your object hierarchy, per-instance transforms, names, colors, and embedded textures. Use it for Blender, game engines, web viewers, or any modern 3D pipeline; the round trip through Blender is tested for fidelity. Only solid geometry exports; sketches and guides stay home.

### USDZ — for AR Quick Look and USD pipelines

A single `.usdz` file — **meters, Y-up**, one named part per object or component instance, with face colors carried over as materials. This is the format iOS understands natively: AirDrop it, attach it to a Message, or open it from Files, and it launches straight into AR Quick Look, dropped into the room in front of you at true scale, no app required. It also opens in any USD-aware tool (Pixar's usdview, Blender's USD importer, Apple's Reality Composer). Textured materials export as their tint color rather than the image itself.

On an iPhone or iPad, [Shop Mode](/learn/hew-on-your-phone/#view-in-ar-ios-only) skips this dialog entirely — a **View in AR** button exports and launches Quick Look directly from the model open in your hand.

### SVG line drawing — for laser, CNC, and vector tools

A true-size hidden-line drawing of the model, written as an `.svg` with a millimeter `viewBox` so laser cutters, CNC software, and any other vector tool reads it at its real dimensions. Choose a **View** (Current or one of the seven standard views), a **Scale** (1:1 by default — the ladder is the same one [Printing](/learn/printing/#scaled) uses), whether **Hidden lines** draw dashed, and whether **Dimensions & text** are included. Hidden surfaces are removed by the same engine that draws vector Line art on a printed page — see [Printing](/learn/printing/) for paper and PDF output of the same drawing.

### What about STEP or `.skp` export?

Hew doesn't import or export STEP/IGES, and it doesn't export `.skp`. For those jobs: STL or 3MF to print, glTF to interchange, USDZ for AR, `.hew` to keep working.
