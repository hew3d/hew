//! Place Image for Hew: a PNG or JPEG placed in the model as a picture.
//!
//! I/O-free and WASM-bound (DEVELOPMENT.md rule 1 — depends on `kernel`
//! only, never the reverse). A placed picture is built exactly the way a
//! `.skp` file's image arrives (`crates/skp-import`'s `image_def` and
//! `image_children`): a component definition named after the file holding
//! one rectangular face painted with the picture, and one instance of it.
//! The face's material carries the image bytes verbatim (nothing is decoded,
//! re-encoded or scaled) with a tile the size of the face, and the face's UV
//! frame stretches one tile over it exactly, so the picture covers the face
//! once whatever the face's later size. The instance's pose carries the
//! placement frame (a rigid frame: a picture can be opened and edited like
//! any component).
//!
//! The only thing read from the bytes is the picture's size in pixels (the
//! PNG `IHDR` chunk, or a JPEG frame header and its EXIF orientation), which
//! sets the face's height from its width.
//!
//! ```text
//! bytes + frame + width ──read_info──> pixel size ──import──> kernel::ImportScene
//! ```

use kernel::{
    DefRecipe, ImageFormat, ImportNode, ImportScene, Material, MeshExtra, MeshRecipe, NO_MATERIAL,
    Point3, Rgba8, Texture, Transform, UvFrame, Vec3,
};

/// The largest picture Hew places, in pixels (64 megapixels). The bytes are
/// kept as they are, so a larger picture would cost its full decoded size
/// in every renderer that shows it; past this the user is asked for a
/// smaller copy instead.
pub const MAX_PIXELS: u64 = 64_000_000;

/// The shortest side a placed picture may have, in meters (one
/// micrometer). The kernel builds no face whose plane it cannot find
/// (twice its area under `tol::NORMALIZE_MIN_LENGTH`, so under 5e-13 m²),
/// and a picture with both sides at least this long has at least twice
/// that area; a smaller placement is refused instead of silently placing
/// nothing.
pub const MIN_SIDE: f64 = 1e-6;

/// The longest side a placed picture may have, in meters (1,000 km). Past
/// this the model's coordinates lose the precision the kernel's tolerances
/// rely on, so a width typed or dragged to an absurd size is refused with
/// its own message instead of being read as a failure to build.
pub const MAX_SIDE: f64 = 1e6;

/// The name a picture with no usable file name gets.
pub const DEFAULT_NAME: &str = "Image";

/// Typed refusals from [`read_info`] and [`import`]. `wasm-api` turns each
/// into a `"CODE: message"` error (see [`ImageError::code`]).
#[derive(Debug, Clone, PartialEq)]
pub enum ImageError {
    /// The bytes are not a PNG or JPEG Hew can read the size of (or are
    /// not the format they were said to be).
    Unreadable(String),
    /// The picture has more than [`MAX_PIXELS`] pixels.
    TooLarge {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
    },
    /// The placement frame or width cannot place a picture: a non-finite
    /// or zero-length axis, two parallel axes, or a width that is not a
    /// positive finite length.
    BadPlacement(String),
    /// The placed picture would have a side shorter than [`MIN_SIDE`]: a
    /// face too small for the kernel to build.
    TooSmall {
        /// The width asked for, meters.
        width: f64,
        /// The height that width gives, meters.
        height: f64,
    },
    /// The placed picture would have a side longer than [`MAX_SIDE`].
    TooBig {
        /// The width asked for, meters.
        width: f64,
        /// The height that width gives, meters.
        height: f64,
    },
    /// The kernel would not build the picture's face (see [`check`]), with
    /// the kernel's own reason. Its code is `ImageTooSmall`, as for a side
    /// under [`MIN_SIDE`]: nothing is placed.
    Unbuildable(String),
}

