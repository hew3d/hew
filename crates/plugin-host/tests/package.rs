//! Reading `.hewplugin` packages: what parses, and every refusal, each
//! with a fixture built here. The property tests at the end drive the
//! reader with arbitrary bytes and with mutated valid packages.

#![cfg(hew_preview)]

mod common;

use common::*;
use plugin_host::{
    InstallError, MAX_ENTRY_BYTES, MAX_PACKAGE_BYTES, MAX_PACKAGE_ENTRIES, MAX_PERMISSIONS,
    normalize_path, read_package_zip, read_package_zip_spans,
};

fn refusal(bytes: &[u8]) -> &'static str {
    match read_package_zip(bytes) {
        Ok(p) => panic!("parsed: {:?}", p.files.keys().collect::<Vec<_>>()),
        Err(e) => e.name(),
    }
}

#[test]
fn a_stored_package_parses() {
    let bytes = package_with(&[("icon.svg", b"<svg/>"), ("dist/", b"")]);
    let package = read_package_zip(&bytes).expect("parses");
    assert_eq!(package.manifest.id, "com.example.fixture");
    assert_eq!(
        package.files.keys().map(String::as_str).collect::<Vec<_>>(),
        ["dist/background.js", "hew-plugin.json", "icon.svg"],
        "folders are not files"
    );
    assert_eq!(package.files["dist/background.js"], b"// empty");
}

#[test]
fn a_deflated_entry_is_refused() {
    let mut bytes = package_with(&[]);
    set_method(&mut bytes, 8);
    assert_eq!(refusal(&bytes), "package_not_store_only");
}

#[test]
fn any_compression_method_but_stored_is_refused() {
    for method in [1u16, 9, 12, 14, 93, 99] {
        let mut bytes = package_with(&[]);
        set_method(&mut bytes, method);
        assert_eq!(refusal(&bytes), "package_not_store_only", "method {method}");
    }
}

#[test]
fn an_encrypted_entry_is_refused() {
    let mut bytes = package_with(&[]);
    set_encrypted(&mut bytes);
    assert_eq!(refusal(&bytes), "package_not_store_only");
}

/// Builds the minimal package plus one entry named `bad`, which the zip
/// writer may refuse to write: then it is written under a placeholder of
/// the same length and patched.
fn with_entry_named(bad: &str) -> Vec<u8> {
    let placeholder: String = (0..bad.len())
        .map(|i| (b'q' + (i % 8) as u8) as char)
        .collect();
    let mut bytes = package_with(&[(placeholder.as_str(), b"x")]);
    patch_name(&mut bytes, &placeholder, bad);
    bytes
}

#[test]
fn paths_that_leave_the_package_are_refused() {
    for bad in [
        "../x.js",
        "dist/../../x.js",
        "/abs.js",
        "a\\b.js",
        "..",
        "dist/..",
    ] {
        assert_eq!(
            refusal(&with_entry_named(bad)),
            "package_path_invalid",
            "{bad:?}"
        );
    }
}

#[test]
fn dot_and_empty_segments_are_refused() {
    for bad in ["./x.js", "a/./b.js", "a//b.js", ".", "a/."] {
        assert_eq!(
            refusal(&with_entry_named(bad)),
            "package_path_invalid",
            "{bad:?}"
        );
    }
}

#[test]
fn windows_device_names_are_refused_in_any_case_and_with_any_extension() {
    for bad in [
        "CON",
        "con",
        "Con.js",
        "NUL",
        "nul.txt",
        "PRN",
        "AUX.json",
        "aux/x.js",
        "COM1",
        "com9.js",
        "LPT1",
        "lpt9.txt",
        "dist/CON.js",
        "CONIN$",
        "conout$.js",
        "COM1 .js",
    ] {
        assert_eq!(
            refusal(&with_entry_named(bad)),
            "package_path_invalid",
            "{bad:?}"
        );
    }
    // Names that only start like one are fine.
    let ok = package_with(&[("console.js", b"x"), ("com10.js", b"x"), ("nulls/a", b"x")]);
    read_package_zip(&ok).expect("not device names");
}

