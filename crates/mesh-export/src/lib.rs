//! STL, 3MF, and GLB writers for `hew.doc.export` (docs/HEW_API.md §7,
//! `format` "stl" | "3mf" | "glb" (alias "gltf")) — shared by every host
//! that implements the command (`crates/hew-cli`'s `CliHost`,
//! `crates/wasm-api`'s `LiveHost`, and `crates/wasm-api`'s interactive
//! `Scene::export` used directly by the desktop app's File > Export),
//! since the writers themselves have nothing host-specific about them: a
//! document in, bytes out. This is now the ONLY set of mesh exporters in
//! the tree — the app's former TypeScript writers
//! (`app/src/io/exporters/{stlExport,threeMfExport,gltfExport}.ts`, which
//! ran over the live three.js scene) have been retired in its favor; their
//! test coverage lives on as this module's own tests.
//!
//! This crate deliberately does NOT depend on `crates/api`: pulling the
//! API layer in here would drag its refusal machinery, and this crate's
//! own `zip`/`serde_json` dependency chain, into places that should not
//! have them (DEVELOPMENT.md rule 1 — kernel-class crates stay clear of
//! the API layer). Instead, failures are a local [`ExportError`] that
//! exposes just what a host needs to build its own typed refusal: a
//! stable machine [`ExportError::name`] and a human
//! [`ExportError::message`]. Those names are a published protocol
//! contract (`crates/api/src/registry.rs`'s `hew.doc.export` refusal
//! inventory, docs/API_REFERENCE.gen.md) — pinned byte-identical by this
//! module's own tests.
//!
//! The 3MF and GLB writers share [`collect_export_solids`]: the same
//! traversal [`export_stl`] uses (`visible_object_ids` plus every placed
//! instance's definition members, poses baked in, winding flipped under a
//! reflected instance pose, triangles from
//! `tessellate::export_triangles_with_faces`) — kept as one named
//! [`ExportSolid`] per solid instead of STL's flattened anonymous soup,
//! since both formats keep per-part structure, and additionally carrying
//! one resolved color/material/UV per triangle (see "Materials" below).
//!
//! **Non-solid inclusion.** Every writer takes a `solids_only` flag.
//! `hew.doc.export` (the headless CLI and the live-API dispatch command)
//! always passes `true`: a non-interactive caller has no dialog to warn it
//! first, so a leaky object is dropped rather than silently handed back in
//! a file it didn't ask to review (`export_stl`'s long-standing behavior).
//! The desktop app's own `Scene::export` passes `false`, matching what its
//! retired TypeScript writers always did: the Object Info / solid-gating
//! dialog (`app/src/App.tsx`'s `collectNonSolidObjects`) WARNS about
//! non-solid objects before an STL/3MF export and never gates glTF at all,
//! but in every case the export itself has always included them — the
//! warning names the risk, it does not filter the file. Switching the app
//! onto solids-only writers here would have silently started dropping
//! geometry the app used to include.
//!
//! **Materials.** A face's effective material is its own
//! (`Face::material`) falling back to its object's
//! (`Object::default_material`), then the document's palette
//! (`Document::materials`) — the exact resolution `crates/tessellate`'s own
//! `tessellate()` uses for the live viewport, so a painted model exports
//! the same colors it renders with. A re-faceted cylinder band (true
//! curves, `segments_per_turn != 0`) has no single originating face for
//! its resampled triangles (the re-facet deliberately abandons the
//! original per-facet boundaries); those triangles take on the material of
//! the band's first face in slot order (see
//! `tessellate::export_triangles_with_faces`) — exact when a curved wall
//! is uniformly painted (the common case), an honest, documented
//! approximation when it is not. UV coordinates (for GLB textures) use the
//! same per-face `UvFrame`/planar-`world_size` projection `tessellate()`
//! uses, evaluated at each exported vertex's position — exact for stored
//! facets, approximate on a re-faceted band for the reason above.
//!
//! **Session normalization.** `Document::open_explode_session`/`open_group_
//! session` bake an entered definition's members into world-owned objects
//! for the session's duration and HIDE every sibling placement of that
//! component (so editing one of eight identical chairs doesn't show the
//! other seven mid-bake twice). `visible_object_ids`/`instance_ids` (this
//! traversal's own inputs) honor that hiding, so exporting mid-session
//! used to silently drop every hidden sibling — a live `hew.doc.export`
//! disagreed with `hew.doc.save` about what the document even was, though
//! `save` was always correct: `Document::save_for_persistence` already
//! normalized against exactly this by writing a session-closed clone
//! instead of the live document. Every entry point below opens with
//! `Document::session_closed` for the same reason `save_for_persistence`
//! does — a live export must see what a live save sees (docs/HEW_API.md:
//! "the same command produces the same file headless or live").

use kernel::{Document, FaceId, MaterialId, MaterialPalette, Object, Point3, Rgba8};
use std::collections::BTreeMap;
use std::io::Cursor;

// ----------------------------------------------------------------- errors

/// Typed export failures — this crate has no host of its own to turn
/// these into a refusal, so it hands back the two things one needs: the
/// stable machine [`name`](ExportError::name) and a human
/// [`message`](ExportError::message).
#[derive(Debug)]
pub enum ExportError {
    /// No watertight solid survived the traversal — an empty document, or
    /// every solid tessellating to nothing.
    NothingToExport,
    /// Re-faceting a solid failed outright.
    Failed(tessellate::TessellateError),
    /// [`export`]'s format dispatch saw a `format` none of the writers
    /// recognize.
    UnsupportedFormat(String),
}

impl ExportError {
    /// The stable machine name (§4.4's `refusal` field) — byte-identical
    /// to what `hew.doc.export`'s refusal inventory has always declared,
    /// pinned by this module's own tests.
    pub fn name(&self) -> &'static str {
        match self {
            ExportError::NothingToExport => "nothing_to_export",
            ExportError::Failed(_) => "export_failed",
            ExportError::UnsupportedFormat(_) => "host_capability_missing",
        }
    }

    /// The plain-language explanation.
    pub fn message(&self) -> String {
        match self {
            ExportError::NothingToExport => {
                "This document has no watertight solids to export. Check the Object Info panel for open (non-watertight) objects.".to_string()
            }
            ExportError::Failed(e) => format!("re-faceting object failed: {e}"),
            ExportError::UnsupportedFormat(format) => {
                format!("Unrecognized export format \"{format}\".")
            }
        }
    }
}

// -------------------------------------------------------------- dispatch

/// Exports `doc` to `format` (`"stl" | "3mf" | "glb" | "gltf"` — the alias
/// `hew.doc.export`'s registry entry declares, every writer that
/// implements one implementing both), routing to the matching writer
/// below. The one call every host makes; an unrecognized `format` answers
/// [`ExportError::UnsupportedFormat`], which maps to the same
/// `host_capability_missing` name the API surface already uses for "this
/// host can't do that."
/// `solids_only` — see the module doc's "Non-solid inclusion" section:
/// `true` for `hew.doc.export` (headless/live-API dispatch), `false` for
/// the desktop app's interactive `Scene::export`.
pub fn export(
    doc: &Document,
    format: &str,
    segments_per_turn: u32,
    solids_only: bool,
) -> Result<Vec<u8>, ExportError> {
    match format {
        "stl" => export_stl(doc, segments_per_turn, solids_only),
        "3mf" => export_3mf(doc, segments_per_turn, solids_only),
        "glb" | "gltf" => export_glb(doc, segments_per_turn, solids_only),
        other => Err(ExportError::UnsupportedFormat(other.to_string())),
    }
}

// -------------------------------------------------------- shared collection

/// One exported solid: a display name plus its world-space triangle soup
/// (meters, 9 `f64`s per triangle, CCW from outside) — pose baked in,
/// winding flipped under a mirrored (negative-determinant) instance pose —
/// plus one resolved color/material/UV-pair per triangle (module doc,
/// "Materials"): `colors` and `materials` have one entry per triangle;
/// `uvs` has one `[u, v]` per vertex (3 per triangle, same winding order as
/// `triangles`).
pub(crate) struct ExportSolid {
    pub name: String,
    pub triangles: Vec<f64>,
    pub colors: Vec<Rgba8>,
    pub materials: Vec<Option<MaterialId>>,
    pub uvs: Vec<[f32; 2]>,
}

/// A face's effective material: its own, else its object's default
/// (`Object::default_material`) — the same fallback `tessellate()` uses for
/// the live viewport (crates/tessellate/src/lib.rs).
fn resolve_face_material(object: &Object, fid: FaceId) -> Option<MaterialId> {
    object.faces()[fid].material.or(object.default_material())
}

/// A material's resolved display color, falling back to
/// [`tessellate::DEFAULT_MATERIAL_RGBA`] for an unpainted face or a stale
/// id — never fabricated, the exact neutral gray the viewport itself falls
/// back to.
fn resolve_color(material_id: Option<MaterialId>, palette: &MaterialPalette) -> Rgba8 {
    material_id
        .and_then(|id| palette.get(id))
        .map(|m| m.color)
        .unwrap_or(tessellate::DEFAULT_MATERIAL_RGBA)
}

/// A point's UV under a face's own mapping: its `UvFrame` when the face has
/// one (imported texcoords), else the planar projection divided by the
/// material's texture `world_size` (1×1 m for an untextured/unpainted
/// face) — the identical two-branch resolution `tessellate()` applies per
/// vertex (crates/tessellate/src/lib.rs), evaluated here at whatever
/// position the caller has (a stored facet's own vertex, or a re-faceted
/// band's resampled station).
fn resolve_uv(
    object: &Object,
    fid: FaceId,
    material_id: Option<MaterialId>,
    palette: &MaterialPalette,
    p: Point3,
) -> [f32; 2] {
    let face = &object.faces()[fid];
    if let Some(frame) = face.uv_frame {
        let uv = frame.apply(p);
        return [uv[0] as f32, uv[1] as f32];
    }
    let (u_ax, v_ax) = tessellate::plane_basis(face.plane.normal());
    let world_size = material_id
        .and_then(|id| palette.get(id))
        .and_then(|m| m.texture.as_ref())
        .map(|t| t.world_size)
        .unwrap_or([1.0, 1.0]);
    let pv = p.to_vec();
    [
        (pv.dot(u_ax) / world_size[0]) as f32,
        (pv.dot(v_ax) / world_size[1]) as f32,
    ]
}