impl ImageError {
    /// The error's code at the wasm boundary (`"CODE: message"`).
    pub fn code(&self) -> &'static str {
        match self {
            ImageError::Unreadable(_) => "ImageUnreadable",
            ImageError::TooLarge { .. } => "ImageTooLarge",
            ImageError::BadPlacement(_) => "BadImagePlacement",
            ImageError::TooSmall { .. } | ImageError::Unbuildable(_) => "ImageTooSmall",
            ImageError::TooBig { .. } => "ImageTooBig",
        }
    }
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageError::Unreadable(why) => write!(f, "the picture can't be read ({why})"),
            ImageError::TooLarge { width, height } => write!(
                f,
                "the picture is {width} x {height} pixels, more than the {} megapixels Hew places",
                MAX_PIXELS / 1_000_000
            ),
            ImageError::BadPlacement(why) => {
                write!(f, "the placement can't hold a picture ({why})")
            }
            ImageError::TooSmall { width, height } => write!(
                f,
                "the picture would be {width} x {height} m, and each side must be at least {MIN_SIDE} m"
            ),
            ImageError::TooBig { width, height } => write!(
                f,
                "the picture would be {width} x {height} m, and each side must be at most {MAX_SIDE} m"
            ),
            ImageError::Unbuildable(why) => {
                write!(f, "the picture's face can't be built ({why})")
            }
        }
    }
}

impl std::error::Error for ImageError {}

/// What [`read_info`] reads from a picture's bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageInfo {
    /// The codec the bytes are in.
    pub format: ImageFormat,
    /// Width in pixels, as the picture is shown (a JPEG's EXIF orientation
    /// applied: a picture taken turned is shown upright, as browsers show
    /// it).
    pub width: u32,
    /// Height in pixels, as shown.
    pub height: u32,
}

/// The picture's format and size in pixels, from its header alone.
///
/// # Errors
/// [`ImageError::Unreadable`] for bytes that are neither a PNG nor a JPEG
/// with a readable size, or a size of zero; [`ImageError::TooLarge`] past
/// [`MAX_PIXELS`].
pub fn read_info(bytes: &[u8]) -> Result<ImageInfo, ImageError> {
    let info = if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        png_info(bytes)?
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        jpeg_info(bytes)?
    } else {
        return Err(ImageError::Unreadable("not a PNG or JPEG".into()));
    };
    if info.width == 0 || info.height == 0 {
        return Err(ImageError::Unreadable("the picture has no pixels".into()));
    }
    if u64::from(info.width) * u64::from(info.height) > MAX_PIXELS {
        return Err(ImageError::TooLarge {
            width: info.width,
            height: info.height,
        });
    }
    Ok(info)
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes([
        *b.get(at)?,
        *b.get(at + 1)?,
        *b.get(at + 2)?,
        *b.get(at + 3)?,
    ]))
}

/// A PNG's size: the `IHDR` chunk, which must come first.
fn png_info(b: &[u8]) -> Result<ImageInfo, ImageError> {
    if b.get(12..16) != Some(b"IHDR") {
        return Err(ImageError::Unreadable("a PNG with no header chunk".into()));
    }
    match (be32(b, 16), be32(b, 20)) {
        (Some(width), Some(height)) => Ok(ImageInfo {
            format: ImageFormat::Png,
            width,
            height,
        }),
        _ => Err(ImageError::Unreadable("a PNG cut short".into())),
    }
}

/// A JPEG's size: the first frame header (`SOFn`), turned by the EXIF
/// orientation when an `APP1` Exif segment says the picture is shown on
/// its side (orientations 5 to 8).
fn jpeg_info(b: &[u8]) -> Result<ImageInfo, ImageError> {
    let short = || ImageError::Unreadable("a JPEG cut short".into());
    let mut i = 2;
    let mut turned = false;
    loop {
        if *b.get(i).ok_or_else(short)? != 0xff {
            return Err(ImageError::Unreadable(
                "a JPEG with a damaged header".into(),
            ));
        }
        // Any number of fill bytes may precede a marker.
        while b.get(i) == Some(&0xff) {
            i += 1;
        }
        let marker = *b.get(i).ok_or_else(short)?;
        i += 1;
        match marker {
            // Markers that stand alone, with no length.
            0x01 | 0xd0..=0xd8 => continue,
            0xd9 | 0xda => {
                return Err(ImageError::Unreadable("a JPEG with no frame header".into()));
            }
            _ => {}
        }
        let len = usize::from(be16(b, i).ok_or_else(short)?);
        if len < 2 {
            return Err(ImageError::Unreadable(
                "a JPEG with a damaged header".into(),
            ));
        }
        let seg = b.get(i + 2..i + len).ok_or_else(short)?;
        match marker {
            // Every start-of-frame marker: baseline, progressive, lossless,
            // arithmetic (not DHT 0xC4, JPG 0xC8 or DAC 0xCC).
            0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => {
                let (Some(h), Some(w)) = (be16(seg, 1), be16(seg, 3)) else {
                    return Err(short());
                };
                let (width, height) = if turned {
                    (u32::from(h), u32::from(w))
                } else {
                    (u32::from(w), u32::from(h))
                };
                return Ok(ImageInfo {
                    format: ImageFormat::Jpeg,
                    width,
                    height,
                });
            }
            0xe1 if seg.starts_with(b"Exif\0\0") => {
                turned = matches!(exif_orientation(&seg[6..]), Some(5..=8));
            }
            _ => {}
        }
        i += len;
    }
}

