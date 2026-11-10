//! A texture's average color, from its image: what SketchUp draws a
//! textured material in with textures off (its `Shaded` style), and keeps
//! beside the image in the material record.

use std::io::Cursor;

/// The mean color of a PNG or JPEG image, opaque; `None` when the bytes
/// do not decode.
pub(crate) fn average_rgba(bytes: &[u8], format: kernel::ImageFormat) -> Option<[u8; 4]> {
    let (pixels, channels) = match format {
        kernel::ImageFormat::Png => png_rgb(bytes)?,
        kernel::ImageFormat::Jpeg => jpeg_rgb(bytes)?,
    };
    average(&pixels, channels)
}

/// Mean of each color channel over every pixel of an interleaved buffer
/// of `channels` per pixel (1 gray, 2 gray + alpha, 3 RGB, 4 RGBA; alpha
/// is not part of the color).
fn average(pixels: &[u8], channels: usize) -> Option<[u8; 4]> {
    if channels == 0 || pixels.len() < channels {
        return None;
    }
    let mut sum = [0u64; 3];
    let n = pixels.len() / channels;
    for px in pixels.chunks_exact(channels) {
        let rgb = match channels {
            1 | 2 => [px[0]; 3],
            _ => [px[0], px[1], px[2]],
        };
        for (s, c) in sum.iter_mut().zip(rgb) {
            *s += u64::from(c);
        }
    }
    let mean = |s: u64| ((s + n as u64 / 2) / n as u64) as u8;
    Some([mean(sum[0]), mean(sum[1]), mean(sum[2]), 255])
}

/// The most pixels an image may claim before it is decoded: a header can
/// claim far more than its bytes hold, and a buffer that size would not
/// fit (a 32-bit browser kernel aborts on it). Larger textures keep the
/// tint for their average.
const MAX_PIXELS: u64 = 64 << 20;

fn png_rgb(bytes: &[u8]) -> Option<(Vec<u8>, usize)> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let info = reader.info();
    if u64::from(info.width) * u64::from(info.height) > MAX_PIXELS {
        return None;
    }
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    Some((buf, info.color_type.samples()))
}

fn jpeg_rgb(bytes: &[u8]) -> Option<(Vec<u8>, usize)> {
    let mut decoder = jpeg_decoder::Decoder::new(bytes);
    decoder.read_info().ok()?;
    let info = decoder.info()?;
    if u64::from(info.width) * u64::from(info.height) > MAX_PIXELS {
        return None;
    }
    let pixels = decoder.decode().ok()?;
    let channels = match decoder.info()?.pixel_format {
        jpeg_decoder::PixelFormat::L8 => 1,
        jpeg_decoder::PixelFormat::RGB24 => 3,
        // CMYK and 16-bit gray are rare in textures: no color for them.
        _ => return None,
    };
    Some((pixels, channels))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_textures_average_is_sketchups() {
        // house.skp's textured materials, with the average color SketchUp
        // itself stored beside each image.
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../skp-import/tests/fixtures/house.skp");
        let m = skpwright::Model::read(&p).unwrap();
        let mut checked = 0;
        for mat in &m.materials {
            let skpwright::Material::Textured {
                name,
                image_bytes: Some(bytes),
                avg_rgba: Some(want),
                ..
            } = mat
            else {
                continue;
            };
            let format = if bytes.starts_with(b"\x89PNG") {
                kernel::ImageFormat::Png
            } else {
                kernel::ImageFormat::Jpeg
            };
            let got = average_rgba(bytes, format).expect("decodes");
            for c in 0..3 {
                assert!(
                    (i32::from(got[c]) - i32::from(want[c])).abs() <= 2,
                    "{name}: {got:?} vs SketchUp's {want:?}"
                );
            }
            checked += 1;
        }
        assert!(checked >= 4, "{checked}");
    }

    #[test]
    fn an_image_claiming_too_many_pixels_is_not_decoded() {
        // A header alone, claiming 30000 × 30000 RGBA: no 3.6 GB buffer.
        let mut png = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png, 30_000, 30_000);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap();
        }
        assert_eq!(average_rgba(&png, kernel::ImageFormat::Png), None);
    }

    #[test]
    fn undecodable_bytes_have_no_average() {
        assert_eq!(
            average_rgba(b"not an image", kernel::ImageFormat::Png),
            None
        );
        assert_eq!(
            average_rgba(b"not an image", kernel::ImageFormat::Jpeg),
            None
        );
    }
}