/// Resolves color/material/UV for every triangle in `raw` (9 `f64`s each,
/// object-local), given the parallel per-triangle `face_ids`
/// `tessellate::export_triangles_with_faces` returns. `vertex_order`
/// selects which of a triangle's 3 vertices to read UVs from in which
/// order — `[0, 1, 2]` normally, `[0, 2, 1]` under a mirrored instance pose
/// so the emitted UVs stay aligned with the (also-reordered) positions.
fn resolve_triangle_materials(
    object: &Object,
    palette: &MaterialPalette,
    raw: &[f64],
    face_ids: &[FaceId],
    vertex_order: [usize; 3],
) -> (Vec<Rgba8>, Vec<Option<MaterialId>>, Vec<[f32; 2]>) {
    let tri_count = face_ids.len();
    let mut colors = Vec::with_capacity(tri_count);
    let mut materials = Vec::with_capacity(tri_count);
    let mut uvs = Vec::with_capacity(tri_count * 3);
    for (t, &fid) in face_ids.iter().enumerate() {
        let mat_id = resolve_face_material(object, fid);
        colors.push(resolve_color(mat_id, palette));
        materials.push(mat_id);
        let o = t * 9;
        let verts = [
            Point3::new(raw[o], raw[o + 1], raw[o + 2]),
            Point3::new(raw[o + 3], raw[o + 4], raw[o + 5]),
            Point3::new(raw[o + 6], raw[o + 7], raw[o + 8]),
        ];
        for &i in &vertex_order {
            uvs.push(resolve_uv(object, fid, mat_id, palette, verts[i]));
        }
    }
    (colors, materials, uvs)
}

/// Walks `doc`'s solids exactly as [`export_stl`] does, but keeps one
/// named [`ExportSolid`] per top-level object and per placed instance
/// member rather than flattening everything into one soup. Display names
/// prefer the document's own object/instance names (`hew.query`'s own
/// source, `Document::object_name`/`instance_name`), falling back to a
/// stable `Object N` counter — deliberately a plain export-local ordinal
/// rather than the kernel's internal handle, which is not a meaningful
/// label to a human opening the file in another tool.
pub(crate) fn collect_export_solids(
    doc: &Document,
    segments_per_turn: u32,
    solids_only: bool,
) -> Result<Vec<ExportSolid>, ExportError> {
    // Session normalization (module doc, "Session normalization"): a live
    // export must see what a live save sees.
    let normalized = doc.session_closed();
    let doc: &Document = &normalized;

    let mut solids: Vec<ExportSolid> = Vec::new();
    let palette = doc.materials();

    for id in doc.visible_object_ids() {
        if solids_only && !doc.object_solid(id) {
            continue;
        }
        let Some(obj) = doc.object(id) else {
            continue;
        };
        let (triangles, face_ids) = tessellate::export_triangles_with_faces(obj, segments_per_turn)
            .map_err(ExportError::Failed)?;
        if triangles.is_empty() {
            continue;
        }
        let (colors, materials, uvs) =
            resolve_triangle_materials(obj, palette, &triangles, &face_ids, [0, 1, 2]);
        let name = doc
            .object_name(id)
            .map(str::to_string)
            .unwrap_or_else(|| format!("Object {}", solids.len() + 1));
        solids.push(ExportSolid {
            name,
            triangles,
            colors,
            materials,
            uvs,
        });
    }

    for instance in doc.instance_ids() {
        let Some(def) = doc.instance_def(instance) else {
            continue;
        };
        let Some(pose) = doc.instance_pose(instance) else {
            continue;
        };
        // Nested definitions expand recursively: one export part per leaf
        // placement, each at its fully composed pose.
        let placements = doc.expanded_def_placements(def);
        let inst_name = doc.instance_name(instance).map(str::to_string);
        let multi_member = placements.len() > 1;

        for (member, local) in placements {
            let pose = local.then(&pose);
            let flip = pose.determinant() < 0.0;
            let vertex_order: [usize; 3] = if flip { [0, 2, 1] } else { [0, 1, 2] };
            if solids_only && !doc.object_solid(member) {
                continue;
            }
            let Some(obj) = doc.object(member) else {
                continue;
            };
            let (raw, face_ids) = tessellate::export_triangles_with_faces(obj, segments_per_turn)
                .map_err(ExportError::Failed)?;
            if raw.is_empty() {
                continue;
            }
            let (colors, materials, uvs) =
                resolve_triangle_materials(obj, palette, &raw, &face_ids, vertex_order);
            let mut triangles = Vec::with_capacity(raw.len());
            for tri in raw.chunks(9) {
                let verts = [
                    Point3::new(tri[0], tri[1], tri[2]),
                    Point3::new(tri[3], tri[4], tri[5]),
                    Point3::new(tri[6], tri[7], tri[8]),
                ];
                for &i in &vertex_order {
                    let p = pose.apply_point(verts[i]);
                    triangles.push(p.x);
                    triangles.push(p.y);
                    triangles.push(p.z);
                }
            }
            let member_name = doc.object_name(member).map(str::to_string);
            // Mirrors threeMfExport.ts's `collectExportParts` naming table:
            // a single-member instance is named after the instance alone; a
            // multi-member one qualifies with the member's own name (or a
            // stable ordinal if the member itself is unnamed).
            let name = match (&inst_name, &member_name) {
                (Some(inst), _) if !multi_member => inst.clone(),
                (Some(inst), Some(member_n)) => format!("{inst} · {member_n}"),
                (Some(inst), None) => format!("{inst} · Object {}", solids.len() + 1),
                (None, Some(member_n)) => member_n.clone(),
                (None, None) => format!("Object {}", solids.len() + 1),
            };
            solids.push(ExportSolid {
                name,
                triangles,
                colors,
                materials,
                uvs,
            });
        }
    }

    Ok(solids)
}

// -------------------------------------------------------------------- STL

/// Builds a binary STL from the document's objects: top-level objects plus
/// every placed instance's definition members, poses baked in and winding
/// flipped under a mirrored (negative-determinant) pose — the same recipe
/// the retired `app/src/io/exporters/stlExport.ts`'s `collectKernelTriangles`
/// used (docs/design/api-kernel-map.md §6.3). When `solids_only` is set,
/// non-watertight objects are left out entirely (never a leaky STL) —
/// `hew.doc.export`'s behavior, with no dialog of its own to have warned
/// the caller first; the desktop app instead warns via its own solid-gating
/// dialog and passes `false` here (module doc, "Non-solid inclusion"),
/// exactly matching what its retired TypeScript writer always exported. If
/// nothing survives, refuses [`ExportError::NothingToExport`] rather than
/// writing an empty file.
///
/// Kept independent of [`collect_export_solids`] rather than built on top
/// of it: STL flattens every solid into one anonymous soup (no per-object
/// names, unlike 3MF/GLB, and no material either — module doc), so there is
/// nothing named or painted to collect — the traversal is duplicated, not
/// shared, deliberately mirroring `stlExport.ts`'s own flattened soup.
pub fn export_stl(
    doc: &Document,
    segments_per_turn: u32,
    solids_only: bool,
) -> Result<Vec<u8>, ExportError> {
    // Session normalization (module doc, "Session normalization"): a live
    // export must see what a live save sees.
    let normalized = doc.session_closed();
    let doc: &Document = &normalized;

    let mut soup: Vec<f64> = Vec::new();

    let export_object = |soup: &mut Vec<f64>, id: kernel::ObjectId| -> Result<bool, ExportError> {
        if solids_only && !doc.object_solid(id) {
            return Ok(false);
        }
        let Some(obj) = doc.object(id) else {
            return Ok(false);
        };
        let triangles =
            tessellate::export_triangles(obj, segments_per_turn).map_err(ExportError::Failed)?;
        soup.extend(triangles);
        Ok(true)
    };

    let mut any_solid = false;
    for id in doc.visible_object_ids() {
        any_solid |= export_object(&mut soup, id)?;
    }

    for instance in doc.instance_ids() {
        let Some(def) = doc.instance_def(instance) else {
            continue;
        };
        let Some(pose) = doc.instance_pose(instance) else {
            continue;
        };
        for (member, local) in doc.expanded_def_placements(def) {
            let pose = local.then(&pose);
            let flip = pose.determinant() < 0.0;
            if solids_only && !doc.object_solid(member) {
                continue;
            }
            let Some(obj) = doc.object(member) else {
                continue;
            };
            let triangles = tessellate::export_triangles(obj, segments_per_turn)
                .map_err(ExportError::Failed)?;
            any_solid = true;
            for tri in triangles.chunks(9) {
                let verts = [
                    Point3::new(tri[0], tri[1], tri[2]),
                    Point3::new(tri[3], tri[4], tri[5]),
                    Point3::new(tri[6], tri[7], tri[8]),
                ];
                let order: [usize; 3] = if flip { [0, 2, 1] } else { [0, 1, 2] };
                for &i in &order {
                    let p = pose.apply_point(verts[i]);
                    soup.push(p.x);
                    soup.push(p.y);
                    soup.push(p.z);
                }
            }
        }
    }

    if !any_solid {
        return Err(ExportError::NothingToExport);
    }

    Ok(write_binary_stl(&soup))
}

