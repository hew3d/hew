//! Versioned binary geometry encoding and document-level save/load — the
//! kernel's half of the native file format (ARCHITECTURE.md).
//!
//! The kernel is I/O-free (DEVELOPMENT.md rule 1): bytes in, bytes out. `Document`
//! owns the whole zip container including the JSON manifest; this module owns
//! the geometry buffer codec, the manifest DTO structs, and the zip framing
//! helpers used by `Document::save`/`load` in `document.rs`.
//!
//! See `docs/HEW_FILE_FORMAT.md` for the authoritative spec. Every constant and
//! layout here must match that document; the spec is updated in the SAME commit
//! as any serialization change (docs/DEVELOPMENT.md file-format rule).

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Seek, Write};

use serde::{Deserialize, Serialize};
use slotmap::SecondaryMap;

use crate::error::TopologyError;
use crate::guide::Guide;
use crate::ids::{ComponentId, GroupId, GuideId, InstanceId, MaterialId, ObjectId, SketchId};
use crate::material::{ImageFormat, Material, Texture, UvFrame};
use crate::math::{Plane, Point3, Vec3};
use crate::sketch::{
    Sketch, SketchCurveId, SketchEdge, SketchRegion, SketchRegionId, SketchVertex, SketchVertexId,
};
use crate::topo::Object;
use crate::transform::Transform;

/// Version of the geometry buffer layout. Bump on any layout change and
/// extend `docs/HEW_FILE_FORMAT.md` plus the golden-file tests in the same
/// commit.
///
/// v1 → v2: per-face optional `UvFrame` added after the material u32
/// (`u8` flag + 8×f64 when present; absent in v1 files → all `uv_frame = None`).
/// v2 → v3 (): a `u8` "imported" flag added to the buffer header after
/// the base-material u32. `1` → the object's faces are validated
/// at the wider [`crate::tol::IMPORT_PLANE_DIST`]; absent (v1/v2) or `0` → strict
/// [`crate::tol::PLANE_DIST`].
pub const GEOMETRY_FORMAT_VERSION: u32 = 3;

/// Version of the `.hew` container / `manifest.json` shape (independent of the
/// geometry buffer version). Bump on any manifest-shape change and extend
/// `docs/HEW_FILE_FORMAT.md` plus the golden files in the same commit.
///
/// v2 (): added optional `name` fields to object/group/component/
/// instance entries. The fields are `#[serde(default)]`, so v1 files still load
/// (names default to `None`) — see the version gate in `decode_document_raw`.
///
/// v3 (): added optional `tags: Vec<Vec<String>>` to object/group/
/// instance entries (not component — tags ride on instances, not definitions).
/// The field is `#[serde(default, skip_serializing_if = "Vec::is_empty")]`, so
/// v1/v2 files still load (tags default to empty) — back-compatible.
///
/// v4 (): added an optional top-level `guides: Vec<GuideDto>` —
/// construction lines + points. The field is
/// `#[serde(default, skip_serializing_if = "Vec::is_empty")]`, so v1-v3 files
/// still load (guides default to empty) — back-compatible. The geometry buffer
/// is unchanged by this bump (`GEOMETRY_FORMAT_VERSION` stays 3).
///
/// v5: added an optional top-level `tags: Vec<TagDto>` — the tag metadata
/// registry (known tag paths + hidden-by-default flags; a `.skp` import
/// registers the source layer list here, hidden layers included). The field
/// is `#[serde(default, skip_serializing_if = "Vec::is_empty")]`, so v1-v4
/// files still load (registry defaults to empty, node-carried tags all
/// visible) — back-compatible. The geometry buffer is unchanged
/// (`GEOMETRY_FORMAT_VERSION` stays 3).
///
/// v6: added optional `hidden: bool` to object/group/instance entries —
/// the USER-hidden (view-state) flag, distinct from tag hiding. The field
/// is `#[serde(default, skip_serializing_if ...)]`, so v1-v5 files still
/// load (nothing user-hidden) — back-compatible. The geometry buffer is
/// unchanged (`GEOMETRY_FORMAT_VERSION` stays 3).
///
/// v7: added optional `curve: u32` to sketch edge entries — the dense
/// per-sketch id of the curve chain the edge belongs to (an arc's or
/// circle's facets, selected/deleted as one unit). The field is
/// `#[serde(default, skip_serializing_if = "Option::is_none")]`, so v1-v6
/// files still load (every edge a plain line) — back-compatible. The
/// geometry buffer is unchanged (`GEOMETRY_FORMAT_VERSION` stays 3).
///
/// v8: added optional `source: [u32; 2]` to object entries — the dense
/// (sketch, region) footprint the object was extruded from, resolved like
/// `consumed` pairs; deleting the object frees the footprint. Absent for
/// boolean results, slice pieces, imports, and all pre-v8 files —
/// back-compatible (provenance degrades to `None`). Geometry buffer
/// unchanged (`GEOMETRY_FORMAT_VERSION` stays 3).
///
/// v9: replaced v8's `source` region handle with optional `footprints` on
/// object entries — the sketch-plane loops (outer + holes, world
/// coordinates) the solid stands on, frozen at extrusion; boolean/slice/
/// push-through results inherit their operands' entries. The consumed set
/// is DERIVED from these polygons (a region is consumed iff its material
/// overlaps a live footprint), so it survives any sketch re-topology —
/// region handles churn, area does not. v9 writers no longer emit
/// `source`; v8 files still load (each `source` pair resolves to its
/// region, whose loops at load time equal the loops at extrusion time).
/// Both fields are `#[serde(default, skip_serializing_if = ...)]` —
/// back-compatible. Geometry buffer unchanged (`GEOMETRY_FORMAT_VERSION`
/// stays 3).
pub const MANIFEST_FORMAT_VERSION: u32 = 9;

/// Sentinel `u32` standing in for `None` wherever a material id is written in a
/// geometry buffer (HEW_FILE_FORMAT.md/). Dense material ids never reach it.
pub const NO_MATERIAL: u32 = u32::MAX;

/// Magic bytes at the start of every geometry buffer.
const GEOMETRY_MAGIC: &[u8; 4] = b"HEWG";

// ════════════════════════════════════════════════════════════════════════════
// Error types
// ════════════════════════════════════════════════════════════════════════════

/// Typed failures of [`Object::decode`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// The buffer announces a version this build does not read.
    UnsupportedVersion {
        /// The version found in the header.
        found: u32,
    },
    /// The buffer ends before its announced contents do.
    Truncated,
    /// A structurally unreadable buffer (bad magic, impossible counts,
    /// out-of-range indices).
    Corrupt {
        /// Byte offset of the first inconsistency.
        offset: usize,
        /// What was wrong there.
        what: &'static str,
    },
    /// The buffer parsed, but the geometry it describes fails the topology
    /// validator. Surfaced verbatim; nothing is repaired.
    InvalidTopology(TopologyError),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::UnsupportedVersion { found } => {
                write!(
                    f,
                    "geometry buffer version {found} is not supported \
                     (this build reads {GEOMETRY_FORMAT_VERSION})"
                )
            }
            DecodeError::Truncated => write!(f, "geometry buffer is truncated"),
            DecodeError::Corrupt { offset, what } => {
                write!(f, "geometry buffer corrupt at byte {offset}: {what}")
            }
            DecodeError::InvalidTopology(e) => {
                write!(f, "decoded geometry is invalid: {e}")
            }
        }
    }
}

impl std::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DecodeError::InvalidTopology(e) => Some(e),
            _ => None,
        }
    }
}

