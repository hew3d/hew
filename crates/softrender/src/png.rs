//! Minimal PNG encoder (RGBA8, RGB8, gray8, gray16): deflate-compressed at a
//! fixed level (`miniz_oxide`, zlib framing), PNG filter "none", chunk
//! framing and CRC32 hand-rolled. Deterministic for a given encoder
//! version; a spec-correct file every viewer opens.

/// Encodes an RGBA8 buffer (row-major, top row first) as a PNG.
pub fn encode(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    assert_eq!(rgba.len(), (width as usize) * (height as usize) * 4);
    write_png(rgba, width, height, 8, 6, 4)
}

/// Encodes an RGB8 buffer (row-major, top row first) as a PNG.
pub fn encode_rgb(rgb: &[u8], width: u32, height: u32) -> Vec<u8> {
    assert_eq!(rgb.len(), (width as usize) * (height as usize) * 3);
    write_png(rgb, width, height, 8, 2, 3)
}

/// Encodes an 8-bit grayscale buffer as a PNG.
pub fn encode_gray8(gray: &[u8], width: u32, height: u32) -> Vec<u8> {
    assert_eq!(gray.len(), (width as usize) * (height as usize));
    write_png(gray, width, height, 8, 0, 1)
}

/// Encodes a 16-bit grayscale buffer as a PNG (samples big-endian, as the
/// format requires).
pub fn encode_gray16(gray: &[u16], width: u32, height: u32) -> Vec<u8> {
    assert_eq!(gray.len(), (width as usize) * (height as usize));
    let mut bytes = Vec::with_capacity(gray.len() * 2);
    for v in gray {
        bytes.extend_from_slice(&v.to_be_bytes());
    }
    write_png(&bytes, width, height, 16, 0, 2)
}

/// The shared writer: `data` is row-major pixels of `bytes_per_pixel`
/// bytes each, `bit_depth`/`color_type` as in the PNG spec's IHDR.
fn write_png(
    data: &[u8],
    width: u32,
    height: u32,
    bit_depth: u8,
    color_type: u8,
    bytes_per_pixel: usize,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 1024);
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[bit_depth, color_type, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);

    // Raw scanlines, filter byte 0 per row.
    let stride = (width as usize) * bytes_per_pixel;
    let mut raw = Vec::with_capacity(data.len() + height as usize);
    for row in data.chunks_exact(stride.max(1)) {
        raw.push(0);
        raw.extend_from_slice(row);
    }

    // zlib stream at deflate level 6 (the encoder's default).
    let idat = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tiny_png_is_well_formed() {
        let rgba = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ];
        let png = encode(&rgba, 2, 2);
        assert_eq!(
            &png[0..8],
            &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]
        );
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[16..20], 2u32.to_be_bytes());
        assert_eq!(&png[20..24], 2u32.to_be_bytes());
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
        // CRC of a known IEND chunk (empty data) is a published constant.
        assert_eq!(&png[png.len() - 4..], &0xae42_6082_u32.to_be_bytes());
    }

    #[test]
    fn gray_and_rgb_pngs_declare_their_color_type_and_depth() {
        let g8 = encode_gray8(&[0, 255, 7, 9], 2, 2);
        assert_eq!(&g8[24..26], &[8, 0], "8-bit grayscale");
        let g16 = encode_gray16(&[0, 65535, 7, 9], 2, 2);
        assert_eq!(&g16[24..26], &[16, 0], "16-bit grayscale");
        let rgb = encode_rgb(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], 2, 2);
        assert_eq!(&rgb[24..26], &[8, 2], "8-bit truecolor");
        for png in [&g8, &g16, &rgb] {
            assert_eq!(&png[png.len() - 4..], &0xae42_6082_u32.to_be_bytes());
        }
    }

    #[test]
    fn known_crc_vector() {
        // CRC-32 of "123456789" is 0xcbf43926 (the classic check value).
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    /// The IDAT payload of `png` (the single data chunk the writer emits).
    fn idat_of(png: &[u8]) -> &[u8] {
        let at = png.windows(4).position(|w| w == b"IDAT").expect("IDAT");
        let len = u32::from_be_bytes(png[at - 4..at].try_into().unwrap()) as usize;
        &png[at + 4..at + 4 + len]
    }

    #[test]
    fn a_flat_128_square_compresses_below_4_kib() {
        let rgba: Vec<u8> = [30u8, 60, 90, 255].repeat(128 * 128);
        let png = encode(&rgba, 128, 128);
        assert!(
            png.len() < 4096,
            "flat 128 x 128 PNG is {} bytes",
            png.len()
        );
    }

    #[test]
    fn the_idat_stream_inflates_to_the_scanlines() {
        let (w, h) = (37usize, 11usize);
        let rgba: Vec<u8> = (0..w * h * 4).map(|i| (i * 7 % 251) as u8).collect();
        let png = encode(&rgba, w as u32, h as u32);
        let raw = miniz_oxide::inflate::decompress_to_vec_zlib(idat_of(&png)).expect("inflates");
        let mut expected = Vec::new();
        for row in rgba.chunks_exact(w * 4) {
            expected.push(0);
            expected.extend_from_slice(row);
        }
        assert_eq!(raw, expected);
    }

    #[test]
    fn the_encoder_is_deterministic() {
        let rgba: Vec<u8> = (0..64 * 64 * 4).map(|i| (i * 13 % 255) as u8).collect();
        assert_eq!(encode(&rgba, 64, 64), encode(&rgba, 64, 64));
    }
}
