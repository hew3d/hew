//! System font enumeration + reading for the 3D Text font picker
//! (docs/design/3d-text-fonts.md). Desktop-only; the web build uses
//! `window.queryLocalFonts()` instead (`app/src/text/fontSources.ts`).
//!
//! Uses `fontdb` (MIT/Apache-2.0, pure Rust, no system font-library
//! linking) to enumerate installed font FACES rather than shelling out to
//! a native font panel: the design (docs/design/3d-text-fonts.md) chose
//! enumeration specifically because outline extraction needs bytes, and a
//! native panel would only hand back a descriptor that still needs
//! resolving to a file on each platform — enumeration already has the file.
//! `fontdb` is a dependency of the TAURI SHELL CRATE ONLY (`shells/tauri/
//! src-tauri/Cargo.toml`) — kernel crates stay free of new dependencies per
//! DEVELOPMENT.md rule 1, and this never touches them.
//!
//! ## Security: `read_font_file` takes a TOKEN, never a path
//!
//! This is the load-bearing property of this module, so it is repeated at
//! every place that could weaken it: `read_font_file` accepts an opaque
//! `token` string and NOTHING ELSE that could resolve to a filesystem
//! path. If it instead took a path — even one that "looked like" it came
//! from `list_system_fonts` — a compromised or malicious webview could
//! simply invoke `read_font_file` with any path on disk, and Tauri's
//! `#[tauri::command]` commands need no capability grant to be reachable
//! (see the `ApprovedPaths` doc comment on the pre-existing `read_file` in
//! `main.rs`, which this module's registry mirrors): that would make this
//! command an unscoped arbitrary-file-read primitive, in an app whose
//! entire purpose is opening user documents that may contain sensitive
//! content. The fix is the same shape `main.rs` already uses for
//! `read_file`/`write_file`: a server-side registry that is the ONLY way an
//! entry can ever exist, populated exclusively by `list_system_fonts`
//! actually walking the filesystem itself (never by anything the renderer
//! sends), mapping an opaque token to the `(PathBuf, face_index)` pair the
//! renderer is never allowed to construct on its own. `read_font_file`
//! looks up the token and refuses anything not found — there is no branch
//! anywhere in this file that reads a caller-supplied path string. Do not
//! "simplify" this by accepting a path directly; that removes the entire
//! guarantee this module exists to provide. See
//! `read_font_file_refuses_a_path_smuggled_through_token` below, red-checked
//! against a stripped guard to confirm it actually catches the regression.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// One selectable system font face, as sent to the webview. Deliberately
/// carries NO filesystem path — only `token` can ever be exchanged for
/// bytes (via `read_font_file`), and a token is meaningless outside this
/// process's own registry. `family`/`style` are for display; previews don't
/// need the raw bytes either (system fonts are already installed on the
/// machine, so the picker renders a row's own name with plain CSS
/// `font-family` — see the design doc's "Previews are free" note).
#[derive(Clone, serde::Serialize)]
pub struct SystemFontFace {
    pub token: String,
    pub family: String,
    /// Human-readable style label derived from weight + italic/oblique
    /// (e.g. "Regular", "Bold", "Bold Italic", "Light Oblique").
    pub style: String,
    pub weight: u16,
    pub italic: bool,
    pub monospace: bool,
}

/// The registry backing the security property above: `tokens` is the only
/// path from a token back to a real file, and it is populated exclusively
/// inside `list_system_fonts_impl`. `cache` memoizes the enumerated list
/// itself — `fontdb`'s system scan walks the font directories on disk and
/// is slow enough to notice if the picker re-ran it on every open
/// (docs/design/3d-text-fonts.md); it runs at most once per app session.
#[derive(Default)]
pub struct SystemFontState {
    cache: Mutex<Option<Vec<SystemFontFace>>>,
    tokens: Mutex<HashMap<String, (PathBuf, u32)>>,
}

/// A coarse weight-name ladder matching CSS's named weight buckets — good
/// enough for a picker's style label; this app doesn't need finer-grained
/// numeric weight display.
fn weight_name(weight: u16) -> &'static str {
    match weight {
        0..=149 => "Thin",
        150..=249 => "Extra Light",
        250..=349 => "Light",
        350..=449 => "Regular",
        450..=549 => "Medium",
        550..=649 => "SemiBold",
        650..=749 => "Bold",
        750..=849 => "Extra Bold",
        _ => "Black",
    }
}