/// Typed failures of [`crate::document::Document::load`] (HEW_FILE_FORMAT.md).
/// A corrupt, truncated, or hand-tampered file produces one of these — never a
/// panic and never a quietly-broken document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// Not a readable `.hew` container (bad zip, missing `manifest.json`).
    NotAContainer,
    /// The manifest announces a `format_version` this build does not read.
    UnsupportedVersion {
        /// The version found in the manifest.
        found: u32,
    },
    /// `manifest.json` is missing a required field or is otherwise malformed.
    MalformedManifest {
        /// A human-readable description of what was wrong.
        what: String,
    },
    /// A manifest reference (member, def, base material, consumed pair, …)
    /// points at a dense id with no corresponding entry.
    DanglingReference {
        /// A human-readable description of the broken reference.
        what: String,
    },
    /// A referenced container entry (a geometry buffer or texture asset) is
    /// missing from the zip.
    MissingAsset {
        /// The entry path that was referenced but not found.
        path: String,
    },
    /// A geometry buffer failed to decode or validate. Surfaced verbatim.
    Geometry(DecodeError),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::NotAContainer => write!(f, "not a readable .hew container"),
            LoadError::UnsupportedVersion { found } => write!(
                f,
                "manifest format version {found} is not supported \
                 (this build reads {MANIFEST_FORMAT_VERSION})"
            ),
            LoadError::MalformedManifest { what } => write!(f, "malformed manifest: {what}"),
            LoadError::DanglingReference { what } => write!(f, "dangling reference: {what}"),
            LoadError::MissingAsset { path } => write!(f, "missing container entry: {path}"),
            LoadError::Geometry(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LoadError::Geometry(e) => Some(e),
            _ => None,
        }
    }
}

impl From<DecodeError> for LoadError {
    fn from(e: DecodeError) -> LoadError {
        LoadError::Geometry(e)
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Geometry buffer codec (HEW_FILE_FORMAT.md)
// ════════════════════════════════════════════════════════════════════════════

impl Object {
    /// Encodes this Object's geometry as a versioned, deterministic binary
    /// buffer (HEW_FILE_FORMAT.md; see module docs for the obligations).
    ///
    /// Per-face and base materials are written as the document's **dense**
    /// material ids: `material_dense` maps a live [`MaterialId`] to its dense id
    /// (the same id space the manifest uses). For a material-free object the
    /// closure is never invoked. `None` materials are written as [`NO_MATERIAL`].
    pub fn encode(&self, material_dense: &impl Fn(MaterialId) -> u32) -> Vec<u8> {
        let mut buf = Vec::new();

        // --- header ---
        buf.extend_from_slice(GEOMETRY_MAGIC);
        buf.extend_from_slice(&GEOMETRY_FORMAT_VERSION.to_le_bytes());

        // watertight byte
        let wt_byte: u8 = match self.watertight {
            crate::topo::WatertightState::Watertight => 1,
            crate::topo::WatertightState::Open => 0,
        };
        buf.push(wt_byte);

        // base material
        let base_mat_id: u32 = match self.default_material {
            Some(m) => material_dense(m),
            None => NO_MATERIAL,
        };
        buf.extend_from_slice(&base_mat_id.to_le_bytes());

        // imported flag (v3): 1 when this object carries the wider import
        // planarity tolerance, 0 for strict native geometry.
        let imported: u8 = u8::from(self.planarity_tol > crate::tol::PLANE_DIST);
        buf.push(imported);

        // --- vertices (in slot order) ---
        let vertex_count = self.vertices.len() as u32;
        buf.extend_from_slice(&vertex_count.to_le_bytes());

        // Build a stable vertex index map (slot order).
        // We iterate self.vertices (SlotMap) — the iteration order is
        // deterministic (ascending slot index).
        let mut vertex_index: SecondaryMap<crate::ids::VertexId, u32> = SecondaryMap::new();
        for (vid, v) in &self.vertices {
            let idx = vertex_index.len() as u32;
            vertex_index.insert(vid, idx);
            buf.extend_from_slice(&v.position.x.to_le_bytes());
            buf.extend_from_slice(&v.position.y.to_le_bytes());
            buf.extend_from_slice(&v.position.z.to_le_bytes());
        }

        // --- faces (in slot order) ---
        let face_count = self.faces.len() as u32;
        buf.extend_from_slice(&face_count.to_le_bytes());

        for (_, face) in &self.faces {
            // per-face material
            let mat_id: u32 = match face.material {
                Some(m) => material_dense(m),
                None => NO_MATERIAL,
            };
            buf.extend_from_slice(&mat_id.to_le_bytes());

            // per-face UV frame (v2): u8 flag (0=none, 1=present) + 8×f64 LE
            match face.uv_frame {
                None => buf.push(0u8),
                Some(f) => {
                    buf.push(1u8);
                    buf.extend_from_slice(&f.s.x.to_le_bytes());
                    buf.extend_from_slice(&f.s.y.to_le_bytes());
                    buf.extend_from_slice(&f.s.z.to_le_bytes());
                    buf.extend_from_slice(&f.t.x.to_le_bytes());
                    buf.extend_from_slice(&f.t.y.to_le_bytes());
                    buf.extend_from_slice(&f.t.z.to_le_bytes());
                    buf.extend_from_slice(&f.u0.to_le_bytes());
                    buf.extend_from_slice(&f.v0.to_le_bytes());
                }
            }

            // outer loop
            let outer_verts: Vec<u32> = self
                .loop_half_edges(face.outer_loop)
                .map(|h| vertex_index[self.half_edges[h].origin])
                .collect();
            let outer_count = outer_verts.len() as u32;
            buf.extend_from_slice(&outer_count.to_le_bytes());
            for &vi in &outer_verts {
                buf.extend_from_slice(&vi.to_le_bytes());
            }

            // hole loops
            let hole_count = face.inner_loops.len() as u32;
            buf.extend_from_slice(&hole_count.to_le_bytes());
            for &inner_loop in &face.inner_loops {
                let hole_verts: Vec<u32> = self
                    .loop_half_edges(inner_loop)
                    .map(|h| vertex_index[self.half_edges[h].origin])
                    .collect();
                let hole_vertex_count = hole_verts.len() as u32;
                buf.extend_from_slice(&hole_vertex_count.to_le_bytes());
                for &vi in &hole_verts {
                    buf.extend_from_slice(&vi.to_le_bytes());
                }
            }
        }

        buf
    }

    /// Decodes a buffer produced by [`Object::encode`] (any supported version),
    /// rebuilding topology via [`Object::from_faces_with_holes`] and running the
    /// full validator before returning (rule 4: validate, never repair).
    ///
    /// `dense_material` maps a dense material id back to the live [`MaterialId`]
    /// the loader inserted into the palette; [`NO_MATERIAL`] / an absent mapping
    /// becomes `None`.
    ///
    /// Roundtrip property (see `tests/serialize_specs.rs`):
    /// `decode(encode(o))` equals `o` topologically and geometrically, and
    /// `encode` is deterministic.
    pub fn decode(
        bytes: &[u8],
        dense_material: &impl Fn(u32) -> Option<MaterialId>,
    ) -> Result<Object, DecodeError> {
        let mut r = ByteReader::new(bytes);

        // --- header ---
        let magic = r.read_bytes::<4>()?;
        if &magic != GEOMETRY_MAGIC {
            return Err(DecodeError::Corrupt {
                offset: 0,
                what: "bad magic (not HEWG)",
            });
        }

        let version = r.read_u32()?;
        if version > GEOMETRY_FORMAT_VERSION || version == 0 {
            return Err(DecodeError::UnsupportedVersion { found: version });
        }

        let wt_byte = r.read_u8()?;
        if wt_byte > 1 {
            return Err(DecodeError::Corrupt {
                offset: r.pos - 1,
                what: "watertight byte must be 0 or 1",
            });
        }
        let stored_watertight = wt_byte == 1;

        let base_mat_raw = r.read_u32()?;
        let base_material = if base_mat_raw == NO_MATERIAL {
            None
        } else {
            Some(dense_material(base_mat_raw).ok_or(DecodeError::Corrupt {
                offset: r.pos - 4,
                what: "base material id out of range",
            })?)
        };

        // imported flag (v3): selects the object's planarity invariant tolerance.
        // Absent in v1/v2 → strict native default.
        let planarity_tol = if version >= 3 {
            let flag = r.read_u8()?;
            if flag > 1 {
                return Err(DecodeError::Corrupt {
                    offset: r.pos - 1,
                    what: "imported flag must be 0 or 1",
                });
            }
            if flag == 1 {
                crate::tol::IMPORT_PLANE_DIST
            } else {
                crate::tol::PLANE_DIST
            }
        } else {
            crate::tol::PLANE_DIST
        };

        // --- vertices ---
        let vertex_count = r.read_u32()? as usize;
        let mut positions = Vec::with_capacity(vertex_count);
        for _ in 0..vertex_count {
            let x = r.read_f64()?;
            let y = r.read_f64()?;
            let z = r.read_f64()?;
            positions.push(Point3::new(x, y, z));
        }

        // --- faces ---
        type FaceSpec = (
            Vec<usize>,
            Vec<Vec<usize>>,
            Plane,
            crate::material::FaceMaterial,
            Option<UvFrame>,
        );
        let face_count = r.read_u32()? as usize;
        let mut face_specs: Vec<FaceSpec> = Vec::with_capacity(face_count);

        for _ in 0..face_count {
            let mat_raw = r.read_u32()?;
            let face_material: crate::material::FaceMaterial = if mat_raw == NO_MATERIAL {
                None
            } else {
                Some(dense_material(mat_raw).ok_or(DecodeError::Corrupt {
                    offset: r.pos - 4,
                    what: "face material id out of range",
                })?)
            };

            // v2: per-face UV frame (flag + optional 8×f64). v1: absent → None.
            let uv_frame: Option<UvFrame> = if version >= 2 {
                let flag = r.read_u8()?;
                if flag == 1 {
                    let sx = r.read_f64()?;
                    let sy = r.read_f64()?;
                    let sz = r.read_f64()?;
                    let tx = r.read_f64()?;
                    let ty = r.read_f64()?;
                    let tz = r.read_f64()?;
                    let u0 = r.read_f64()?;
                    let v0 = r.read_f64()?;
                    Some(UvFrame::new(
                        Vec3::new(sx, sy, sz),
                        Vec3::new(tx, ty, tz),
                        u0,
                        v0,
                    ))
                } else if flag == 0 {
                    None
                } else {
                    return Err(DecodeError::Corrupt {
                        offset: r.pos - 1,
                        what: "uv_frame flag must be 0 or 1",
                    });
                }
            } else {
                None // v1 files: all faces have no UV frame
            };

            let outer_count = r.read_u32()? as usize;
            if outer_count < 3 {
                return Err(DecodeError::Corrupt {
                    offset: r.pos - 4,
                    what: "outer_count < 3 (degenerate face)",
                });
            }
            let mut outer = Vec::with_capacity(outer_count);
            for _ in 0..outer_count {
                let vi = r.read_u32()? as usize;
                if vi >= vertex_count {
                    return Err(DecodeError::Corrupt {
                        offset: r.pos - 4,
                        what: "vertex index out of range",
                    });
                }
                outer.push(vi);
            }

            // Recompute face plane from outer-loop vertices (HEW_FILE_FORMAT.md).
            let outer_pts: Vec<Point3> = outer.iter().map(|&i| positions[i]).collect();
            let plane = Plane::from_polygon(&outer_pts).map_err(|_| DecodeError::Corrupt {
                offset: r.pos,
                what: "outer loop vertices do not span a plane",
            })?;

            let hole_count = r.read_u32()? as usize;
            let mut holes = Vec::with_capacity(hole_count);
            for _ in 0..hole_count {
                let hole_vc = r.read_u32()? as usize;
                if hole_vc < 3 {
                    return Err(DecodeError::Corrupt {
                        offset: r.pos - 4,
                        what: "hole_vertex_count < 3 (degenerate hole)",
                    });
                }
                let mut hole = Vec::with_capacity(hole_vc);
                for _ in 0..hole_vc {
                    let vi = r.read_u32()? as usize;
                    if vi >= vertex_count {
                        return Err(DecodeError::Corrupt {
                            offset: r.pos - 4,
                            what: "hole vertex index out of range",
                        });
                    }
                    hole.push(vi);
                }
                holes.push(hole);
            }

            face_specs.push((outer, holes, plane, face_material, uv_frame));
        }

        // Rebuild topology via the existing builder path.
        let mut obj = Object::from_faces_with_holes(&positions, &face_specs);
        obj.default_material = base_material;
        // Restore the per-object planarity gate before validating, so an imported
        // object's near-planar faces are held to IMPORT_PLANE_DIST, not the
        // strict default.
        obj.planarity_tol = planarity_tol;

        // Validate the rebuilt topology (rule 4: validate, never repair).
        obj.validate().map_err(DecodeError::InvalidTopology)?;

        // Cross-check watertightness (cheap tamper detection).
        let rebuilt_wt = obj.watertight == crate::topo::WatertightState::Watertight;
        if rebuilt_wt != stored_watertight {
            return Err(DecodeError::Corrupt {
                offset: 8,
                what: "watertight byte disagrees with rebuilt topology",
            });
        }

        Ok(obj)
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Manifest DTO structs (HEW_FILE_FORMAT.md)
// ════════════════════════════════════════════════════════════════════════════

/// Top-level JSON manifest.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub format_version: u32,
    pub geometry_version: u32,
    pub app: String,
    pub app_version: String,
    pub materials: Vec<MaterialDto>,
    pub objects: Vec<ObjectDto>,
    pub groups: Vec<GroupDto>,
    pub components: Vec<ComponentDto>,
    pub instances: Vec<InstanceDto>,
    pub sketches: Vec<SketchDto>,
    pub roots: Vec<NodeRefDto>,
    /// Sorted ascending for determinism.
    pub consumed: Vec<[u32; 2]>,
    /// Construction guides (manifest v4+). Absent/empty in v1-v3 files →
    /// no guides.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guides: Vec<GuideDto>,
    /// Tag metadata registry (manifest v5+): known tag paths with their
    /// hidden-by-default flags, sorted by path. Absent/empty in v1-v4
    /// files → empty registry (all node-carried tags visible).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<TagDto>,
}

/// A tag metadata entry (manifest v5+).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct TagDto {
    /// Root-first tag path segments.
    pub path: Vec<String>,
    /// Hidden by default (seeds the UI's tag visibility on load).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

/// A construction-geometry guide entry.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct GuideDto {
    pub id: u32,
    /// `"line"` | `"point"`.
    pub kind: String,
    /// line: origin; point: position.
    pub p: [f64; 3],
    /// line only: unit direction (omitted for points).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<[f64; 3]>,
}

/// A material palette entry.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MaterialDto {
    pub id: u32,
    pub name: String,
    /// [r, g, b, a], 0–255 each.
    pub color: [u8; 4],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub texture: Option<TextureDto>,
}

/// A texture entry inside a material.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct TextureDto {
    /// Path inside the zip (e.g. `textures/tex_0.png`).
    pub asset: String,
    /// `"png"` or `"jpg"`.
    pub format: String,
    /// Meters per image tile [w, h].
    pub world_size: [f64; 2],
}