#[test]
fn a_colon_anywhere_is_refused() {
    for bad in ["a:b.js", "dist/x.js:stream", "C:x.js", "dist/x.js::$DATA"] {
        assert_eq!(
            refusal(&with_entry_named(bad)),
            "package_path_invalid",
            "{bad:?}"
        );
    }
}

#[test]
fn a_segment_ending_in_a_dot_or_a_space_is_refused() {
    for bad in ["a.", "x.js.", "dist./a.js", "a ", "dist /a.js", "x.js "] {
        assert_eq!(
            refusal(&with_entry_named(bad)),
            "package_path_invalid",
            "{bad:?}"
        );
    }
}

#[test]
fn control_invisible_and_reserved_characters_are_refused() {
    for bad in [
        "a\u{0}b",
        "a\nb",
        "a\u{7f}",
        "\u{202E}sj.exe",
        "a\u{200B}b",
        "a<b",
        "a>b",
        "a|b",
        "a?b",
        "a*b",
        "a\"b",
    ] {
        assert_eq!(
            refusal(&with_entry_named(bad)),
            "package_path_invalid",
            "{bad:?}"
        );
    }
}

#[test]
fn a_name_that_is_not_utf8_is_refused() {
    let mut bytes = package_with(&[("qq.js", b"x")]);
    let at = bytes.windows(5).position(|w| w == b"qq.js").unwrap();
    bytes[at] = 0xff;
    let central = bytes.windows(5).rposition(|w| w == b"qq.js").unwrap();
    bytes[central] = 0xff;
    assert_eq!(refusal(&bytes), "package_path_invalid");
}

#[test]
fn a_name_that_is_not_ascii_is_refused() {
    for bad in ["日本.js", "dist/caf\u{e9}.js", "e\u{301}.js", "a\u{a0}b"] {
        assert_eq!(
            refusal(&package_with(&[(bad, b"x")])),
            "package_path_invalid",
            "{bad:?}"
        );
    }
    // The composed and decomposed forms of one name, which macOS stores as
    // one file: refused, not merged.
    let bytes = package_with(&[("caf\u{e9}.js", b"1"), ("cafe\u{301}.js", b"2")]);
    assert_eq!(refusal(&bytes), "package_path_invalid");
}

#[test]
fn two_names_equal_after_case_folding_are_refused() {
    for (a, b) in [
        ("readme.md", "README.md"),
        ("dist/A.js", "dist/a.js"),
        ("Dist/x.js", "dist/x.js"),
    ] {
        let bytes = package_with(&[(a, b"1"), (b, b"2")]);
        assert_eq!(refusal(&bytes), "package_path_invalid", "{a} {b}");
    }
    // A folder entry and a file of the same folded name.
    let bytes = package_with(&[("Assets/", b""), ("assets", b"x")]);
    assert_eq!(refusal(&bytes), "package_path_invalid");
}

#[test]
fn a_repeated_entry_name_is_refused() {
    let mut bytes = package_with(&[("one.js", b"1"), ("two.js", b"2")]);
    patch_name(&mut bytes, "two.js", "one.js");
    assert_eq!(refusal(&bytes), "package_path_invalid");
}

#[test]
fn a_file_that_is_also_a_folder_is_refused() {
    let bytes = package_with(&[("lib", b"1"), ("lib/x.js", b"2")]);
    assert_eq!(refusal(&bytes), "package_path_invalid");
}

#[test]
fn a_symlink_entry_is_refused() {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in minimal_files() {
        w.start_file(name, options).unwrap();
        std::io::Write::write_all(&mut w, &data).unwrap();
    }
    w.add_symlink("dist/link.js", "/etc/passwd", options)
        .unwrap();
    let bytes = w.finish().unwrap().into_inner();
    assert_eq!(refusal(&bytes), "package_path_invalid");
}

