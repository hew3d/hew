# Hew — Import and export

Hew's native format is a documented, open zip container (`.hew` — see
`docs/HEW_FILE_FORMAT.md`), used for saving and loading with no round-trip
loss. Everything else — COLLADA, SketchUp, glTF, STL — is interop: a way to
bring outside geometry in, or send Hew models out to other tools.

## Format matrix

| Format | Direction | Status | Notes |
|---|---|---|---|
| `.hew` (native) | save / load | Supported | Zip container, JSON manifest, binary geometry buffers. Lossless round-trip; the only format that preserves components/instances, tags, guides, and document metadata. |
| COLLADA `.dae` | Import | Supported | SketchUp's own COLLADA export, and any COLLADA 1.4 source. Geometry, materials/textures, node hierarchy, and component instancing all map through. |
| SketchUp `.skp` (2017) | Import | Supported | Direct binary read via OpenSKP (see below). Names, materials, textures, tags (from layers), guides, and component instancing carry over. |
| glTF / GLB | Import | Supported | `.gltf` and `.glb`, including Hew's own export output. Geometry, materials, and embedded textures round-trip; see known gaps below. |
| glTF / GLB | Export | Supported | Binary `.glb`, authored via three.js `GLTFExporter` over the live render scene. Y-up, meters. |
| STL | Export | Supported | Binary STL, Z-up, millimeters (the slicer convention). Export is gated on the model being watertight — a non-solid Object is flagged before writing. |
| STL | Import | Not implemented | No `.stl` reader exists yet. |
| OBJ | Import / Export | Not implemented | No OBJ path exists in either direction. |
| 3MF | Import / Export | Not implemented | No work has started on 3MF. |
| STEP / IGES | Import / Export | Planned, not started | Planned as a C++ sidecar using OpenCASCADE, invoked from the desktop shell and converting through the native format. A full B-rep converter is a heavyweight piece of work and is not on the near-term path. |

## Import pipeline

Every import format funnels through the same shape: **parse → heal → build
Objects**.

1. **Parse.** Each format has its own crate (`dae-import`, `skp-import`,
   `gltf-import`) that reads the source bytes and extracts raw geometry
   (positions, per-face vertex loops, material references), independent of
   Hew's internal representation. These crates are I/O-free — they take
   bytes in and, for COLLADA, a caller-resolved image map (COLLADA references
   external texture files; SketchUp and glTF embed them) — and never touch
   the filesystem or network themselves.

2. **Heal.** Raw mesh data from other tools is rarely watertight by Hew's
   standards: exporters triangulate everything, SketchUp emits T-junctions
   and occasional inside-out faces, and floating-point noise leaves
   near-duplicate vertices. A shared `mesh-heal` crate (used by both
   `dae-import` and `gltf-import`) runs a fixed pipeline over each mesh:
   - **Unit and up-axis normalization** — scale to meters, rotate the
     source's up-axis onto Hew's +Z.
   - **Weld** — merge vertices within a fixed tolerance and remap faces.
   - **Drop zero-area faces** — collinear sliver triangles from T-junctions.
   - **Two-sided dedup** — collapse the duplicate reversed-winding faces some
     exporters emit for "double-sided" rendering.
   - **T-junction splicing** — insert vertices that sit on a neighboring
     face's edge, so half-edges pair up into a closed shell.
   - **Orientation** — flip a closed but inside-out shell so normals face
     outward.
   - **Coplanar merge** — rebuild a triangulated flat face back into a single
     n-gon, so it behaves as one drawable face rather than a triangle soup.

   This is boundary normalization, not silent repair of kernel invariants:
   the healing steps are geometric operations on the *input*, applied before
   anything reaches the kernel's own validated data structures. A mesh that
   is still degenerate or non-manifold after healing is not patched further —
   it is handed to the kernel, which refuses it and reports why.

3. **Build Objects.** The healed geometry, now expressed as a
   format-independent recipe (materials, meshes, component definitions,
   instances, groups), is handed to the kernel's `Document::ingest`. This is
   the same insertion path used for loading a `.hew` file: it creates
   Objects, resolves materials into the document's palette, wires up
   component definitions and instances, and reports which meshes became
   watertight solids, which are open ("leaky") shells, and which were
   skipped outright as invalid. Import is additive and undoable as a single
   step — it never touches existing geometry, and one undo removes
   everything an import added.

### What survives from each format

| | Names | Hierarchy / instancing | Materials & textures | Tags / layers | Guides |
|---|---|---|---|---|---|
| COLLADA `.dae` | Yes | Yes (nodes, component instancing) | Yes | No | No |
| SketchUp `.skp` | Yes | Yes (components/groups as instances) | Yes | Yes (layers → tags) | Yes |
| glTF / GLB | Yes (node/mesh names) | No — instancing flattens to independent Objects | Yes, deduplicated by content (identical materials collapse to one) | No | No |

## SketchUp import provenance

`.skp` import is powered by **OpenSKP**
(https://github.com/hew3d/openskp, published on crates.io as `openskp`), a
clean-room SketchUp file reader written specifically for this purpose. It is
derived solely from:

- self-authored test files, built and inspected independently;
- those same files' COLLADA exports, used as ground truth to cross-check the
  binary reader's output;
- public knowledge of the MFC `CArchive` serialization convention that
  SketchUp's file format is built on.

No Trimble SketchUp SDK material — headers, constants, or format knowledge
obtained under Trimble's license — exists anywhere in OpenSKP or in Hew's
dependency chain on it. That separation is a hard project rule, not a
one-time check: every future change to OpenSKP or `crates/skp-import` must
keep holding it.

**Supported:** SketchUp's 2017 classic binary format (internally versioned
`{17.x}`). Every SketchUp release since can save back down to 2017 (File ▸
Save As ▸ SketchUp Version 2017), so this one decoded version covers
effectively all SketchUp content in circulation.

**Known limitations:**
- Newer `.skp` versions (2018 and later) are detected but rejected with a
  message pointing at the Save As 2017 workaround, rather than partially
  imported.
- `.skp` write/export does not exist — OpenSKP is a reader only.
- As with all imports, a mesh that is invalid even after healing (genuinely
  non-manifold source geometry) is skipped and reported, not repaired.

## Known limitations & future work

- **Non-manifold input is reported, not fixed.** Across every import path,
  a face or mesh that the kernel's topology validator rejects — even after
  the full healing pipeline — is dropped and listed in the import report
  with its reason. Hew will not silently produce a solid that doesn't match
  its source; this is a deliberate project-wide rule, not a gap to be closed
  later.
- **glTF/GLB flattens component instancing.** glTF has no first-class concept
  of a shared, editable component, so repeated geometry re-imports as
  independent Objects rather than instances of one definition. `.hew`,
  `.dae`, and `.skp` all preserve instancing; glTF is interchange-only in
  this respect.
- **glTF/GLB material identity collapses to content.** Materials that share
  a color and texture merge into one on import, even if the source file gave
  them distinct names or slots.
- **Curved/faceted surfaces stay triangulated on glTF import.** Coplanar
  merge only reconstructs a face where the source triangles are actually
  coplanar; genuinely curved geometry keeps its triangle-fan structure.
- **No STL, OBJ, or 3MF import**, and no OBJ or 3MF export, in either
  direction.
- **STEP/IGES is planned but not started.** It is scoped as an OpenCASCADE
  sidecar process rather than an in-process crate, reflecting how much
  heavier a full B-rep converter is than a mesh importer.
