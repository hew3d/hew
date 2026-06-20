//! Materials: a document-level palette of named surfaces (ARCHITECTURE.md).
//!
//! A [`Material`] is a solid color plus an optional image [`Texture`]. Faces
//! reference a material by [`crate::ids::MaterialId`] (or `None` = default);
//! the palette itself lives on the [`crate::document::Document`]. This keeps
//! materials shareable across objects and gives the native file format a single
//! `materials` table to serialize (per-face data is just an index).
//!
//! Kernel purity (DEVELOPMENT.md rule 1): the kernel never decodes image data. A
//! [`Texture`] holds the **authored encoded bytes** (PNG/JPEG) as an opaque
//! blob — the shell supplies them, the renderer decodes them, the file format
//! stores them verbatim. No image-codec dependency enters the kernel.

use crate::ids::MaterialId;
use crate::math::{Point3, Vec3};
use slotmap::SlotMap;

/// The document's material palette: a generational map of [`MaterialId`] →
/// [`Material`]. Exposed via [`crate::document::Document::materials`] so the
/// tessellator can resolve a face's color/texture/world-size for render buffers
/// without depending on the document model directly.
pub type MaterialPalette = SlotMap<MaterialId, Material>;

/// An 8-bit-per-channel straight-alpha color. Compact and deterministic for the
/// file format; the renderer divides by 255 for linear/sRGB upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba8 {
    /// Red, 0–255.
    pub r: u8,
    /// Green, 0–255.
    pub g: u8,
    /// Blue, 0–255.
    pub b: u8,
    /// Alpha, 0–255 (255 = opaque).
    pub a: u8,
}

impl Rgba8 {
    /// An opaque color from its three channels.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Rgba8 {
        Rgba8 { r, g, b, a: 255 }
    }

    /// A color with explicit alpha.
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Rgba8 {
        Rgba8 { r, g, b, a }
    }
}

/// The container/codec of a texture's authored bytes. The kernel does not
/// decode; this is a hint for the renderer and the file-format asset entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    /// PNG (`image/png`).
    Png,
    /// JPEG (`image/jpeg`).
    Jpeg,
}

impl ImageFormat {
    /// The MIME type, for building a renderer `Blob`/`ImageBitmap`.
    pub fn mime(self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
        }
    }
}

/// An image texture applied to a material via planar projection.
///
/// `world_size` is the real-world extent (meters) that one full image tile
/// covers along the face-plane basis; tessellation derives UVs as the planar
/// projection divided by this size, so a texture keeps a fixed physical scale
/// regardless of face size (SketchUp-style).
// `world_size` carries f64s, so this is `PartialEq` but not `Eq`.
#[derive(Debug, Clone, PartialEq)]
pub struct Texture {
    /// Authored encoded image bytes (opaque to the kernel).
    pub image: Vec<u8>,
    /// Codec of `image`.
    pub format: ImageFormat,
    /// World-space tile size `[width, height]` in meters; both must be > 0.
    pub world_size: [f64; 2],
}

/// A palette entry: a named solid color with an optional image texture.
///
/// When `texture` is `Some`, the renderer uses the image (modulated by
/// `color`); otherwise the face is a flat `color`.
// Holds a `Texture` (f64 `world_size`), so `PartialEq` but not `Eq`.
#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    /// Human-facing name (for the palette UI and the file manifest).
    pub name: String,
    /// Solid color, and the modulation tint when `texture` is set.
    pub color: Rgba8,
    /// Optional image texture (Stage B); `None` = flat color.
    pub texture: Option<Texture>,
}

impl Material {
    /// A flat-color material.
    pub fn solid(name: impl Into<String>, color: Rgba8) -> Material {
        Material {
            name: name.into(),
            color,
            texture: None,
        }
    }

    /// A textured material (color modulates the image).
    pub fn textured(name: impl Into<String>, color: Rgba8, texture: Texture) -> Material {
        Material {
            name: name.into(),
            color,
            texture: Some(texture),
        }
    }

    /// Whether this material carries an image texture.
    pub fn has_texture(&self) -> bool {
        self.texture.is_some()
    }
}

/// A face's material reference: `None` is the default (unpainted) material,
/// rendered with a neutral color the renderer owns.
pub type FaceMaterial = Option<MaterialId>;

// ────────────────────────────────────────────────────── UvFrame ────────────────

/// Per-face affine UV map: `uv = (s·p + u0,  t·p + v0)` (ARCHITECTURE.md ext.).
///
/// `s` and `t` are 3D gradient vectors (dimensionless UV/meter); `u0`/`v0` are
/// the constant offsets. The map is expressed in the same coordinate space as
/// the face's vertex positions, so it is pose-invariant: the texture rides the
/// geometry through any instance transformation.
///
/// Imported from COLLADA via a least-squares fit from source per-corner texcoords
/// (see `crates/dae-import/src/uv.rs`). Absent on Hew-drawn faces → tessellate
/// falls back to the  `world_size` planar projection.
///
/// Serialized in geometry buffer v2 (HEW_FILE_FORMAT.md, after the per-face
/// material u32): a `u8` flag (0=absent, 1=present) + 8 × little-endian f64 in
/// order `s.x s.y s.z t.x t.y t.z u0 v0` (64 bytes when present).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UvFrame {
    /// UV-space gradient along the U axis, in world space.
    pub s: Vec3,
    /// UV-space gradient along the V axis, in world space.
    pub t: Vec3,
    /// U offset (constant term).
    pub u0: f64,
    /// V offset (constant term).
    pub v0: f64,
}

impl UvFrame {
    /// Create a new `UvFrame` from its components.
    pub fn new(s: Vec3, t: Vec3, u0: f64, v0: f64) -> UvFrame {
        UvFrame { s, t, u0, v0 }
    }

    /// Apply this frame to a 3D position, returning `[u, v]`.
    ///
    /// `uv = (s·p + u0,  t·p + v0)` where `·` is the dot product.
    pub fn apply(&self, p: Point3) -> [f64; 2] {
        let u = self.s.x * p.x + self.s.y * p.y + self.s.z * p.z + self.u0;
        let v = self.t.x * p.x + self.t.y * p.y + self.t.z * p.z + self.v0;
        [u, v]
    }
}
