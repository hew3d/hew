//! Minimal PNG encoder: RGBA8, stored (uncompressed) zlib deflate
//! blocks, CRC32 + Adler32 hand-rolled — deterministic bytes, zero
//! dependencies. The point is not compression (snapshots travel base64
//! over a local pipe); it is a spec-correct file every viewer opens.

/// Encodes an RGBA8 buffer (row-major, top row first) as a PNG.
pub fn encode(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    assert_eq!(rgba.len(), (width as usize) * (height as usize) * 4);
    let mut out = Vec::with_capacity(rgba.len() + 1024);
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);

    // IHDR: 8-bit RGBA.
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);

    // Raw scanlines, filter byte 0 per row.
    let stride = (width as usize) * 4;
    let mut raw = Vec::with_capacity(rgba.len() + height as usize);
    for row in rgba.chunks_exact(stride) {
        raw.push(0);
        raw.extend_from_slice(row);
    }

    // zlib stream: header + stored deflate blocks + adler32.
    let mut idat = vec![0x78, 0x01];
    let mut rest = raw.as_slice();
    loop {
        let take = rest.len().min(65_535);
        let (block, tail) = rest.split_at(take);
        let last = tail.is_empty();
        idat.push(if last { 1 } else { 0 });
        idat.extend_from_slice(&(take as u16).to_le_bytes());
        idat.extend_from_slice(&(!(take as u16)).to_le_bytes());
        idat.extend_from_slice(block);
        if last {
            break;
        }
        rest = tail;
    }
    idat.extend_from_slice(&adler32(&raw).to_be_bytes());
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

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
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
    fn known_crc_vector() {
        // CRC-32 of "123456789" is 0xcbf43926 (the classic check value).
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        // Adler-32 of "Wikipedia" is 0x11e60398.
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }
}
