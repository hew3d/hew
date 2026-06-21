//! COLLADA material → Hew `Material` translation (contract).
//!
//! Walks `<library_effects>` → `<profile_COMMON>` → Phong/Lambert/Blinn/Constant
//! shader to extract the diffuse color (or texture) and the transparency channel
//! (`<transparent>` + `<transparency>`, A_ONE), folding the latter into the
//! material's alpha so glass imports semi-transparent. SketchUp's `<constant>`
//! glass carries its color in `<transparent>`, so that becomes the base color
//! when there is no diffuse. Unresolved texture URIs fall back to a neutral
//! color and append the URI to `textures_missing`.

use std::collections::HashMap;

use dae_parser::{Document as DaeDocument, Image, ParseLibrary};
use kernel::{Material, Rgba8, Texture};

use crate::ImageMap;
use crate::meta::decode_meta;

// ── Url helper ────────────────────────────────────────────────────────────────

/// Extract the plain string from a `dae_parser::Url`.
/// Fragment URLs (`#foo`) return `"foo"`; non-fragment URLs return as-is.
fn url_as_str(url: &dae_parser::Url) -> &str {
    match url {
        dae_parser::Url::Fragment(s) => s.as_str(),
        dae_parser::Url::Other(s) => s.as_str(),
    }
}

// ── Percent-decode helper ──────────────────────────────────────────────────────

/// Decode `%XX` percent-encoded URI sequences to their UTF-8 equivalents.
///
/// Rules:
/// - `%XX` (uppercase or lowercase hex digits) → decoded byte.
/// - `+` is NOT a space in path URIs; it is left as-is.
/// - Malformed or incomplete escapes (`%` not followed by two hex digits) are
///   left verbatim (the literal `%` and following chars are passed through).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_val(bytes[i + 1]);
            let lo = hex_val(bytes[i + 2]);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    // Interpret decoded bytes as UTF-8; fall back to lossy if not valid.
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Return the final path segment of `s` (the part after the last `/`).
/// If there is no `/`, returns `s` itself.
fn uri_basename(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}

// ── Public result type ────────────────────────────────────────────────────────

