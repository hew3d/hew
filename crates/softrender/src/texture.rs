//! Image textures for the headless renderer (KB-37 / issue #61).
//!
//! A textured material's image is decoded once per render, then sampled by
//! UV with a fixed, deterministic filter: nearest texel, repeating, with
//! `v = 0` at the image's BOTTOM edge (the viewport's `TextureLoader` flips
//! the same way). The material color modulates the texel, as the kernel's
//! `Material` doc says. Transparent texels are composited over white.
//! An image that will not decode is left out of the set, so its faces keep
//! the base color, and its material name is listed in
//! [`TextureSet::undecodable`].

use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::Arc;

use kernel::{ImageFormat, MaterialId};

/// The most pixels an image may claim before it is decoded: a header can
/// claim far more than its bytes hold.
const MAX_PIXELS: u64 = 64 << 20;

/// A decoded image: 8-bit RGB, row-major, top row first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

impl Image {
    /// Decodes a PNG or JPEG; `None` when the bytes do not decode (or claim
    /// an absurd size, or are an unsupported JPEG color model).
    pub fn decode(bytes: &[u8], format: ImageFormat) -> Option<Image> {
        let (width, height, rgb) = match format {
            ImageFormat::Png => decode_png(bytes)?,
            ImageFormat::Jpeg => decode_jpeg(bytes)?,
        };
        if width == 0 || height == 0 || rgb.len() != (width as usize) * (height as usize) * 3 {
            return None;
        }
        Some(Image { width, height, rgb })
    }

    /// Image size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The texel under `(u, v)`, as 0..=1 RGB.
    pub fn sample(&self, u: f32, v: f32) -> [f32; 3] {
        let wrap = |t: f32| {
            let f = t - t.floor();
            if f.is_finite() { f } else { 0.0 }
        };
        let x = ((wrap(u) * self.width as f32) as u32).min(self.width - 1);
        let y = (((1.0 - wrap(v)) * self.height as f32) as u32).min(self.height - 1);
        let i = ((y as usize) * (self.width as usize) + x as usize) * 3;
        [
            self.rgb[i] as f32 / 255.0,
            self.rgb[i + 1] as f32 / 255.0,
            self.rgb[i + 2] as f32 / 255.0,
        ]
    }
}

fn decode_png(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut decoder = ::png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(::png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let (w, h) = {
        let info = reader.info();
        (info.width, info.height)
    };
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return None;
    }
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let over_white = |c: u8, a: u8| {
        ((u16::from(c) * u16::from(a) + 255 * (255 - u16::from(a)) + 127) / 255) as u8
    };
    let mut rgb = Vec::with_capacity((w as usize) * (h as usize) * 3);
    match info.color_type {
        ::png::ColorType::Rgb => rgb = buf,
        ::png::ColorType::Rgba => {
            for p in buf.as_chunks::<4>().0 {
                rgb.extend([
                    over_white(p[0], p[3]),
                    over_white(p[1], p[3]),
                    over_white(p[2], p[3]),
                ]);
            }
        }
        ::png::ColorType::Grayscale => {
            for &g in &buf {
                rgb.extend([g, g, g]);
            }
        }
        ::png::ColorType::GrayscaleAlpha => {
            for p in buf.as_chunks::<2>().0 {
                let g = over_white(p[0], p[1]);
                rgb.extend([g, g, g]);
            }
        }
        // normalize_to_color8 expands palettes; nothing else is left.
        ::png::ColorType::Indexed => return None,
    }
    Some((w, h, rgb))
}

fn decode_jpeg(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut decoder = jpeg_decoder::Decoder::new(bytes);
    decoder.read_info().ok()?;
    let info = decoder.info()?;
    let (w, h) = (u32::from(info.width), u32::from(info.height));
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return None;
    }
    let pixels = decoder.decode().ok()?;
    let rgb = match decoder.info()?.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => pixels,
        jpeg_decoder::PixelFormat::L8 => pixels.iter().flat_map(|&g| [g, g, g]).collect(),
        // CMYK and 16-bit gray are rare in textures: keep the base color.
        _ => return None,
    };
    Some((w, h, rgb))
}

/// The decoded textures a render samples, by material. Built once per
/// document ([`TextureSet::for_materials`]) and shared by every item.
#[derive(Debug, Default, Clone)]
pub struct TextureSet {
    by_material: BTreeMap<MaterialId, Arc<Image>>,
    /// Names of textured materials whose image would not decode; their
    /// faces render in the base color.
    pub undecodable: Vec<String>,
}

impl TextureSet {
    /// Decodes the texture of each listed material that has one.
    pub fn for_materials(
        palette: &kernel::MaterialPalette,
        used: impl IntoIterator<Item = MaterialId>,
    ) -> TextureSet {
        let mut set = TextureSet::default();
        for id in used {
            if set.by_material.contains_key(&id) {
                continue;
            }
            let Some(material) = palette.get(id) else {
                continue;
            };
            let Some(texture) = &material.texture else {
                continue;
            };
            match Image::decode(&texture.image, texture.format) {
                Some(image) => {
                    set.by_material.insert(id, Arc::new(image));
                }
                None => {
                    if !set.undecodable.contains(&material.name) {
                        set.undecodable.push(material.name.clone());
                    }
                }
            }
        }
        set
    }

    /// The decoded image for a material, if it has one.
    pub fn get(&self, id: MaterialId) -> Option<&Image> {
        self.by_material.get(&id).map(|a| &**a)
    }

    /// Whether no material has a decoded image.
    pub fn is_empty(&self) -> bool {
        self.by_material.is_empty()
    }
}
