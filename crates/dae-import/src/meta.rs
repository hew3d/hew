//! HEWMETA name decoding for COLLADA import (Workstream 2).
//!
//! SketchUp's COLLADA exporter sanitizes node/material names to `[A-Za-z0-9_]`,
//! which is lossy. A companion Ruby script embeds the TRUE name + tag paths into
//! the sanitized name as a reversible hex payload:
//!
//! ```text
//! <readable-prefix>__HEWMETA__<lowercase-hex-of-UTF-8-JSON>
//! ```
//!
//! JSON payload: `{"n":"<real name>","t":[["Structure","Roof"],...]}`.
//! `n` = real display name; `t` = list of root-first tag paths (may be absent).
//! Materials use only `n`.
//!
//! Token matching is tolerant (`_+HEWMETA_+`) because SketchUp may pad extra
//! underscores around the token (mirrors the existing `_+HEWTAG_+` handling
//! in `app/src/panels/tagModel.ts`).
//!
//! Back-compat: if no `HEWMETA` token, fall back to the legacy `__HEWTAG__` /
//! `__HEWSEP__` scheme (same as `tagModel.ts`). If neither, the raw string is
//! the name with no tags.

/// Decoded metadata from a potentially-encoded COLLADA name.
pub struct NodeMeta {
    /// The real display name, or `None` if the decoded name was empty/absent
    /// (callers should fall back to a positional label).
    pub name: Option<String>,
    /// Root-first tag paths, e.g. `[["Structure", "Roof"]]`.
    pub tags: Vec<Vec<String>>,
}

/// Decode a raw COLLADA name into a [`NodeMeta`].
///
/// Handles three cases in priority order:
/// 1. `_+HEWMETA_+` present → hex-decode → JSON parse for name + tags.
/// 2. `_+HEWTAG_+` present (legacy) → split on `_+HEWSEP_+` for tag path.
/// 3. Plain name (no token) → name = raw string, no tags.
///
/// Any malformed HEWMETA payload (bad hex, bad UTF-8, bad JSON) falls back
/// gracefully to the legacy / plain path. Never panics.
pub fn decode_meta(raw: &str) -> NodeMeta {
    // ── HEWMETA path ─────────────────────────────────────────────────────────
    if let Some(meta) = try_decode_hewmeta(raw) {
        return meta;
    }

    // ── Legacy HEWTAG path ───────────────────────────────────────────────────
    if let Some(meta) = try_decode_hewtag(raw) {
        return meta;
    }

    // ── Plain name ───────────────────────────────────────────────────────────
    NodeMeta {
        name: non_empty(raw.to_string()),
        tags: vec![],
    }
}

// ── HEWMETA decoder ───────────────────────────────────────────────────────────

/// Attempt to decode a `_+HEWMETA_+<hex>` payload.
/// Returns `None` on any malformed input so the caller can fall back.
fn try_decode_hewmeta(raw: &str) -> Option<NodeMeta> {
    // Find the literal "HEWMETA" token in the string.
    let token_pos = raw.find("HEWMETA")?;

    // Walk backward over leading underscores.
    let prefix_end = scan_underscores_back(raw, token_pos);

    // Walk forward past "HEWMETA" and trailing underscores to find the hex start.
    let after_token = token_pos + "HEWMETA".len();
    let hex_start = scan_underscores_forward(raw, after_token);

    // Everything from hex_start onward is the hex payload.
    let hex = &raw[hex_start..];

    // Hex must be non-empty and consist only of lowercase hex digits.
    if hex.is_empty() || !hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }

    // Hex-decode to bytes.
    let json_bytes = hex_decode(hex)?;

    // UTF-8 decode.
    let json_str = std::str::from_utf8(&json_bytes).ok()?;

    // JSON parse via serde_json.
    let value: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let obj = value.as_object()?;

    // Extract "n" (real name).
    let name = obj.get("n").and_then(|v| v.as_str()).map(|s| s.to_string());

    // Extract "t" (tag paths), optional.
    let tags = extract_tag_paths(obj);

    // Validate: prefix_end must be <= token_pos (sanity; always true by construction).
    let _ = prefix_end; // used only to establish that the token was found

    Some(NodeMeta {
        name: name.and_then(non_empty),
        tags,
    })
}

/// Extract `t` field from a JSON object as `Vec<Vec<String>>`.
/// Returns empty vec if absent, `null`, or malformed.
fn extract_tag_paths(obj: &serde_json::Map<String, serde_json::Value>) -> Vec<Vec<String>> {
    let Some(t_val) = obj.get("t") else {
        return vec![];
    };
    let Some(arr) = t_val.as_array() else {
        return vec![];
    };
    arr.iter()
        .filter_map(|path_val| {
            let path_arr = path_val.as_array()?;
            let segs: Vec<String> = path_arr
                .iter()
                .filter_map(|seg| seg.as_str().map(|s| s.to_string()))
                .collect();
            if segs.is_empty() { None } else { Some(segs) }
        })
        .collect()
}

// ── Legacy HEWTAG decoder ─────────────────────────────────────────────────────

