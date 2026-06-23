//! glTF material → kernel [`Material`] translation.
//!
//! Maps each glTF material's `pbrMetallicRoughness.baseColorFactor` to a solid
//! color and, when a `baseColorTexture` is present with an *embedded* image
//! (GLB buffer view or `data:` URI), attaches a kernel [`Texture`] carrying the
//! image's **encoded** bytes (PNG/JPEG) — the pixel data is never decoded here.
//!
//! The resulting `materials` vec is dense and parallel to `document.materials()`,
//! so a primitive's `material().index()` is also its kernel material index.

use std::collections::HashMap;

use gltf::Gltf;
use gltf::image::Source as ImageSource;
use kernel::{ImageFormat, Material, Rgba8, Texture};

use crate::buffers::decode_data_uri;

/// Built materials plus any image URIs that could not be resolved in-memory.
pub struct MaterialTable {
    /// Deduplicated materials.
    pub materials: Vec<Material>,
    /// Maps each glTF material index → dense index into `materials`.
    pub remap: Vec<u32>,
    /// External image URIs / unsupported codecs that were skipped.
    pub missing: Vec<String>,
}

/// Build the deduplicated material table from a parsed glTF document.
///
/// SketchUp→glTF (and many exporters) emit a separate material per object even
/// when they share the same color + texture image — here, 748 materials over
/// just 17 images. Resolving a texture per material copies the (often large)
/// encoded image bytes once *per material*, ballooning a 21 MB file to ~1.5 GB
/// resident. We dedup by (color, image): each distinct (color, image) pair
/// becomes one kernel material (image resolved once), and `remap` rewrites each
/// glTF material index onto it.
pub fn build(gltf: &Gltf, buffers: &[Option<Vec<u8>>]) -> MaterialTable {
    let mut materials: Vec<Material> = Vec::new();
    let mut missing = Vec::new();
    let mut remap: Vec<u32> = Vec::new();
    let mut seen: HashMap<(u32, Option<usize>), u32> = HashMap::new();

    for (i, mat) in gltf.document.materials().enumerate() {
        let pbr = mat.pbr_metallic_roughness();
        let color = factor_to_rgba8(pbr.base_color_factor());
        let img_idx = pbr
            .base_color_texture()
            .map(|info| info.texture().source().index());

        let key = (pack_rgba(color), img_idx);
        if let Some(&dense) = seen.get(&key) {
            remap.push(dense);
            continue;
        }

        let name = mat
            .name()
            .map(str::to_string)
            .unwrap_or_else(|| format!("material_{i}"));
        let texture = pbr.base_color_texture().and_then(|info| {
            let image = info.texture().source();
            match resolve_image(image.source(), buffers) {
                Some((bytes, format)) => Some(Texture {
                    image: bytes,
                    format,
                    world_size: [1.0, 1.0],
                }),
                None => {
                    if let ImageSource::Uri { uri, .. } = image.source() {
                        missing.push(uri.to_string());
                    }
                    None
                }
            }
        });

        let dense = materials.len() as u32;
        materials.push(match texture {
            Some(tex) => Material::textured(name, color, tex),
            None => Material::solid(name, color),
        });
        seen.insert(key, dense);
        remap.push(dense);
    }

    MaterialTable {
        materials,
        remap,
        missing,
    }
}

/// Pack an `Rgba8` into a `u32` for use as a hash key.
fn pack_rgba(c: Rgba8) -> u32 {
    (c.r as u32) << 24 | (c.g as u32) << 16 | (c.b as u32) << 8 | c.a as u32
}

/// Resolve an image source to `(encoded bytes, format)` if it is embedded and a
/// supported codec; otherwise `None`.
fn resolve_image(
    source: ImageSource,
    buffers: &[Option<Vec<u8>>],
) -> Option<(Vec<u8>, ImageFormat)> {
    match source {
        ImageSource::View { view, mime_type } => {
            let format = mime_to_format(mime_type)?;
            let buf = buffers.get(view.buffer().index())?.as_ref()?;
            let start = view.offset();
            let end = start.checked_add(view.length())?;
            let bytes = buf.get(start..end)?.to_vec();
            Some((bytes, format))
        }
        ImageSource::Uri { uri, mime_type } => {
            // A `data:` URI may declare its own mime in the header; prefer the
            // glTF-declared `mime_type` when present, else sniff the URI header.
            let format = mime_type
                .and_then(mime_to_format)
                .or_else(|| uri_mime(uri).and_then(mime_to_format))?;
            let bytes = decode_data_uri(uri)?;
            Some((bytes, format))
        }
    }
}

fn mime_to_format(mime: &str) -> Option<ImageFormat> {
    match mime {
        "image/png" => Some(ImageFormat::Png),
        "image/jpeg" => Some(ImageFormat::Jpeg),
        _ => None,
    }
}

/// Extract the mime from a `data:<mime>;base64,...` URI header.
fn uri_mime(uri: &str) -> Option<&str> {
    let rest = uri.strip_prefix("data:")?;
    let end = rest.find([';', ','])?;
    Some(&rest[..end])
}

/// Convert a linear `[r,g,b,a]` factor in `[0,1]` to 8-bit channels.
fn factor_to_rgba8(f: [f32; 4]) -> Rgba8 {
    let c = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgba8::rgba(c(f[0]), c(f[1]), c(f[2]), c(f[3]))
}