/// The orientation tag (0x0112) of an Exif block's first directory, or
/// `None` when it is absent or the block cannot be read.
fn exif_orientation(tiff: &[u8]) -> Option<u16> {
    let le = match tiff.get(0..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    // Every offset is checked: an Exif block is untrusted. Each is formed
    // only after a read below it succeeded, so none can overflow as the
    // walk stands; the checks keep that true should the walk change.
    let u16_at = |at: usize| -> Option<u16> {
        let p = [*tiff.get(at)?, *tiff.get(at.checked_add(1)?)?];
        Some(if le {
            u16::from_le_bytes(p)
        } else {
            u16::from_be_bytes(p)
        })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let p = [
            *tiff.get(at)?,
            *tiff.get(at.checked_add(1)?)?,
            *tiff.get(at.checked_add(2)?)?,
            *tiff.get(at.checked_add(3)?)?,
        ];
        Some(if le {
            u32::from_le_bytes(p)
        } else {
            u32::from_be_bytes(p)
        })
    };
    if u16_at(2)? != 42 {
        return None;
    }
    let ifd = usize::try_from(u32_at(4)?).ok()?;
    let count = usize::from(u16_at(ifd)?);
    // A directory runs past the block's end only in a damaged file: stop
    // there rather than walk a count the block cannot hold.
    let entries = ifd.checked_add(2)?;
    let room = tiff.len().saturating_sub(entries) / 12;
    (0..count.min(room)).find_map(|k| {
        let e = entries.checked_add(12 * k)?;
        // Orientation is a SHORT (type 3) with one value.
        (u16_at(e)? == 0x0112 && u16_at(e.checked_add(2)?)? == 3)
            .then(|| u16_at(e.checked_add(8)?))?
    })
}

/// The name a picture's definition gets from its file name: the base name
/// without a `.png`/`.jpg`/`.jpeg` extension, or [`DEFAULT_NAME`] for none.
pub fn picture_name(file_name: &str) -> String {
    let base = file_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(file_name)
        .trim();
    let lower = base.to_ascii_lowercase();
    let stem = [".png", ".jpg", ".jpeg"]
        .iter()
        .find(|ext| lower.ends_with(*ext))
        .map_or(base, |ext| &base[..base.len() - ext.len()])
        .trim();
    if stem.is_empty() {
        DEFAULT_NAME.to_string()
    } else {
        stem.to_string()
    }
}

/// Where a picture goes: its lower-left corner, the direction its bottom
/// edge runs (`u`) and a direction toward its top (`v`, any vector not
/// parallel to `u`; the picture's up is `v`'s part square to `u`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// The picture's lower-left corner, world meters.
    pub origin: Point3,
    /// Along the picture's bottom edge, left to right.
    pub u: Vec3,
    /// Toward the picture's top.
    pub v: Vec3,
    /// The picture's width along `u`, in meters; its height follows from
    /// the picture's own proportions.
    pub width: f64,
}