#[test]
fn an_entry_that_is_not_a_file_or_folder_is_refused() {
    // A FIFO, a socket, a device: mode bits a zip can carry for any entry.
    for mode in [0o010644, 0o140644, 0o020644, 0o060644, 0o120777] {
        let mut bytes = package_with(&[("pipe", b"")]);
        set_unix_mode(&mut bytes, "pipe", mode);
        assert_eq!(refusal(&bytes), "package_path_invalid", "{mode:o}");
    }
    // A plain file's mode is fine, and so is a folder's on a folder entry.
    let mut bytes = package_with(&[("plain", b""), ("dir/", b"")]);
    set_unix_mode(&mut bytes, "plain", 0o100644);
    set_unix_mode(&mut bytes, "dir/", 0o040755);
    read_package_zip(&bytes).expect("a regular file and a folder");
    // A folder's mode on a file entry is not.
    let mut bytes = package_with(&[("plain", b"")]);
    set_unix_mode(&mut bytes, "plain", 0o040755);
    assert_eq!(refusal(&bytes), "package_path_invalid");
}

#[test]
fn a_package_without_a_manifest_is_refused() {
    let bytes = stored_zip(&[("dist/background.js", b"x")]);
    assert_eq!(refusal(&bytes), "manifest_missing");
    // Not at the root.
    let bytes = stored_zip(&[
        (
            "pkg/hew-plugin.json",
            manifest_json("com.example.fixture", &[]).as_bytes(),
        ),
        ("pkg/dist/background.js", b"x".as_slice()),
    ]);
    assert_eq!(refusal(&bytes), "manifest_missing");
}

#[test]
fn a_manifest_that_does_not_parse_is_refused_by_its_own_name() {
    let bytes = stored_zip(&[("hew-plugin.json", b"{".as_slice()), ("x", b"".as_slice())]);
    assert_eq!(refusal(&bytes), "manifest_malformed");
    let bytes = stored_zip(&[
        (
            "hew-plugin.json",
            manifest_json("com.example.fixture", &["everything"]).as_bytes(),
        ),
        ("dist/background.js", b"x".as_slice()),
    ]);
    assert_eq!(refusal(&bytes), "unknown_permission");
    let bytes = stored_zip(&[("hew-plugin.json", [0xffu8, 0xfe].as_slice())]);
    assert_eq!(refusal(&bytes), "manifest_malformed");
}

fn manifest_naming(extra: serde_json::Value) -> String {
    let mut m: serde_json::Value =
        serde_json::from_str(&manifest_json("com.example.fixture", &[])).unwrap();
    for (k, v) in extra.as_object().unwrap() {
        m[k] = v.clone();
    }
    m.to_string()
}

#[test]
fn files_the_manifest_names_must_exist() {
    let bytes = stored_zip(&[(
        "hew-plugin.json",
        manifest_json("com.example.fixture", &[]).as_bytes(),
    )]);
    assert_eq!(
        read_package_zip(&bytes).unwrap_err(),
        InstallError::ManifestFileMissing("dist/background.js".into())
    );
    for (extra, missing) in [
        (serde_json::json!({ "icon": "icon.svg" }), "icon.svg"),
        (
            serde_json::json!({ "agent": { "notes": "AGENTS.md" } }),
            "AGENTS.md",
        ),
        (
            serde_json::json!({ "locales": { "default": "en", "catalogs": "locales/" } }),
            "locales/",
        ),
        (
            serde_json::json!({ "main": "./dist/background.js" }),
            "./dist/background.js",
        ),
    ] {
        let bytes = stored_zip(&[
            ("hew-plugin.json", manifest_naming(extra).as_bytes()),
            ("dist/background.js", b"x".as_slice()),
        ]);
        assert_eq!(
            read_package_zip(&bytes).unwrap_err(),
            InstallError::ManifestFileMissing(missing.into())
        );
    }
    // Present, they parse.
    let bytes = stored_zip(&[
        (
            "hew-plugin.json",
            manifest_naming(serde_json::json!({
                "icon": "icon.svg",
                "agent": { "notes": "AGENTS.md" },
                "locales": { "default": "en", "catalogs": "locales" }
            }))
            .as_bytes(),
        ),
        ("dist/background.js", b"x".as_slice()),
        ("icon.svg", b"<svg/>".as_slice()),
        ("AGENTS.md", b"notes".as_slice()),
        ("locales/en.json", b"{}".as_slice()),
    ]);
    read_package_zip(&bytes).expect("every named file is there");
}