/// A live object entry.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ObjectDto {
    pub id: u32,
    /// Path of the geometry buffer inside the zip.
    pub geometry: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_material: Option<u32>,
    /// Optional display name (manifest v2+). Absent in v1 files → `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Per-node tag paths (manifest v3+). Absent in v1/v2 files → empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Vec<String>>,
    /// USER-hidden view state (manifest v6+). Absent in v1-v5 → visible.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// Extrusion footprint provenance (manifest v8 only): the dense
    /// `[sketch, region]` pair this object was extruded from, resolved like
    /// `consumed` entries. v9+ writers emit `footprints` instead; kept for
    /// decoding v8 files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<[u32; 2]>,
    /// Extrusion footprints (manifest v9+): the sketch-plane loops this
    /// solid stands on, frozen at extrusion (boolean/slice/push-through
    /// results inherit their operands'). The consumed set derives from
    /// these polygons. Absent for imports and all pre-v9 files.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub footprints: Vec<FootprintDto>,
}

/// One extrusion footprint (manifest v9+): the loops of the profile a solid
/// was extruded from, in world coordinates on the sketch plane.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FootprintDto {
    /// Dense id of the sketch the footprint shadows.
    pub sketch: u32,
    /// Outer boundary loop.
    pub outer: Vec<[f64; 3]>,
    /// Hole loops.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<Vec<[f64; 3]>>,
}