/// Attempt to decode the legacy `_+HEWTAG_+<seg>_+HEWSEP_+<seg>...` scheme.
/// Returns `None` if the token is absent.
fn try_decode_hewtag(raw: &str) -> Option<NodeMeta> {
    let token_pos = raw.find("HEWTAG")?;

    // Display name = everything before the leading underscores.
    let display_end = scan_underscores_back(raw, token_pos);
    let display = raw[..display_end].to_string();

    // Everything after "HEWTAG" + trailing underscores is the tag data.
    let after_token = token_pos + "HEWTAG".len();
    let tag_data_start = scan_underscores_forward(raw, after_token);
    let tag_data = &raw[tag_data_start..];

    // Split on `_+HEWSEP_+`, filter empty segments.
    let segs: Vec<String> = split_hewsep(tag_data);
    let tags = if segs.is_empty() { vec![] } else { vec![segs] };

    Some(NodeMeta {
        name: non_empty(display),
        tags,
    })
}

/// Split `s` on runs of underscores containing "HEWSEP", filtering empty parts.
fn split_hewsep(s: &str) -> Vec<String> {
    // We split by locating each "HEWSEP" token and the surrounding underscores.
    let mut parts: Vec<String> = Vec::new();
    let mut remaining = s;
    loop {
        match remaining.find("HEWSEP") {
            None => {
                // No more tokens; the rest is a segment.
                let seg = remaining.trim_matches('_');
                if !seg.is_empty() {
                    parts.push(seg.to_string());
                }
                break;
            }
            Some(pos) => {
                // Everything before the leading underscores of this token.
                let before_end = scan_underscores_back(remaining, pos);
                let seg = remaining[..before_end].to_string();
                if !seg.is_empty() {
                    parts.push(seg);
                }
                // Advance past "HEWSEP" and its trailing underscores.
                let after = pos + "HEWSEP".len();
                let next_start = scan_underscores_forward(remaining, after);
                remaining = &remaining[next_start..];
            }
        }
    }
    parts
}

// ── Low-level helpers ─────────────────────────────────────────────────────────

/// Find the end of non-underscore content before `pos` (i.e. strip trailing `_`
/// run from `raw[..pos]`). Returns the index just after the last non-`_` char.
fn scan_underscores_back(s: &str, pos: usize) -> usize {
    let prefix = &s[..pos];
    let trimmed = prefix.trim_end_matches('_');
    trimmed.len()
}

/// Find the start of the next non-underscore character at or after `pos`.
fn scan_underscores_forward(s: &str, pos: usize) -> usize {
    let tail = &s[pos..];
    let stripped = tail.trim_start_matches('_');
    pos + (tail.len() - stripped.len())
}

/// Hex-decode a lowercase-hex string to bytes. Returns `None` on any error.
fn hex_decode(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    let hex_bytes = hex.as_bytes();
    for i in (0..hex.len()).step_by(2) {
        let hi = hex_nibble(hex_bytes[i])?;
        let lo = hex_nibble(hex_bytes[i + 1])?;
        bytes.push((hi << 4) | lo);
    }
    Some(bytes)
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    }
}