#[test]
fn agent_notes_over_their_cap_are_refused() {
    let notes = vec![b'n'; plugin_manifest::MAX_AGENT_NOTES_BYTES + 1];
    let bytes = stored_zip(&[
        (
            "hew-plugin.json",
            manifest_naming(serde_json::json!({ "agent": { "notes": "AGENTS.md" } })).into_bytes(),
        ),
        ("dist/background.js", b"x".to_vec()),
        ("AGENTS.md", notes),
    ]);
    assert_eq!(refusal(&bytes), "package_too_large");
}

#[test]
fn an_entry_over_the_entry_cap_is_refused() {
    let big = vec![0u8; MAX_ENTRY_BYTES + 1];
    let bytes = package_with(&[("big.bin", &big)]);
    assert_eq!(refusal(&bytes), "package_too_large");
    let fits = vec![0u8; MAX_ENTRY_BYTES];
    read_package_zip(&package_with(&[("big.bin", &fits)])).expect("at the cap is fine");
}

#[test]
fn a_package_over_the_total_cap_is_refused() {
    let a = vec![0u8; MAX_ENTRY_BYTES];
    let b = vec![1u8; MAX_ENTRY_BYTES];
    let bytes = package_with(&[("a.bin", &a), ("b.bin", &b)]);
    assert!(bytes.len() > MAX_PACKAGE_BYTES);
    assert_eq!(refusal(&bytes), "package_too_large");
}

#[test]
fn too_many_entries_are_refused() {
    let names: Vec<String> = (0..MAX_PACKAGE_ENTRIES).map(|i| format!("f/{i}")).collect();
    let extra: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), b"".as_slice())).collect();
    assert_eq!(refusal(&package_with(&extra)), "package_too_large");
}

#[test]
fn sizes_come_from_the_bytes_read_not_the_header() {
    // A header that understates a large entry: the reader never takes the
    // small number on trust.
    let big = vec![7u8; MAX_ENTRY_BYTES + 1];
    let mut bytes = package_with(&[("big.bin", &big)]);
    set_sizes(&mut bytes, 4);
    assert!(read_package_zip(&bytes).is_err());
    // A header that overstates a small one does not make it too large, and
    // does not make the reader trust the claimed length either.
    let mut bytes = package_with(&[]);
    set_sizes(&mut bytes, u32::MAX - 1);
    assert!(read_package_zip(&bytes).is_err());
}

#[test]
fn garbage_is_unreadable() {
    assert_eq!(refusal(b""), "package_unreadable");
    assert_eq!(refusal(b"not a zip at all"), "package_unreadable");
}

#[test]
fn every_refusal_has_a_distinct_name_and_an_explanation() {
    let all = [
        InstallError::Manifest(plugin_manifest::ManifestError::NoEntry),
        InstallError::PackageNotStoreOnly,
        InstallError::PackageTooLarge,
        InstallError::PackagePathInvalid("x".into()),
        InstallError::PackageUnreadable("x".into()),
        InstallError::ManifestMissing,
        InstallError::ManifestFileMissing("x".into()),
        InstallError::ReservedId("org.hew3d.x".into()),
        InstallError::HewVersionUnsupported {
            required: ">=9".into(),
            running: "1.4.0".into(),
        },
        InstallError::DependencyMissing(vec!["a.b".into()]),
        InstallError::DependencyCycle(vec!["a.b".into(), "a.b".into()]),
        InstallError::NotInstalled("a.b".into()),
        InstallError::NoPreviousVersion("a.b".into()),
        InstallError::PendingExpired,
        InstallError::PendingUnknown,
        InstallError::PendingStale,
        InstallError::TooManyPermissions(65),
        InstallError::InstalledUnreadable {
            id: "a.b".into(),
            reason: "x".into(),
        },
        InstallError::Io("x".into()),
    ];
    let names: std::collections::BTreeSet<&str> = all.iter().map(InstallError::name).collect();
    assert_eq!(names.len(), all.len());
    for e in &all {
        let name = e.name();
        assert!(
            name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
            "{name}"
        );
        assert!(!e.explanation().is_empty());
    }
    assert_eq!(InstallError::ReservedId("x".into()).name(), "reserved_id");
}

