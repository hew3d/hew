---
title: "Import and export"
description: "Hew reads SketchUp, COLLADA, and glTF, and writes manifold STL for printing plus glTF for everything else."
order: 14
---

| Format | Import | Export |
|---|---|---|
| `.hew` (native) | ✓ | ✓ |
| glTF / GLB (`.gltf`, `.glb`) | ✓ | ✓ (`.glb`) |
| COLLADA (`.dae`) | ✓ | — |
| SketchUp (`.skp`, 2017 format) | ✓ | — |
| STL (`.stl`) | — | ✓ |

## Importing

**File ▸ Import…** accepts SketchUp, COLLADA, and glTF files; the format is detected from the file you pick. Imported geometry is rebuilt into real, editable Hew Objects that you can keep modeling, and each object's watertightness is assessed on the way in.

After every import, an **Import Complete** report summarizes what happened: how many objects were created (and how many are solid vs. leaky), any meshes that had to be skipped and why, and any texture images the file referenced but Hew couldn't find. Nothing is silently repaired or silently dropped.

### SketchUp (`.skp`)

Hew reads SketchUp 2017-format files directly; no SketchUp installation required. Names, materials, components, tags, and guides all come across natively. (The reader is [OpenSKP](https://github.com/hew3d/openskp), a clean-room implementation with no Trimble SDK code in its lineage.) If a file has damaged sections, the importer recovers what it can and lists warnings in the import report. Newer `.skp` versions can usually be saved back to the 2017 format from SketchUp itself.

### COLLADA (`.dae`)

COLLADA import covers SketchUp's own export path, including a healing pass that repairs the specific non-manifold artifacts SketchUp's exporter is known to produce. COLLADA stores textures as separate image files: the desktop app scans the folders next to the file automatically, and the web app lets you point at the folder containing the images.

### glTF / GLB

Both `.gltf` and `.glb` import, with embedded textures and the full node hierarchy.

## Exporting

**File ▸ Export…** opens one dialog with a format choice:

![The Export dialog showing the format selector](/docs/export-dialog.png)

### STL — for 3D printing

Binary STL, in **millimeters**, Z-up, ready for any slicer. Hew models are watertight solids, so the exported STL is manifold: no repair step, no "fix errors?" prompt in your slicer.

The export is **gated on solidity**. If any object is leaky, Hew shows *Export STL Anyway?* with the offending objects listed by name; you can export regardless, but you've been told exactly what's wrong and where. STL contains geometry only (no names, colors, or units metadata) and merges everything into one mesh.

### glTF (GLB) — for everything else

A single `.glb` file in the industry-standard format: **meters, Y-up**, with your object hierarchy, per-instance transforms, names, colors, and embedded textures. Use it for Blender, game engines, web viewers, or any modern 3D pipeline; the round trip through Blender is tested for fidelity. Only solid geometry exports; sketches and guides stay home.

### What about 3MF, STEP, or `.skp` export?

3MF (a modern print format with units and per-object color) is next on the roadmap; STEP/IGES interchange and `.skp` export are planned further out. For today: STL to print, glTF to interchange, `.hew` to keep working.