/// Millimeter-scale binary STL layout (docs/design/api-kernel-map.md §6.3;
/// mirrors `app/src/io/exporters/stlExport.ts`'s `writeBinaryStl`): an
/// 80-byte header, a `u32` triangle count, then 50 bytes per triangle (3×f32
/// normal, 3×3×f32 vertices, a zero `u16` attribute count). Degenerate
/// (near-zero-area) triangles are skipped, never emitted with a fabricated
/// normal. `soup` is meters, 9 `f64`s per triangle, CCW from outside.
fn write_binary_stl(soup: &[f64]) -> Vec<u8> {
    const HEADER_BYTES: usize = 80;
    const METERS_TO_MM: f64 = 1000.0;
    // Degeneracy gate on the squared winding-cross length, in mm^4 — the
    // same threshold the TypeScript writer uses.
    const DEGENERATE_CROSS_LENGTH_SQ_MM4: f64 = 1e-12;

    let mut kept: Vec<([f32; 3], [f32; 9])> = Vec::new();
    for tri in soup.chunks(9) {
        let mm: Vec<f64> = tri.iter().map(|v| v * METERS_TO_MM).collect();
        let (ax, ay, az) = (mm[0], mm[1], mm[2]);
        let (bx, by, bz) = (mm[3], mm[4], mm[5]);
        let (cx, cy, cz) = (mm[6], mm[7], mm[8]);
        let (ux, uy, uz) = (bx - ax, by - ay, bz - az);
        let (vx, vy, vz) = (cx - ax, cy - ay, cz - az);
        let nx = uy * vz - uz * vy;
        let ny = uz * vx - ux * vz;
        let nz = ux * vy - uy * vx;
        let len_sq = nx * nx + ny * ny + nz * nz;
        if len_sq <= DEGENERATE_CROSS_LENGTH_SQ_MM4 {
            continue;
        }
        let inv_len = 1.0 / len_sq.sqrt();
        let normal = [
            (nx * inv_len) as f32,
            (ny * inv_len) as f32,
            (nz * inv_len) as f32,
        ];
        let verts = [
            ax as f32, ay as f32, az as f32, bx as f32, by as f32, bz as f32, cx as f32, cy as f32,
            cz as f32,
        ];
        kept.push((normal, verts));
    }

    let version = env!("CARGO_PKG_VERSION");
    let header_text = format!("Hew {version} binary STL, millimeters");
    let mut header = [0u8; HEADER_BYTES];
    for (i, b) in header_text.bytes().take(HEADER_BYTES).enumerate() {
        header[i] = b & 0x7f;
    }

    let mut out = Vec::with_capacity(HEADER_BYTES + 4 + kept.len() * 50);
    out.extend_from_slice(&header);
    out.extend_from_slice(&(kept.len() as u32).to_le_bytes());
    for (normal, verts) in &kept {
        for c in normal {
            out.extend_from_slice(&c.to_le_bytes());
        }
        for c in verts {
            out.extend_from_slice(&c.to_le_bytes());
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    out
}

// -------------------------------------------------------------------- 3MF

/// The 3MF core-spec model namespace (same as `threeMfExport.ts`'s
/// `MODEL_XMLNS`).
const MODEL_3MF_XMLNS: &str = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02";

/// Exports `doc` to a 3MF container (docs/HEW_API.md §7): one `<object>`
/// mesh per exported solid, vertices deduplicated per object (bit-identical
/// positions only, never a tolerance), `unit="meter"` — Hew's own world
/// frame (docs/HEW_API.md §4.3: "+Z up… always"), so unlike the retired
/// `threeMfExport.ts`'s millimeter convention there is no scale to bake and
/// 3MF stays natively Z-up. Colors ride core-spec `<basematerials>` — see
/// [`build_3mf_objects`] for the exact dedup/default/override rule, carried
/// over unchanged from that TypeScript writer. Refuses `nothing_to_export`
/// when nothing survives (an empty document, or every solid collapsing to
/// degenerate triangles).
pub fn export_3mf(
    doc: &Document,
    segments_per_turn: u32,
    solids_only: bool,
) -> Result<Vec<u8>, ExportError> {
    let solids = collect_export_solids(doc, segments_per_turn, solids_only)?;
    write_3mf(&solids).ok_or(ExportError::NothingToExport)
}

struct Model3mfObject {
    id: u32,
    name: String,
    default_color: u32,
    vertex_lines: Vec<String>,
    triangle_lines: Vec<String>,
}

/// Format one coordinate for XML: plain decimal (Rust's `f64` `Display`
/// never emits scientific notation, unlike JS's — no exponent-expansion
/// fallback needed the way `threeMfExport.ts`'s `formatCoord` requires),
/// `-0` normalized to `0`. Vertex welding keys on this string, so the
/// formatting IS the identity.
fn format_coord(n: f64) -> String {
    let n = if n == 0.0 { 0.0 } else { n };
    format!("{n}")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A resolved color as the `#RRGGBB`/`#RRGGBBAA` hex string 3MF's
/// `<base displaycolor="…">` and the retired TypeScript writer both use —
/// uppercase, alpha channel present only when not fully opaque (mirrors
/// `threeMfExport.ts`'s `alphaHex`: `''` at alpha ≥ 1, else two hex digits).
fn color_hex(c: Rgba8) -> String {
    if c.a == 255 {
        format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
    } else {
        format!("#{:02X}{:02X}{:02X}{:02X}", c.r, c.g, c.b, c.a)
    }
}

/// Insertion-ordered color palette: distinct colors dedupe into one
/// document-wide index (module doc, "Materials"; `threeMfExport.ts`'s
/// `paletteIndex`/`colorOf`) — a `BTreeMap` would reorder by hex string
/// instead of first-use order, so this keeps a side `Vec` for iteration.
#[derive(Default)]
struct ColorPalette {
    index: BTreeMap<String, u32>,
    order: Vec<String>,
}

impl ColorPalette {
    fn index_of(&mut self, c: Rgba8) -> u32 {
        let hex = color_hex(c);
        if let Some(&i) = self.index.get(&hex) {
            return i;
        }
        let i = self.order.len() as u32;
        self.index.insert(hex.clone(), i);
        self.order.push(hex);
        i
    }
}

/// Builds one `<object>` per solid, welding vertices within it and
/// dropping triangles a weld collapsed to fewer than 3 distinct vertices
/// (forbidden by the 3MF spec) — mirrors `writeThreeMf`'s per-part loop.
/// Also returns the document-wide [`ColorPalette`] every object's
/// `pid`/`pindex` and triangle-level `p1` override reference.
fn build_3mf_objects(solids: &[ExportSolid]) -> (Vec<Model3mfObject>, ColorPalette) {
    let mut objects = Vec::new();
    let mut palette = ColorPalette::default();
    let mut next_id = 2u32; // 1 is the basematerials group; objects follow.

    for solid in solids {
        let tri_count = solid.triangles.len() / 9;
        if tri_count == 0 {
            continue;
        }

        let mut vertex_index: BTreeMap<String, u32> = BTreeMap::new();
        let mut vertex_lines: Vec<String> = Vec::new();
        let mut triangle_lines: Vec<String> = Vec::new();
        let mut default_color: Option<u32> = None;

        let mut vertex_at = |o: usize| -> u32 {
            let x = format_coord(solid.triangles[o]);
            let y = format_coord(solid.triangles[o + 1]);
            let z = format_coord(solid.triangles[o + 2]);
            let key = format!("{x} {y} {z}");
            if let Some(&i) = vertex_index.get(&key) {
                return i;
            }
            let i = vertex_lines.len() as u32;
            vertex_index.insert(key, i);
            vertex_lines.push(format!("<vertex x=\"{x}\" y=\"{y}\" z=\"{z}\"/>"));
            i
        };

        for t in 0..tri_count {
            let o = t * 9;
            let v1 = vertex_at(o);
            let v2 = vertex_at(o + 3);
            let v3 = vertex_at(o + 6);
            if v1 == v2 || v2 == v3 || v1 == v3 {
                // A collapsed triangle carries no geometry — skipped, never
                // repaired (rule 4 in spirit).
                continue;
            }
            let color = palette.index_of(solid.colors[t]);
            let default = *default_color.get_or_insert(color);
            let p1 = if color == default {
                String::new()
            } else {
                format!(" p1=\"{color}\"")
            };
            triangle_lines.push(format!(
                "<triangle v1=\"{v1}\" v2=\"{v2}\" v3=\"{v3}\"{p1}/>"
            ));
        }

        if triangle_lines.is_empty() {
            continue;
        }
        objects.push(Model3mfObject {
            id: next_id,
            name: solid.name.clone(),
            default_color: default_color.expect("a non-empty object saw at least one triangle"),
            vertex_lines,
            triangle_lines,
        });
        next_id += 1;
    }

    (objects, palette)
}

fn render_3mf_model(objects: &[Model3mfObject], palette: &ColorPalette, version: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string());
    lines.push(format!(
        "<model unit=\"meter\" xml:lang=\"und\" xmlns=\"{MODEL_3MF_XMLNS}\">"
    ));
    lines.push(format!(
        " <metadata name=\"Application\">Hew {}</metadata>",
        xml_escape(version)
    ));
    lines.push(" <resources>".to_string());
    lines.push("  <basematerials id=\"1\">".to_string());
    for hex in &palette.order {
        lines.push(format!("   <base name=\"{hex}\" displaycolor=\"{hex}\"/>"));
    }
    lines.push("  </basematerials>".to_string());
    for obj in objects {
        lines.push(format!(
            "  <object id=\"{}\" type=\"model\" name=\"{}\" pid=\"1\" pindex=\"{}\">",
            obj.id,
            xml_escape(&obj.name),
            obj.default_color,
        ));
        lines.push("   <mesh>".to_string());
        lines.push("    <vertices>".to_string());
        for v in &obj.vertex_lines {
            lines.push(format!("     {v}"));
        }
        lines.push("    </vertices>".to_string());
        lines.push("    <triangles>".to_string());
        for t in &obj.triangle_lines {
            lines.push(format!("     {t}"));
        }
        lines.push("    </triangles>".to_string());
        lines.push("   </mesh>".to_string());
        lines.push("  </object>".to_string());
    }
    lines.push(" </resources>".to_string());
    lines.push(" <build>".to_string());
    for obj in objects {
        lines.push(format!("  <item objectid=\"{}\"/>", obj.id));
    }
    lines.push(" </build>".to_string());
    lines.push("</model>".to_string());
    lines.join("\n") + "\n"
}

const CONTENT_TYPES_XML: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
    "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\n",
    " <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\n",
    " <Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/>\n",
    "</Types>\n"
);

const RELS_XML: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
    "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n",
    " <Relationship Target=\"/3D/3dmodel.model\" Id=\"rel-1\"",
    " Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/>\n",
    "</Relationships>\n"
);