#[test]
fn normalize_path_accepts_plain_paths_unchanged() {
    for ok in [
        "a",
        "dist/background.js",
        "a..b.js",
        ".hidden",
        "locales/ja.json",
    ] {
        assert_eq!(normalize_path(ok).as_deref(), Ok(ok), "{ok}");
    }
    assert!(normalize_path(&"a".repeat(plugin_host::MAX_PATH_BYTES + 1)).is_err());
}

/// When the archive reads, every span is exactly the file's bytes, the
/// spans cover the same paths as the files, no two overlap, and their sum
/// is within the cap.
fn assert_spans_exact(bytes: &[u8]) -> bool {
    let Ok((package, spans)) = read_package_zip_spans(bytes) else {
        return false;
    };
    assert_eq!(
        spans.keys().collect::<Vec<_>>(),
        package.files.keys().collect::<Vec<_>>()
    );
    let mut ranges = Vec::new();
    for (path, span) in &spans {
        let (from, to) = (span.offset as usize, (span.offset + span.size) as usize);
        assert_eq!(&bytes[from..to], &package.files[path][..], "{path}");
        ranges.push((from, to));
    }
    ranges.sort_unstable();
    assert!(ranges.windows(2).all(|w| w[0].1 <= w[1].0), "{ranges:?}");
    assert!(spans.values().map(|s| s.size).sum::<u64>() <= MAX_PACKAGE_BYTES as u64);
    true
}

