//! On-disk naming rules: the category subfolders, item-name validation, and
//! the collision-proof file-name scheme — ports of
//! `shells/tauri/src-tauri/src/main.rs`'s `valid_library_item_name`/
//! `valid_thumb_key`/`LIBRARY_CATEGORY_DIRS` and `app/src/library/
//! fileNaming.ts`'s `itemFileName`, unified here so a headless host and the
//! desktop shell can never drift into two validators for the same folder.

use crate::meta::Category;

/// The category subfolders a two-segment item name's first segment may
/// name — mirrors `LIBRARY_CATEGORY_DIRS` (`main.rs`).
pub const CATEGORY_DIRS: [&str; 4] = ["Components", "Materials", "Models", "Profiles"];

impl Category {
    /// This category's subfolder name.
    pub fn dir_name(self) -> &'static str {
        match self {
            Category::Component => "Components",
            Category::Material => "Materials",
            Category::Model => "Models",
            Category::Profile => "Profiles",
        }
    }
}

/// Rejects a name that could escape the library folder (path separators
/// beyond one, `..`) or that isn't a Hew document. A valid name is 1 or 2
/// `/`-separated segments: each segment non-empty, with no `\`, no `..`, no
/// `:`, and no leading `.`; the last segment must end `.hew`; a two-segment
/// name's first segment must be exactly one of [`CATEGORY_DIRS`]. Identical
/// to `valid_library_item_name`, formerly in
/// `shells/tauri/src-tauri/src/main.rs` (deleted in the same change that
/// switched the shell to this crate) — this module's tests below carry the
/// same fixture cases that copy's tests did.
pub fn valid_item_name(name: &str) -> bool {
    let segments: Vec<&str> = name.split('/').collect();
    if segments.is_empty() || segments.len() > 2 {
        return false;
    }
    for segment in &segments {
        if segment.is_empty()
            || segment.contains('\\')
            || segment.contains("..")
            || segment.contains(':')
            || segment.starts_with('.')
        {
            return false;
        }
    }
    let last = segments[segments.len() - 1];
    if !last.to_ascii_lowercase().ends_with(".hew") {
        return false;
    }
    if segments.len() == 2 {
        CATEGORY_DIRS.contains(&segments[0])
    } else {
        true
    }
}

/// Rejects a thumbnail key that isn't lowercase hex, 8-64 characters —
/// identical to `valid_thumb_key` in `main.rs`.
pub fn valid_thumb_key(key: &str) -> bool {
    (8..=64).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Source-cache key charset: 1-96 chars, lowercase ASCII letters, digits,
/// `-` and `_`, never starting with `-`/`_`. No dots (so no `..`, no hidden
/// names), no separators.
pub fn valid_source_cache_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    (1..=96).contains(&bytes.len())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-' || *b == b'_')
        && !matches!(bytes[0], b'-' | b'_')
}

/// Slugify `name`, append a 6-hex-char suffix from `id`, and prefix with
/// `category`'s subfolder. Pure — no I/O, no randomness of its own (the
/// suffix comes entirely from the caller's `id`), so the same `(name, id,
/// category)` triple always names the same file. Identical to
/// `itemFileName` in `app/src/library/fileNaming.ts`.
pub fn item_file_name(name: &str, id: &str, category: Category) -> String {
    let slug = slugify(name);
    let base = if slug.is_empty() { "item" } else { &slug };
    let hex: String = id
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let suffix = if hex.len() >= 6 {
        hex[..6].to_string()
    } else {
        format!("{hex:0<6}")
    };
    let suffix = if suffix.is_empty() {
        "000000".to_string()
    } else {
        suffix
    };
    format!("{}/{base}-{suffix}.hew", category.dir_name())
}

