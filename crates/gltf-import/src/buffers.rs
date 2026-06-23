//! Buffer resolution: turn each glTF `buffer` into its bytes.
//!
//! Two embedded sources are supported in-memory:
//! - **GLB binary chunk** (`buffer::Source::Bin`) — the blob `Gltf::from_slice`
//!   peeled off the `.glb`. This is what Hew's own export and Blender's GLB
//!   export use.
//! - **`data:` URI** (base64) — common in self-contained `.gltf` files.
//!
//! External-file URIs cannot be fetched (no filesystem here, rule 1); those
//! buffers resolve to `None` and their URI is reported as missing.

use gltf::Gltf;
use gltf::buffer::Source;

/// Resolve all buffers. Returns `(data, missing)` where `data[i]` is the bytes
/// for buffer `i` (or `None` if unresolved), and `missing` lists the external
/// URIs that could not be resolved.
pub fn resolve(gltf: &Gltf) -> (Vec<Option<Vec<u8>>>, Vec<String>) {
    let mut data = Vec::new();
    let mut missing = Vec::new();
    for buffer in gltf.document.buffers() {
        match buffer.source() {
            Source::Bin => data.push(gltf.blob.clone()),
            Source::Uri(uri) => match decode_data_uri(uri) {
                Some(bytes) => data.push(Some(bytes)),
                None => {
                    missing.push(uri.to_string());
                    data.push(None);
                }
            },
        }
    }
    (data, missing)
}

/// Decode a `data:[<mime>][;base64],<payload>` URI to raw bytes. Returns `None`
/// for non-data URIs (external files) or a malformed payload.
pub fn decode_data_uri(uri: &str) -> Option<Vec<u8>> {
    let rest = uri.strip_prefix("data:")?;
    // Split header (mime + ;base64) from the payload at the first comma.
    let comma = rest.find(',')?;
    let (header, payload) = rest.split_at(comma);
    let payload = &payload[1..]; // drop the comma
    if !header.contains("base64") {
        // Percent-encoded text payloads are not used for geometry/images; skip.
        return None;
    }
    base64_decode(payload)
}

/// Minimal standard-alphabet base64 decoder (no external dep — keeps the WASM
/// bundle lean). Ignores ASCII whitespace; honours `=` padding.
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut quad = [0u8; 4];
    let mut n = 0usize;
    let mut pads = 0usize;
    for &c in s.as_bytes() {
        if c.is_ascii_whitespace() {
            continue;
        }
        if c == b'=' {
            quad[n] = 0;
            n += 1;
            pads += 1;
        } else {
            quad[n] = val(c)?;
            n += 1;
        }
        if n == 4 {
            out.push((quad[0] << 2) | (quad[1] >> 4));
            out.push((quad[1] << 4) | (quad[2] >> 2));
            out.push((quad[2] << 6) | quad[3]);
            n = 0;
        }
    }
    // A well-formed payload ends on a quad boundary (with 0–2 '=' pads).
    if n != 0 {
        return None;
    }
    out.truncate(out.len().saturating_sub(pads));
    Some(out)
}