/// Write a single stored (uncompressed) zip entry with a fixed timestamp —
/// same recipe as `crates/kernel/src/serialize.rs`'s `zip_add_stored_entry`
/// (not shared code: that helper is private to the `.hew` container writer),
/// so the same document exports to the same bytes every time.
fn zip_add_stored_entry<W: std::io::Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    name: &str,
    data: &[u8],
) -> std::io::Result<()> {
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    zip.start_file(name, options)?;
    std::io::Write::write_all(zip, data)?;
    Ok(())
}

/// Assembles the minimal core-spec OPC container: `[Content_Types].xml`,
/// `_rels/.rels`, `3D/3dmodel.model` — mirrors `threeMfExport.ts`'s
/// `writeThreeMf`'s three-entry layout. Returns `None` when no solid
/// contributes a triangle (every object empty or fully degenerate).
fn write_3mf(solids: &[ExportSolid]) -> Option<Vec<u8>> {
    let (objects, palette) = build_3mf_objects(solids);
    if objects.is_empty() {
        return None;
    }
    let version = env!("CARGO_PKG_VERSION");
    let model = render_3mf_model(&objects, &palette, version);

    let out_cursor = Cursor::new(Vec::<u8>::new());
    let mut zip = zip::ZipWriter::new(out_cursor);
    zip_add_stored_entry(
        &mut zip,
        "[Content_Types].xml",
        CONTENT_TYPES_XML.as_bytes(),
    )
    .expect("zip write must not fail");
    zip_add_stored_entry(&mut zip, "_rels/.rels", RELS_XML.as_bytes())
        .expect("zip write must not fail");
    zip_add_stored_entry(&mut zip, "3D/3dmodel.model", model.as_bytes())
        .expect("zip write must not fail");
    Some(zip.finish().expect("zip finish must not fail").into_inner())
}

// -------------------------------------------------------------------- GLB

/// Exports `doc` to binary glTF 2.0 (docs/HEW_API.md §7): hand-rolled, no
/// new dependency for the container itself — the retired `gltfExport.ts`
/// leaned entirely on three's `GLTFExporter`, which has no headless
/// equivalent here, so this writes the JSON + BIN chunk container directly
/// from the glTF 2.0 spec. One node + one mesh per solid, one primitive per
/// distinct material the solid uses (module doc, "Materials"): a painted
/// face's material becomes a `pbrMetallicRoughness.baseColorFactor` (sRGB
/// bytes gamma-decoded to the linear space the spec requires), and a
/// textured material's authored image bytes are embedded in the BIN chunk
/// and referenced as `baseColorTexture` alongside the same tint factor —
/// exactly the modulation `kernel::Material` itself defines. An unpainted
/// face shares one `"Default"` material (the same neutral gray the
/// viewport falls back to). Positions (and, for a textured primitive, UVs)
/// are welded per primitive the same way 3MF's vertices are.
///
/// Hew's world frame is +Z up (docs/HEW_API.md §4.3); glTF's convention —
/// and `crates/gltf-import`'s own assumption on the way back in
/// (`mesh_heal::y_up_to_z_up`, a +90° rotation about +X) — is +Y up. This
/// writer applies that rotation's exact inverse, `(x, y, z) → (x, z, −y)`,
/// so a file this writes round-trips through Hew's own importer and reads
/// right-side-up in any conventional glTF consumer (Blender, three.js,
/// …). Winding survives untouched: it's a proper rotation (determinant
/// +1), so it can't flip a triangle's front face.
///
/// Known difference from the retired TypeScript writer: three's
/// `GLTFExporter` emits full node hierarchy and per-instance transforms
/// (an instance is a glTF node referencing shared mesh data); this writer
/// bakes every instance member's pose into world space instead, the same
/// choice 3MF makes and for the same reason (module doc predates this
/// change — core reasons: mirrored instances need winding-flip handling
/// glTF's node transforms don't express, and baking keeps one writer path
/// for both top-level objects and instance members). A file this writes is
/// therefore flatter (no reusable mesh referenced by multiple nodes) but
/// renders and round-trips identically.
pub fn export_glb(
    doc: &Document,
    segments_per_turn: u32,
    solids_only: bool,
) -> Result<Vec<u8>, ExportError> {
    let solids = collect_export_solids(doc, segments_per_turn, solids_only)?;
    write_glb(&solids, doc.materials()).ok_or(ExportError::NothingToExport)
}

