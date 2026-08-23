---
title: "Hew for FreeCAD users"
package: "FreeCAD"
description: "The other open-source answer: direct modeling instead of a feature tree, a low floor instead of workbenches."
order: 4
---

You know FreeCAD: open source, parametric, workbenches for everything — constrained sketches in Part Design, TechDraw for drawings, FEM, CAM. Hew is the other open-source answer to "model a real thing," from the opposite school. There is no feature tree, no constraint solver, and no workbench switching: you draw on a face, push/pull, and combine solids explicitly. Nothing regenerates, so nothing breaks downstream when you edit — the topological-naming class of grief doesn't exist, because there's no dependency graph to invalidate.

What replaces the constraint solver is SketchUp-style precision: typed unit-aware lengths mid-gesture, inference snapping to endpoints/midpoints/axes, guide lines, a tape measure, dimensions. You state sizes as you go instead of constraining them afterward. The floor is much lower — a first-session user builds something real — and the ceiling is much lower too: no parametric re-use, no STEP, no FEM, no CAM, no TechDraw-grade drawings.

Both are free software you can trust with your files: FreeCAD's format is open, and Hew's `.hew` is a documented zip of JSON and geometry buffers. Which one fits is mostly about whether your models are *driven by* parameters and downstream engineering, or are things you shape directly and then build in the shop.

## Feature comparison

| | FreeCAD | Hew |
| --- | --- | --- |
| License | Free, LGPL | Free, AGPL-3.0 |
| Modeling paradigm | Parametric feature tree, constrained sketches | Direct modeling: draw, push/pull, boolean; undo is the only history |
| Editing later | Change parameters, model regenerates (and can break) | Push/pull the geometry again; nothing regenerates or breaks |
| Precision | Constraint solver + typed dimensions | Typed lengths mid-gesture, inference snapping, guides, dimensions |
| Learning curve | Steep — workbenches, sketcher discipline | SketchUp-shaped; productive in the first hour |
| Platforms | Windows, macOS, Linux | Windows, macOS, Linux, plus the full app in a browser |
| CAD interchange | STEP/IGES native | No STEP — STL, 3MF, glTF, USDZ, SVG |
| Drawings | TechDraw workbench | Printing to exact scale on paper/PDF; SVG line drawings |
| Assemblies, FEM, CAM | Built in or first-party workbenches | None |

Below that line FreeCAD is an engineering suite and Hew is not — no point tabulating a column of "no."