/// Combines the weight name with an italic/oblique suffix, e.g. "Bold
/// Italic" — the `style` field of `SystemFontFace`.
fn style_label(style: fontdb::Style, weight: u16) -> String {
    let base = weight_name(weight);
    match style {
        fontdb::Style::Normal => base.to_string(),
        fontdb::Style::Italic => format!("{base} Italic"),
        fontdb::Style::Oblique => format!("{base} Oblique"),
    }
}

/// True for macOS's private/hidden UI-internal family names (a leading
/// `.`, e.g. `.Apple SD Gothic NeoI Regular`) — Font Book and every native
/// font picker hide these from the user too; they are implementation
/// details of the system UI, not fonts anyone would deliberately choose
/// for 3D Text.
fn is_hidden_family(name: &str) -> bool {
    name.starts_with('.')
}

/// Enumerates every installed font face via `fontdb::Database::load_system_fonts`,
/// assigns each a fresh opaque token, records `token -> (path, face_index)`
/// in `tokens` (the ONLY writer of that map — see this module's top-of-file
/// security doc comment), and returns the display-facing list. A face with
/// no on-disk path (`fontdb::Source::Binary`, which `load_system_fonts`
/// never actually produces, but the enum allows) is skipped — there would
/// be nothing for `read_font_file` to read later. Hidden dot-prefixed
/// families are skipped too (see `is_hidden_family`).
fn enumerate_system_fonts(tokens: &mut HashMap<String, (PathBuf, u32)>) -> Vec<SystemFontFace> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();

    let mut result = Vec::new();
    let mut counter: u64 = 0;
    for face in db.faces() {
        let path = match &face.source {
            // `fontdb`'s default features (this crate never overrides them)
            // include `fs` + `memmap`, so `SharedFile` is always present in
            // this build — no `cfg` gate needed on OUR crate's own features,
            // which don't define a "memmap"/"fs" feature at all.
            fontdb::Source::File(p) | fontdb::Source::SharedFile(p, _) => p.clone(),
            fontdb::Source::Binary(_) => continue,
        };
        let Some((family, _lang)) = face.families.first() else {
            continue;
        };
        if is_hidden_family(family) {
            continue;
        }
        counter += 1;
        let token = format!("sysfont-{counter}");
        tokens.insert(token.clone(), (path, face.index));
        result.push(SystemFontFace {
            token,
            family: family.clone(),
            style: style_label(face.style, face.weight.0),
            weight: face.weight.0,
            italic: face.style != fontdb::Style::Normal,
            monospace: face.monospaced,
        });
    }
    result
}

/// List every enumerable system font face (cached after the first call —
/// see [`SystemFontState`]'s doc comment).
#[tauri::command]
pub fn list_system_fonts(state: tauri::State<SystemFontState>) -> Vec<SystemFontFace> {
    let mut cache = state.cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(list) = cache.as_ref() {
        return list.clone();
    }
    let mut tokens = state.tokens.lock().unwrap_or_else(|e| e.into_inner());
    let list = enumerate_system_fonts(&mut tokens);
    *cache = Some(list.clone());
    list
}