/// One glTF material: a resolved tint plus an optional embedded texture
/// (authored bytes + MIME, straight from `kernel::Texture`). Index 0 is
/// always the shared `"Default"` material for unpainted faces.
struct GlbMaterialDef {
    name: String,
    color: Rgba8,
    texture: Option<(Vec<u8>, &'static str)>,
}

/// One primitive: a single material's triangles from one solid, welded
/// independently of every other primitive (glTF primitives own their own
/// attribute accessors, so there is no cross-material vertex sharing to
/// manage). `uvs` is empty for a primitive whose material carries no
/// texture — nothing to sample, so nothing is emitted.
struct GlbPrimitive {
    /// Index into the document-wide `GlbMaterialDef` list.
    material: usize,
    /// f32 xyz per vertex, already Y-up-converted.
    positions: Vec<f32>,
    /// f32 uv per vertex; empty unless the primitive's material has a
    /// texture.
    uvs: Vec<f32>,
    indices: Vec<u32>,
    min: [f32; 3],
    max: [f32; 3],
}

struct GlbMesh {
    name: String,
    primitives: Vec<GlbPrimitive>,
}

/// glTF component type / buffer-view target constants (glTF 2.0 spec §5).
const COMPONENT_TYPE_FLOAT: u32 = 5126;
const COMPONENT_TYPE_UNSIGNED_INT: u32 = 5125;
const TARGET_ARRAY_BUFFER: u32 = 34962;
const TARGET_ELEMENT_ARRAY_BUFFER: u32 = 34963;

/// `-0.0` normalized to `0.0` — keeps vertex welding and min/max stable
/// across bit-identical-but-differently-signed zeros.
fn norm_zero(x: f32) -> f32 {
    if x == 0.0 { 0.0 } else { x }
}

/// sRGB (IEC 61966-2-1) electro-optical transfer function: decodes an
/// 8-bit-per-channel display color (`kernel::Rgba8`, the same bytes the
/// viewport uploads via `THREE.SRGBColorSpace`) to the LINEAR space glTF's
/// `baseColorFactor` requires (glTF 2.0 spec §3.9.2). Applied to color
/// channels only — alpha stays linear, per spec.
fn srgb_channel_to_linear(c: u8) -> f32 {
    let c = f32::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn base_color_factor(c: Rgba8) -> [f32; 4] {
    [
        srgb_channel_to_linear(c.r),
        srgb_channel_to_linear(c.g),
        srgb_channel_to_linear(c.b),
        f32::from(c.a) / 255.0,
    ]
}

/// Builds every solid's meshes (one primitive per distinct material) and
/// the document-wide material list they reference. A cylinder band's
/// representative-face material (module doc, "Materials") applies to every
/// triangle the band contributed, same as its resolved color.
fn build_glb_meshes(
    solids: &[ExportSolid],
    palette: &MaterialPalette,
) -> (Vec<GlbMesh>, Vec<GlbMaterialDef>) {
    let mut materials: Vec<GlbMaterialDef> = vec![GlbMaterialDef {
        name: "Default".to_string(),
        color: tessellate::DEFAULT_MATERIAL_RGBA,
        texture: None,
    }];
    let mut material_index: BTreeMap<MaterialId, usize> = BTreeMap::new();
    let mut meshes = Vec::new();

    for solid in solids {
        let tri_count = solid.triangles.len() / 9;
        if tri_count == 0 {
            continue;
        }

        // Bucket this solid's triangles by resolved material, first-seen
        // order — mirrors `tessellate()`'s own `RenderMesh::groups`.
        let mut bucket_order: Vec<usize> = Vec::new();
        let mut bucket_of_material: BTreeMap<usize, usize> = BTreeMap::new();
        let mut bucket_tris: Vec<Vec<usize>> = Vec::new();
        for t in 0..tri_count {
            let material_idx = match solid.materials[t] {
                None => 0,
                Some(id) => *material_index.entry(id).or_insert_with(|| {
                    let mat = palette.get(id);
                    materials.push(GlbMaterialDef {
                        name: mat
                            .map(|m| m.name.clone())
                            .unwrap_or_else(|| "Material".to_string()),
                        color: solid.colors[t],
                        texture: mat
                            .and_then(|m| m.texture.as_ref())
                            .map(|tex| (tex.image.clone(), tex.format.mime())),
                    });
                    materials.len() - 1
                }),
            };
            let bi = *bucket_of_material.entry(material_idx).or_insert_with(|| {
                bucket_order.push(material_idx);
                bucket_tris.push(Vec::new());
                bucket_tris.len() - 1
            });
            bucket_tris[bi].push(t);
        }

        let mut primitives = Vec::new();
        for (bi, &material_idx) in bucket_order.iter().enumerate() {
            let has_texture = materials[material_idx].texture.is_some();
            let mut vertex_index: BTreeMap<(u32, u32, u32, u32, u32), u32> = BTreeMap::new();
            let mut positions: Vec<f32> = Vec::new();
            let mut uvs: Vec<f32> = Vec::new();
            let mut indices: Vec<u32> = Vec::new();
            let mut min = [f32::MAX; 3];
            let mut max = [f32::MIN; 3];

            let mut vertex_at = |t: usize, k: usize| -> u32 {
                let o = t * 9 + k * 3;
                let x = solid.triangles[o] as f32;
                let y = solid.triangles[o + 1] as f32;
                let z = solid.triangles[o + 2] as f32;
                // Z-up (Hew) -> Y-up (glTF): (x, y, z) -> (x, z, -y).
                let gx = norm_zero(x);
                let gy = norm_zero(z);
                let gz = norm_zero(-y);
                let [uu, vv] = solid.uvs[t * 3 + k];
                // Only a textured primitive's UV participates in welding
                // (and is stored at all) — an untextured primitive dedupes
                // on position alone, unchanged from before materials.
                let (wu, wv) = if has_texture { (uu, vv) } else { (0.0, 0.0) };
                let key = (
                    gx.to_bits(),
                    gy.to_bits(),
                    gz.to_bits(),
                    wu.to_bits(),
                    wv.to_bits(),
                );
                if let Some(&i) = vertex_index.get(&key) {
                    return i;
                }
                let i = (positions.len() / 3) as u32;
                vertex_index.insert(key, i);
                positions.extend([gx, gy, gz]);
                if has_texture {
                    uvs.extend([uu, vv]);
                }
                min[0] = min[0].min(gx);
                min[1] = min[1].min(gy);
                min[2] = min[2].min(gz);
                max[0] = max[0].max(gx);
                max[1] = max[1].max(gy);
                max[2] = max[2].max(gz);
                i
            };

            for &t in &bucket_tris[bi] {
                let i0 = vertex_at(t, 0);
                let i1 = vertex_at(t, 1);
                let i2 = vertex_at(t, 2);
                if i0 == i1 || i1 == i2 || i0 == i2 {
                    continue;
                }
                indices.push(i0);
                indices.push(i1);
                indices.push(i2);
            }

            if indices.is_empty() {
                continue;
            }
            primitives.push(GlbPrimitive {
                material: material_idx,
                positions,
                uvs,
                indices,
                min,
                max,
            });
        }

        if primitives.is_empty() {
            continue;
        }
        meshes.push(GlbMesh {
            name: solid.name.clone(),
            primitives,
        });
    }

    (meshes, materials)
}

/// Assembles the binary container: a 12-byte header, a `JSON` chunk (glTF
/// document: asset/scene/nodes/meshes/accessors/bufferViews/buffer), then a
/// `BIN\0` chunk (positions f32 + indices u32, per mesh, back to back).
/// Each chunk is padded to a 4-byte boundary — spaces (`0x20`) for JSON,
/// zeros for BIN, per the GLB container spec. Returns `None` when no solid
/// survives welding with at least one triangle.
fn write_glb(solids: &[ExportSolid], palette: &MaterialPalette) -> Option<Vec<u8>> {
    let (meshes, materials) = build_glb_meshes(solids, palette);
    if meshes.is_empty() {
        return None;
    }
    let version = env!("CARGO_PKG_VERSION");

    let mut bin: Vec<u8> = Vec::new();
    let mut accessors: Vec<serde_json::Value> = Vec::new();
    let mut buffer_views: Vec<serde_json::Value> = Vec::new();
    let mut gltf_meshes: Vec<serde_json::Value> = Vec::new();
    let mut nodes: Vec<serde_json::Value> = Vec::new();

    for mesh in &meshes {
        let mut primitives: Vec<serde_json::Value> = Vec::new();
        for prim in &mesh.primitives {
            let pos_offset = bin.len();
            for f in &prim.positions {
                bin.extend_from_slice(&f.to_le_bytes());
            }
            let pos_len = bin.len() - pos_offset;
            let pos_bufferview = buffer_views.len();
            buffer_views.push(serde_json::json!({
                "buffer": 0,
                "byteOffset": pos_offset,
                "byteLength": pos_len,
                "target": TARGET_ARRAY_BUFFER,
            }));
            let pos_accessor = accessors.len();
            accessors.push(serde_json::json!({
                "bufferView": pos_bufferview,
                "byteOffset": 0,
                "componentType": COMPONENT_TYPE_FLOAT,
                "count": prim.positions.len() / 3,
                "type": "VEC3",
                "min": prim.min,
                "max": prim.max,
            }));

            let mut attributes = serde_json::json!({ "POSITION": pos_accessor });
            if !prim.uvs.is_empty() {
                let uv_offset = bin.len();
                for f in &prim.uvs {
                    bin.extend_from_slice(&f.to_le_bytes());
                }
                let uv_len = bin.len() - uv_offset;
                let uv_bufferview = buffer_views.len();
                buffer_views.push(serde_json::json!({
                    "buffer": 0,
                    "byteOffset": uv_offset,
                    "byteLength": uv_len,
                    "target": TARGET_ARRAY_BUFFER,
                }));
                let uv_accessor = accessors.len();
                accessors.push(serde_json::json!({
                    "bufferView": uv_bufferview,
                    "byteOffset": 0,
                    "componentType": COMPONENT_TYPE_FLOAT,
                    "count": prim.uvs.len() / 2,
                    "type": "VEC2",
                }));
                attributes["TEXCOORD_0"] = serde_json::json!(uv_accessor);
            }

            let idx_offset = bin.len();
            for i in &prim.indices {
                bin.extend_from_slice(&i.to_le_bytes());
            }
            let idx_len = bin.len() - idx_offset;
            let idx_bufferview = buffer_views.len();
            buffer_views.push(serde_json::json!({
                "buffer": 0,
                "byteOffset": idx_offset,
                "byteLength": idx_len,
                "target": TARGET_ELEMENT_ARRAY_BUFFER,
            }));
            let idx_accessor = accessors.len();
            accessors.push(serde_json::json!({
                "bufferView": idx_bufferview,
                "byteOffset": 0,
                "componentType": COMPONENT_TYPE_UNSIGNED_INT,
                "count": prim.indices.len(),
                "type": "SCALAR",
            }));

            primitives.push(serde_json::json!({
                "attributes": attributes,
                "indices": idx_accessor,
                "material": prim.material,
            }));
        }

        let mesh_index = gltf_meshes.len();
        gltf_meshes.push(serde_json::json!({
            "name": mesh.name,
            "primitives": primitives,
        }));
        nodes.push(serde_json::json!({
            "name": mesh.name,
            "mesh": mesh_index,
        }));
    }

    // Embed each material's texture (if any) as an image in the same BIN
    // chunk — one bufferView + `images[]` entry + `textures[]` entry per
    // texture, glTF 2.0 spec §3.9 ("Images/Textures"). A single sampler is
    // shared by every textured material (repeat wrap, auto filtering).
    let any_textured = materials.iter().any(|m| m.texture.is_some());
    let mut gltf_materials: Vec<serde_json::Value> = Vec::new();
    let mut images: Vec<serde_json::Value> = Vec::new();
    let mut textures: Vec<serde_json::Value> = Vec::new();
    for mat in &materials {
        let mut pbr = serde_json::json!({
            "baseColorFactor": base_color_factor(mat.color),
            "metallicFactor": 0.0,
            "roughnessFactor": 0.8,
        });
        if let Some((image, mime)) = &mat.texture {
            let img_offset = bin.len();
            bin.extend_from_slice(image);
            let img_len = bin.len() - img_offset;
            while !bin.len().is_multiple_of(4) {
                bin.push(0);
            }
            let bufferview_idx = buffer_views.len();
            buffer_views.push(serde_json::json!({
                "buffer": 0,
                "byteOffset": img_offset,
                "byteLength": img_len,
            }));
            let image_idx = images.len();
            images.push(serde_json::json!({
                "bufferView": bufferview_idx,
                "mimeType": mime,
            }));
            let texture_idx = textures.len();
            textures.push(serde_json::json!({ "source": image_idx, "sampler": 0 }));
            pbr["baseColorTexture"] = serde_json::json!({ "index": texture_idx });
        }
        gltf_materials.push(serde_json::json!({
            "name": mat.name,
            "pbrMetallicRoughness": pbr,
        }));
    }

    let mut document = serde_json::json!({
        "asset": { "version": "2.0", "generator": format!("Hew {version}") },
        "scene": 0,
        "scenes": [{ "nodes": (0..nodes.len()).collect::<Vec<_>>() }],
        "nodes": nodes,
        "meshes": gltf_meshes,
        "materials": gltf_materials,
        "accessors": accessors,
        "bufferViews": buffer_views,
        "buffers": [{ "byteLength": bin.len() }],
    });
    if any_textured {
        document["images"] = serde_json::json!(images);
        document["textures"] = serde_json::json!(textures);
        document["samplers"] = serde_json::json!([{}]);
    }

    let mut json_bytes = serde_json::to_vec(&document).expect("glTF JSON serializes");
    while !json_bytes.len().is_multiple_of(4) {
        json_bytes.push(b' ');
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }

    let total_len = 12 + 8 + json_bytes.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total_len);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total_len as u32).to_le_bytes());

    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_bytes);

    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);

    Some(out)
}

