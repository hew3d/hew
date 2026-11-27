# Hew — Import and export

Hew's native format, `.hew`, is a documented, open zip container — see
`docs/dev/HEW_FILE_FORMAT.md`. Save and load through it with no round-trip
loss. Everything else — COLLADA, SketchUp, glTF, STL, 3MF — is interop:
bring outside geometry in, or send Hew models out to other tools. SketchUp
`.skp` is both: it opens and saves as a document format, and exports as a copy.

## Format matrix

| Format | Direction | Status | Notes |
|---|---|---|---|
| `.hew` (native) | save / load | Supported | Zip container, JSON manifest, binary geometry buffers. Lossless round-trip; the only format that preserves components/instances, tags, guides, and document metadata. |
| COLLADA `.dae` | Import | Supported | SketchUp's own COLLADA export, and any COLLADA 1.4 source. Geometry, materials/textures, node hierarchy, and component instancing all map through. |
| SketchUp `.skp` | Import / open | Supported | Direct read via Skpwright (see below) of every SketchUp version, 3 through 2027: the 2021-and-later container every SketchUp since 2021 saves, and every older stream-format release (3 to 8 and 2013 to 2020). Names, materials, textures, tags (from layers), guides, scenes, and component instancing carry over. File ▸ Open makes the file the document's own; File ▸ Import… adds it to the current document. |
| SketchUp `.skp` | Save / Save As / Export | Supported | Writes every SketchUp version, 3 through 2027. Written in the 2021-and-later container (`skp` on the API, `skp2026` in the app; the lowest revision that holds the content, or with `compat` / "Avoid features older SketchUp versions cannot open" revision 17, which leaves out what only the newest SketchUp opens: layer folders, hatch patterns, PBR materials, environments, snaps, procedures, page extras and the coordinate system) or as any stream release Skpwright writes: `skp3` to `skp8` and `skp2013` to `skp2020` (SketchUp 3 to 8 and 2013 to 2020), each in its own release's layout (a file opens in the release it was written for and every later one). Save writes back in the format the file was opened in, so every file of release 3 to 8 or 2013 to 2020 saves in its own release; A file of revision 17 saves back at revision 17. Save As and Export choose one: the 2021-and-later container first, then an "Older SketchUp versions" group of the stream releases, newest to oldest, preselected to the file's own release. A file of SketchUp 2021 to 2027 is read as the 2021-and-later container and saved back in it, because those releases share that container and its revision gate. Releases 3 and 4 carry fewer kinds of content than newer ones (no layer or attribute records, and release 3 neither hidden state nor radial dimensions); the writer's notes say what was left out. What a SketchUp file cannot hold exactly, and what an older release's format or revision 17 leaves out (one entry per kind of content, for the document's own content and for what the opened file carried), is reported as a short list in the app (a one-time notice per document for Save, every time for Save As and Export); nothing is reported when nothing is lost. |
| glTF / GLB | Import | Supported | `.gltf` and `.glb`, including Hew's own export output. Geometry, materials, and embedded textures round-trip; see known gaps below. |
| glTF / GLB | Export | Supported | Binary `.glb`, written by the shared kernel-side writer in `mesh-export`. Y-up, meters. |
| STL | Export | Supported | Binary STL, Z-up, millimeters (the slicer convention). Export is gated on the model being watertight — a non-solid Object is flagged before writing. |
| STL | Import | Supported | Binary and ASCII, auto-detected (`stl-import`). STL carries no units, so the app asks for the source unit on import. |
| PNG / JPEG picture | Import (place) | Supported | File ▸ Import Image…, a dropped picture, or `hew.doc.import` of a `.png`/`.jpg` (`image-import`): placed as a component holding one face painted with the picture, the shape a `.skp` image arrives in. The bytes are kept verbatim; over 64 megapixels is refused. |
| OBJ | Import / Export | Not implemented | No OBJ path exists in either direction. |
| 3MF | Export | Supported | OPC container with explicit millimeter units, Z-up; one `<object>` per world object / instance member carrying its display name and per-face colors (core-spec `basematerials`). Solid-gated like STL. |
| 3MF | Import | Not implemented | No `.3mf` reader exists yet. |
| USDZ | Export | Supported | Uncompressed USD zip (`model.usda`), Y-up, meters; one `Mesh` per part with `UsdPreviewSurface` materials. Validates under the ARKit profile, so it opens directly in AR Quick Look. |
| SVG line drawing | Export | Supported | True-size hidden-line drawing of a view at a chosen scale, millimeter `viewBox`, hidden lines removed or dashed; the same engine (`hlr`) that draws vector Line art on a printed page. Also headless as `hew.view.line_drawing`. |
| PDF | Export (print) | Supported | File ▸ Print… ▸ Save PDF…: the composed print pages (Standard or Scaled, tiled), written by `pdfwrite` — vector Line art stays vector; also headless as `hew.print.pdf`. |
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
   is still non-manifold after healing is split at the offending edges into
   separate open shells — the geometry is unchanged, and the import report
   says so loudly. Anything still invalid after that is handed to the
   kernel, which refuses it and reports why.

3. **Build Objects.** The kernel's `Document::ingest` takes the healed
   geometry, now expressed as a format-independent recipe (materials,
   meshes, component definitions, instances, groups). This is the same
   insertion path used for loading a `.hew` file: it creates Objects,
   resolves materials into the document's palette, wires up component
   definitions and instances, and reports which meshes became watertight
   solids, which are open ("leaky") shells, and which were skipped outright
   as invalid. Import is additive and undoable as a single step — it never
   touches existing geometry, and one undo removes everything an import
   added.

### What survives from each format

| | Names | Hierarchy / instancing | Materials & textures | Tags / layers | Guides |
|---|---|---|---|---|---|
| COLLADA `.dae` | Yes | Yes (nodes, component instancing) | Yes | No | No |
| SketchUp `.skp` | Yes | Yes (components/groups as instances) | Yes | Yes (layers → tags) | Yes |
| glTF / GLB | Yes (node/mesh names) | No — instancing flattens to independent Objects | Yes, deduplicated by content (identical materials collapse to one) | No | No |

## SketchUp import provenance

`.skp` import is powered by **Skpwright**
(https://github.com/hew3d/skpwright, published on crates.io as `skpwright`), a
clean-room SketchUp file reader written specifically for this purpose. It is
derived solely from:

- self-authored test files, built and inspected independently;
- those same files' COLLADA exports, used as ground truth to cross-check the
  binary reader's output;
- public knowledge of the MFC `CArchive` serialization convention that
  SketchUp's file format is built on.

No Trimble SketchUp SDK material — headers, constants, or format knowledge
obtained under Trimble's license — exists anywhere in Skpwright or in Hew's
dependency chain on it. That separation is a hard project rule, not a
one-time check: every future change to Skpwright or `crates/skp-import` must
keep holding it.

**Supported, read and write: every SketchUp version, 3 through 2027.** The
2021-and-later container (internally versioned `{26.x}`), which is what
every SketchUp release since 2021 saves, the free web app included — a
`.skp` saved today is this format. Also the
older stream format (`{3.x}` to `{8.x}` and `{13.x}` to `{20.x}`: SketchUp 3
to 8 and 2013 to 2020), including the 2017 release, the last free desktop
version, which many existing models still carry. All come out of the same
reader as the same model: the importer's test suite holds
every corpus file's 2026 save to its fully decoded 2017 original, and a
10.7 MB production model imports to the same document from either.

**Known limitations:**
- A file is read by its body, not its header's version: a stream
  container or a ZIP container goes to Skpwright whatever version the header
  says. A stream file of a release Skpwright does not list (9 to 12, or
  later than 2020, none of which has been observed) is read as far as
  Skpwright can read it and opens with a warning that its release is
  unlisted; it is refused, with its version, only when Skpwright itself
  cannot read it (an error, or nothing in it). Saving it back writes the
  nearest release Skpwright writes, and the writer's notes say so. An intact
  file whose container is neither layout and whose version is not one the
  reader decodes is refused with its version; a file of a decoded
  version that is cut short or damaged is reported as such instead.
- What gates opening in SketchUp is the model revision, not the header's version or label: revision 18 and below open in SketchUp 2027, and SketchUp 2027 desktop refuses a hand-edited file of revision 19 or above (the web app accepted it; no real file above 18 exists). Hew reads any revision (one above 18 as 18) and writes only revision 17 or 18. SketchUp 2021 to 2025 are supported: they use the same container, and the files to hand them are the revision 17 ones.
- Hatch patterns, environments, snaps, procedures, layer line styles, the current style record and layer page behaviours are kept through a save back verbatim but are neither shown nor editable; an open reports a count of each kind in one line.
- Geo reference (coordinate reference system) opens and is shown read-only at the top of Model Info; `hew.geo.get` / `hew.geo.set` read and set it. Layer folders map to tags, and scene timing, guide dash and the section plane's look are kept.
- Physically based materials: metallic and roughness are editable (sliders in the material palette, `hew.material.set_pbr`) and drawn with a Phong approximation; the maps and the workflow are kept as the file held them.
- `compat` (revision 17) applies only to the 2021-and-later container; asking for it with any other target is refused as `format_mismatch`.
- A save back matches the file's scenes (pages) to Hew's by name, so a scene renamed in Hew loses the file-side state of the page it replaces (its display name, timing, environment, hidden entities, style, and shadow and thumbnail settings).
- A record a future SketchUp release adds to the 2026 container is
  reported in the import warnings, not silently dropped.
- Section planes: the cut of the scene the file was saved on, or else the
  model's own active plane, becomes Hew's section plane, from either
  format. Hew has one section plane, so a deactivated plane and planes
  inside groups and components are not imported.
- A `.skp` cannot hold everything a Hew document can (for example, curved
  surfaces are written as smooth facets, and an object with several tags
  keeps only its first). Writing reports what it simplified; the `.hew`
  format remains the lossless one.
- As with all imports, a mesh that is still non-manifold after healing
  imports as separate open shells with a warning; geometry the kernel
  rejects outright is skipped and reported, not repaired.

## Known limitations & future work

- **Non-manifold input is split or reported, never fixed.** Across every
  import path, a mesh that is still non-manifold after the healing pipeline
  imports as separate open shells, split at the offending edges with the
  geometry left untouched, and the import report calls it out. A face or
  mesh that the kernel's topology validator still rejects is dropped and
  listed with its reason. Hew will not silently produce a solid that
  doesn't match its source; this is a deliberate project-wide rule, not a
  gap to be closed later.
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
- **STL import carries no structure.** STL is a bare triangle soup — no
  names, materials, units, or instancing — so imported parts arrive as
  anonymous solids at the unit the user picks.
- **No OBJ or 3MF import**, and no OBJ export.
- **STEP/IGES is planned but not started.** It is scoped as an OpenCASCADE
  sidecar process rather than an in-process crate, reflecting how much
  heavier a full B-rep converter is than a mesh importer.