/// Read `data[offset..offset+4]` as a big-endian `u32`, or `None` if out of
/// bounds — every accessor below goes through this (and its `u16` sibling)
/// rather than direct indexing, since a `.ttc`/`.ttf` on disk is untrusted
/// input as far as this parser is concerned (installed by the OS, not by
/// this app) and a malformed one must fail typed, never panic.
fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    data.get(offset..offset + 4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    data.get(offset..offset + 2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
}

/// Extracts face `face_index` out of a TrueType/OpenType COLLECTION
/// (`ttcf`) buffer into a standalone, freestanding sfnt buffer that a
/// normal single-face parser (opentype.js included) can read directly.
///
/// Why this exists, and why it is safe to ship (docs/design/3d-text-fonts.md's
/// ".ttc problem" investigation): opentype.js 1.3.4, this app's pinned
/// parser, cannot read a `ttcf` buffer AT ALL — empirically confirmed
/// against three real macOS system collections (Menlo.ttc,
/// AppleSDGothicNeo.ttc, Hiragino Sans GB.ttc): it throws "Unsupported
/// OpenType signature ttcf" regardless of face index, since its signature
/// switch only recognizes plain TrueType/CFF/WOFF. A naive byte-slice
/// starting at face N's own table directory does NOT work either
/// (empirically confirmed on the same three files: "cmap table version
/// should be 0" / "No valid cmap sub-tables found") — an sfnt table
/// directory's table offsets are absolute from the START OF THE WHOLE
/// FILE, not from the face's own directory, so slicing without rewriting
/// those offsets hands the parser tables at the wrong addresses. What DOES
/// work, confirmed empirically (all 26 faces across those same three
/// collections round-tripped through exactly this algorithm and re-parsed
/// with correct, renderable glyph outlines — checked by rendering a glyph
/// path from each result, not just a successful header parse), is this
/// mechanical repack: read face N's table directory, copy each table's
/// OPAQUE bytes (glyf/cmap/etc. contents are never reinterpreted or
/// modified) to fresh, contiguous, 4-byte-aligned offsets in a new buffer,
/// and write a fresh sfnt header + table directory pointing at those new
/// offsets. This is the standard "extract one face from a .ttc" operation
/// — what font tools call TTC splitting — not glyph-level font surgery, and
/// is exactly the "table-directory rewrite" the design doc's decision tree
/// called for over silently dropping every collection from the list.
/// Hard ceiling on how many table records a single face's sfnt directory may
/// declare. Real-world fonts — including large CJK OpenType/CFF2 collections
/// (empirically: the macOS system `.ttc`s this module's own doc comment
/// cites top out well under 30 tables per face) — never come close to this;
/// the format's own field is a `u16`, technically allowing up to 65,535, but
/// nothing legitimate needs anywhere near that. A directory claiming more is
/// refused outright rather than trusted — see `extract_ttc_face`'s doc
/// comment for the memory-amplification story this guards against.
const MAX_TABLE_RECORDS: usize = 128;

pub fn extract_ttc_face(data: &[u8], face_index: u32) -> Result<Vec<u8>, String> {
    if data.len() < 4 || &data[0..4] != b"ttcf" {
        return Err("extract_ttc_face: not a ttcf buffer".into());
    }
    let num_fonts = read_u32(data, 8).ok_or("extract_ttc_face: truncated ttcf header")?;
    if face_index >= num_fonts {
        return Err(format!(
            "extract_ttc_face: face index {face_index} out of range (numFonts={num_fonts})"
        ));
    }
    let dir_offset = read_u32(data, 12 + face_index as usize * 4)
        .ok_or("extract_ttc_face: truncated offset table")? as usize;

    let sfnt_version = data
        .get(dir_offset..dir_offset + 4)
        .ok_or("extract_ttc_face: truncated sfnt header")?;
    let num_tables =
        read_u16(data, dir_offset + 4).ok_or("extract_ttc_face: truncated sfnt header")?;

    // A face with no tables at all is not a usable font (nothing downstream
    // — glyf/cmap/etc — could ever be found), and letting `num_tables == 0`
    // through would underflow the `range_shift` computation below (whose
    // minimum `search_range` is 16, but 0 tables' "occupied" span is
    // `0 * 16 = 0`) and panic in a debug/test build (adversarial-review
    // finding 2). Reject up front rather than let the arithmetic discover it.
    if num_tables == 0 {
        return Err("extract_ttc_face: sfnt directory declares zero tables".into());
    }
    // See `MAX_TABLE_RECORDS`'s doc comment: an absurd `numTables` is refused
    // outright, both because no legitimate font needs one and because it
    // bounds the padding-overhead term in the aggregate-size sanity check
    // below.
    if num_tables as usize > MAX_TABLE_RECORDS {
        return Err(format!(
            "extract_ttc_face: sfnt directory declares {num_tables} tables (max {MAX_TABLE_RECORDS})"
        ));
    }

    struct TableRecord {
        tag: [u8; 4],
        checksum: u32,
        offset: u32,
        length: u32,
    }
    let mut records: Vec<TableRecord> = Vec::with_capacity(num_tables as usize);
    for i in 0..num_tables as usize {
        let rec = dir_offset + 12 + i * 16;
        let tag_bytes = data
            .get(rec..rec + 4)
            .ok_or("extract_ttc_face: truncated table record")?;
        let mut tag = [0u8; 4];
        tag.copy_from_slice(tag_bytes);
        let checksum = read_u32(data, rec + 4).ok_or("extract_ttc_face: truncated table record")?;
        let offset = read_u32(data, rec + 8).ok_or("extract_ttc_face: truncated table record")?;
        let length = read_u32(data, rec + 12).ok_or("extract_ttc_face: truncated table record")?;
        // Bounds-check the referenced table data up front so the copy loop
        // below can slice without re-checking (and so a malformed/truncated
        // font fails typed here rather than panicking on a copy).
        data.get(offset as usize..offset as usize + length as usize)
            .ok_or("extract_ttc_face: table data out of bounds")?;
        // Reject a record whose byte range overlaps any record already
        // accepted for this face (adversarial-review finding 1). Without
        // this, nothing stops multiple distinct records citing the SAME (or
        // overlapping) source range — each individually bounds-checked
        // above, but with no cap on how many can share one payload, so
        // `cursor` below (which sums every accepted record's length
        // unconditionally) can be driven arbitrarily far past `data.len()`.
        // With this guard, every ACCEPTED record's range is (a) fully
        // inside `data` and (b) disjoint from every other accepted record's
        // range, so the SUM of their lengths can never exceed `data.len()`
        // — that structural fact, not a chosen multiplier, is what bounds
        // the aggregate allocation below.
        for prior in &records {
            let (po, pl) = (u64::from(prior.offset), u64::from(prior.length));
            let (no, nl) = (u64::from(offset), u64::from(length));
            if no < po + pl && po < no + nl {
                return Err(format!(
                    "extract_ttc_face: table record {i} (tag {:?}) overlaps an earlier record's byte range",
                    String::from_utf8_lossy(&tag)
                ));
            }
        }
        records.push(TableRecord {
            tag,
            checksum,
            offset,
            length,
        });
    }

    // Standard sfnt binary-search header fields, recomputed for the new
    // (same) table count — copying the originals would also be valid since
    // numTables is unchanged, but deriving them keeps this function correct
    // even if a future caller ever drops/adds tables.
    let mut entry_selector: u16 = 0;
    while (1u32 << (entry_selector + 1)) <= num_tables as u32 {
        entry_selector += 1;
    }
    let search_range = (1u32 << entry_selector) * 16;
    let range_shift = num_tables as u32 * 16 - search_range;

    let header_size = 12 + num_tables as usize * 16;
    let mut cursor = header_size;
    let mut layout = Vec::with_capacity(records.len());
    for r in &records {
        let new_offset = cursor;
        cursor += r.length as usize;
        cursor = (cursor + 3) & !3; // pad to 4-byte boundary
        layout.push(new_offset);
    }

    // Defense-in-depth on top of the overlap rejection above: since every
    // accepted table record's range is disjoint and fully inside `data`,
    // the output can never legitimately need to be much bigger than the
    // input it was extracted from (the new header + the non-overlapping
    // table bytes it references + up to 3 padding bytes per table,
    // `MAX_TABLE_RECORDS`-bounded). If it somehow is, refuse rather than
    // allocate — this is the explicit belt to the overlap check's
    // suspenders, not the primary guard.
    let max_reasonable = data.len() + header_size + MAX_TABLE_RECORDS * 4;
    if cursor > max_reasonable {
        return Err(format!(
            "extract_ttc_face: computed output size {cursor} exceeds the sane bound {max_reasonable} for a {}-byte input",
            data.len()
        ));
    }

    let mut out = vec![0u8; cursor];
    out[0..4].copy_from_slice(sfnt_version);
    out[4..6].copy_from_slice(&num_tables.to_be_bytes());
    out[6..8].copy_from_slice(&(search_range as u16).to_be_bytes());
    out[8..10].copy_from_slice(&entry_selector.to_be_bytes());
    out[10..12].copy_from_slice(&(range_shift as u16).to_be_bytes());

    for (i, r) in records.iter().enumerate() {
        let rec = 12 + i * 16;
        let new_offset = layout[i];
        out[rec..rec + 4].copy_from_slice(&r.tag);
        out[rec + 4..rec + 8].copy_from_slice(&r.checksum.to_be_bytes());
        out[rec + 8..rec + 12].copy_from_slice(&(new_offset as u32).to_be_bytes());
        out[rec + 12..rec + 16].copy_from_slice(&r.length.to_be_bytes());
        let src = &data[r.offset as usize..(r.offset + r.length) as usize];
        out[new_offset..new_offset + r.length as usize].copy_from_slice(src);
    }

    Ok(out)
}

/// The real logic behind the `read_font_file` command, factored out as a
/// plain function so a unit test can drive it directly against a real
/// `SystemFontState` without needing a live `tauri::AppHandle` (`tauri::State`
/// can only be constructed from one — see `main.rs`'s `resolved_window_title`
/// doc comment for the same "pure core, thin command wrapper" split, used
/// there for exactly this reason).
///
/// `token` is the ONLY input; there is no path parameter here and there
/// must never be one (see this module's top-of-file security doc comment).
/// An unrecognized token (never issued by `list_system_fonts`, or issued in
/// a since-restarted process — tokens are never persisted across launches)
/// is refused BEFORE any filesystem access is attempted — the lookup
/// happens first, and a miss returns `Err` without ever calling
/// `std::fs::read`. A `.ttc` source is extracted to a standalone face via
/// [`extract_ttc_face`] first; a plain `.ttf`/`.otf` is returned as-is.
fn read_font_file_impl(state: &SystemFontState, token: &str) -> Result<Vec<u8>, String> {
    let (path, face_index) = {
        let tokens = state.tokens.lock().map_err(|e| e.to_string())?;
        tokens
            .get(token)
            .cloned()
            .ok_or_else(|| format!("read_font_file: unrecognized token {token:?}"))?
    };
    let bytes = std::fs::read(&path)
        .map_err(|e| format!("read_font_file failed for token {token:?}: {e}"))?;
    if bytes.len() >= 4 && &bytes[0..4] == b"ttcf" {
        extract_ttc_face(&bytes, face_index)
    } else {
        Ok(bytes)
    }
}

/// Read the font bytes for `token` — the renderer's ONLY way to fetch font
/// bytes. See [`read_font_file_impl`] for the actual logic and this
/// module's top-of-file doc comment for why `token` (never a path) is the
/// only accepted input.
///
/// Returns a raw IPC response (like `main.rs`'s `read_file`) rather than a
/// `Vec<u8>`, for the same reason: a multi-megabyte CJK collection face
/// marshalled as a JSON number array would block the webview thread parsing
/// it.
#[tauri::command]
pub fn read_font_file(
    state: tauri::State<SystemFontState>,
    token: String,
) -> Result<tauri::ipc::Response, String> {
    read_font_file_impl(&state, &token).map(tauri::ipc::Response::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal, syntactically valid `ttcf` buffer with `n` faces,
    /// each a tiny standalone sfnt containing one made-up table so
    /// `extract_ttc_face` has real (if fake) table bytes to relocate and
    /// verify. Not a real font — `extract_ttc_face` never interprets table
    /// contents, only relocates them by (offset, length) from the
    /// directory, so a fake table is exactly as valid a test fixture as a
    /// real `glyf` would be for exercising this function's own logic.
    fn build_fake_ttc(face_table_bytes: &[&[u8]]) -> Vec<u8> {
        let num_fonts = face_table_bytes.len() as u32;
        // Each face gets one table: a 12-byte sfnt header + one 16-byte
        // table record, followed by the table's own bytes.
        let mut faces_bytes: Vec<Vec<u8>> = Vec::new();
        for (i, table_data) in face_table_bytes.iter().enumerate() {
            let mut face = Vec::new();
            face.extend_from_slice(b"\x00\x01\x00\x00"); // sfnt version (TrueType)
            face.extend_from_slice(&1u16.to_be_bytes()); // numTables = 1
            face.extend_from_slice(&0u16.to_be_bytes()); // searchRange (unused by our reader)
            face.extend_from_slice(&0u16.to_be_bytes()); // entrySelector
            face.extend_from_slice(&0u16.to_be_bytes()); // rangeShift
                                                         // Table record: tag "tst{i}", checksum 0, offset/length filled below.
            let tag = format!("tst{i}");
            face.extend_from_slice(tag.as_bytes()[..4.min(tag.len())].as_ref());
            while face.len() < 12 + 4 {
                face.push(b'X'); // pad tag to 4 bytes if the format! ran short (never does for i<10)
            }
            face.extend_from_slice(&0u32.to_be_bytes()); // checksum
            let table_offset_placeholder = face.len();
            face.extend_from_slice(&0u32.to_be_bytes()); // offset (patched below)
            face.extend_from_slice(&(table_data.len() as u32).to_be_bytes()); // length
            let table_start = face.len();
            face.extend_from_slice(table_data);
            let abs_table_offset = table_start; // filled in once we know the face's placement in the whole file
            face[table_offset_placeholder..table_offset_placeholder + 4]
                .copy_from_slice(&(abs_table_offset as u32).to_be_bytes());
            faces_bytes.push(face);
        }

        // ttcf header: tag(4) + version(4) + numFonts(4) + offsetTable(4*n).
        let header_len = 12 + 4 * num_fonts as usize;
        let mut out = Vec::new();
        out.extend_from_slice(b"ttcf");
        out.extend_from_slice(&0x00010000u32.to_be_bytes());
        out.extend_from_slice(&num_fonts.to_be_bytes());

        // Lay out faces back-to-back after the offset table, and patch each
        // face's internal table-offset (which was relative to ITS OWN start
        // above) to be absolute from the start of the whole file — mirroring
        // how a real .ttc's table offsets are always file-absolute.
        let mut cursor = header_len;
        let mut face_dir_offsets = Vec::new();
        let mut patched_faces = Vec::new();
        for face in &faces_bytes {
            face_dir_offsets.push(cursor as u32);
            let mut patched = face.clone();
            // The table record's offset field (bytes 20..24 of the face
            // buffer: tag@12..16, checksum@16..20, offset@20..24) currently
            // holds the table's offset RELATIVE to this face's own start;
            // rewrite it to be absolute in the final file.
            let rel_offset = u32::from_be_bytes(patched[20..24].try_into().unwrap());
            patched[20..24].copy_from_slice(&(rel_offset + cursor as u32).to_be_bytes());
            cursor += patched.len();
            patched_faces.push(patched);
        }
        for off in &face_dir_offsets {
            out.extend_from_slice(&off.to_be_bytes());
        }
        for patched in &patched_faces {
            out.extend_from_slice(patched);
        }
        out
    }

    #[test]
    fn extract_ttc_face_relocates_the_requested_faces_table_bytes() {
        let ttc = build_fake_ttc(&[b"face-zero-data", b"face-one-payload!!"]);

        let face0 = extract_ttc_face(&ttc, 0).expect("face 0 extracts");
        // A valid standalone sfnt: version + numTables=1 + one 16-byte
        // record + the table's own bytes, nothing more.
        assert_eq!(&face0[0..4], b"\x00\x01\x00\x00");
        assert_eq!(u16::from_be_bytes([face0[4], face0[5]]), 1);
        let table_offset =
            u32::from_be_bytes([face0[20], face0[21], face0[22], face0[23]]) as usize;
        let table_len = u32::from_be_bytes([face0[24], face0[25], face0[26], face0[27]]) as usize;
        assert_eq!(
            &face0[table_offset..table_offset + table_len],
            b"face-zero-data"
        );

        let face1 = extract_ttc_face(&ttc, 1).expect("face 1 extracts");
        let table_offset =
            u32::from_be_bytes([face1[20], face1[21], face1[22], face1[23]]) as usize;
        let table_len = u32::from_be_bytes([face1[24], face1[25], face1[26], face1[27]]) as usize;
        assert_eq!(
            &face1[table_offset..table_offset + table_len],
            b"face-one-payload!!"
        );
    }

    #[test]
    fn extract_ttc_face_rejects_a_non_ttcf_buffer() {
        let err = extract_ttc_face(b"\x00\x01\x00\x00not-a-collection", 0).unwrap_err();
        assert!(err.contains("not a ttcf buffer"));
    }

    #[test]
    fn extract_ttc_face_rejects_an_out_of_range_face_index() {
        let ttc = build_fake_ttc(&[b"only-face"]);
        let err = extract_ttc_face(&ttc, 5).unwrap_err();
        assert!(err.contains("out of range"));
    }

    #[test]
    fn extract_ttc_face_rejects_truncated_input_instead_of_panicking() {
        let ttc = build_fake_ttc(&[b"face-data"]);
        // Truncate mid-header: must fail typed, never panic/index-out-of-bounds.
        let truncated = &ttc[..ttc.len() / 2];
        assert!(extract_ttc_face(truncated, 0).is_err());
    }

    /// Builds a syntactically-valid single-face `ttcf` buffer whose sfnt
    /// directory declares `num_records` table records that all cite the
    /// SAME underlying byte range (`shared_len` bytes right after the
    /// directory) — the crafted shape the memory-amplification guard exists
    /// to refuse (adversarial-review finding 1). Each record individually
    /// passes the existing per-record bounds check (it genuinely IS
    /// `shared_len` bytes inside a valid range of `data`), so nothing
    /// before an aggregate/overlap guard would ever catch this: without one,
    /// `cursor` sums every record's length regardless of whether they
    /// overlap, so `num_records` copies of one `shared_len`-byte payload
    /// balloon the output allocation to `num_records * shared_len` bytes
    /// from an input barely larger than `shared_len` itself.
    fn build_aliased_ttc(num_records: usize, shared_len: usize) -> Vec<u8> {
        assert!(num_records < 1000, "test tag scheme needs i < 1000");
        let dir_offset = 12 + 4; // ttcf header (12 bytes) + one offset-table entry (4 bytes)
        let header_size = 12 + num_records * 16; // this face's own sfnt header + records
        let shared_offset = dir_offset + header_size; // one real payload right after the directory
        let mut out = vec![0u8; shared_offset + shared_len];
        out[0..4].copy_from_slice(b"ttcf");
        out[4..8].copy_from_slice(&0x00010000u32.to_be_bytes());
        out[8..12].copy_from_slice(&1u32.to_be_bytes()); // numFonts = 1
        out[12..16].copy_from_slice(&(dir_offset as u32).to_be_bytes());

        out[dir_offset..dir_offset + 4].copy_from_slice(b"\x00\x01\x00\x00"); // sfnt version
        out[dir_offset + 4..dir_offset + 6].copy_from_slice(&(num_records as u16).to_be_bytes());
        // searchRange/entrySelector/rangeShift (dir_offset+6..+12) are never
        // read by extract_ttc_face; leave zeroed.

        for i in 0..num_records {
            let rec = dir_offset + 12 + i * 16;
            let tag = format!("t{i:03}");
            out[rec..rec + 4].copy_from_slice(&tag.as_bytes()[..4]);
            out[rec + 4..rec + 8].copy_from_slice(&0u32.to_be_bytes()); // checksum
            out[rec + 8..rec + 12].copy_from_slice(&(shared_offset as u32).to_be_bytes());
            out[rec + 12..rec + 16].copy_from_slice(&(shared_len as u32).to_be_bytes());
        }
        out
    }

    /// THE reproduction for finding 1: 100 table records (under any
    /// plausible "too many tables" ceiling on their own) all alias the same
    /// 100,000-byte payload. A real input of ~200KB must not be allowed to
    /// balloon into a ~10MB (and, per the review, unboundedly larger for a
    /// larger shared payload or a `numTables` closer to the u16 ceiling)
    /// allocation. Must be refused with a typed error, never silently
    /// amplified.
    #[test]
    fn extract_ttc_face_rejects_aliased_table_records_instead_of_amplifying() {
        let ttc = build_aliased_ttc(100, 100_000);
        assert!(
            ttc.len() < 300_000,
            "sanity: the crafted input itself must stay small"
        );
        let result = extract_ttc_face(&ttc, 0);
        assert!(
            result.is_err(),
            "100 table records aliasing the same 100,000-byte payload (a ~{}x amplification from a {}-byte input) must be refused, not amplified into a ~{}-byte allocation",
            (100 * 100_000) / ttc.len().max(1),
            ttc.len(),
            100 * 100_000,
        );
    }

    /// A more realistic partial-overlap case (not exact duplicates): two
    /// records whose ranges overlap by a few bytes must also be refused —
    /// the guard must be a genuine overlap test, not merely an
    /// exact-duplicate-range check that a crafted file could dodge by
    /// shifting each record's offset by one byte.
    #[test]
    fn extract_ttc_face_rejects_partially_overlapping_table_records() {
        let dir_offset = 12 + 4;
        let header_size = 12 + 2 * 16;
        let payload_start = dir_offset + header_size;
        let mut out = vec![0u8; payload_start + 20];
        out[0..4].copy_from_slice(b"ttcf");
        out[4..8].copy_from_slice(&0x00010000u32.to_be_bytes());
        out[8..12].copy_from_slice(&1u32.to_be_bytes());
        out[12..16].copy_from_slice(&(dir_offset as u32).to_be_bytes());
        out[dir_offset..dir_offset + 4].copy_from_slice(b"\x00\x01\x00\x00");
        out[dir_offset + 4..dir_offset + 6].copy_from_slice(&2u16.to_be_bytes());

        // Record 0: [payload_start, payload_start+10)
        let rec0 = dir_offset + 12;
        out[rec0..rec0 + 4].copy_from_slice(b"tst0");
        out[rec0 + 8..rec0 + 12].copy_from_slice(&(payload_start as u32).to_be_bytes());
        out[rec0 + 12..rec0 + 16].copy_from_slice(&10u32.to_be_bytes());

        // Record 1: [payload_start+5, payload_start+15) — overlaps record 0
        // by 5 bytes (neither contained in nor identical to it).
        let rec1 = dir_offset + 12 + 16;
        out[rec1..rec1 + 4].copy_from_slice(b"tst1");
        out[rec1 + 8..rec1 + 12].copy_from_slice(&((payload_start + 5) as u32).to_be_bytes());
        out[rec1 + 12..rec1 + 16].copy_from_slice(&10u32.to_be_bytes());

        assert!(extract_ttc_face(&out, 0).is_err());
    }

    /// finding 2's exact reproduction: an sfnt directory declaring
    /// `numTables == 0`. Before the fix, `range_shift`'s computation
    /// (`num_tables as u32 * 16 - search_range`, with `search_range`'s
    /// minimum value of 16) underflows and panics in a debug/test build —
    /// a panic reachable from a Tauri command via a crafted font file must
    /// instead be a typed `Err`.
    #[test]
    fn extract_ttc_face_rejects_zero_tables_instead_of_panicking() {
        let dir_offset = 12 + 4;
        let mut out = vec![0u8; dir_offset + 12];
        out[0..4].copy_from_slice(b"ttcf");
        out[4..8].copy_from_slice(&0x00010000u32.to_be_bytes());
        out[8..12].copy_from_slice(&1u32.to_be_bytes());
        out[12..16].copy_from_slice(&(dir_offset as u32).to_be_bytes());
        out[dir_offset..dir_offset + 4].copy_from_slice(b"\x00\x01\x00\x00");
        out[dir_offset + 4..dir_offset + 6].copy_from_slice(&0u16.to_be_bytes()); // numTables = 0

        let err = extract_ttc_face(&out, 0).expect_err("zero tables must be refused, not panic");
        assert!(err.contains("zero tables") || err.contains("numTables"));
    }

    /// THE security property this module exists to guarantee, exercised
    /// against the REAL `read_font_file_impl` (not just its backing
    /// HashMap): a token `list_system_fonts` never issued must be refused,
    /// and — the actual arbitrary-file-read risk — a caller cannot smuggle
    /// a filesystem path through the `token` parameter and have it resolve
    /// as if it had been enumerated. A real temp file stands in for an
    /// enumerated font; the registry is seeded with a token pointing at it
    /// (mirroring what `list_system_fonts` would have done), and only THAT
    /// exact token may read it.
    ///
    /// Red-checked: temporarily changing `read_font_file_impl` to treat its
    /// `token` argument as a path and read it directly (deleting the
    /// registry lookup) makes this test fail — the "a path smuggled through
    /// `token`" assertion below starts passing (the file it must never be
    /// able to read gets read), proving the test actually exercises the
    /// guard rather than passing vacuously. Restored immediately after
    /// confirming the failure; see the implementation report for the
    /// transcript.
    #[test]
    fn read_font_file_refuses_a_path_smuggled_through_token() {
        let dir = std::env::temp_dir().join(format!("hew-fonts-redcheck-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let enumerated_path = dir.join("enumerated.ttf");
        std::fs::write(&enumerated_path, b"pretend-font-bytes-for-the-real-token").unwrap();
        let not_enumerated_path = dir.join("never-enumerated-secret.txt");
        std::fs::write(
            &not_enumerated_path,
            b"this must never be readable via read_font_file",
        )
        .unwrap();

        let state = SystemFontState::default();
        {
            let mut tokens = state.tokens.lock().unwrap();
            // Mirrors exactly what `list_system_fonts` would have inserted
            // for one enumerated face — the ONLY legitimate entry.
            tokens.insert("sysfont-1".into(), (enumerated_path.clone(), 0));
        }

        // The real token round-trips to the real bytes.
        let ok =
            read_font_file_impl(&state, "sysfont-1").expect("the enumerated token must resolve");
        assert_eq!(ok, b"pretend-font-bytes-for-the-real-token");

        // A token nobody issued is refused outright.
        assert!(read_font_file_impl(&state, "sysfont-999").is_err());

        // THE attack this guard exists to stop: passing a real, readable
        // filesystem path (one that was never enumerated) AS the token.
        // If `read_font_file_impl` ever degenerated into reading its
        // argument as a path, this would succeed and return the secret
        // file's contents — it must instead be refused exactly like any
        // other unrecognized token.
        let smuggled = read_font_file_impl(&state, not_enumerated_path.to_str().unwrap());
        assert!(
            smuggled.is_err(),
            "a filesystem path passed as `token` must be refused, not read"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn weight_name_covers_the_full_ladder() {
        assert_eq!(weight_name(100), "Thin");
        assert_eq!(weight_name(400), "Regular");
        assert_eq!(weight_name(700), "Bold");
        assert_eq!(weight_name(900), "Black");
    }

    #[test]
    fn style_label_appends_italic_or_oblique() {
        assert_eq!(style_label(fontdb::Style::Normal, 400), "Regular");
        assert_eq!(style_label(fontdb::Style::Italic, 700), "Bold Italic");
        assert_eq!(style_label(fontdb::Style::Oblique, 300), "Light Oblique");
    }

    #[test]
    fn is_hidden_family_flags_only_dot_prefixed_names() {
        assert!(is_hidden_family(".Apple SD Gothic NeoI Regular"));
        assert!(!is_hidden_family("Apple SD Gothic Neo"));
        assert!(!is_hidden_family("Menlo"));
    }
}