// ------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::{Document, ImageFormat, ImportNode, ImportScene, Material, MeshRecipe, Texture};

    /// A unit-cube mesh recipe (CCW-from-outside faces) — same fixture
    /// `crates/hew-cli/src/host.rs`'s own tests use.
    fn unit_box_mesh(name: &str) -> MeshRecipe {
        let positions = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
            Point3::new(0.0, 0.0, 1.0),
            Point3::new(1.0, 0.0, 1.0),
            Point3::new(1.0, 1.0, 1.0),
            Point3::new(0.0, 1.0, 1.0),
        ];
        let faces = vec![
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ];
        let face_count = faces.len();
        MeshRecipe {
            name: name.to_string(),
            positions,
            faces,
            face_materials: vec![kernel::serialize::NO_MATERIAL; face_count],
            face_uv_frames: vec![None; face_count],
            face_holes: vec![Vec::new(); face_count],
            base_material: kernel::serialize::NO_MATERIAL,
            tags: Vec::new(),
        }
    }

    fn box_document() -> Document {
        let mut doc = Document::new();
        let scene = ImportScene {
            materials: Vec::new(),
            defs: Vec::new(),
            roots: vec![ImportNode::Mesh(unit_box_mesh("Box"))],
            guides: Vec::new(),
            tags: Vec::new(),
        };
        doc.ingest(scene, Vec::new()).expect("ingest a plain box");
        doc
    }

    /// A unit box with face 0 painted red, face 1 painted blue, and faces
    /// 2–5 left unpainted (falling back to the default gray) — the fixture
    /// every materials/color test below builds on. Returns the document
    /// plus the two painted `MaterialId`s in dense-index order.
    fn painted_box_document() -> (Document, MaterialId, MaterialId) {
        let mut doc = Document::new();
        let mut mesh = unit_box_mesh("Box");
        mesh.face_materials[0] = 0;
        mesh.face_materials[1] = 1;
        let scene = ImportScene {
            materials: vec![
                Material::solid("Red", Rgba8::rgb(0xff, 0x00, 0x00)),
                Material::solid("Blue", Rgba8::rgb(0x00, 0x00, 0xff)),
            ],
            defs: Vec::new(),
            roots: vec![ImportNode::Mesh(mesh)],
            guides: Vec::new(),
            tags: Vec::new(),
        };
        doc.ingest(scene, Vec::new()).expect("ingest a painted box");
        let ids = doc.material_ids();
        (doc, ids[0], ids[1])
    }

    /// A unit box with face 0 painted with a textured material (a tiny fake
    /// "PNG" — the kernel never decodes image bytes, so any opaque blob
    /// stands in) and the rest left unpainted.
    fn textured_box_document() -> Document {
        let mut doc = Document::new();
        let mut mesh = unit_box_mesh("Box");
        mesh.face_materials[0] = 0;
        let texture = Texture {
            image: b"not a real png, just an opaque authored blob".to_vec(),
            format: ImageFormat::Png,
            world_size: [0.5, 0.5],
        };
        let scene = ImportScene {
            materials: vec![Material::textured(
                "Brick",
                Rgba8::rgb(0x20, 0x40, 0x60),
                texture,
            )],
            defs: Vec::new(),
            roots: vec![ImportNode::Mesh(mesh)],
            guides: Vec::new(),
            tags: Vec::new(),
        };
        doc.ingest(scene, Vec::new())
            .expect("ingest a textured box");
        doc
    }

    /// A single open (non-watertight) quad face — one boundary edge with no
    /// twin, so `Document::object_solid` reports `false`. Used to prove
    /// `solids_only` toggles inclusion the way the module doc promises.
    fn open_quad_document() -> Document {
        let mut doc = Document::new();
        let mesh = MeshRecipe {
            name: "OpenQuad".to_string(),
            positions: vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
            ],
            faces: vec![vec![0, 1, 2, 3]],
            face_materials: vec![kernel::serialize::NO_MATERIAL],
            face_uv_frames: vec![None],
            face_holes: vec![Vec::new()],
            base_material: kernel::serialize::NO_MATERIAL,
            tags: Vec::new(),
        };
        let scene = ImportScene {
            materials: Vec::new(),
            defs: Vec::new(),
            roots: vec![ImportNode::Mesh(mesh)],
            guides: Vec::new(),
            tags: Vec::new(),
        };
        doc.ingest(scene, Vec::new()).expect("ingest an open quad");
        doc
    }

    // ---------------------------------------------------------------- STL

    #[test]
    fn export_stl_produces_a_well_formed_binary_stl_with_twelve_triangles() {
        let doc = box_document();
        let bytes = export_stl(&doc, 0, true).expect("a solid box exports");
        assert_eq!(&bytes[0..5], b"Hew 0");
        let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap());
        assert_eq!(count, 12, "a box triangulates to 12 triangles (2 per face)");
        assert_eq!(bytes.len(), 80 + 4 + 12 * 50);
    }

    #[test]
    fn export_stl_refuses_nothing_to_export_on_an_empty_document() {
        let doc = Document::new();
        let err = export_stl(&doc, 0, true).unwrap_err();
        assert_eq!(err.name(), "nothing_to_export");
    }

    /// A zero-area (degenerate) triangle carries no meaningful normal and
    /// is skipped rather than emitted with a fabricated one
    /// (`write_binary_stl`'s doc comment) — ported from the retired
    /// `stlExport.test.ts`'s "skips zero-area (degenerate) triangles and
    /// counts them", which exercised the pure writer directly on a
    /// synthetic soup the same way.
    #[test]
    fn write_binary_stl_skips_degenerate_triangles() {
        #[rustfmt::skip]
        let soup = [
            // One real triangle in the XY plane (unit right triangle).
            0.0, 0.0, 0.0,  1.0, 0.0, 0.0,  0.0, 1.0, 0.0,
            // One degenerate (collinear, zero-area) triangle.
            0.0, 0.0, 0.0,  1.0, 0.0, 0.0,  2.0, 0.0, 0.0,
        ];
        let bytes = write_binary_stl(&soup);
        let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap());
        assert_eq!(count, 1, "the degenerate triangle is skipped, not repaired");
    }

    /// A mirrored (negative-determinant) instance pose flips triangle
    /// winding under `collect_export_solids` — the same recipe
    /// `stlExport.ts`'s retired `collectKernelTriangles` used, now
    /// exercised through the material-aware traversal (module doc):
    /// per-triangle color/UV must stay aligned with the flipped vertex
    /// order rather than panicking or silently misindexing. Two instances
    /// of the same one-object component (identity + mirror pose) export as
    /// 2 separate 3MF objects, each the box's usual 8 vertices/12
    /// triangles.
    #[test]
    fn export_3mf_handles_a_mirrored_instance_without_corrupting_materials() {
        let mut doc = box_document();
        let obj_id = doc.visible_object_ids()[0];
        let (component, _first_instance, _change) = doc
            .make_component(&[kernel::NodeId::Object(obj_id)])
            .expect("make a component from the box");
        let mirrored = kernel::Transform::scale(kernel::Vec3::new(-1.0, 1.0, 1.0));
        doc.place_instance(component, mirrored)
            .expect("place a mirrored second instance");

        let bytes = export_3mf(&doc, 0, true).expect("both instances export");
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("a valid zip");
        let mut model = String::new();
        std::io::Read::read_to_string(
            &mut zip
                .by_name("3D/3dmodel.model")
                .expect("model entry present"),
            &mut model,
        )
        .unwrap();
        assert_eq!(model.matches("<object ").count(), 2, "{model}");
        assert_eq!(
            model.matches("<vertex ").count(),
            16,
            "8 per instance: {model}"
        );
        assert_eq!(
            model.matches("<triangle ").count(),
            24,
            "12 per instance: {model}"
        );
    }

    /// The session-normalization defect this fixes (module doc, "Session
    /// normalization"): a component placed twice exports both instances;
    /// opening a component-edit session on ONE of them used to make the
    /// other's export silently vanish (`open_explode_session` hides every
    /// sibling placement for the session's duration, and the traversal
    /// honored that hiding) even though `hew.doc.save` was never fooled.
    /// Export must see what save sees — same triangle count before and
    /// during the session.
    #[test]
    fn export_sees_the_same_document_a_session_normalized_save_would() {
        let mut doc = box_document();
        let obj_id = doc.visible_object_ids()[0];
        let (component, first_instance, _change) = doc
            .make_component(&[kernel::NodeId::Object(obj_id)])
            .expect("make a component from the box");
        let translated = kernel::Transform::translation(kernel::Vec3::new(2.0, 0.0, 0.0));
        doc.place_instance(component, translated)
            .expect("place a second instance");

        let before = export_stl(&doc, 0, true).expect("both instances export");
        let count_before = u32::from_le_bytes(before[80..84].try_into().unwrap());
        assert_eq!(count_before, 24, "two box instances, 12 triangles each");

        doc.open_explode_session(first_instance)
            .expect("open a session on the first instance");

        let after = export_stl(&doc, 0, true).expect("still exports mid-session");
        let count_after = u32::from_le_bytes(after[80..84].try_into().unwrap());
        assert_eq!(
            count_after, count_before,
            "a live export mid-session must see what a live save sees — \
             the sibling instance must not silently vanish"
        );
    }

    /// A painted, multi-object, instanced model — the acceptance scenario
    /// this whole effort was checked against (compared by hand against the
    /// retired TypeScript writers over an equivalent hand-built scene; see
    /// this effort's final report for that comparison). Two top-level
    /// objects (one painted, one plain) plus a plain component placed
    /// twice: 4 parts, 4×8 = 32 vertices, 4×12 = 48 triangles, exactly 2
    /// distinct colors (the painted face's red, and the default gray every
    /// unpainted face and instance shares).
    #[test]
    fn export_3mf_handles_a_painted_multi_object_instanced_model() {
        let mut doc = box_document(); // "Box": one object, all faces default.
        let mut painted_mesh = unit_box_mesh("Painted");
        painted_mesh.face_materials[0] = 0;
        let scene = ImportScene {
            materials: vec![Material::solid("Red", Rgba8::rgb(0xff, 0x00, 0x00))],
            defs: Vec::new(),
            roots: vec![ImportNode::Mesh(painted_mesh)],
            guides: Vec::new(),
            tags: Vec::new(),
        };
        doc.ingest(scene, Vec::new())
            .expect("ingest a second, painted box");

        // A plain (unpainted) third box becomes a component, placed twice —
        // the instanced half of the scenario.
        let scene = ImportScene {
            materials: Vec::new(),
            defs: Vec::new(),
            roots: vec![ImportNode::Mesh(unit_box_mesh("LegSrc"))],
            guides: Vec::new(),
            tags: Vec::new(),
        };
        doc.ingest(scene, Vec::new())
            .expect("ingest the component source box");
        let leg_src_id = *doc
            .visible_object_ids()
            .iter()
            .find(|&&id| doc.object_name(id) == Some("LegSrc"))
            .expect("the component source object is present");
        let (component, _first_instance, _change) = doc
            .make_component(&[kernel::NodeId::Object(leg_src_id)])
            .expect("make a component from the plain box");
        doc.place_instance(
            component,
            kernel::Transform::translation(kernel::Vec3::new(4.0, 0.0, 0.0)),
        )
        .expect("place a second instance");

        let bytes = export_3mf(&doc, 0, true).expect("the whole scene exports");
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("a valid zip");
        let mut model = String::new();
        std::io::Read::read_to_string(
            &mut zip
                .by_name("3D/3dmodel.model")
                .expect("model entry present"),
            &mut model,
        )
        .unwrap();

        // 4 parts total: 2 top-level objects (Box, Painted) + 2 placed
        // instances of the LegSrc component.
        assert_eq!(model.matches("<object ").count(), 4, "{model}");
        assert_eq!(model.matches("<vertex ").count(), 32, "{model}");
        assert_eq!(model.matches("<triangle ").count(), 48, "{model}");
        assert_eq!(
            model.matches("<base ").count(),
            2,
            "red + default gray: {model}"
        );
        assert!(model.contains("#FF0000"), "{model}");
        assert!(model.contains("#CCCCCC"), "{model}");

        let stl = export_stl(&doc, 0, true).expect("the whole scene exports to stl too");
        let count = u32::from_le_bytes(stl[80..84].try_into().unwrap());
        assert_eq!(count, 48, "4 boxes, 12 triangles each");
    }

    // ------------------------------------------------------------- dispatch

    /// [`export`]'s format dispatch reaches the same writer the direct
    /// per-format entry point does — pinned on STL since it is cheapest to
    /// assert byte-for-byte.
    #[test]
    fn export_dispatches_by_format() {
        let doc = box_document();
        let direct = export_stl(&doc, 0, true).unwrap();
        let dispatched = export(&doc, "stl", 0, true).unwrap();
        assert_eq!(direct, dispatched);
    }

    #[test]
    fn export_refuses_host_capability_missing_for_an_unrecognized_format() {
        let doc = box_document();
        let err = export(&doc, "obj", 0, true).unwrap_err();
        assert_eq!(err.name(), "host_capability_missing");
        assert_eq!(err.message(), "Unrecognized export format \"obj\".");
    }

    /// `hew.doc.export`'s refusal inventory (`crates/api/src/registry.rs`)
    /// and docs/API_REFERENCE.gen.md publish these three names verbatim —
    /// a changed name here is a breaking protocol change, so it is pinned
    /// directly rather than left to drift with a rename.
    #[test]
    fn export_error_names_are_pinned() {
        assert_eq!(ExportError::NothingToExport.name(), "nothing_to_export");
        assert_eq!(
            ExportError::Failed(tessellate::TessellateError::DegenerateFace {
                face: kernel::FaceId::default()
            })
            .name(),
            "export_failed"
        );
        assert_eq!(
            ExportError::UnsupportedFormat("obj".to_string()).name(),
            "host_capability_missing"
        );
    }

    // ---------------------------------------------------------------- 3MF

    /// A box exports as one 3MF `<object>` with 8 welded vertices and 12
    /// triangles (2 per face) — unzips the real container and greps the
    /// model XML rather than trusting the writer's own counters.
    #[test]
    fn export_3mf_produces_one_object_with_eight_vertices_and_twelve_triangles() {
        let doc = box_document();
        let bytes = export_3mf(&doc, 0, true).expect("a solid box exports to 3mf");

        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("a valid zip");
        assert!(zip.by_name("[Content_Types].xml").is_ok());
        assert!(zip.by_name("_rels/.rels").is_ok());
        let mut model = String::new();
        std::io::Read::read_to_string(
            &mut zip
                .by_name("3D/3dmodel.model")
                .expect("model entry present"),
            &mut model,
        )
        .unwrap();

        assert!(model.contains("unit=\"meter\""), "{model}");
        assert_eq!(model.matches("<object ").count(), 1, "{model}");
        assert_eq!(model.matches("<vertex ").count(), 8, "{model}");
        assert_eq!(model.matches("<triangle ").count(), 12, "{model}");
        assert_eq!(model.matches("<item ").count(), 1, "{model}");
    }

    #[test]
    fn export_3mf_refuses_nothing_to_export_on_an_empty_document() {
        let doc = Document::new();
        let err = export_3mf(&doc, 0, true).unwrap_err();
        assert_eq!(err.name(), "nothing_to_export");
    }

    #[test]
    fn export_3mf_is_byte_identical_across_two_exports() {
        let doc = box_document();
        let a = export_3mf(&doc, 0, true).unwrap();
        let b = export_3mf(&doc, 0, true).unwrap();
        assert_eq!(a, b, "3MF export must be deterministic");
    }

    /// The defect this whole effort fixes: a painted document now exports
    /// colors. Two painted faces plus four default-gray ones dedupe into
    /// exactly 3 `<base>` entries (module doc, "Materials"); the object
    /// declares one of them as its `pindex` default, and at least one
    /// triangle needs a `p1` override for the colors that differ from it.
    #[test]
    fn export_3mf_encodes_face_colors_via_basematerials() {
        let (doc, _red, _blue) = painted_box_document();
        let bytes = export_3mf(&doc, 0, true).expect("a painted box exports to 3mf");

        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("a valid zip");
        let mut model = String::new();
        std::io::Read::read_to_string(
            &mut zip
                .by_name("3D/3dmodel.model")
                .expect("model entry present"),
            &mut model,
        )
        .unwrap();

        assert!(model.contains("<basematerials id=\"1\">"), "{model}");
        assert!(
            model.contains("<base name=\"#FF0000\" displaycolor=\"#FF0000\"/>"),
            "{model}"
        );
        assert!(
            model.contains("<base name=\"#0000FF\" displaycolor=\"#0000FF\"/>"),
            "{model}"
        );
        assert!(
            model.contains("<base name=\"#CCCCCC\" displaycolor=\"#CCCCCC\"/>"),
            "{model}"
        );
        assert_eq!(
            model.matches("<base ").count(),
            3,
            "3 distinct colors: {model}"
        );
        assert!(model.contains("pid=\"1\" pindex="), "{model}");
        assert!(
            model.contains(" p1=\""),
            "at least one triangle overrides the object default: {model}"
        );
    }

    /// A textured face has no core-3MF representation, so it exports as its
    /// material's TINT color (module doc, "Materials") — the same rule the
    /// retired TypeScript writer's doc comment specified. No image bytes of
    /// any kind end up in the container.
    #[test]
    fn export_3mf_renders_a_textured_face_as_its_tint_color() {
        let doc = textured_box_document();
        let bytes = export_3mf(&doc, 0, true).expect("a textured box exports to 3mf");

        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("a valid zip");
        let mut model = String::new();
        std::io::Read::read_to_string(
            &mut zip
                .by_name("3D/3dmodel.model")
                .expect("model entry present"),
            &mut model,
        )
        .unwrap();
        assert!(
            model.contains("<base name=\"#204060\" displaycolor=\"#204060\"/>"),
            "{model}"
        );
        assert!(
            !model.to_lowercase().contains("texture") && !model.to_lowercase().contains("image"),
            "core 3MF carries no texture data: {model}"
        );
    }

    /// Ported from the retired `threeMfExport.test.ts`'s "skips triangles
    /// whose welded indices collapse, and counts them": a triangle with
    /// three coincident corners welds to fewer than 3 distinct vertices
    /// (forbidden by the 3MF spec) and is dropped — before its color ever
    /// reaches the palette, not after. Exercises [`build_3mf_objects`]
    /// directly over a hand-built [`ExportSolid`], the pure-writer level
    /// the retired test worked at (no kernel document needed).
    #[test]
    fn build_3mf_objects_skips_a_collapsed_triangle_before_its_color_enters_the_palette() {
        #[rustfmt::skip]
        let triangles = vec![
            // Collapsed: two corners coincide.
            3.0, 3.0, 3.0,  3.0, 3.0, 3.0,  4.0, 4.0, 4.0,
            // Real.
            0.0, 0.0, 0.0,  1.0, 0.0, 0.0,  0.0, 1.0, 0.0,
        ];
        let solid = ExportSolid {
            name: "Part".to_string(),
            triangles,
            colors: vec![Rgba8::rgb(0x00, 0x00, 0xff), Rgba8::rgb(0xff, 0x00, 0x00)],
            materials: vec![None, None],
            uvs: vec![[0.0; 2]; 6],
        };
        let (objects, palette) = build_3mf_objects(&[solid]);
        assert_eq!(objects.len(), 1);
        assert_eq!(
            objects[0].triangle_lines.len(),
            1,
            "the collapsed triangle is dropped"
        );
        assert_eq!(
            palette.order,
            vec!["#FF0000".to_string()],
            "the dropped triangle's blue never entered the palette"
        );
    }

    /// Ported from the retired `threeMfExport.test.ts`'s "escapes XML
    /// metacharacters in part names".
    #[test]
    fn render_3mf_model_escapes_xml_metacharacters_in_names() {
        let solid = ExportSolid {
            name: "A<B & \"C\"".to_string(),
            triangles: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            colors: vec![Rgba8::rgb(0xff, 0x00, 0x00)],
            materials: vec![None],
            uvs: vec![[0.0; 2]; 3],
        };
        let (objects, palette) = build_3mf_objects(&[solid]);
        let model = render_3mf_model(&objects, &palette, "1.2.3");
        assert!(
            model.contains("name=\"A&lt;B &amp; &quot;C&quot;\""),
            "{model}"
        );
    }

    // -------------------------------------------------------- solids_only

    /// `solids_only=true` (the headless/live-API default) drops a non-solid
    /// object entirely — nothing survives an all-open document. `false`
    /// (the interactive app's own `Scene::export`) includes it, matching
    /// what the retired TypeScript writers always exported (module doc,
    /// "Non-solid inclusion").
    #[test]
    fn solids_only_toggles_non_solid_inclusion() {
        let doc = open_quad_document();
        assert!(
            export_stl(&doc, 0, true).is_err(),
            "solids-only drops the open quad"
        );
        assert!(export_3mf(&doc, 0, true).is_err());
        assert!(export_glb(&doc, 0, true).is_err());

        let stl = export_stl(&doc, 0, false).expect("permissive mode includes it");
        let count = u32::from_le_bytes(stl[80..84].try_into().unwrap());
        assert_eq!(count, 2, "one quad triangulates to 2 triangles");

        let glb = export_glb(&doc, 0, false).expect("permissive mode includes it");
        let (json, _bin) = glb_chunks(&glb);
        assert_eq!(json["meshes"].as_array().unwrap().len(), 1);
    }

    // ----------------------------------------------------------------- GLB

    fn glb_chunks(bytes: &[u8]) -> (serde_json::Value, Vec<u8>) {
        assert_eq!(&bytes[0..4], b"glTF", "GLB magic");
        let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        assert_eq!(version, 2, "glTF version");
        let total_len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        assert_eq!(total_len, bytes.len(), "declared length matches the file");

        let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        assert_eq!(&bytes[16..20], b"JSON");
        let json_start = 20;
        let json_bytes = &bytes[json_start..json_start + json_len];
        let json: serde_json::Value = serde_json::from_slice(json_bytes).expect("valid JSON chunk");

        let bin_header = json_start + json_len;
        let bin_len =
            u32::from_le_bytes(bytes[bin_header..bin_header + 4].try_into().unwrap()) as usize;
        assert_eq!(&bytes[bin_header + 4..bin_header + 8], b"BIN\0");
        let bin_start = bin_header + 8;
        let bin_bytes = bytes[bin_start..bin_start + bin_len].to_vec();
        assert_eq!(bin_start + bin_len, bytes.len(), "no trailing garbage");

        (json, bin_bytes)
    }

    /// A box exports as one GLB node/mesh: 8 welded vertices, 12 triangles
    /// (36 indices) — parses the container headers and the JSON chunk by
    /// hand rather than trusting a glTF library.
    #[test]
    fn export_glb_produces_a_well_formed_container_with_one_mesh() {
        let doc = box_document();
        let bytes = export_glb(&doc, 0, true).expect("a solid box exports to glb");
        let (json, bin) = glb_chunks(&bytes);

        assert_eq!(json["asset"]["version"], "2.0");
        assert_eq!(json["nodes"].as_array().unwrap().len(), 1);
        assert_eq!(json["meshes"].as_array().unwrap().len(), 1);
        assert_eq!(json["materials"].as_array().unwrap().len(), 1);

        let accessors = json["accessors"].as_array().unwrap();
        assert_eq!(accessors.len(), 2, "one POSITION + one indices accessor");
        let pos_accessor = &accessors[0];
        assert_eq!(pos_accessor["type"], "VEC3");
        assert_eq!(pos_accessor["componentType"], 5126);
        assert_eq!(pos_accessor["count"], 8, "8 welded box vertices");
        assert!(pos_accessor["min"].is_array(), "min is required by spec");
        assert!(pos_accessor["max"].is_array(), "max is required by spec");

        let idx_accessor = &accessors[1];
        assert_eq!(idx_accessor["type"], "SCALAR");
        assert_eq!(idx_accessor["componentType"], 5125);
        assert_eq!(idx_accessor["count"], 36, "12 triangles * 3 indices");

        let buffer_views = json["bufferViews"].as_array().unwrap();
        assert_eq!(buffer_views.len(), 2);
        let pos_bv = &buffer_views[0];
        assert_eq!(pos_bv["byteLength"], 8 * 3 * 4, "8 vertices * vec3 * f32");
        let idx_bv = &buffer_views[1];
        assert_eq!(idx_bv["byteLength"], 36 * 4, "36 indices * u32");

        let declared_buffer_len = json["buffers"][0]["byteLength"].as_u64().unwrap() as usize;
        assert_eq!(
            declared_buffer_len,
            bin.len(),
            "BIN chunk matches the declared buffer length"
        );
    }

    #[test]
    fn export_glb_refuses_nothing_to_export_on_an_empty_document() {
        let doc = Document::new();
        let err = export_glb(&doc, 0, true).unwrap_err();
        assert_eq!(err.name(), "nothing_to_export");
    }

    #[test]
    fn export_glb_is_byte_identical_across_two_exports() {
        let doc = box_document();
        let a = export_glb(&doc, 0, true).unwrap();
        let b = export_glb(&doc, 0, true).unwrap();
        assert_eq!(a, b, "GLB export must be deterministic");
    }

    #[test]
    fn export_glb_chunks_are_four_byte_aligned() {
        let doc = box_document();
        let bytes = export_glb(&doc, 0, true).unwrap();
        let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        assert_eq!(json_len % 4, 0, "JSON chunk length must be 4-byte aligned");
        let bin_header = 20 + json_len as usize;
        let bin_len = u32::from_le_bytes(bytes[bin_header..bin_header + 4].try_into().unwrap());
        assert_eq!(bin_len % 4, 0, "BIN chunk length must be 4-byte aligned");
    }

    /// The defect this whole effort fixes, GLB side: a painted document
    /// gets one PBR material per distinct color it uses (module doc,
    /// "Materials") — here Default (unpainted faces 2–5), Red (face 0),
    /// Blue (face 1) — one mesh split into 3 primitives, one per material,
    /// 12 triangles total across them.
    #[test]
    fn export_glb_encodes_face_colors_as_distinct_materials() {
        let (doc, _red, _blue) = painted_box_document();
        let bytes = export_glb(&doc, 0, true).expect("a painted box exports to glb");
        let (json, _bin) = glb_chunks(&bytes);

        let materials = json["materials"].as_array().unwrap();
        assert_eq!(materials.len(), 3, "Default + Red + Blue: {json}");
        assert_eq!(materials[0]["name"], "Default");

        let names: Vec<&str> = materials
            .iter()
            .map(|m| m["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"Red"));
        assert!(names.contains(&"Blue"));

        let red = materials.iter().find(|m| m["name"] == "Red").unwrap();
        let factor = red["pbrMetallicRoughness"]["baseColorFactor"]
            .as_array()
            .unwrap();
        // Pure sRGB red (255,0,0) decodes to linear (1.0, 0.0, 0.0, 1.0) —
        // the one point on the sRGB curve exact in both spaces, so this
        // pins the gamma-decode direction without a tolerance comparison.
        assert_eq!(factor[0].as_f64().unwrap(), 1.0);
        assert_eq!(factor[1].as_f64().unwrap(), 0.0);
        assert_eq!(factor[2].as_f64().unwrap(), 0.0);
        assert_eq!(factor[3].as_f64().unwrap(), 1.0);

        let meshes = json["meshes"].as_array().unwrap();
        assert_eq!(meshes.len(), 1);
        let primitives = meshes[0]["primitives"].as_array().unwrap();
        assert_eq!(
            primitives.len(),
            3,
            "one primitive per distinct material: {json}"
        );
        let accessors = json["accessors"].as_array().unwrap();
        let total_indices: u64 = primitives
            .iter()
            .map(|p| {
                accessors[p["indices"].as_u64().unwrap() as usize]["count"]
                    .as_u64()
                    .unwrap()
            })
            .sum();
        assert_eq!(
            total_indices, 36,
            "12 triangles * 3 indices, split across primitives"
        );
    }

    /// A textured material embeds its authored image bytes in the BIN chunk
    /// (glTF 2.0 §3.9), referenced by an `images`/`textures` entry and the
    /// material's `baseColorTexture` — the round-trip capability the retired
    /// TypeScript writer had via three's `GLTFExporter` and the old
    /// hand-rolled Rust writer did not.
    #[test]
    fn export_glb_embeds_a_textured_materials_image_bytes() {
        let doc = textured_box_document();
        let bytes = export_glb(&doc, 0, true).expect("a textured box exports to glb");
        let (json, bin) = glb_chunks(&bytes);

        let materials = json["materials"].as_array().unwrap();
        let brick = materials
            .iter()
            .find(|m| m["name"] == "Brick")
            .expect("Brick material present");
        let tex_index = brick["pbrMetallicRoughness"]["baseColorTexture"]["index"]
            .as_u64()
            .expect("baseColorTexture set") as usize;
        let textures = json["textures"].as_array().unwrap();
        let image_index = textures[tex_index]["source"].as_u64().unwrap() as usize;
        let images = json["images"].as_array().unwrap();
        let image = &images[image_index];
        assert_eq!(image["mimeType"], "image/png");

        let bv_index = image["bufferView"].as_u64().unwrap() as usize;
        let bv = &json["bufferViews"].as_array().unwrap()[bv_index];
        let offset = bv["byteOffset"].as_u64().unwrap() as usize;
        let len = bv["byteLength"].as_u64().unwrap() as usize;
        assert_eq!(
            &bin[offset..offset + len],
            b"not a real png, just an opaque authored blob",
            "authored image bytes round-trip verbatim into the BIN chunk"
        );

        // The textured primitive carries UVs; a same-mesh untextured
        // primitive (the box's other 5 faces, sharing the Default material)
        // does not need them.
        let primitives = json["meshes"][0]["primitives"].as_array().unwrap();
        let any_has_uv = primitives
            .iter()
            .any(|p| p["attributes"].get("TEXCOORD_0").is_some());
        assert!(
            any_has_uv,
            "the textured primitive carries TEXCOORD_0: {json}"
        );
    }
}