/// A merge group entry.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct GroupDto {
    pub id: u32,
    pub members: Vec<NodeRefDto>,
    /// Optional display name (manifest v2+). Absent in v1 files → `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Per-node tag paths (manifest v3+). Absent in v1/v2 files → empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Vec<String>>,
    /// USER-hidden view state (manifest v6+). Absent in v1-v5 → visible.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

/// A component definition.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ComponentDto {
    pub id: u32,
    /// Dense object ids belonging to this definition.
    pub members: Vec<u32>,
    /// Optional definition name (manifest v2+). Absent in v1 files → `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A component instance.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct InstanceDto {
    pub id: u32,
    pub def: u32,
    /// Row-major 3×4 affine: [m00,m01,m02,tx, m10,m11,m12,ty, m20,m21,m22,tz].
    pub pose: [f64; 12],
    /// Optional per-instance display name (manifest v2+). Absent in v1 → `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Per-node tag paths (manifest v3+). Absent in v1/v2 files → empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Vec<String>>,
    /// USER-hidden view state (manifest v6+). Absent in v1-v5 → visible.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

/// A first-class sketch.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SketchDto {
    pub id: u32,
    /// [nx, ny, nz, offset] — unit normal + plane offset.
    pub plane: [f64; 4],
    pub vertices: Vec<SketchVertexDto>,
    pub edges: Vec<SketchEdgeDto>,
    pub regions: Vec<SketchRegionDto>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SketchVertexDto {
    pub id: u32,
    pub p: [f64; 3],
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SketchEdgeDto {
    pub id: u32,
    pub from: u32,
    pub to: u32,
    /// Dense per-sketch curve-chain id, or absent for a plain line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SketchRegionDto {
    pub id: u32,
    pub outer: Vec<u32>,
    pub holes: Vec<Vec<u32>>,
}

/// A reference to a node in the document tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct NodeRefDto {
    pub kind: String,
    pub id: u32,
}

// ════════════════════════════════════════════════════════════════════════════
// Zip container helpers (HEW_FILE_FORMAT.md)
// ════════════════════════════════════════════════════════════════════════════

/// Write a single stored (uncompressed) entry into a zip writer with zeroed
/// timestamps and external attributes for determinism.
fn zip_add_stored_entry<W: Write + Seek>(
    zip: &mut zip::ZipWriter<W>,
    name: &str,
    data: &[u8],
) -> std::io::Result<()> {
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    zip.start_file(name, options)?;
    zip.write_all(data)?;
    Ok(())
}

/// Upper bound on the decompressed size of any single `.hew` container entry.
/// Zip entries carry an attacker-controlled compression ratio, so an entry that
/// is a few KB on disk can inflate to gigabytes in `read_to_end` — a classic
/// decompression-bomb DoS. Hew writes every entry `Stored` (uncompressed), so a
/// legitimate file never approaches this cap; it exists purely to reject
/// hostile/tampered containers. 1 GiB is far above any real model's per-buffer
/// footprint while still bounding a load to a survivable allocation.
const MAX_ENTRY_BYTES: u64 = 1024 * 1024 * 1024;

/// Read a named entry from a zip archive, returning its bytes.
fn zip_read_entry(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    name: &str,
) -> Result<Vec<u8>, LoadError> {
    let entry = zip.by_name(name).map_err(|_| LoadError::MissingAsset {
        path: name.to_string(),
    })?;
    // Cap the read at MAX_ENTRY_BYTES + 1: if the entry yields more than the cap
    // it's a decompression bomb (or corrupt) and the container is rejected. We
    // read via `take` rather than trusting the header's declared size, which is
    // itself attacker-controlled.
    let mut buf = Vec::new();
    let mut limited = entry.take(MAX_ENTRY_BYTES + 1);
    limited
        .read_to_end(&mut buf)
        .map_err(|_| LoadError::MissingAsset {
            path: name.to_string(),
        })?;
    if buf.len() as u64 > MAX_ENTRY_BYTES {
        return Err(LoadError::NotAContainer);
    }
    Ok(buf)
}

// ════════════════════════════════════════════════════════════════════════════
// Document save/load orchestration
// Called from document.rs which owns the private fields.
// ════════════════════════════════════════════════════════════════════════════

/// Serialization row for a merge group: (id, members, name, tags).
pub(crate) type GroupSaveRow = (
    GroupId,
    Vec<crate::document::NodeId>,
    Option<String>,
    Vec<Vec<String>>,
);

/// Serialization row for an instance: (id, def, pose, name, tags).
pub(crate) type InstanceSaveRow = (
    InstanceId,
    ComponentId,
    Transform,
    Option<String>,
    Vec<Vec<String>>,
);

/// Data that document.rs extracts from its private fields and hands off to
/// `encode_document` for serialization into zip bytes.
pub(crate) struct DocSaveData {
    pub materials: Vec<(MaterialId, Material)>,
    pub world_objects: Vec<(ObjectId, Object)>,
    pub def_objects: Vec<(ObjectId, Object, ComponentId)>,
    pub groups: Vec<GroupSaveRow>,
    pub components: Vec<(ComponentId, Vec<ObjectId>, Option<String>)>,
    pub instances: Vec<InstanceSaveRow>,
    pub sketches: Vec<(SketchId, Sketch)>,
    /// Construction guides, in slotmap key order.
    pub guides: Vec<(GuideId, Guide)>,
    /// Per-object display name, keyed by id (covers world + def members).
    pub obj_names: std::collections::BTreeMap<ObjectId, Option<String>>,
    /// Per-object extrusion footprints (manifest v9), keyed by id.
    pub obj_footprints: std::collections::BTreeMap<ObjectId, Vec<crate::document::Footprint>>,
    /// Per-object tag list, keyed by id (covers world + def members).
    pub obj_tags: std::collections::BTreeMap<ObjectId, Vec<Vec<String>>>,
    /// All live world roots (objects/groups/instances with no parent).
    pub roots: Vec<crate::document::NodeId>,
    /// (sketch_id, region_id) pairs that are consumed.
    pub consumed: Vec<(SketchId, SketchRegionId)>,
    /// Tag metadata registry (path → hidden), sorted by path (manifest v5).
    pub tag_meta: Vec<(Vec<String>, bool)>,
    /// USER-hidden flags keyed by id (manifest v6), same key spaces as the
    /// name/tag maps.
    pub obj_hidden: std::collections::BTreeSet<ObjectId>,
    pub group_hidden: std::collections::BTreeSet<GroupId>,
    pub instance_hidden: std::collections::BTreeSet<InstanceId>,
}