/// Convert an empty string to `None`.
fn non_empty(s: String) -> Option<String> {
    if s.is_empty() { None } else { Some(s) }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a name + tag list into the HEWMETA format (mirrors the Ruby script).
    fn encode_hewmeta(readable_prefix: &str, name: &str, tags: &[Vec<String>]) -> String {
        let json = if tags.is_empty() {
            format!(r#"{{"n":{}}}"#, serde_json::to_string(name).unwrap())
        } else {
            let tags_json = serde_json::to_string(tags).unwrap();
            format!(
                r#"{{"n":{},"t":{}}}"#,
                serde_json::to_string(name).unwrap(),
                tags_json
            )
        };
        let hex: String = json.bytes().map(|b| format!("{b:02x}")).collect();
        format!("{readable_prefix}__HEWMETA__{hex}")
    }

    // ── HEWMETA tests ─────────────────────────────────────────────────────────

    /// Real name `8' 5/8" Drywall`, tag path ["Structure","Roof"].
    #[test]
    fn hewmeta_drywall_with_tags() {
        let tags = vec![vec!["Structure".to_string(), "Roof".to_string()]];
        let encoded = encode_hewmeta("_8__5_8__Drywall", "8' 5/8\" Drywall", &tags);
        let meta = decode_meta(&encoded);
        assert_eq!(meta.name.as_deref(), Some("8' 5/8\" Drywall"));
        assert_eq!(meta.tags, tags);
    }

    /// Real name `[Metal_Aluminum_Anodized]1` (material, no tags).
    #[test]
    fn hewmeta_metal_material_no_tags() {
        let encoded = encode_hewmeta(
            "__Metal_Aluminum_Anodized_1",
            "[Metal_Aluminum_Anodized]1",
            &[],
        );
        let meta = decode_meta(&encoded);
        assert_eq!(meta.name.as_deref(), Some("[Metal_Aluminum_Anodized]1"));
        assert!(meta.tags.is_empty());
    }

    /// Real name `Pretty Ceilings`, no tags.
    #[test]
    fn hewmeta_pretty_ceilings_no_tags() {
        let encoded = encode_hewmeta("Pretty_Ceilings", "Pretty Ceilings", &[]);
        let meta = decode_meta(&encoded);
        assert_eq!(meta.name.as_deref(), Some("Pretty Ceilings"));
        assert!(meta.tags.is_empty());
    }

    /// Tolerance: various underscore-pad counts decode the same way.
    #[test]
    fn hewmeta_underscore_tolerance() {
        let name = "Test Name";
        let tags: Vec<Vec<String>> = vec![];
        let json = format!(r#"{{"n":{}}}"#, serde_json::to_string(name).unwrap());
        let hex: String = json.bytes().map(|b| format!("{b:02x}")).collect();

        // Single underscore each side (after SketchUp might produce _HEWMETA__).
        let single = format!("prefix_HEWMETA_{hex}");
        let meta = decode_meta(&single);
        assert_eq!(meta.name.as_deref(), Some(name));
        assert!(meta.tags.is_empty());

        // Triple underscore on left, triple on right.
        let triple = format!("prefix___HEWMETA___{hex}");
        let meta = decode_meta(&triple);
        assert_eq!(meta.name.as_deref(), Some(name));
        assert_eq!(meta.tags, tags);
    }

    // ── Legacy HEWTAG tests ───────────────────────────────────────────────────

    /// Legacy: `Roof_Truss_A__HEWTAG__Structure__HEWSEP__Roof`.
    #[test]
    fn legacy_hewtag_basic() {
        let raw = "Roof_Truss_A__HEWTAG__Structure__HEWSEP__Roof";
        let meta = decode_meta(raw);
        assert_eq!(meta.name.as_deref(), Some("Roof_Truss_A"));
        assert_eq!(
            meta.tags,
            vec![vec!["Structure".to_string(), "Roof".to_string()]]
        );
    }

    /// Legacy: underscore-padded token (mirrors tagModel.ts behaviour).
    #[test]
    fn legacy_hewtag_underscore_tolerant() {
        // Extra underscores around both tokens.
        let raw = "Roof_Truss_A___HEWTAG___Structure___HEWSEP___Roof";
        let meta = decode_meta(raw);
        assert_eq!(meta.name.as_deref(), Some("Roof_Truss_A"));
        assert_eq!(
            meta.tags,
            vec![vec!["Structure".to_string(), "Roof".to_string()]]
        );
    }

    // ── Plain name tests ──────────────────────────────────────────────────────

    /// No token at all → name is the raw string.
    #[test]
    fn plain_name_passthrough() {
        let meta = decode_meta("Counter_Base");
        assert_eq!(meta.name.as_deref(), Some("Counter_Base"));
        assert!(meta.tags.is_empty());
    }

    /// Empty string → name is None.
    #[test]
    fn empty_string_gives_none() {
        let meta = decode_meta("");
        assert!(meta.name.is_none());
        assert!(meta.tags.is_empty());
    }

    // ── Malformed HEWMETA → graceful fallback ─────────────────────────────────

    /// Bad hex after HEWMETA → graceful: whole raw string, no tags, no panic.
    #[test]
    fn hewmeta_bad_hex_falls_back_to_plain() {
        // "zz" is invalid hex.
        let raw = "some_prefix__HEWMETA__zz1234notvalidhex";
        let meta = decode_meta(raw);
        // Should fall back gracefully — name is the whole raw string.
        assert_eq!(meta.name.as_deref(), Some(raw));
        assert!(meta.tags.is_empty());
    }

    /// Valid hex but not valid UTF-8 → graceful fallback.
    #[test]
    fn hewmeta_bad_utf8_falls_back_to_plain() {
        // 0xff 0xfe is not valid UTF-8.
        let raw = "prefix__HEWMETA__fffe";
        let meta = decode_meta(raw);
        assert_eq!(meta.name.as_deref(), Some(raw));
        assert!(meta.tags.is_empty());
    }

    /// Valid hex, valid UTF-8 but not JSON → graceful fallback.
    #[test]
    fn hewmeta_bad_json_falls_back_to_plain() {
        // "hello" in hex = 68656c6c6f.
        let raw = "prefix__HEWMETA__68656c6c6f";
        let meta = decode_meta(raw);
        assert_eq!(meta.name.as_deref(), Some(raw));
        assert!(meta.tags.is_empty());
    }

    /// Odd-length hex → graceful fallback.
    #[test]
    fn hewmeta_odd_length_hex_falls_back() {
        let raw = "prefix__HEWMETA__abc";
        let meta = decode_meta(raw);
        assert_eq!(meta.name.as_deref(), Some(raw));
        assert!(meta.tags.is_empty());
    }

    /// Decoded real name that is empty string → name: None.
    #[test]
    fn hewmeta_empty_decoded_name_gives_none() {
        // JSON: {"n":""}
        let json = r#"{"n":""}"#;
        let hex: String = json.bytes().map(|b| format!("{b:02x}")).collect();
        let raw = format!("prefix__HEWMETA__{hex}");
        let meta = decode_meta(&raw);
        assert!(meta.name.is_none());
        assert!(meta.tags.is_empty());
    }
}
