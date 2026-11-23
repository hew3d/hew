//! The relative-path rule shared by index item paths, thumbnails, snapshot
//! paths and `file_url` (6.1, `path`).

/// Longest accepted relative path, in bytes (6.1).
pub(crate) const MAX_PATH_BYTES: usize = 1_024;

/// Checks that `p` is a safe relative, `/`-separated path (6.1 `path` rule):
/// non-empty, at most 1,024 bytes, no control characters, no backslash, `:`,
/// `?` or `#`, no empty or dot-leading segment (so no `.` or `..`), and no
/// percent escape that decodes to `.`, `/` or `\`. A lone `%` is data.
///
/// Returns the reason on refusal.
pub(crate) fn check_rel_path(p: &str) -> Result<(), String> {
    if p.is_empty() {
        return Err("path is empty".into());
    }
    if p.len() > MAX_PATH_BYTES {
        return Err(format!("path is longer than {MAX_PATH_BYTES} bytes"));
    }
    if let Some(c) = p
        .chars()
        .find(|c| c.is_control() || matches!(c, '\\' | ':' | '?' | '#'))
    {
        return Err(format!("path contains {c:?}"));
    }
    if p.split('/')
        .any(|seg| seg.is_empty() || seg.starts_with('.'))
    {
        return Err("path has an empty or dot-leading segment".into());
    }
    if has_dangerous_escape(p) {
        return Err("path has a percent escape of '.', '/' or '\\'".into());
    }
    Ok(())
}

/// `check_rel_path` plus a required, case-sensitive suffix such as `.hew`.
pub(crate) fn check_rel_path_with_suffix(p: &str, suffix: &str) -> Result<(), String> {
    check_rel_path(p)?;
    if !p.ends_with(suffix) {
        return Err(format!("path does not end in {suffix}"));
    }
    Ok(())
}

/// True when `s` holds `%XX` (any hex case) that decodes to `.`, `/` or `\`.
fn has_dangerous_escape(s: &str) -> bool {
    s.as_bytes().windows(3).any(|w| {
        matches!(w, [b'%', hi, lo]
            if matches!((hex_val(*hi), hex_val(*lo)),
                (Some(h), Some(l)) if matches!(h * 16 + l, 0x2e | 0x2f | 0x5c)))
    })
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// True for 64 lowercase hex characters (6.1 `sha256`).
pub(crate) fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && is_lower_hex(s)
}

/// True for a 40 or 64 character lowercase hex commit (6.2).
pub(crate) fn is_commit_hex(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && is_lower_hex(s)
}

fn is_lower_hex(s: &str) -> bool {
    s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}