impl Placement {
    /// On the ground at the world origin, the picture's up along +Y,
    /// `width` meters wide: `hew.doc.import`'s default.
    pub fn ground(width: f64) -> Placement {
        Placement {
            origin: Point3::new(0.0, 0.0, 0.0),
            u: Vec3::new(1.0, 0.0, 0.0),
            v: Vec3::new(0.0, 1.0, 0.0),
            width,
        }
    }

    /// A placement from optional parts, each defaulting to
    /// [`Placement::ground`]'s (the world origin, +X along the width, +Y
    /// toward the top, 1 m wide): what `hew.doc.import` of a picture takes.
    pub fn from_parts(
        origin: Option<[f64; 3]>,
        u: Option<[f64; 3]>,
        v: Option<[f64; 3]>,
        width: Option<f64>,
    ) -> Placement {
        let g = Placement::ground(width.unwrap_or(1.0));
        let p3 = |a: [f64; 3]| Point3::new(a[0], a[1], a[2]);
        let v3 = |a: [f64; 3]| Vec3::new(a[0], a[1], a[2]);
        Placement {
            origin: origin.map_or(g.origin, p3),
            u: u.map_or(g.u, v3),
            v: v.map_or(g.v, v3),
            width: g.width,
        }
    }

    /// The rigid pose this placement stands for: the picture's local X
    /// along `u`, local Y along `v` made square to `u`, local Z the face's
    /// normal (`u × v`), and its local origin at `origin`.
    ///
    /// # Errors
    /// [`ImageError::BadPlacement`] for a non-finite value, a zero-length
    /// axis or two parallel ones.
    pub fn pose(&self) -> Result<Transform, ImageError> {
        let finite = |v: Vec3| v.x.is_finite() && v.y.is_finite() && v.z.is_finite();
        let o = self.origin;
        if !finite(Vec3::new(o.x, o.y, o.z)) || !finite(self.u) || !finite(self.v) {
            return Err(ImageError::BadPlacement(
                "a value that is not a number".into(),
            ));
        }
        let x = self
            .u
            .normalized()
            .map_err(|_| ImageError::BadPlacement("no direction along the width".into()))?;
        let z = x
            .cross(self.v)
            .normalized()
            .map_err(|_| ImageError::BadPlacement("the two directions are parallel".into()))?;
        let y = z.cross(x);
        Ok(Transform::from_affine(&[
            x.x, y.x, z.x, o.x, //
            x.y, y.y, z.y, o.y, //
            x.z, y.z, z.z, o.z,
        ]))
    }
}

/// A placed picture, ready for `Document::ingest`.
pub struct ImageImport {
    /// The recipe: one material, one definition, one root instance (its
    /// node key is [`INSTANCE_KEY`]).
    pub scene: ImportScene,
    /// What was read from the bytes.
    pub info: ImageInfo,
    /// The face's width, meters (the placement's).
    pub width: f64,
    /// The face's height, meters: `width` times the picture's proportions.
    pub height: f64,
}

/// The node key [`import`] gives the placed instance, so a caller finds it
/// in `ImportReport::keys`.
pub const INSTANCE_KEY: u64 = 1;

/// What an ingested [`ImageImport`] placed: its definition and its one
/// instance, or `None` when the kernel built neither (a face it skipped,
/// see `ImportReport::skipped`). A host [`check`]s the recipe before it
/// ingests, so `None` here means a kernel bug: the host refuses, and says
/// the document may hold part of the import rather than pretend it does
/// not (an undo to take it back would cost the user their redo history).
pub fn placed(report: &kernel::ImportReport) -> Option<(kernel::ComponentId, kernel::InstanceId)> {
    let definition = report.defs.first().copied().flatten()?;
    match report.keys.get(&INSTANCE_KEY).map(Vec::as_slice) {
        Some([kernel::NodeId::Instance(i)]) => Some((definition, *i)),
        _ => None,
    }
}

