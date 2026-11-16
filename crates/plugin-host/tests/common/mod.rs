//! Package fixtures built in code: stored zips from `(name, bytes)` pairs,
//! and byte patches for what the zip writer will not produce itself (a
//! deflated method, a repeated name, a lying size).

#![allow(dead_code)]

use std::io::Write;

/// A manifest for `com.example.fixture` with `main` at
/// `dist/background.js` and the given permissions.
pub fn manifest_json(id: &str, permissions: &[&str]) -> String {
    serde_json::json!({
        "manifest": 1,
        "id": id,
        "name": "Fixture",
        "version": "1.0.0",
        "api": 1,
        "main": "dist/background.js",
        "permissions": permissions,
    })
    .to_string()
}

/// The smallest valid package's files.
pub fn minimal_files() -> Vec<(String, Vec<u8>)> {
    vec![
        (
            "hew-plugin.json".to_string(),
            manifest_json("com.example.fixture", &["model.read"]).into_bytes(),
        ),
        ("dist/background.js".to_string(), b"// empty".to_vec()),
    ]
}

/// A zip whose entries are all stored. A name ending in `/` is a folder.
pub fn stored_zip<N: AsRef<str>, B: AsRef<[u8]>>(entries: &[(N, B)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in entries {
        let name = name.as_ref();
        if name.ends_with('/') {
            w.add_directory(name, options).expect("a folder entry");
        } else {
            w.start_file(name, options).expect("an entry");
            w.write_all(data.as_ref()).expect("entry bytes");
        }
    }
    w.finish().expect("the zip finishes").into_inner()
}

/// The minimal package as a zip, plus `extra` entries.
pub fn package_with(extra: &[(&str, &[u8])]) -> Vec<u8> {
    let mut entries: Vec<(String, Vec<u8>)> = minimal_files();
    entries.extend(extra.iter().map(|(n, b)| (n.to_string(), b.to_vec())));
    stored_zip(&entries)
}

/// Replaces every occurrence of `from` with `to` (same length) in the
/// archive: the name appears in the local header and the central
/// directory, and the CRC covers only the data, so the archive stays
/// well formed. Returns how many places were patched.
pub fn patch_name(bytes: &mut [u8], from: &str, to: &str) -> usize {
    let hits = patch_name_anywhere(bytes, from, to);
    assert!(hits >= 2, "the name is in both directories");
    hits
}

/// [`patch_name`] without the check, for generated names.
pub fn patch_name_anywhere(bytes: &mut [u8], from: &str, to: &str) -> usize {
    assert_eq!(from.len(), to.len(), "a patch keeps the length");
    let (from, to) = (from.as_bytes(), to.as_bytes());
    let mut hits = 0;
    let mut i = 0;
    while i + from.len() <= bytes.len() {
        if &bytes[i..i + from.len()] == from {
            bytes[i..i + from.len()].copy_from_slice(to);
            hits += 1;
            i += from.len();
        } else {
            i += 1;
        }
    }
    hits
}

/// Offsets of every local header (`PK\3\4`) and central header (`PK\1\2`).
fn headers(bytes: &[u8]) -> (Vec<usize>, Vec<usize>) {
    let mut local = Vec::new();
    let mut central = Vec::new();
    for i in 0..bytes.len().saturating_sub(4) {
        match &bytes[i..i + 4] {
            b"PK\x03\x04" => local.push(i),
            b"PK\x01\x02" => central.push(i),
            _ => {}
        }
    }
    (local, central)
}

/// Sets every entry's compression method to `method` in both directories,
/// leaving the bytes stored: what a deflated entry looks like to a reader
/// that checks the method before reading.
pub fn set_method(bytes: &mut [u8], method: u16) {
    let (local, central) = headers(bytes);
    for at in local {
        bytes[at + 8..at + 10].copy_from_slice(&method.to_le_bytes());
    }
    for at in central {
        bytes[at + 10..at + 12].copy_from_slice(&method.to_le_bytes());
    }
}

/// Sets the encrypted flag on every entry in both directories.
pub fn set_encrypted(bytes: &mut [u8]) {
    let (local, central) = headers(bytes);
    for at in local {
        bytes[at + 6] |= 1;
    }
    for at in central {
        bytes[at + 8] |= 1;
    }
}

/// Writes `size` as both the compressed and uncompressed size of every
/// entry in both directories: a header that understates what follows.
pub fn set_sizes(bytes: &mut [u8], size: u32) {
    let (local, central) = headers(bytes);
    for at in local {
        bytes[at + 18..at + 22].copy_from_slice(&size.to_le_bytes());
        bytes[at + 22..at + 26].copy_from_slice(&size.to_le_bytes());
    }
    for at in central {
        bytes[at + 20..at + 24].copy_from_slice(&size.to_le_bytes());
        bytes[at + 24..at + 28].copy_from_slice(&size.to_le_bytes());
    }
}

/// Sets the unix mode (file type and permissions) the central directory
/// records for entry `name`, marking the entry as made on unix.
pub fn set_unix_mode(bytes: &mut [u8], name: &str, mode: u32) {
    let (_, central) = headers(bytes);
    let mut hits = 0;
    for at in central {
        let len = u16::from_le_bytes([bytes[at + 28], bytes[at + 29]]) as usize;
        if &bytes[at + 46..at + 46 + len] == name.as_bytes() {
            bytes[at + 5] = 3;
            bytes[at + 38..at + 42].copy_from_slice(&(mode << 16).to_le_bytes());
            hits += 1;
        }
    }
    assert_eq!(hits, 1, "one central entry named {name}");
}