/// Encodes a complete document into `.hew` zip bytes (HEW_FILE_FORMAT.md).
/// Called by `Document::save` after extracting the necessary data.
pub(crate) fn encode_document(data: DocSaveData) -> Vec<u8> {
    // ── 1. Assign dense ids ───────────────────────────────────────────────
    // Materials: iteration order of the slotmap (stable ascending slot).
    let mut mat_to_dense: BTreeMap<MaterialId, u32> = BTreeMap::new();
    for (i, (mid, _)) in data.materials.iter().enumerate() {
        mat_to_dense.insert(*mid, i as u32);
    }

    // Objects (world + def): assign in the order provided (slotmap order from
    // document.rs). We collect all objects and assign dense ids.
    let mut obj_to_dense: BTreeMap<ObjectId, u32> = BTreeMap::new();
    let mut all_objects: Vec<(ObjectId, &Object)> = Vec::new();
    for (oid, obj) in &data.world_objects {
        obj_to_dense.insert(*oid, all_objects.len() as u32);
        all_objects.push((*oid, obj));
    }
    for (oid, obj, _) in &data.def_objects {
        obj_to_dense.insert(*oid, all_objects.len() as u32);
        all_objects.push((*oid, obj));
    }

    let mut grp_to_dense: BTreeMap<GroupId, u32> = BTreeMap::new();
    for (i, (gid, ..)) in data.groups.iter().enumerate() {
        grp_to_dense.insert(*gid, i as u32);
    }

    let mut comp_to_dense: BTreeMap<ComponentId, u32> = BTreeMap::new();
    for (i, (cid, ..)) in data.components.iter().enumerate() {
        comp_to_dense.insert(*cid, i as u32);
    }

    let mut inst_to_dense: BTreeMap<InstanceId, u32> = BTreeMap::new();
    for (i, (iid, ..)) in data.instances.iter().enumerate() {
        inst_to_dense.insert(*iid, i as u32);
    }

    let mut sketch_to_dense: BTreeMap<SketchId, u32> = BTreeMap::new();
    for (i, (sid, _)) in data.sketches.iter().enumerate() {
        sketch_to_dense.insert(*sid, i as u32);
    }

    let material_dense = |mid: MaterialId| -> u32 { *mat_to_dense.get(&mid).unwrap() };
    let node_to_dto = |n: &crate::document::NodeId| -> NodeRefDto {
        match n {
            crate::document::NodeId::Object(oid) => NodeRefDto {
                kind: "object".to_string(),
                id: obj_to_dense[oid],
            },
            crate::document::NodeId::Group(gid) => NodeRefDto {
                kind: "group".to_string(),
                id: grp_to_dense[gid],
            },
            crate::document::NodeId::Instance(iid) => NodeRefDto {
                kind: "instance".to_string(),
                id: inst_to_dense[iid],
            },
        }
    };

    // ── 2. Build geometry buffers ─────────────────────────────────────────
    // One buffer per object (world + def), in ascending dense id order.
    let mut geometry_buffers: Vec<Vec<u8>> = Vec::with_capacity(all_objects.len());
    for (_, obj) in &all_objects {
        geometry_buffers.push(obj.encode(&material_dense));
    }

    // ── 3. Collect texture assets ─────────────────────────────────────────
    // Textures in ascending material dense id order.
    let mut texture_entries: Vec<(String, Vec<u8>)> = Vec::new();
    let mut mat_dtos: Vec<MaterialDto> = Vec::new();
    for (i, (_, mat)) in data.materials.iter().enumerate() {
        let tex_dto = if let Some(tex) = &mat.texture {
            let ext = match tex.format {
                ImageFormat::Png => "png",
                ImageFormat::Jpeg => "jpg",
            };
            let path = format!("textures/tex_{i}.{ext}");
            texture_entries.push((path.clone(), tex.image.clone()));
            Some(TextureDto {
                asset: path,
                format: ext.to_string(),
                world_size: tex.world_size,
            })
        } else {
            None
        };
        mat_dtos.push(MaterialDto {
            id: i as u32,
            name: mat.name.clone(),
            color: [mat.color.r, mat.color.g, mat.color.b, mat.color.a],
            texture: tex_dto,
        });
    }

    // ── 4. Build manifest ─────────────────────────────────────────────────
    let obj_dtos: Vec<ObjectDto> = all_objects
        .iter()
        .enumerate()
        .map(|(i, (oid, _))| {
            // Find base_material: check world_objects first, then def_objects.
            let base_mat = data
                .world_objects
                .iter()
                .find(|(o, _)| o == oid)
                .map(|(_, obj)| obj.default_material)
                .or_else(|| {
                    data.def_objects
                        .iter()
                        .find(|(o, _, _)| o == oid)
                        .map(|(_, obj, _)| obj.default_material)
                })
                .unwrap_or(None);
            ObjectDto {
                id: i as u32,
                geometry: format!("geometry/obj_{i}.bin"),
                base_material: base_mat.map(&material_dense),
                name: data.obj_names.get(oid).cloned().flatten(),
                tags: data.obj_tags.get(oid).cloned().unwrap_or_default(),
                hidden: data.obj_hidden.contains(oid),
                source: None, // v8 field; v9 writers emit `footprints`
                footprints: data
                    .obj_footprints
                    .get(oid)
                    .map(|fps| {
                        fps.iter()
                            .filter_map(|fp| {
                                // A footprint on a hidden/gone sketch is
                                // dropped, matching `consumed` filtering.
                                let dense_sid = sketch_to_dense.get(&fp.sketch).copied()?;
                                let coords =
                                    |p: &crate::math::Point3| -> [f64; 3] { [p.x, p.y, p.z] };
                                Some(FootprintDto {
                                    sketch: dense_sid,
                                    outer: fp.outer.iter().map(coords).collect(),
                                    holes: fp
                                        .holes
                                        .iter()
                                        .map(|h| h.iter().map(coords).collect())
                                        .collect(),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            }
        })
        .collect();

    let group_dtos: Vec<GroupDto> = data
        .groups
        .iter()
        .enumerate()
        .map(|(i, (gid, members, name, tags))| GroupDto {
            id: i as u32,
            members: members.iter().map(&node_to_dto).collect(),
            name: name.clone(),
            tags: tags.clone(),
            hidden: data.group_hidden.contains(gid),
        })
        .collect();

    let component_dtos: Vec<ComponentDto> = data
        .components
        .iter()
        .enumerate()
        .map(|(i, (_, members, name))| ComponentDto {
            id: i as u32,
            members: members.iter().map(|oid| obj_to_dense[oid]).collect(),
            name: name.clone(),
        })
        .collect();

    let instance_dtos: Vec<InstanceDto> = data
        .instances
        .iter()
        .enumerate()
        .map(|(i, (iid, def, pose, name, tags))| InstanceDto {
            id: i as u32,
            def: comp_to_dense[def],
            pose: pose.to_affine(),
            name: name.clone(),
            tags: tags.clone(),
            hidden: data.instance_hidden.contains(iid),
        })
        .collect();

    let sketch_dtos: Vec<SketchDto> = data
        .sketches
        .iter()
        .map(|(_, sk)| encode_sketch(sk))
        .collect();
    // Patch the ids to be dense (encode_sketch uses slotmap-iteration indices):
    let sketch_dtos: Vec<SketchDto> = sketch_dtos
        .into_iter()
        .enumerate()
        .map(|(i, mut dto)| {
            dto.id = i as u32;
            dto
        })
        .collect();

    // Guides: dense ids in enumerate order (their own slotmap-derived
    // order from `Document::save`).
    let guide_dtos: Vec<GuideDto> = data
        .guides
        .iter()
        .enumerate()
        .map(|(i, (_, guide))| match guide {
            Guide::Line { origin, direction } => GuideDto {
                id: i as u32,
                kind: "line".to_string(),
                p: [origin.x, origin.y, origin.z],
                dir: Some([direction.x, direction.y, direction.z]),
            },
            Guide::Point { position } => GuideDto {
                id: i as u32,
                kind: "point".to_string(),
                p: [position.x, position.y, position.z],
                dir: None,
            },
        })
        .collect();

    let root_dtos: Vec<NodeRefDto> = data.roots.iter().map(&node_to_dto).collect();

    // consumed: sort ascending (first by sketch dense id, then region dense id).
    // The region dense ids are per-sketch; we embed them in the SketchDtos.
    // But wait — we need to map (SketchId, SketchRegionId) → (dense_sketch_id,
    // dense_region_id). The region dense ids are those assigned inside
    // encode_sketch. We need to re-derive them here.
    let mut consumed_pairs: Vec<[u32; 2]> = data
        .consumed
        .iter()
        .filter_map(|(sid, rid)| {
            let dense_sid = sketch_to_dense.get(sid).copied()?;
            let sk_idx = data.sketches.iter().position(|(s, _)| s == sid)?;
            let (_, sk) = &data.sketches[sk_idx];
            let dense_rid = sk
                .regions()
                .keys()
                .enumerate()
                .find(|(_, r)| r == rid)
                .map(|(i, _)| i as u32)?;
            Some([dense_sid, dense_rid])
        })
        .collect();
    // Sort for determinism (encode in dense-id order, not raw set order).
    consumed_pairs.sort_unstable();

    let manifest = Manifest {
        format_version: MANIFEST_FORMAT_VERSION,
        geometry_version: GEOMETRY_FORMAT_VERSION,
        app: "hew".to_string(),
        app_version: "0.1.0".to_string(),
        materials: mat_dtos,
        objects: obj_dtos,
        groups: group_dtos,
        components: component_dtos,
        instances: instance_dtos,
        sketches: sketch_dtos,
        roots: root_dtos,
        consumed: consumed_pairs,
        guides: guide_dtos,
        tags: data
            .tag_meta
            .iter()
            .map(|(path, hidden)| TagDto {
                path: path.clone(),
                hidden: *hidden,
            })
            .collect(),
    };

    let manifest_json =
        serde_json::to_vec_pretty(&manifest).expect("manifest serialization must not fail");

    // ── 5. Assemble zip (HEW_FILE_FORMAT.md fixed entry order) ────────────
    let out_cursor = Cursor::new(Vec::<u8>::new());
    let mut zip = zip::ZipWriter::new(out_cursor);

    zip_add_stored_entry(&mut zip, "manifest.json", &manifest_json)
        .expect("zip write must not fail");

    for (i, geom_buf) in geometry_buffers.iter().enumerate() {
        let name = format!("geometry/obj_{i}.bin");
        zip_add_stored_entry(&mut zip, &name, geom_buf).expect("zip write must not fail");
    }

    for (path, bytes) in &texture_entries {
        zip_add_stored_entry(&mut zip, path, bytes).expect("zip write must not fail");
    }

    zip.finish().expect("zip finish must not fail").into_inner()
}

/// Encode a sketch's internal structure into a SketchDto, assigning dense ids
/// for vertices, edges, and regions in slotmap iteration order.
fn encode_sketch(sk: &Sketch) -> SketchDto {
    let plane = sk.plane();
    let n = plane.normal();
    // offset = n · p for any point p on the plane. Recover via
    // signed_distance(origin) = n·origin - offset = -offset.
    let offset = -plane.signed_distance(Point3::ORIGIN);
    let plane_arr = [n.x, n.y, n.z, offset];

    // Map vertex ids to dense indices.
    let mut vert_to_dense: SecondaryMap<SketchVertexId, u32> = SecondaryMap::new();
    let mut vert_dtos = Vec::new();
    for (vid, v) in sk.vertices() {
        let idx = vert_dtos.len() as u32;
        vert_to_dense.insert(vid, idx);
        vert_dtos.push(SketchVertexDto {
            id: idx,
            p: [v.position.x, v.position.y, v.position.z],
        });
    }

    let mut edge_dtos = Vec::new();
    // Dense curve ids by first appearance in edge slotmap order —
    // deterministic for a deterministically built sketch.
    let mut curve_to_dense: BTreeMap<SketchCurveId, u32> = BTreeMap::new();
    for (_, e) in sk.edges() {
        let eid = edge_dtos.len() as u32;
        let curve = e.curve.map(|c| {
            let next = curve_to_dense.len() as u32;
            *curve_to_dense.entry(c).or_insert(next)
        });
        edge_dtos.push(SketchEdgeDto {
            id: eid,
            from: vert_to_dense[e.from],
            to: vert_to_dense[e.to],
            curve,
        });
    }

    let mut region_dtos = Vec::new();
    for (_, r) in sk.regions() {
        let rid = region_dtos.len() as u32;
        let outer: Vec<u32> = r.outer.iter().map(|v| vert_to_dense[*v]).collect();
        let holes: Vec<Vec<u32>> = r
            .holes
            .iter()
            .map(|h| h.iter().map(|v| vert_to_dense[*v]).collect())
            .collect();
        region_dtos.push(SketchRegionDto {
            id: rid,
            outer,
            holes,
        });
    }

    SketchDto {
        id: 0, // patched by caller
        plane: plane_arr,
        vertices: vert_dtos,
        edges: edge_dtos,
        regions: region_dtos,
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Document load orchestration
// ════════════════════════════════════════════════════════════════════════════

/// Raw data decoded from a `.hew` zip container. `Document::load` inserts
/// materials first (to get live MaterialIds), then calls `Object::decode` for
/// each geometry buffer with the reverse dense→MaterialId closure.
pub(crate) struct DocLoadRaw {
    pub materials: Vec<Material>,
    pub geom_buffers: Vec<Vec<u8>>,
    /// base_material dense id for each object (None = no base material).
    pub obj_base_materials: Vec<Option<u32>>,
    pub groups: Vec<Vec<NodeRefDto>>,
    pub components: Vec<Vec<u32>>,
    pub instances: Vec<(u32, Transform)>,
    pub sketches: Vec<Sketch>,
    /// Construction guides (manifest v4+), in manifest dense-id order.
    pub guides: Vec<Guide>,
    pub consumed: Vec<[u32; 2]>,
    /// For each object dense id: is it a definition member? (and which component dense id)
    pub def_membership: Vec<Option<u32>>,
    /// Optional display name per object/group/component/instance, in dense order
    /// (manifest v2+; all `None` for v1 files).
    pub obj_names: Vec<Option<String>>,
    /// Extrusion provenance per object, in dense order (manifest v8 files).
    pub obj_sources: Vec<Option<[u32; 2]>>,
    /// Extrusion footprints per object, in dense order (manifest v9+).
    pub obj_footprints: Vec<Vec<FootprintDto>>,
    pub group_names: Vec<Option<String>>,
    pub component_names: Vec<Option<String>>,
    pub instance_names: Vec<Option<String>>,
    /// Tag lists per object/group/instance, in dense order (manifest v3+; empty
    /// for v1/v2 files — `#[serde(default)]` fills them in).
    pub obj_tags: Vec<Vec<Vec<String>>>,
    pub group_tags: Vec<Vec<Vec<String>>>,
    pub instance_tags: Vec<Vec<Vec<String>>>,
    /// Tag metadata registry (manifest v5+; empty for v1–v4 files).
    pub tag_meta: Vec<(Vec<String>, bool)>,
    /// USER-hidden flags per object/group/instance dense id (manifest v6+;
    /// all false for older files).
    pub obj_hidden: Vec<bool>,
    pub group_hidden: Vec<bool>,
    pub instance_hidden: Vec<bool>,
}

pub(crate) fn decode_document_raw(bytes: &[u8]) -> Result<DocLoadRaw, LoadError> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| LoadError::NotAContainer)?;

    // Read manifest.
    let manifest_bytes = zip_read_entry(&mut zip, "manifest.json")?;
    let manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).map_err(|e| LoadError::MalformedManifest {
            what: e.to_string(),
        })?;

    // Accept any version this build knows how to read: 0 is invalid, anything
    // newer than we understand is rejected. Older versions (e.g. v1 without
    // node names) still load — the missing fields default to `None`.
    if manifest.format_version == 0 || manifest.format_version > MANIFEST_FORMAT_VERSION {
        return Err(LoadError::UnsupportedVersion {
            found: manifest.format_version,
        });
    }

    let obj_count = manifest.objects.len();
    let mat_count = materials_count_validate(&manifest)?;

    // Rebuild materials (with texture image bytes).
    let mut materials: Vec<Material> = Vec::with_capacity(mat_count);
    for mat_dto in &manifest.materials {
        let texture = if let Some(tex_dto) = &mat_dto.texture {
            let image = zip_read_entry(&mut zip, &tex_dto.asset)?;
            let format = match tex_dto.format.as_str() {
                "png" => ImageFormat::Png,
                "jpg" | "jpeg" => ImageFormat::Jpeg,
                _ => {
                    return Err(LoadError::MalformedManifest {
                        what: format!("unknown texture format '{}'", tex_dto.format),
                    });
                }
            };
            Some(Texture {
                image,
                format,
                world_size: tex_dto.world_size,
            })
        } else {
            None
        };
        let c = mat_dto.color;
        materials.push(Material {
            name: mat_dto.name.clone(),
            color: crate::material::Rgba8::rgba(c[0], c[1], c[2], c[3]),
            texture,
        });
    }

    // Read geometry buffers.
    let mut geom_buffers: Vec<Vec<u8>> = Vec::with_capacity(obj_count);
    let mut obj_base_materials: Vec<Option<u32>> = Vec::with_capacity(obj_count);
    let mut obj_names: Vec<Option<String>> = Vec::with_capacity(obj_count);
    let mut obj_sources: Vec<Option<[u32; 2]>> = Vec::with_capacity(obj_count);
    let mut obj_footprints: Vec<Vec<FootprintDto>> = Vec::with_capacity(obj_count);
    for obj_dto in &manifest.objects {
        let buf = zip_read_entry(&mut zip, &obj_dto.geometry)?;
        geom_buffers.push(buf);
        obj_base_materials.push(obj_dto.base_material);
        obj_names.push(obj_dto.name.clone());
        obj_sources.push(obj_dto.source);
        obj_footprints.push(obj_dto.footprints.clone());
    }

    // Decode sketches.
    let mut sketches: Vec<Sketch> = Vec::with_capacity(manifest.sketches.len());
    for sk_dto in &manifest.sketches {
        let sk = decode_sketch(sk_dto, mat_count)?;
        sketches.push(sk);
    }

    // Decode guides (manifest v4+; absent in v1-v3 files → empty).
    let mut guides: Vec<Guide> = Vec::with_capacity(manifest.guides.len());
    for guide_dto in &manifest.guides {
        guides.push(decode_guide(guide_dto)?);
    }

    // Validate manifest references.
    validate_manifest_references(&manifest, obj_count, mat_count)?;

    // Build def membership: for each object dense id, which component owns it?
    let mut def_membership: Vec<Option<u32>> = vec![None; obj_count];
    for (ci, comp) in manifest.components.iter().enumerate() {
        for &oid in &comp.members {
            if (oid as usize) < obj_count {
                def_membership[oid as usize] = Some(ci as u32);
            }
        }
    }

    let group_names = manifest.groups.iter().map(|g| g.name.clone()).collect();
    let component_names = manifest.components.iter().map(|c| c.name.clone()).collect();
    let instance_names = manifest.instances.iter().map(|i| i.name.clone()).collect();
    let obj_tags: Vec<Vec<Vec<String>>> = manifest.objects.iter().map(|o| o.tags.clone()).collect();
    let group_tags: Vec<Vec<Vec<String>>> =
        manifest.groups.iter().map(|g| g.tags.clone()).collect();
    let instance_tags: Vec<Vec<Vec<String>>> =
        manifest.instances.iter().map(|i| i.tags.clone()).collect();

    Ok(DocLoadRaw {
        materials,
        geom_buffers,
        obj_base_materials,
        groups: manifest.groups.iter().map(|g| g.members.clone()).collect(),
        components: manifest
            .components
            .iter()
            .map(|c| c.members.clone())
            .collect(),
        instances: manifest
            .instances
            .iter()
            .map(|i| (i.def, Transform::from_affine(&i.pose)))
            .collect(),
        sketches,
        guides,
        consumed: manifest.consumed.clone(),
        def_membership,
        obj_names,
        obj_sources,
        obj_footprints,
        group_names,
        component_names,
        instance_names,
        obj_tags,
        group_tags,
        instance_tags,
        tag_meta: manifest
            .tags
            .iter()
            .map(|t| (t.path.clone(), t.hidden))
            .collect(),
        obj_hidden: manifest.objects.iter().map(|o| o.hidden).collect(),
        group_hidden: manifest.groups.iter().map(|g| g.hidden).collect(),
        instance_hidden: manifest.instances.iter().map(|i| i.hidden).collect(),
    })
}

/// Validate all cross-references in the manifest.
fn validate_manifest_references(
    manifest: &Manifest,
    obj_count: usize,
    mat_count: usize,
) -> Result<(), LoadError> {
    for grp in &manifest.groups {
        for m in &grp.members {
            match m.kind.as_str() {
                "object" => {
                    if m.id as usize >= obj_count {
                        return Err(LoadError::DanglingReference {
                            what: format!(
                                "group {} member object id {} out of range",
                                grp.id, m.id
                            ),
                        });
                    }
                }
                "group" => {
                    if m.id as usize >= manifest.groups.len() {
                        return Err(LoadError::DanglingReference {
                            what: format!("group {} member group id {} out of range", grp.id, m.id),
                        });
                    }
                }
                "instance" => {
                    if m.id as usize >= manifest.instances.len() {
                        return Err(LoadError::DanglingReference {
                            what: format!(
                                "group {} member instance id {} out of range",
                                grp.id, m.id
                            ),
                        });
                    }
                }
                _ => {
                    return Err(LoadError::MalformedManifest {
                        what: format!("unknown node kind '{}'", m.kind),
                    });
                }
            }
        }
    }

    for comp in &manifest.components {
        for &m in &comp.members {
            if m as usize >= obj_count {
                return Err(LoadError::DanglingReference {
                    what: format!("component {} member object id {} out of range", comp.id, m),
                });
            }
        }
    }

    for inst in &manifest.instances {
        if inst.def as usize >= manifest.components.len() {
            return Err(LoadError::DanglingReference {
                what: format!("instance {} def id {} out of range", inst.id, inst.def),
            });
        }
    }

    for root in &manifest.roots {
        match root.kind.as_str() {
            "object" => {
                if root.id as usize >= obj_count {
                    return Err(LoadError::DanglingReference {
                        what: format!("root object id {} out of range", root.id),
                    });
                }
            }
            "group" => {
                if root.id as usize >= manifest.groups.len() {
                    return Err(LoadError::DanglingReference {
                        what: format!("root group id {} out of range", root.id),
                    });
                }
            }
            "instance" => {
                if root.id as usize >= manifest.instances.len() {
                    return Err(LoadError::DanglingReference {
                        what: format!("root instance id {} out of range", root.id),
                    });
                }
            }
            _ => {
                return Err(LoadError::MalformedManifest {
                    what: format!("unknown node kind '{}' in roots", root.kind),
                });
            }
        }
    }

    for obj_dto in &manifest.objects {
        if let Some(bm) = obj_dto.base_material
            && bm as usize >= mat_count
        {
            return Err(LoadError::DanglingReference {
                what: format!("object {} base_material id {} out of range", obj_dto.id, bm),
            });
        }
    }

    for [sid, rid] in &manifest.consumed {
        if *sid as usize >= manifest.sketches.len() {
            return Err(LoadError::DanglingReference {
                what: format!("consumed sketch id {sid} out of range"),
            });
        }
        let sk_dto = &manifest.sketches[*sid as usize];
        if *rid as usize >= sk_dto.regions.len() {
            return Err(LoadError::DanglingReference {
                what: format!("consumed region id {rid} in sketch {sid} out of range"),
            });
        }
    }

    let _ = mat_count; // used above
    Ok(())
}

fn materials_count_validate(manifest: &Manifest) -> Result<usize, LoadError> {
    Ok(manifest.materials.len())
}

// ════════════════════════════════════════════════════════════════════════════
// Sketch reconstruction
// ════════════════════════════════════════════════════════════════════════════

/// Structural sketch reconstruction: inserts vertices, edges, and regions
/// directly without re-tracing the geometry (which would churn region ids
/// and break the `consumed` mapping).
pub(crate) fn decode_sketch(dto: &SketchDto, _mat_count: usize) -> Result<Sketch, LoadError> {
    let [nx, ny, nz, offset] = dto.plane;
    let normal = Vec3::new(nx, ny, nz);
    let plane =
        Plane::from_point_normal(Point3::ORIGIN + normal * offset, normal).map_err(|_| {
            LoadError::MalformedManifest {
                what: format!("sketch {} has degenerate plane normal", dto.id),
            }
        })?;

    let mut sk = Sketch::reconstruct(plane);

    // Insert vertices in dense id order.
    let mut vert_ids: Vec<SketchVertexId> = Vec::with_capacity(dto.vertices.len());
    for v in &dto.vertices {
        let id = sk.insert_vertex_raw(SketchVertex {
            position: Point3::new(v.p[0], v.p[1], v.p[2]),
        });
        vert_ids.push(id);
    }

    // Insert edges in dense id order, minting one SketchCurveId per dense
    // curve index as they appear (indices need not arrive sorted).
    let mut curve_ids: Vec<SketchCurveId> = Vec::new();
    for e in &dto.edges {
        let from_idx = e.from as usize;
        let to_idx = e.to as usize;
        if from_idx >= vert_ids.len() || to_idx >= vert_ids.len() {
            return Err(LoadError::DanglingReference {
                what: format!(
                    "sketch {} edge {} references out-of-range vertex",
                    dto.id, e.id
                ),
            });
        }
        let curve = e.curve.map(|ci| {
            let ci = ci as usize;
            while curve_ids.len() <= ci {
                curve_ids.push(sk.insert_curve_raw());
            }
            curve_ids[ci]
        });
        sk.insert_edge_raw(SketchEdge {
            from: vert_ids[from_idx],
            to: vert_ids[to_idx],
            curve,
        });
    }

    // Insert regions in dense id order.
    for r in &dto.regions {
        let outer: Vec<SketchVertexId> = r
            .outer
            .iter()
            .map(|&vi| {
                let i = vi as usize;
                if i < vert_ids.len() {
                    Ok(vert_ids[i])
                } else {
                    Err(LoadError::DanglingReference {
                        what: format!(
                            "sketch {} region {} outer vertex id {} out of range",
                            dto.id, r.id, vi
                        ),
                    })
                }
            })
            .collect::<Result<_, _>>()?;
        let holes: Vec<Vec<SketchVertexId>> = r
            .holes
            .iter()
            .map(|h| {
                h.iter()
                    .map(|&vi| {
                        let i = vi as usize;
                        if i < vert_ids.len() {
                            Ok(vert_ids[i])
                        } else {
                            Err(LoadError::DanglingReference {
                                what: format!(
                                    "sketch {} region {} hole vertex id {} out of range",
                                    dto.id, r.id, vi
                                ),
                            })
                        }
                    })
                    .collect::<Result<_, _>>()
            })
            .collect::<Result<_, _>>()?;
        sk.insert_region_raw(SketchRegion { outer, holes });
    }

    // Islands are derived, never serialized — compute them from the
    // reconstructed edge graph.
    sk.recompute_islands();

    Ok(sk)
}

// ════════════════════════════════════════════════════════════════════════════
// Guide reconstruction
// ════════════════════════════════════════════════════════════════════════════

/// Decodes one [`GuideDto`] into a [`Guide`], rejecting (never repairing) a
/// malformed entry: an unknown `kind`, a non-finite coordinate, or a `"line"`
/// with a missing/non-unit-normalizable `dir`.
fn decode_guide(dto: &GuideDto) -> Result<Guide, LoadError> {
    let p = Point3::new(dto.p[0], dto.p[1], dto.p[2]);
    if !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() {
        return Err(LoadError::MalformedManifest {
            what: format!("guide {} has a non-finite coordinate", dto.id),
        });
    }
    match dto.kind.as_str() {
        "line" => {
            let dir = dto.dir.ok_or_else(|| LoadError::MalformedManifest {
                what: format!("guide {} is a line but has no direction", dto.id),
            })?;
            let direction = Vec3::new(dir[0], dir[1], dir[2]);
            if !direction.x.is_finite() || !direction.y.is_finite() || !direction.z.is_finite() {
                return Err(LoadError::MalformedManifest {
                    what: format!("guide {} has a non-finite direction", dto.id),
                });
            }
            let direction = direction
                .normalized()
                .map_err(|_| LoadError::MalformedManifest {
                    what: format!("guide {} has a zero-length direction", dto.id),
                })?;
            Ok(Guide::Line {
                origin: p,
                direction,
            })
        }
        "point" => Ok(Guide::Point { position: p }),
        other => Err(LoadError::MalformedManifest {
            what: format!("guide {} has unknown kind '{other}'", dto.id),
        }),
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Low-level byte reader
// ════════════════════════════════════════════════════════════════════════════

struct ByteReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        ByteReader { data, pos: 0 }
    }

    fn read_bytes<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        if self.pos + N > self.data.len() {
            return Err(DecodeError::Truncated);
        }
        let arr: [u8; N] = self.data[self.pos..self.pos + N].try_into().unwrap();
        self.pos += N;
        Ok(arr)
    }

    fn read_u8(&mut self) -> Result<u8, DecodeError> {
        if self.pos >= self.data.len() {
            return Err(DecodeError::Truncated);
        }
        let v = self.data[self.pos];
        self.pos += 1;
        Ok(v)
    }

    fn read_u32(&mut self) -> Result<u32, DecodeError> {
        let bytes = self.read_bytes::<4>()?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn read_f64(&mut self) -> Result<f64, DecodeError> {
        let bytes = self.read_bytes::<8>()?;
        Ok(f64::from_le_bytes(bytes))
    }
}

#[cfg(test)]
mod name_compat_tests {
    use super::{ComponentDto, GroupDto, InstanceDto, ObjectDto};

    /// v1 manifests have no `name` field on tree entries. The `#[serde(default)]`
    /// attributes must let them deserialize, defaulting the name to `None`, so
    /// pre-naming `.hew` files keep loading after the v2 bump.
    #[test]
    fn dtos_default_name_to_none_for_v1_manifests() {
        let obj: ObjectDto =
            serde_json::from_str(r#"{"id":0,"geometry":"geometry/obj_0.bin"}"#).unwrap();
        assert!(obj.name.is_none());

        let grp: GroupDto = serde_json::from_str(r#"{"id":0,"members":[]}"#).unwrap();
        assert!(grp.name.is_none());

        let comp: ComponentDto = serde_json::from_str(r#"{"id":0,"members":[]}"#).unwrap();
        assert!(comp.name.is_none());

        let inst: InstanceDto =
            serde_json::from_str(r#"{"id":0,"def":0,"pose":[1,0,0,0,0,1,0,0,0,0,1,0]}"#).unwrap();
        assert!(inst.name.is_none());
    }

    /// A present `name` round-trips through serde.
    #[test]
    fn dto_name_round_trips() {
        let obj: ObjectDto =
            serde_json::from_str(r#"{"id":0,"geometry":"g.bin","name":"Counter_Base"}"#).unwrap();
        assert_eq!(obj.name.as_deref(), Some("Counter_Base"));
    }
}

#[cfg(test)]
mod planarity_flag_tests {
    use crate::topo::Object;
    use crate::{Point3, tol};

    /// The v3 geometry buffer round-trips an imported object's wider planarity
    /// tolerance, so a near-planar face still validates after load (#35). A
    /// strict native object round-trips at `PLANE_DIST`.
    #[test]
    fn imported_planarity_tol_round_trips() {
        // Unit box with the top quad bent 1e-5 m (>PLANE_DIST, <IMPORT_PLANE_DIST).
        let positions = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
            Point3::new(0.0, 0.0, 1.0),
            Point3::new(1.0, 0.0, 1.0),
            Point3::new(1.0, 1.0, 1.0 + 1e-5),
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
        let mats = vec![None; faces.len()];
        let frames = vec![None; faces.len()];
        let imported = Object::from_polygons_with_materials_and_frames_import(
            &positions, &faces, &mats, &frames,
        )
        .expect("import build");
        assert_eq!(imported.planarity_tol, tol::IMPORT_PLANE_DIST);

        let bytes = imported.encode(&|_| 0);
        let decoded = Object::decode(&bytes, &|_| None).expect("decode v3 import buffer");
        assert_eq!(
            decoded.planarity_tol,
            tol::IMPORT_PLANE_DIST,
            "imported flag must survive save/load"
        );
        decoded
            .validate()
            .expect("decoded import object validates at its restored tolerance");

        // A strict native object round-trips at PLANE_DIST (flag = 0).
        let native = Object::tetrahedron();
        assert_eq!(native.planarity_tol, tol::PLANE_DIST);
        let nb = native.encode(&|_| 0);
        let nd = Object::decode(&nb, &|_| None).unwrap();
        assert_eq!(nd.planarity_tol, tol::PLANE_DIST);
    }
}