/// Whether `Document::ingest` would place `scene` whole: every mesh in it
/// built through the kernel's own import path
/// ([`kernel::MeshRecipe::check_builds`]), without touching a document.
/// A host checks a picture's recipe with this BEFORE ingesting it, so a
/// refusal leaves the document, its undo and redo history and its palette
/// exactly as they were. [`import`] makes the same check on the recipe it
/// returns.
///
/// # Errors
/// [`ImageError::Unbuildable`] with the kernel's reason for the first mesh
/// it would skip.
pub fn check(scene: &ImportScene) -> Result<(), ImageError> {
    fn walk(nodes: &[ImportNode]) -> Result<(), ImageError> {
        for node in nodes {
            match node {
                ImportNode::Mesh(mesh) => mesh
                    .check_builds()
                    .map_err(|e| ImageError::Unbuildable(e.to_string()))?,
                ImportNode::Group { children, .. } => walk(children)?,
                ImportNode::Instance { .. } => {}
            }
        }
        Ok(())
    }
    for def in &scene.defs {
        walk(&def.children)?;
    }
    walk(&scene.roots)
}

/// The recipe for `bytes` placed by `placement`, its definition (and
/// material) named `name` (see [`picture_name`]).
///
/// # Errors
/// [`read_info`]'s refusals; [`ImageError::Unreadable`] when `format` is
/// given and the bytes are in the other one; [`ImageError::BadPlacement`]
/// for a placement [`Placement::pose`] refuses or a width that is not a
/// positive finite length; [`ImageError::TooSmall`] when either side of
/// the placed picture would be under [`MIN_SIDE`] and
/// [`ImageError::TooBig`] when either would be over [`MAX_SIDE`];
/// [`ImageError::Unbuildable`] when the kernel would not build its face
/// all the same ([`check`]).
pub fn import(
    bytes: &[u8],
    format: Option<ImageFormat>,
    name: &str,
    placement: &Placement,
) -> Result<ImageImport, ImageError> {
    let info = read_info(bytes)?;
    if format.is_some_and(|f| f != info.format) {
        return Err(ImageError::Unreadable(
            "the bytes are not in the format they were said to be".into(),
        ));
    }
    let width = placement.width;
    if !width.is_finite() || width <= 0.0 {
        return Err(ImageError::BadPlacement(
            "the width is not a positive length".into(),
        ));
    }
    let height = width * f64::from(info.height) / f64::from(info.width);
    // Too big is checked first, and before the finite check on the height:
    // a width so large its height overflows is too big, not malformed.
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(ImageError::TooBig { width, height });
    }
    if !height.is_finite() || height <= 0.0 {
        return Err(ImageError::BadPlacement(
            "the height is not a positive length".into(),
        ));
    }
    if width < MIN_SIDE || height < MIN_SIDE {
        return Err(ImageError::TooSmall { width, height });
    }
    let pose = placement.pose()?;
    let name = name.trim();
    let name = if name.is_empty() { DEFAULT_NAME } else { name };

    let material = Material::textured(
        name,
        Rgba8::rgba(255, 255, 255, 255),
        Texture {
            image: bytes.to_vec(),
            format: info.format,
            world_size: [width, height],
        },
    );
    // One face, counter-clockwise seen from +Z (the side the picture
    // faces), its corners where the texture's corners go.
    let mesh = MeshRecipe {
        name: name.to_string(),
        positions: vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(width, 0.0, 0.0),
            Point3::new(width, height, 0.0),
            Point3::new(0.0, height, 0.0),
        ],
        faces: vec![vec![0, 1, 2, 3]],
        face_materials: vec![0],
        face_uv_frames: vec![Some(UvFrame::new(
            Vec3::new(1.0 / width, 0.0, 0.0),
            Vec3::new(0.0, 1.0 / height, 0.0),
            0.0,
            0.0,
        ))],
        face_holes: vec![Vec::new()],
        base_material: NO_MATERIAL,
        tags: Vec::new(),
        extra: MeshExtra::default(),
    };
    let scene = ImportScene {
        materials: vec![material],
        defs: vec![DefRecipe::from_meshes(Some(name.to_string()), vec![mesh])],
        roots: vec![ImportNode::Instance {
            def: 0,
            pose,
            name: None,
            tags: Vec::new(),
            hidden: false,
            key: Some(INSTANCE_KEY),
        }],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
    };
    check(&scene)?;
    Ok(ImageImport {
        scene,
        info,
        width,
        height,
    })
}

#[cfg(test)]
mod tests;
