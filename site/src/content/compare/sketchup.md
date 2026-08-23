---
title: "Hew for SketchUp users"
package: "SketchUp"
description: "The same push/pull way of working on a stricter foundation — with a tool-by-tool accounting of what carries over, what's missing, and what gets better."
order: 1
---

You know SketchUp — and Hew works the way SketchUp taught you: draw on a face, push/pull it into shape, inference snapping everywhere, mostly the same keyboard shortcuts. Your muscle memory carries over almost untouched. The reflexes you had to *learn*, though, you can drop: in Hew every push/pull produces a discrete, watertight object. Nothing welds to a neighbor because you forgot to group it, nothing goes hollow because one face got deleted, and Union/Subtract/Intersect only run when you ask. It's Select All + Make Group as the data model instead of a habit.

The practical differences are about money and reach. Hew is free and open source — native apps for macOS, Windows, and Linux, plus a browser app that is the full modeler, not a reduced tier. It opens your `.skp` files (2017 format, which newer SketchUp versions can Save As) through a clean-room reader, and saves to an open, documented format.

Because the two are so alike, whether Hew works *for you* comes down to whether your particular tools and workflows are there. So instead of a broad-strokes table, here is the tool-by-tool accounting — including the places where SketchUp is ahead, the places where Hew is ahead (paper output and the phone, mostly), and where the most popular extensions land, since Hew has no extension system to port them to.

## Tool by tool

<details open>
<summary><strong>Drawing</strong> — near-complete overlap</summary>

| | SketchUp | Hew |
| --- | --- | --- |
| Line | Yes | Yes |
| Rectangle | Yes, plus a Rotated Rectangle tool | Yes; no rotated-rectangle tool — lock a plane with the arrow keys or rotate the shape after |
| Circle | Yes | Yes, with typed radius |
| Polygon | Yes | Yes; side count typed (`6s`) and remembered |
| Arcs | Three tools: 2-Point, 3-Point, Pie | One arc tool: endpoints plus bulge; `Alt` cycles closing it as a pie or a chord |
| Freehand | Yes | No |
| Drawing on a face | Splits the face (sticky geometry) | Same — drawing on a solid's face splits it for push/pull |
| Choosing the drawing plane | Inferred from faces and view | Hovered face, or arrow-key locks to a world-axis plane through the next click |
| Editing a drawing | Sticky edges, Eraser tool | Lines select and delete individually, a drawn curve selects as one, each connected shape moves as its own island, and an Edit Vertex tool adjusts single points; no eraser-drag tool |

</details>

<details>
<summary><strong>Modify &amp; solid operations</strong> — the core carries over; Trim, Split, and radial arrays don't</summary>

| | SketchUp | Hew |
| --- | --- | --- |
| Push/Pull | Yes; double-click repeats, modifier key leaves a face behind | Same, including double-click repeat and a new-coincident-solid modifier — plus through-cuts that can split a solid in two, and push/pull on any planar face however oblique its neighbors |
| Push/pull on curved walls | JointPushPull extension territory | Drawn circle and arc walls move as one whole wall (the radius changes); other curved surfaces move one facet at a time |
| Follow Me | Yes | Yes — partial sweeps by dragging with a live length readout, flat profiles stood upright automatically, spheres and goblets via pole closure, and a merge-as-you-sweep gesture |
| Offset | Yes | Yes — sketch regions and solid faces |
| Move / Copy / arrays | Move with copy toggle; `xN` and `/N` arrays | Same: `Alt` toggles copy, `3x` and `3/` arrays |
| Rotate | Yes; rotate-copy builds radial arrays | Rotate on any axis with snapping and typed angles; no rotate-copy, so no radial arrays |
| Scale | Yes | Yes — face/edge/corner grips, typed factors, center anchoring |
| Solid tools | Outer Shell (free); Union, Subtract, Trim, Intersect, Split (paid tiers) | Union, Subtract, Intersect — on solids and whole groups alike, included; no Trim or Split |
| Slice | — | Cut any solid along a plane into two watertight solids |
| Intersect Faces | Yes | No equivalent; booleans are the only way geometry combines |
| Soften / Smooth edges | Manual | Automatic on drawn curves' facet seams; no manual soften tool |