/// Lowercases `name`, replaces every run of non-`[a-z0-9]` with a single
/// `-`, and trims leading/trailing `-` — the exact transform
/// `fileNaming.ts`'s regex chain performs.
fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_was_dash = false;
    for ch in name.trim().chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            out.push(lower);
            last_was_dash = false;
        } else if !last_was_dash && !out.is_empty() {
            out.push('-');
            last_was_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_have_their_own_subfolder() {
        assert_eq!(Category::Profile.dir_name(), "Profiles");
        assert!(CATEGORY_DIRS.contains(&"Profiles"));
        assert!(valid_item_name("Profiles/ogee-3f2a1b.hew"));
        assert_eq!(
            item_file_name("My Ogee", "3f2a1bc0-0000", Category::Profile),
            "Profiles/my-ogee-3f2a1b.hew"
        );
    }

    #[test]
    fn valid_item_name_accepts_legacy_flat_names() {
        assert!(valid_item_name("chair.hew"));
        assert!(valid_item_name("theater-chair-3f2a.hew"));
        assert!(valid_item_name("UPPER.HEW"));
    }

    #[test]
    fn valid_item_name_accepts_the_three_category_subfolders() {
        assert!(valid_item_name("Components/theater-chair-3f2a.hew"));
        assert!(valid_item_name("Materials/oak.hew"));
        assert!(valid_item_name("Models/house.hew"));
    }

    #[test]
    fn valid_item_name_rejects_any_other_first_segment() {
        assert!(!valid_item_name("components/chair.hew")); // wrong case
        assert!(!valid_item_name("Textures/chair.hew"));
        assert!(!valid_item_name(".thumbnails/x.hew"));
    }

    #[test]
    fn valid_item_name_rejects_more_than_one_subfolder_segment() {
        assert!(!valid_item_name("Components/sub/chair.hew"));
    }

    #[test]
    fn valid_item_name_rejects_escapes_and_bad_shapes() {
        for bad in [
            "",
            "a/b/c.hew",
            "../escape.hew",
            "Components/../escape.hew",
            "a\\b.hew",
            "C:evil.hew",
            ".hidden.hew",
            "Components/.hidden.hew",
            "no-extension",
            "Sketchy/thing.hew",
        ] {
            assert!(!valid_item_name(bad), "should reject {bad:?}");
        }
    }

    #[test]
    fn valid_thumb_key_accepts_only_lowercase_hex_in_range() {
        assert!(valid_thumb_key("deadbeef"));
        assert!(!valid_thumb_key("short"));
        assert!(!valid_thumb_key("DEADBEEF"));
        assert!(!valid_thumb_key("not-hex!!"));
        assert!(!valid_thumb_key(&"a".repeat(65)));
    }

    #[test]
    fn item_file_name_slugifies_and_suffixes() {
        assert_eq!(
            item_file_name(
                "My Chair!",
                "3f2a9900-aaaa-bbbb-cccc-000000000000",
                Category::Component
            ),
            "Components/my-chair-3f2a99.hew"
        );
        assert_eq!(
            item_file_name("", "abc", Category::Model),
            "Models/item-abc000.hew"
        );
    }

    #[test]
    fn item_file_name_matches_the_ts_fixture_table() {
        // The literal cases from app/src/library/fileNaming.test.ts, so a
        // future edit to either side that drifts the slug/suffix rules
        // fails a test instead of shipping two file-naming dialects.
        let cases: &[(&str, &str, Category, &str)] = &[
            (
                "Patio Chair",
                "12345678-aaaa-bbbb-cccc-ddddeeeeffff",
                Category::Component,
                "Components/patio-chair-123456.hew",
            ),
            (
                "  Fancy!!  Door//Frame  ",
                "abcdef00-0000-0000-0000-000000000000",
                Category::Model,
                "Models/fancy-door-frame-abcdef.hew",
            ),
            (
                "###",
                "11112222-0000-0000-0000-000000000000",
                Category::Component,
                "Components/item-111122.hew",
            ),
            (
                "Patio Chair",
                "ABCDEF00-0000-0000-0000-000000000000",
                Category::Component,
                "Components/patio-chair-abcdef.hew",
            ),
        ];
        for (name, id, category, expected) in cases {
            assert_eq!(&item_file_name(name, id, *category), expected);
        }
    }
}
