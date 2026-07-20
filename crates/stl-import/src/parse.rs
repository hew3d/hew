//! Binary + ASCII STL parsing: bytes → raw (unwelded) triangle soup.
//!
//! Hand-written on `std` (DESIGN §1 — no external STL crate). Every STL
//! triangle owns three private vertices (the format never shares an index
//! buffer), so [`RawTriangles`] is deliberately "soup": `positions.len() ==
//! 3 * faces.len()` and no two triangles ever reference the same slot.
//! Welding (deduplicating coincident vertices) is [`crate::build`]'s job, not
//! this module's — that keeps detection/parsing format-only and geometry-free.
//!
//! Facet normals are read for binary files (they are fixed-width and free to
//! skip over) but never trusted or returned: STL normals are frequently
//! zero, unnormalized, or simply wrong, so orientation is always recovered
//! from topology downstream (`mesh_heal::orient_outward`), never from the
//! file (DEVELOPMENT.md rule 4 — we do not trust foreign normals).

use kernel::Point3;

use crate::StlError;

/// Byte length of the binary STL header (an 80-byte free-form comment).
const BINARY_HEADER_LEN: usize = 80;
/// Byte length of the little-endian `u32` triangle count that follows the header.
const BINARY_COUNT_LEN: usize = 4;
/// Byte offset where the triangle-count `u32` begins.
const BINARY_COUNT_OFFSET: usize = BINARY_HEADER_LEN;
/// Byte offset where the first triangle record begins.
const BINARY_BODY_OFFSET: usize = BINARY_HEADER_LEN + BINARY_COUNT_LEN;
/// Byte length of one binary triangle record: a normal (3×f32) + three
/// vertices (3×3×f32) + a `u16` attribute byte count, all little-endian.
const BINARY_RECORD_LEN: usize = 50;
/// Byte length of one `f32` LE coordinate.
const F32_LEN: usize = 4;

/// Unwelded triangle soup: every triangle owns its own three vertex slots.
/// `positions.len() == 3 * faces.len()`; `faces[i] == [3*i, 3*i+1, 3*i+2]`.
pub struct RawTriangles {
    pub positions: Vec<Point3>,
    pub faces: Vec<Vec<usize>>,
}

impl RawTriangles {
    fn with_capacity(n: usize) -> Self {
        RawTriangles {
            positions: Vec::with_capacity(n * 3),
            faces: Vec::with_capacity(n),
        }
    }

    fn push_triangle(&mut self, v0: Point3, v1: Point3, v2: Point3) {
        let base = self.positions.len();
        self.positions.push(v0);
        self.positions.push(v1);
        self.positions.push(v2);
        self.faces.push(vec![base, base + 1, base + 2]);
    }
}

/// Parse STL bytes (binary or ASCII, auto-detected per DESIGN §2) into raw
/// triangle soup, plus any parse-level warnings (lenient-fallback /
/// truncation notices). Returns `Err(StlError::Parse)` when neither encoding
/// can make sense of the bytes. An empty-but-well-formed file (zero
/// triangles) is NOT an error here — [`crate::import`] maps that to
/// `StlError::Empty` once, in one place, covering both encodings.
pub fn parse(bytes: &[u8]) -> Result<(RawTriangles, Vec<String>), StlError> {
    let mut warnings = Vec::new();

    // Step 1: too short for even a binary header — the only chance is ASCII.
    if bytes.len() < BINARY_BODY_OFFSET {
        return Ok((parse_ascii_or_fail(bytes)?, warnings));
    }

    let declared_n = read_u32_le(bytes, BINARY_COUNT_OFFSET) as usize;

    // Step 2: the reliable discriminator — exact size identity. Deliberately
    // NOT gated on the header's leading bytes (some binary exporters write
    // the literal text "solid" into the 80-byte header, which would fool a
    // naive keyword check into misreading a binary file as ASCII).
    if let Some(expected_len) = exact_binary_len(declared_n)
        && bytes.len() == expected_len
    {
        return Ok((parse_binary(bytes, declared_n), warnings));
    }

    // Step 3: not an exact binary size — ASCII if it looks like text and
    // carries the `facet` token.
    if let Ok(text) = std::str::from_utf8(bytes)
        && text.contains("facet")
    {
        return Ok((parse_ascii_or_fail(bytes)?, warnings));
    }

    // Step 4: lenient binary fallback — tolerate a size mismatch (trailing
    // junk, or a truncated file with fewer complete records than declared),
    // reading however many complete 50-byte records actually fit. Loud, per
    // DEVELOPMENT.md rule 4: the caller is told this file didn't match
    // cleanly.
    let available_records = (bytes.len() - BINARY_BODY_OFFSET) / BINARY_RECORD_LEN;
    let used_n = declared_n.min(available_records);
    if used_n < declared_n {
        warnings.push(format!(
            "binary STL declares {declared_n} triangle{}, but only {used_n} complete record{} fit in the file; the file may be truncated",
            if declared_n == 1 { "" } else { "s" },
            if used_n == 1 { "" } else { "s" },
        ));
    } else {
        warnings.push(
            "file size did not exactly match the declared binary triangle count; \
             parsed as binary anyway, ignoring trailing bytes"
                .to_string(),
        );
    }
    Ok((parse_binary(bytes, used_n), warnings))
}