</details>

<details>
<summary><strong>Precision &amp; measurement</strong> — same philosophy, near parity</summary>

| | SketchUp | Hew |
| --- | --- | --- |
| Measurements box | Typed mid-gesture | Same: `24cm`, `9.5"`, `0.24m`, unit-aware everywhere |
| Units | Metric and imperial, incl. architectural | Same, feet-and-inches included |
| Tape Measure | Point-to-point, parallel guides, resize model | Same — plus guides on any plane, and in-context resize that rescales just a group or component from one typed measurement |
| Protractor | Angled guides | Same |
| Dimensions | Linear dimensions in the model | Linear and radial dimensions, anchored to geometry, constant on-screen size |
| Text | Screen and leader text | Leader text anchored to geometry |
| 3D Text | Yes | Yes — always a watertight solid, placed on any face or plane, bundled and system fonts or load your own |
| Drawing axes | Relocatable | Same — move and orient the document axes |

</details>

<details>
<summary><strong>Organization &amp; reuse</strong> — same vocabulary, minus the parametric and cloud parts</summary>

| | SketchUp | Hew |
| --- | --- | --- |
| Groups | Yes | Yes — nested, and booleans accept whole groups |
| Components | Definitions and instances, edit in place | Same, including nested components, drill-down editing, Make Unique, and Explode |
| Dynamic / Live Components | Yes (paid or legacy) | No parametric components |
| Tags | Tags with folders | Nested tag paths (`Desk/Set`), multiple tags per object, per-tag visibility, select-by-tag, rename in place |
| Outliner | Desktop and paid tiers only — the free web tier has none | Yes, everywhere the app runs — double-click drills into any level, breadcrumb steps back out |
| Scenes | Saved views plus slideshow/animation export | Scenes save camera, visibility, and the section cut, with animated transitions in the app; no animation export |
| 3D Warehouse | Enormous | Nothing like it — a personal Library of your own components, materials, and models (desktop app only) |

</details>

<details>
<summary><strong>Camera &amp; display</strong> — navigation carries over; the presentation layer doesn't</summary>

| | SketchUp | Hew |
| --- | --- | --- |
| Orbit / Pan / Zoom | Yes | Same, same mouse buttons, `O`/`H`/`Z` |
| Standard views | Yes | Same set, one click above the viewport |
| Parallel projection | Yes | Yes |
| Field of view | Typed FOV | Through the Zoom tool: Shift-drag, or type degrees or a 35mm focal length; no separate FOV command |
| Position Camera / Look Around / Walk | Yes, with collision in Walk | Yes; no collision detection |
| Section planes | Several planes, fills in newer versions | One section plane at a time, no cut fill; saved with the file and captured by Scenes |
| Styles | Sketchy edges, profiles, watermarks… | No style system — one clean draw style, light and dark themes |
| Shadows, geolocation, fog | Yes | No |
| Match Photo | Yes | No |
| X-ray / back edges | Yes | No |

</details>

<details>
<summary><strong>Materials</strong> — the same model, without the bundled catalogs</summary>

| | SketchUp | Hew |
| --- | --- | --- |
| Painting | Paint Bucket: faces or whole objects | Same — per-face paint plus an object default material |
| Image textures | Yes, at real-world scale | Yes — PNG/JPEG at a physical size, tiling across faces |
| Position Texture | Pin-based | Same, corner pins |
| Opacity | Per material | Per material |
| Sample a material | Alt-click | Alt-click |
| Bundled material catalogs | Large | None — the palette starts empty; the Library keeps materials you make |

</details>

<details>
<summary><strong>Files &amp; interop</strong> — open formats on Hew's side, CAD formats on SketchUp's</summary>