#[test]
fn spans_locate_exactly_the_bytes_that_were_read() {
    let plain = package_with(&[
        ("icon.svg", b"<svg/>"),
        ("lib/", b""),
        ("lib/a.js", b"export const a = 1;"),
        ("lib/empty.js", b""),
    ]);
    assert!(assert_spans_exact(&plain));
    assert!(assert_spans_exact(&package_with(&[])));

    // A trailing fake end-of-archive record whose comment length runs past
    // the end of the file: the reader skips it and finds the real one, and
    // every span still holds.
    let mut fake_end = plain.clone();
    fake_end.extend_from_slice(b"PK\x05\x06");
    fake_end.extend_from_slice(&[0, 0, 0, 0, 9, 0, 9, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    fake_end.extend_from_slice(&[0xff, 0xff]);
    assert!(assert_spans_exact(&fake_end), "the real record is used");

    // Junk before the archive (a self-extracting stub): spans count from
    // the start of the bytes given, junk included.
    let stub = b"#!/bin/sh\nexit 0\n";
    let mut prefixed = stub.to_vec();
    prefixed.extend_from_slice(&plain);
    assert!(
        assert_spans_exact(&prefixed),
        "the archive after the stub reads"
    );
    let (_, spans) = read_package_zip_spans(&prefixed).unwrap();
    let (_, plain_spans) = read_package_zip_spans(&plain).unwrap();
    for (path, span) in spans {
        assert_eq!(span.size, plain_spans[&path].size, "{path}");
        assert_eq!(
            span.offset,
            plain_spans[&path].offset + stub.len() as u64,
            "{path}"
        );
    }
}

#[test]
fn too_many_permissions_are_refused() {
    let hosts = |n: usize| -> Vec<String> {
        (0..n)
            .map(|i| format!("network:h{i}.example.com"))
            .collect()
    };
    let manifest = |required: Vec<String>, optional: Vec<String>| {
        serde_json::json!({
            "manifest": 1, "id": "com.example.fixture", "name": "Fixture",
            "version": "1.0.0", "api": 1, "main": "dist/background.js",
            "permissions": required, "optionalPermissions": optional
        })
        .to_string()
    };
    let zip_of = |m: String| {
        stored_zip(&[
            ("hew-plugin.json", m.into_bytes()),
            ("dist/background.js", b"x".to_vec()),
        ])
    };
    let all = hosts(MAX_PERMISSIONS + 1);
    let (a, b) = all.split_at(40);
    assert_eq!(
        read_package_zip(&zip_of(manifest(a.to_vec(), b.to_vec()))).unwrap_err(),
        InstallError::TooManyPermissions(MAX_PERMISSIONS + 1)
    );
    let fits = hosts(MAX_PERMISSIONS);
    let (a, b) = fits.split_at(40);
    read_package_zip(&zip_of(manifest(a.to_vec(), b.to_vec()))).expect("at the cap");
}

// ------------------------------------------------------------ properties

use proptest::prelude::*;

/// What any accepted package must satisfy: every path is one
/// `normalize_path` accepts unchanged, and none folds onto another.
fn assert_paths_sound(bytes: &[u8]) {
    assert_spans_exact(bytes);
    if let Ok(package) = read_package_zip(bytes) {
        let mut folded = std::collections::BTreeSet::new();
        for path in package.files.keys() {
            assert_eq!(normalize_path(path).as_deref(), Ok(path.as_str()));
            assert!(
                !path
                    .split('/')
                    .any(|s| s == ".." || s == "." || s.is_empty())
            );
            assert!(
                folded.insert(path.to_lowercase()),
                "{path} folds onto another"
            );
        }
    }
}

/// A valid package with a few more files, to mutate.
fn seed_package() -> Vec<u8> {
    package_with(&[
        ("icon.svg", b"<svg/>"),
        ("lib/a.js", b"export const a = 1;"),
        ("lib/b.js", b"export const b = 2;"),
        ("locales/", b""),
    ])
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        assert_paths_sound(&bytes);
    }

    #[test]
    fn a_zip_signature_followed_by_noise_never_panics(
        tail in proptest::collection::vec(any::<u8>(), 0..1024),
    ) {
        let mut bytes = b"PK\x03\x04".to_vec();
        bytes.extend(tail);
        bytes.extend_from_slice(b"PK\x05\x06");
        bytes.extend(std::iter::repeat_n(0u8, 18));
        assert_paths_sound(&bytes);
    }

    #[test]
    fn mutated_packages_never_panic_or_escape(
        edits in proptest::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..16),
    ) {
        let mut bytes = seed_package();
        for (at, value) in edits {
            let i = at.index(bytes.len());
            bytes[i] = value;
        }
        assert_paths_sound(&bytes);
    }

    #[test]
    fn mutated_names_never_escape(
        name in "[a-zA-Z0-9./\\\\: \u{202E}$-]{1,12}",
    ) {
        // An entry named by the generator, through the same patch the
        // fixtures above use.
        let placeholder: String = (0..name.len()).map(|i| (b'q' + (i % 8) as u8) as char).collect();
        let mut bytes = package_with(&[(placeholder.as_str(), b"x")]);
        let hits = patch_name_anywhere(&mut bytes, &placeholder, &name);
        assert_paths_sound(&bytes);
        if hits == 2 && let Ok(p) = read_package_zip(&bytes) {
            prop_assert!(p.files.contains_key(&name) || name.ends_with('/'));
        }
    }

    #[test]
    fn truncated_packages_never_panic(cut in any::<prop::sample::Index>()) {
        let bytes = seed_package();
        let at = cut.index(bytes.len());
        assert_paths_sound(&bytes[..at]);
    }
}