/// Result of resolving all materials in a COLLADA document.
pub struct MaterialTable {
    /// The kernel materials, in declaration order.
    pub materials: Vec<Material>,
    /// Map from COLLADA material id (string) → dense index into `materials`.
    pub id_to_dense: HashMap<String, u32>,
    /// Image URIs that could not be resolved from the `ImageMap`.
    pub textures_missing: Vec<String>,
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Build the `MaterialTable` from a parsed COLLADA document.
pub fn build_material_table(doc: &DaeDocument, images: &ImageMap) -> MaterialTable {
    let mut materials: Vec<Material> = Vec::new();
    let mut id_to_dense: HashMap<String, u32> = HashMap::new();
    let mut textures_missing: Vec<String> = Vec::new();

    // Build a map: effect_id → ProfileCommon, by walking library.
    let profile_map = collect_profiles(doc);
    // Build a map: image_id → Image, by walking library.
    let image_map = collect_images(doc);

    for lib_elem in &doc.library {
        let Some(lib) = dae_parser::Material::extract_element(lib_elem) else {
            continue;
        };
        for dae_mat in &lib.items {
            let mat_id = match &dae_mat.id {
                Some(id) => id.clone(),
                None => continue,
            };
            // Decode HEWMETA/HEWTAG payload to recover the real material name.
            // Materials have no tags — meta.tags is intentionally ignored here.
            let mat_name = dae_mat
                .name
                .as_deref()
                .and_then(|n| decode_meta(n).name)
                .unwrap_or_else(|| mat_id.clone());

            // Follow material → instance_effect → effect.
            // url_as_str already strips the '#' from Fragment URLs.
            let effect_id = url_as_str(&dae_mat.instance_effect.url);
            let kernel_mat = if let Some(profile) = profile_map.get(effect_id) {
                resolve_profile(
                    profile,
                    &image_map,
                    images,
                    &mat_name,
                    &mut textures_missing,
                )
            } else {
                Material::solid(&mat_name, Rgba8::rgb(200, 200, 200))
            };

            let dense_idx = materials.len() as u32;
            materials.push(kernel_mat);
            id_to_dense.insert(mat_id, dense_idx);
        }
    }

    MaterialTable {
        materials,
        id_to_dense,
        textures_missing,
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Collect all `ProfileCommon`s from the document, keyed by their parent
/// effect's id.
fn collect_profiles(doc: &DaeDocument) -> HashMap<String, dae_parser::ProfileCommon> {
    let mut map = HashMap::new();
    for lib_elem in &doc.library {
        let Some(lib) = dae_parser::Effect::extract_element(lib_elem) else {
            continue;
        };
        for effect in &lib.items {
            let effect_id = &effect.id;
            // Find the profile_COMMON among the effect's profiles.
            for profile in &effect.profile {
                if let Some(common) = profile.as_common() {
                    map.insert(effect_id.clone(), common.clone());
                    break;
                }
            }
        }
    }
    map
}

/// Collect all `Image`s from the document, keyed by id.
fn collect_images(doc: &DaeDocument) -> HashMap<String, Image> {
    let mut map = HashMap::new();
    for lib_elem in &doc.library {
        let Some(lib) = dae_parser::Image::extract_element(lib_elem) else {
            continue;
        };
        for img in &lib.items {
            if let Some(id) = &img.id {
                map.insert(id.clone(), img.clone());
            }
        }
    }
    map
}

/// Resolve a `ProfileCommon`'s shader to a kernel `Material`.
fn resolve_profile(
    profile: &dae_parser::ProfileCommon,
    image_map: &HashMap<String, Image>,
    images: &ImageMap,
    name: &str,
    textures_missing: &mut Vec<String>,
) -> Material {
    // Shaders live in profile.technique.data.shaders.
    let shader = match profile.technique.data.shaders.first() {
        Some(s) => s,
        None => return Material::solid(name, Rgba8::rgb(200, 200, 200)),
    };

    // Pull the diffuse color/texture and the transparency channel from whichever
    // fixed-function shader this is. SketchUp emits `<constant>` (no diffuse) for
    // colored-glass and unlit surfaces, carrying the color in `<transparent>`.
    // WithSid<…> Derefs to its inner ColorParam/FloatParam.
    let (diffuse, transparent, transparency) = match shader {
        dae_parser::Shader::Phong(p) => (
            p.diffuse.as_deref(),
            p.transparent.as_deref(),
            p.transparency.as_deref(),
        ),
        dae_parser::Shader::Lambert(l) => (
            l.diffuse.as_deref(),
            l.transparent.as_deref(),
            l.transparency.as_deref(),
        ),
        dae_parser::Shader::Blinn(b) => (
            b.diffuse.as_deref(),
            b.transparent.as_deref(),
            b.transparency.as_deref(),
        ),
        dae_parser::Shader::Constant(c) => {
            (None, c.transparent.as_deref(), c.transparency.as_deref())
        }
    };

    // Opacity (0..1). COLLADA's transparency channel is authoritative when
    // present; otherwise the diffuse color's own alpha is used. We assume the
    // `A_ONE` mode (opacity = transparent.alpha · transparency) — modern
    // SketchUp's default, and a safe fallback for its legacy `RGB_ZERO` glass
    // (which uses a gray ramp where alpha ≈ 1 − luminance). The `opaque`
    // attribute itself isn't surfaced by the parser, so we can't branch on it.
    let alpha_byte = opacity_alpha(diffuse, transparent, transparency);

    match diffuse {
        // No diffuse channel (typically `<constant>`): take the color from the
        // `<transparent>` channel if it's a literal color (SketchUp's glass),
        // else from a transparent texture, else a neutral gray.
        None => match transparent {
            Some(dae_parser::ColorParam::Color(rgba)) => {
                Material::solid(name, with_alpha(float4_to_rgba8(rgba), alpha_byte))
            }
            Some(dae_parser::ColorParam::Texture(tex_ref)) => with_texture_alpha(
                resolve_texture(
                    &tex_ref.texture,
                    profile,
                    image_map,
                    images,
                    name,
                    textures_missing,
                ),
                alpha_byte,
            ),
            _ => Material::solid(name, with_alpha(Rgba8::rgb(200, 200, 200), alpha_byte)),
        },
        Some(dae_parser::ColorParam::Color(rgba)) => {
            Material::solid(name, with_alpha(float4_to_rgba8(rgba), alpha_byte))
        }
        Some(dae_parser::ColorParam::Texture(tex_ref)) => with_texture_alpha(
            resolve_texture(
                &tex_ref.texture,
                profile,
                image_map,
                images,
                name,
                textures_missing,
            ),
            alpha_byte,
        ),
        Some(dae_parser::ColorParam::Param(_)) => {
            Material::solid(name, with_alpha(Rgba8::rgb(200, 200, 200), alpha_byte))
        }
    }
}

/// Literal scalar value of a `FloatParam`, or `None` for a `<param>` reference.
fn float_param_value(fp: &dae_parser::FloatParam) -> Option<f32> {
    match fp {
        dae_parser::FloatParam::Float(v) => Some(*v),
        dae_parser::FloatParam::Param(_) => None,
    }
}

/// Compute the 0–255 opacity byte from the COLLADA transparency channel,
/// falling back to the diffuse color's own alpha (then fully opaque).
fn opacity_alpha(
    diffuse: Option<&dae_parser::ColorParam>,
    transparent: Option<&dae_parser::ColorParam>,
    transparency: Option<&dae_parser::FloatParam>,
) -> u8 {
    let t = transparency
        .and_then(float_param_value)
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    let opacity = match transparent {
        // A_ONE: opacity = transparent.alpha · transparency.
        Some(dae_parser::ColorParam::Color(rgba)) => rgba[3] * t,
        // A transparent texture: treat as fully opaque tint scaled by the float.
        Some(_) => t,
        // No transparency channel: use the diffuse color's own alpha.
        None => match diffuse {
            Some(dae_parser::ColorParam::Color(rgba)) => rgba[3],
            _ => 1.0,
        },
    };
    (opacity.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Replace a color's alpha channel.
fn with_alpha(c: Rgba8, a: u8) -> Rgba8 {
    Rgba8::rgba(c.r, c.g, c.b, a)
}

/// Apply an alpha byte to a (possibly textured) material's modulation color so
/// glass textures still render semi-transparent.
fn with_texture_alpha(mut mat: Material, a: u8) -> Material {
    mat.color = with_alpha(mat.color, a);
    mat
}

/// Resolve sampler SID → Surface → Image → URI → kernel Material.
fn resolve_texture(
    sampler_sid: &str,
    profile: &dae_parser::ProfileCommon,
    image_map: &HashMap<String, Image>,
    images: &ImageMap,
    name: &str,
    textures_missing: &mut Vec<String>,
) -> Material {
    // Step 1: sampler SID → Sampler2D via new_param.
    let sampler = find_sampler(sampler_sid, profile);
    let surface_sid = match sampler {
        Some(s) => s.source.as_str().to_string(),
        None => {
            maybe_missing(sampler_sid, textures_missing);
            return Material::solid(name, Rgba8::rgb(200, 200, 200));
        }
    };

    // Step 2: surface SID → Surface via new_param.
    let surface = find_surface(&surface_sid, profile);
    let image_name = match surface {
        Some(surf) => match &surf.init {
            dae_parser::SurfaceInit::From { image, .. } => image.as_str().to_string(),
            _ => {
                maybe_missing(&surface_sid, textures_missing);
                return Material::solid(name, Rgba8::rgb(200, 200, 200));
            }
        },
        None => {
            // Could be a direct image reference (some exporters skip the surface step).
            surface_sid.clone()
        }
    };

    // Step 3: image name → Image → URI.
    let uri = match image_map.get(&image_name) {
        Some(img) => match &img.source {
            dae_parser::ImageSource::InitFrom(url) => url_as_str(url).to_string(),
            _ => {
                maybe_missing(&image_name, textures_missing);
                return Material::solid(name, Rgba8::rgb(200, 200, 200));
            }
        },
        None => {
            // image_name might itself be a URI (direct reference, no library_images entry).
            // Decode percent-encoding so direct image references also match real paths.
            percent_decode(&image_name)
        }
    };

    // Step 4: URI → bytes from ImageMap.
    // SketchUp (and other exporters) percent-encode path URIs, but the host
    // keys ImageMap by real filesystem names (real spaces, etc.).  Try four
    // candidates in priority order so that both encoded and plain URIs match:
    //   1. decoded full URI   (e.g. "Guest House Kitchen/wood.jpg")
    //   2. raw full URI       (back-compat with hosts that pre-decode)
    //   3. decoded basename   (e.g. "wood.jpg")  — for flat texture directories
    //   4. raw basename
    let decoded_uri = percent_decode(&uri);
    let raw_basename = uri_basename(&uri);
    let decoded_basename = percent_decode(raw_basename);

    let hit = images
        .get(decoded_uri.as_str())
        .or_else(|| images.get(uri.as_str()))
        .or_else(|| images.get(decoded_basename.as_str()))
        .or_else(|| images.get(raw_basename));

    if let Some((bytes, format)) = hit {
        let texture = Texture {
            image: bytes.clone(),
            format: *format,
            world_size: [1.0, 1.0],
        };
        Material::textured(name, Rgba8::rgb(255, 255, 255), texture)
    } else {
        // Report the decoded URI so the user sees a readable path.
        if !textures_missing.contains(&decoded_uri) {
            textures_missing.push(decoded_uri);
        }
        Material::solid(name, Rgba8::rgb(200, 200, 200))
    }
}

fn find_sampler<'a>(
    sid: &str,
    profile: &'a dae_parser::ProfileCommon,
) -> Option<&'a dae_parser::Sampler2D> {
    // Search in profile.new_param first, then profile.technique.data.image_param.
    for np in &profile.new_param {
        if np.sid == sid {
            return np.ty.as_sampler2d();
        }
    }
    for ip in &profile.technique.data.image_param {
        if let dae_parser::ImageParam::NewParam(np) = ip
            && np.sid == sid
        {
            return np.ty.as_sampler2d();
        }
    }
    None
}

fn find_surface<'a>(
    sid: &str,
    profile: &'a dae_parser::ProfileCommon,
) -> Option<&'a dae_parser::Surface> {
    for np in &profile.new_param {
        if np.sid == sid {
            return np.ty.as_surface();
        }
    }
    for ip in &profile.technique.data.image_param {
        if let dae_parser::ImageParam::NewParam(np) = ip
            && np.sid == sid
        {
            return np.ty.as_surface();
        }
    }
    None
}

fn maybe_missing(uri: &str, textures_missing: &mut Vec<String>) {
    if !textures_missing.contains(&uri.to_string()) {
        textures_missing.push(uri.to_string());
    }
}

fn float4_to_rgba8(rgba: &[f32; 4]) -> Rgba8 {
    Rgba8::rgba(
        (rgba[0] * 255.0).clamp(0.0, 255.0) as u8,
        (rgba[1] * 255.0).clamp(0.0, 255.0) as u8,
        (rgba[2] * 255.0).clamp(0.0, 255.0) as u8,
        (rgba[3] * 255.0).clamp(0.0, 255.0) as u8,
    )
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::{percent_decode, uri_basename};
    use crate::ImageMap;
    use kernel::ImageFormat;

    fn dummy_image() -> (Vec<u8>, ImageFormat) {
        (vec![0u8; 4], ImageFormat::Png)
    }

    #[test]
    fn percent_decode_spaces() {
        assert_eq!(
            percent_decode("Guest%20House%20Kitchen%20Countertops/__Wood_Floor_Light_1.jpg"),
            "Guest House Kitchen Countertops/__Wood_Floor_Light_1.jpg"
        );
    }

    #[test]
    fn percent_decode_plain_passthrough() {
        assert_eq!(percent_decode("wood.png"), "wood.png");
    }

    #[test]
    fn percent_decode_malformed_escape_literal() {
        // `%` at end of string (incomplete — only 0 chars follow) must be left verbatim.
        assert_eq!(percent_decode("foo%"), "foo%");
        // `%G` followed by one char (incomplete — only 1 char follows) must be left verbatim.
        assert_eq!(percent_decode("foo%GG"), "foo%GG");
        // `%ZZ` is invalid hex — must be passed through verbatim.
        assert_eq!(percent_decode("foo%ZZbar"), "foo%ZZbar");
    }

    #[test]
    fn percent_decode_plus_not_space() {
        // `+` is NOT a space in path URIs.
        assert_eq!(percent_decode("a+b"), "a+b");
    }

    #[test]
    fn uri_basename_extracts_last_segment() {
        assert_eq!(uri_basename("Guest%20House/textures/wood.png"), "wood.png");
        assert_eq!(uri_basename("wood.png"), "wood.png");
    }

    /// ImageMap keyed by real path (spaces) must resolve from a percent-encoded URI.
    #[test]
    fn imagemap_lookup_encoded_full_uri() {
        let mut map: ImageMap = ImageMap::new();
        map.insert(
            "Guest House Kitchen Countertops/__Wood_Floor_Light_1.jpg".to_string(),
            dummy_image(),
        );

        let encoded_uri = "Guest%20House%20Kitchen%20Countertops/__Wood_Floor_Light_1.jpg";

        // Simulate the four-step lookup from resolve_texture.
        use super::{percent_decode, uri_basename};
        let decoded = percent_decode(encoded_uri);
        let raw_basename = uri_basename(encoded_uri);
        let decoded_basename = percent_decode(raw_basename);

        let hit = map
            .get(decoded.as_str())
            .or_else(|| map.get(encoded_uri))
            .or_else(|| map.get(decoded_basename.as_str()))
            .or_else(|| map.get(raw_basename));

        assert!(
            hit.is_some(),
            "encoded URI should resolve against a plain-path ImageMap key"
        );
    }

    /// Plain `wood.png` key must still resolve from a plain URI (back-compat).
    #[test]
    fn imagemap_lookup_plain_uri() {
        let mut map: ImageMap = ImageMap::new();
        map.insert("wood.png".to_string(), dummy_image());

        let uri = "wood.png";
        let decoded = percent_decode(uri);
        let raw_basename = uri_basename(uri);
        let decoded_basename = percent_decode(raw_basename);

        let hit = map
            .get(decoded.as_str())
            .or_else(|| map.get(uri))
            .or_else(|| map.get(decoded_basename.as_str()))
            .or_else(|| map.get(raw_basename));

        assert!(
            hit.is_some(),
            "plain URI should resolve from a plain-path ImageMap key"
        );
    }
}
