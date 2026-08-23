---
title: "Hew for Blender users"
package: "Blender"
description: "Same open-source philosophy, opposite scope: Hew does dimensioned real-world objects and nothing else."
order: 2
---

You know Blender as the free, open-source tool that does everything: modeling, sculpting, rendering, animation, geometry nodes, compositing. Hew shares the license philosophy — free, open source, open file format — and almost nothing about the scope. It models dimensioned physical objects (furniture, decks, shop jigs, buildings) and stops there. No renderer, no animation, no sculpting, no nodes.

The modeling itself is a different discipline. In Blender you edit a mesh directly — verts, edges, faces — and keeping a model manifold is on you. In Hew you never touch a vertex: you draw a closed profile, push/pull it into a solid, and cut or merge solids with explicit booleans. Every object is watertight by construction and the app refuses operations that would break that, so the STL/3MF you export needs no cleanup pass. Precision works the way CAD users expect rather than the way Blender's snapping does: you type `24cm` or `9.5"` mid-gesture, snap to inferred endpoints and axes, and dimension the result.

If Blender already does what you need, Hew offers you little — unless the thing you build next has to be *right* at real-world sizes and printable or buildable. Then Hew is the shorter path, and glTF round-trips your model back into Blender for rendering.

## Feature comparison

| | Blender | Hew |
| --- | --- | --- |
| License | Free, GPL | Free, AGPL-3.0 |
| Modeling paradigm | Direct mesh editing: verts/edges/faces, modifiers, sculpt | Sketch-and-extrude solids: draw profile, push/pull, boolean |
| Watertightness | Your responsibility | Guaranteed by construction; violations are refused |
| Precision | Snapping, numeric transforms; unit workflows take setup | Typed unit-aware lengths mid-gesture, inference snapping, guides, dimensions |
| Platforms | Windows, macOS, Linux | Windows, macOS, Linux, plus the full app in a browser |
| Interop | Nearly everything | `.skp`/COLLADA/glTF/STL in; glTF/STL/3MF/USDZ/SVG out — round-trip to Blender is tested |
| Rendering, animation, sculpting, nodes, physics | Blender's core | None — export to Blender for that |

The table ends there because the products barely overlap: Blender is a complete 3D content-creation suite, and Hew is a workshop modeler. The honest comparison is scope, not feature-by-feature.
