# Hew — Roadmap

Hew is already featureful and very usable today. Native apps are
available for macOS, Windows, and Linux, and the same full app runs
in any modern browser at [app.hew3d.com](https://app.hew3d.com) — no
reduced tier, no account. If you learned 3D modeling on SketchUp, your
muscle memory works here.

This doc answers two questions: what works now, and where Hew is going.

## What works now

**The SketchUp moves.** Draw on any face or plane — lines, rectangles,
circles, arcs, polygons — and push/pull the result into a solid.
Orbit, pan, and zoom, same mouse buttons. Inference snapping everywhere,
mostly the same shortcuts. Follow Me sweeps profiles into moldings,
spheres, and goblets. Offset, Slice, through-cuts, and Move/Rotate/Scale
with copy and `3x` arrays.

**Solids that stay solid.** Every extrusion is a discrete, watertight
object. Nothing welds to a neighbor by accident, nothing goes hollow;
Union, Subtract, and Intersect run only when you ask. A drawn circle
stays a true circle underneath its facets, so exports can re-facet it
at any smoothness.

**Precision.** Type exact sizes mid-gesture — `24cm`, `9.5"`,
feet-and-inches. Tape measure, protractor, guides, linear and radial
dimensions, leader text, and solid 3D text. Measure an edge, type what
it should be, and the model — or just one part — rescales to match.

**Organization.** Nested groups, components that edit once and update
everywhere, tags with per-tag visibility, saved Scenes, an outliner on
every platform, and a personal Library of your components, materials,
and models — each item an ordinary `.hew` file.

**Materials.** Flat colors and image textures at real-world scale,
painted per face or per object, with opacity and drag-the-pins texture
positioning.

**Paper and the shop.** Print at an exact drawing scale from every
platform, browser included — or at 1:1, tiled across pages with trim
marks and a scale bar, for full-size templates you tape together at the
bench. Vector line art, direct PDF export, a cut-list page. Shop Mode
puts the model on any phone via a QR code: tap a part for its size,
check the parts list, view it in AR on an iPhone or iPad.

**Files that are yours.** The native `.hew` format is an open,
documented container.

| Format | Import | Export |
|---|---|---|
| `.hew` (native) | yes | yes |
| SketchUp (`.skp`, 2017 format) | yes | — |
| glTF / GLB | yes | yes |
| COLLADA (`.dae`) | yes | — |
| STL | yes | yes |
| 3MF | — | yes |
| USDZ | — | yes |
| SVG line drawing | — | yes |

**Trustworthy underneath.** The kernel is deterministic — the same
steps always produce the same result, so bugs reproduce instead of
haunting. Undo never corrupts a model. Anything Hew refuses to do, it
explains in plain language and leaves your geometry untouched.

## Where it's going

### Near-term
- Near 100% parity with SketchUp's built-in tools, plus the jobs of
  its most popular extensions where they fit Hew's model — radial
  arrays, a mirror tool, section fill, animation export, and cutting
  diagrams among the known gaps; the
  [SketchUp comparison](https://hew3d.com/compare/sketchup/) is the
  complete accounting, and its "no" cells are the to-do list
- SketchUp `.skp` export in addition to the existing read support
- STEP/IGES import and export, for interchange with engineering CAD
- DWG/DXF for the architectural crowd
- Chamfer/fillet
- True toroidal surfaces, so swept bends are exact instead of faceted
- Push/pull an outer face past its own holes (the one push/pull case
  still refused)
- A single-file embedded model viewer — one self-contained HTML file
  carrying the model and a read-only viewer, for dropping into any web
  page or sending as an attachment
- A bring-your-own-cloud Library — point the browser's Library at a
  WebDAV server you control (Nextcloud, a NAS) so it follows you
  without any Hew account or Hew server involved
- "Play" mode with object constraints

### Longer-term
- Signed Windows installers — dependent on Hew becoming "notable"
- A plugin API, sandboxed by design
- Multi-user collaboration

### Maybe
- An out-of-process kernel option for very large models — not sure if
  it's needed
- A WebGPU rendering path — dependent on support for all platforms in
  the underlying framework
- iOS and Android mobile apps instead of the current PWA
- Target UX "views" that hyper-focus on alternate workflows. BIM as a
  workflow? Parametric CAD? CAM?

## What Hew will never do

- Silently repair geometry. An operation that would produce an
  invalid solid fails with a clear error instead of being patched up
  behind your back.
- Merge geometry implicitly. Objects never weld together on their own.
  Combining is always your explicit act.
- Raw mesh editing. Hew works in whole, watertight objects. An
  operation that would tear one open is refused.

---

The engineering-level inventory behind this summary — every shipped
behavior, deferred edge case, and known gap in full detail — is
[agents/ROADMAP.md](agents/ROADMAP.md).