/// `84 + 50*n`, or `None` on overflow (a hostile/corrupt `n` must never wrap).
fn exact_binary_len(n: usize) -> Option<usize> {
    n.checked_mul(BINARY_RECORD_LEN)
        .and_then(|body| body.checked_add(BINARY_BODY_OFFSET))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_f32_le(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

/// Read `n` complete triangle records starting at [`BINARY_BODY_OFFSET`].
/// Callers guarantee `bytes.len() >= BINARY_BODY_OFFSET + n * BINARY_RECORD_LEN`.
fn parse_binary(bytes: &[u8], n: usize) -> RawTriangles {
    let mut raw = RawTriangles::with_capacity(n);
    for i in 0..n {
        let record = BINARY_BODY_OFFSET + i * BINARY_RECORD_LEN;
        // Skip the 3×f32 facet normal (12 bytes) — untrusted, never read
        // (see module docs).
        let mut v = record + 3 * F32_LEN;
        let mut verts = [Point3::ORIGIN; 3];
        for vert in &mut verts {
            let x = read_f32_le(bytes, v) as f64;
            let y = read_f32_le(bytes, v + F32_LEN) as f64;
            let z = read_f32_le(bytes, v + 2 * F32_LEN) as f64;
            *vert = Point3::new(x, y, z);
            v += 3 * F32_LEN;
        }
        // The trailing u16 attribute byte count is skipped implicitly — the
        // next record starts at record + BINARY_RECORD_LEN regardless.
        raw.push_triangle(verts[0], verts[1], verts[2]);
    }
    raw
}

/// ASCII entry point that distinguishes "not STL at all" from "recognizably
/// STL-shaped but empty": fails outright (`StlError::Parse`) only when the
/// bytes are not valid UTF-8, or are valid text with no sign of being STL
/// (no `solid`/`facet`/`vertex` keyword anywhere) — zero-triangle text that
/// DOES carry that structure (e.g. `"solid empty\nendsolid empty\n"`) comes
/// back `Ok` with empty `RawTriangles`, and `crate::import`'s single
/// zero-triangle check turns that into `StlError::Empty` uniformly for both
/// encodings, rather than this module guessing Empty-vs-Parse itself.
fn parse_ascii_or_fail(bytes: &[u8]) -> Result<RawTriangles, StlError> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        StlError::Parse("not a valid binary STL and not valid UTF-8 text".to_string())
    })?;
    let raw = parse_ascii_tokens(text);
    if raw.faces.is_empty() && !looks_like_stl_ascii(text) {
        return Err(StlError::Parse(
            "no STL structure (a `solid`/`facet` keyword or a `vertex` line) was found".to_string(),
        ));
    }
    Ok(raw)
}

/// Whether `text` carries any sign of being ASCII STL — the spec's own
/// keywords, checked loosely (matching [`parse_ascii_tokens`]'s tolerance of
/// the surrounding grammar).
fn looks_like_stl_ascii(text: &str) -> bool {
    text.contains("solid") || text.contains("facet") || text.contains("vertex")
}

/// Token-scan ASCII parser: tolerant of the `solid`/`facet normal`/`outer
/// loop`/`endloop`/`endfacet`/`endsolid` keyword structure rather than
/// requiring it verbatim — it simply looks for every `vertex x y z` token
/// run, which is what the spec's grammar reduces to. Every facet has exactly
/// three vertices, so three consecutive `vertex` hits make one triangle.
/// Always returns (possibly with zero triangles); see [`parse_ascii_or_fail`]
/// for the Empty-vs-Parse decision.
fn parse_ascii_tokens(text: &str) -> RawTriangles {
    let mut tokens = text.split_ascii_whitespace();
    let mut raw = RawTriangles::with_capacity(0);
    let mut pending: Vec<Point3> = Vec::with_capacity(3);

    while let Some(tok) = tokens.next() {
        if tok != "vertex" {
            continue;
        }
        let x = tokens.next().and_then(|t| t.parse::<f64>().ok());
        let y = tokens.next().and_then(|t| t.parse::<f64>().ok());
        let z = tokens.next().and_then(|t| t.parse::<f64>().ok());
        let (Some(x), Some(y), Some(z)) = (x, y, z) else {
            // A `vertex` token not followed by three numbers is malformed;
            // stop scanning rather than mis-pairing later tokens.
            break;
        };
        pending.push(Point3::new(x, y, z));
        if pending.len() == 3 {
            raw.push_triangle(pending[0], pending[1], pending[2]);
            pending.clear();
        }
    }

    raw
}

#[cfg(test)]
mod specs {
    use super::*;

    #[test]
    fn exact_binary_len_overflow_is_none() {
        assert_eq!(exact_binary_len(usize::MAX), None);
    }

    #[test]
    fn ascii_parses_simple_triangle() {
        let text = "solid t\n\
                     facet normal 0 0 1\n\
                       outer loop\n\
                         vertex 0 0 0\n\
                         vertex 1 0 0\n\
                         vertex 0 1 0\n\
                       endloop\n\
                     endfacet\n\
                     endsolid t\n";
        let (raw, warnings) = parse(text.as_bytes()).unwrap();
        assert_eq!(raw.faces.len(), 1);
        assert_eq!(raw.positions.len(), 3);
        assert!(warnings.is_empty());
    }

    #[test]
    fn ascii_recognized_but_empty_is_ok_with_zero_faces() {
        let raw = parse_ascii_or_fail(b"solid empty\nendsolid empty\n").expect("recognized as STL");
        assert!(raw.faces.is_empty());
    }

    #[test]
    fn ascii_with_no_stl_structure_at_all_is_parse_error() {
        assert!(parse_ascii_or_fail(b"hello world, this is not STL at all").is_err());
    }
}