| | SketchUp | Hew |
| --- | --- | --- |
| Native format | `.skp`, proprietary | `.hew` — an open, documented zip of JSON and geometry buffers |
| Reading the other's files | — | Imports `.skp` (2017 format; newer SketchUp can Save As 2017); does not write `.skp` |
| DWG / DXF | Import and export (paid tiers) | No |
| COLLADA (`.dae`) | Import and export | Import, including a healing pass for SketchUp's exporter's quirks |
| STL | Import and export | Import and export — export re-facets curves at your chosen smoothness and is gated on solidity |
| 3MF | No | Export, with named parts and colors |
| glTF | Native import/export since SketchUp 2025; extensions before that | Import and export, Blender round-trip tested |
| USDZ | Native import/export since SketchUp 2025 (paid tiers) | Export — AR-ready on iPhone/iPad |
| Autosave / crash recovery | Yes | Yes — a recovery snapshot every 12 seconds |

</details>

<details>
<summary><strong>Paper, phone &amp; rendering</strong> — where moving to Hew is an upgrade, except rendering</summary>

| | SketchUp | Hew |
| --- | --- | --- |
| Print to scale | Possible, but it's work: the desktop print dialog needs a standard view, Parallel Projection, and window-cropping (tiling is Windows-only); the polished route is LayOut, which is a paid tier | One print dialog on every platform, browser included: pick a scale, get exact-size output — bitmap or true vector |
| Full-size paper templates | LayOut poster printing at 100%, or the fiddly dialog above | First-class: print at 1:1, tiled across pages with trim marks and a scale bar, tape the sheets together at the bench |
| 2D vector export | PDF/EPS/DWG via paid tiers and LayOut | SVG line drawings from any view |
| Your model on a phone | SketchUp Viewer app — iOS only (the Android app was discontinued in 2025), Trimble account sign-in, AR costs extra without a subscription | Shop Mode in any phone browser, Android included: QR handoff from the desktop app, no account, a measurement-first touch UI — and View in AR on iPhone/iPad, free |
| Rendering | A deep bench of extensions: V-Ray, Enscape, Twilight… | None — export glTF to Blender or a renderer |

</details>

<details>
<summary><strong>The extension question</strong> — Hew has no extension system; here's where the big ones land</summary>

Hew has no equivalent of the Extension Warehouse, so the honest question is what each popular extension's *job* maps to. (The superscripts in Solid Inspector² and CleanUp³ aren't footnotes — they're part of the extensions' names.)

| Extension | Its job | In Hew |
| --- | --- | --- |
| OpenCutList | Cut lists, cutting diagrams, labels, and cost reports | Partial: printing can append a cut-list page (part, quantity, L × W × H, with identical parts folded), and Shop Mode's parts sheet is a live cutlist on your phone — but no cutting-diagram/nesting layout for sheet stock |
| Fredo6 RoundCorner | Fillets and bevels | No fillet/chamfer tool; edge profiles only via Follow Me |
| Fredo6 JointPushPull | Push/pull on curved surfaces | Partly built in: drawn circle/arc walls push/pull as one wall; arbitrary curved surfaces don't |
| Curviloft | Lofting and skinning | No equivalent |
| Solid Inspector² | Find and fix the holes that keep a group from being solid | Not needed — objects can't stop being solid, and watertight status is shown live |
| CleanUp³ | Purge stray edges, faces, and duplicate geometry | Mostly not needed — there is no loose-geometry soup to clean |
| Artisan | Subdivision and organic sculpting | No equivalent; Hew isn't for organic modeling |
| Profile Builder | Parametric profile assemblies | No equivalent |

</details>

If your SketchUp life runs through LayOut, dynamic components, rendering extensions, or OpenCutList, Hew doesn't replace those today. If it runs through the core modeling loop — draw, push/pull, measure, organize, and get the result onto paper or out to the shop — everything above says exactly which parts carry over, and the paper-and-phone row is the one place a move is an outright upgrade.
