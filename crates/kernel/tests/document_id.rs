//! Specs for the persistent document id (docs/dev/HEW_FILE_FORMAT.md §2,
//! manifest v26+): a UUID minted from host-supplied randomness on the
//! first save, carried in the manifest, kept across save, load, and copy,
//! and invisible to `state_hash`.

use kernel::{CompoundMeta, Document, HistoryOrigin, LoadError, Plane, Point3};

/// Fixed "randomness" so the minted id is predictable in a test.
const RANDOM: [u8; 16] = [
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
];

fn a_box(doc: &mut Document) {
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let s = doc.add_sketch(plane);
    {
        let sk = doc.sketch_mut(s).unwrap();
        for (a, b) in [
            (Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0)),
            (Point3::new(1.0, 0.0, 0.0), Point3::new(1.0, 1.0, 0.0)),
            (Point3::new(1.0, 1.0, 0.0), Point3::new(0.0, 1.0, 0.0)),
            (Point3::new(0.0, 1.0, 0.0), Point3::new(0.0, 0.0, 0.0)),
        ] {
            sk.add_segment(a, b).unwrap();
        }
    }
    let r = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, r, 1.0).unwrap();
}

fn manifest_of(bytes: &[u8]) -> serde_json::Value {
    use std::io::{Cursor, Read as _};
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut s = Vec::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_end(&mut s)
        .unwrap();
    serde_json::from_slice(&s).unwrap()
}

fn patch_manifest(bytes: &[u8], patch: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    use std::io::{Cursor, Read as _, Write as _};
    let mut manifest = manifest_of(bytes);
    patch(&mut manifest);
    let patched = serde_json::to_vec_pretty(&manifest).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::<u8>::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    out.start_file("manifest.json", opts).unwrap();
    out.write_all(&patched).unwrap();
    let mut src = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    for i in 0..src.len() {
        let mut entry = src.by_index(i).unwrap();
        let name = entry.name().unwrap().to_string();
        if name == "manifest.json" {
            continue;
        }
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).unwrap();
        out.start_file(name, opts).unwrap();
        out.write_all(&buf).unwrap();
    }
    out.finish().unwrap().into_inner()
}

fn is_uuid_v4_text(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && s.chars().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_digit() || ('a'..='f').contains(&c),
        })
        && b[14] == b'4'
        && matches!(b[19], b'8' | b'9' | b'a' | b'b')
}

#[test]
fn new_document_has_no_id() {
    let doc = Document::new();
    assert_eq!(doc.document_id(), None);
}

#[test]
fn ensure_is_idempotent() {
    let mut doc = Document::new();
    let first = doc.ensure_document_id(RANDOM);
    assert!(
        is_uuid_v4_text(&first),
        "a v4 UUID in lowercase text: {first}"
    );
    assert_eq!(first, "00112233-4455-4677-8899-aabbccddeeff");
    let again = doc.ensure_document_id([0x5a; 16]);
    assert_eq!(again, first, "a second call keeps the first id");
    assert_eq!(doc.document_id().as_deref(), Some(first.as_str()));
}

#[test]
fn save_load_round_trips_the_id() {
    let mut doc = Document::new();
    a_box(&mut doc);
    let id = doc.ensure_document_id(RANDOM);
    let bytes = doc.save();
    assert_eq!(manifest_of(&bytes)["document_id"], id.as_str());
    let loaded = Document::load(&bytes).expect("loads");
    assert_eq!(loaded.document_id(), Some(id.clone()));
    // A resave of the loaded document writes the same id.
    assert_eq!(manifest_of(&loaded.save())["document_id"], id.as_str());
}

#[test]
fn copy_keeps_the_id() {
    let mut doc = Document::new();
    a_box(&mut doc);
    doc.ensure_document_id(RANDOM);
    let bytes = doc.save();
    let a = Document::load(&bytes).unwrap();
    let b = Document::load(&bytes).unwrap();
    assert!(a.document_id().is_some());
    assert_eq!(a.document_id(), b.document_id());
    // An in-memory clone (a transaction snapshot) keeps it too.
    assert_eq!(doc.clone().document_id(), doc.document_id());
}

#[test]
fn state_hash_ignores_the_id() {
    let mut doc = Document::new();
    a_box(&mut doc);
    let before = doc.state_hash();
    doc.ensure_document_id(RANDOM);
    assert_eq!(doc.state_hash(), before, "minting does not move the hash");
    let loaded = Document::load(&doc.save()).unwrap();
    assert_eq!(loaded.state_hash(), before, "nor does a save and load");

    // Two documents differing only in their ids hash alike.
    let mut other = Document::new();
    a_box(&mut other);
    other.ensure_document_id([0x42; 16]);
    assert_eq!(other.state_hash(), before);
}

#[test]
fn id_is_not_undoable() {
    let mut doc = Document::new();
    a_box(&mut doc);
    let id = doc.ensure_document_id(RANDOM);
    doc.undo().unwrap();
    assert_eq!(doc.document_id(), Some(id.clone()));
    doc.redo().unwrap();
    assert_eq!(doc.document_id(), Some(id));
}

#[test]
fn pre_v26_file_with_document_id_is_rejected() {
    let mut doc = Document::new();
    a_box(&mut doc);
    doc.ensure_document_id(RANDOM);
    let bytes = doc.save();
    let smuggled = patch_manifest(&bytes, |m| m["format_version"] = 25.into());
    assert!(
        matches!(
            Document::load(&smuggled),
            Err(LoadError::MalformedManifest { .. })
        ),
        "a document_id below v26 is a smuggled field"
    );

    // The same file without the member still loads at v25, with no id.
    let clean = patch_manifest(&bytes, |m| {
        m["format_version"] = 25.into();
        m.as_object_mut().unwrap().remove("document_id");
    });
    let old = Document::load(&clean).expect("a v25 file loads");
    assert_eq!(old.document_id(), None);
}

#[test]
fn malformed_id_is_rejected() {
    let mut doc = Document::new();
    a_box(&mut doc);
    doc.ensure_document_id(RANDOM);
    let bytes = doc.save();
    for bad in [
        "",
        "not-a-uuid",
        "00112233-4455-4677-8899-AABBCCDDEEFF",  // uppercase
        "00112233445546778899aabbccddeeff",      // unhyphenated
        "00112233-4455-4677-8899-aabbccddeef",   // short
        "00112233-4455-4677-8899-aabbccddeeff0", // long
        "0011223-34455-4677-8899-aabbccddeeff",  // hyphen misplaced
        "00112233-4455-4677-8899-aabbccddeefg",  // not hex
        "{0112233-4455-4677-8899-aabbccddeef}",
    ] {
        let patched = patch_manifest(&bytes, |m| m["document_id"] = bad.into());
        match Document::load(&patched) {
            Err(LoadError::MalformedManifest { what }) => {
                assert_eq!(what, "document_id is not a UUID", "for {bad:?}");
            }
            other => panic!("{bad:?} should be rejected, got {other:?}"),
        }
    }
    let not_text = patch_manifest(&bytes, |m| m["document_id"] = 7.into());
    assert!(matches!(
        Document::load(&not_text),
        Err(LoadError::MalformedManifest { .. })
    ));
}

#[test]
fn encode_omits_the_member_when_absent() {
    let mut doc = Document::new();
    a_box(&mut doc);
    let manifest = manifest_of(&doc.save());
    assert!(
        manifest.as_object().unwrap().get("document_id").is_none(),
        "no key without an id"
    );
    assert_eq!(manifest["format_version"], 26);
}

#[test]
fn the_member_follows_app_version() {
    use std::io::{Cursor, Read as _};
    let mut doc = Document::new();
    doc.ensure_document_id(RANDOM);
    let bytes = doc.save();
    let mut zip = zip::ZipArchive::new(Cursor::new(&bytes[..])).unwrap();
    let mut text = String::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let app = text.find("\"app_version\"").unwrap();
    let id = text.find("\"document_id\"").unwrap();
    let materials = text.find("\"materials\"").unwrap();
    assert!(app < id && id < materials, "{text}");
}

#[test]
fn persistence_bytes_without_the_id_leave_it_out() {
    let mut doc = Document::new();
    a_box(&mut doc);
    doc.ensure_document_id(RANDOM);
    let bytes = doc.save_for_persistence_without_document_id();
    assert!(manifest_of(&bytes).get("document_id").is_none());
    assert_eq!(Document::load(&bytes).unwrap().document_id(), None);
    // The ordinary persistence bytes keep it.
    let kept = Document::load(&doc.save_for_persistence()).unwrap();
    assert_eq!(kept.document_id(), doc.document_id());
}

#[test]
fn a_failed_save_keeps_no_fresh_id() {
    let mut doc = Document::new();
    let r: Result<(), ()> = doc.with_document_id_for_save(RANDOM, |d| {
        assert!(d.document_id().is_some(), "the save sees the minted id");
        Err(())
    });
    assert!(r.is_err());
    assert_eq!(doc.document_id(), None, "a failed save forgets a fresh id");

    let ok: Result<String, ()> =
        doc.with_document_id_for_save(RANDOM, |d| Ok(d.document_id().unwrap()));
    let id = ok.unwrap();
    assert_eq!(doc.document_id().as_deref(), Some(id.as_str()));

    // An id the document already had survives a failed save.
    let r: Result<(), ()> = doc.with_document_id_for_save([0x99; 16], |_| Err(()));
    assert!(r.is_err());
    assert_eq!(doc.document_id(), Some(id));
}

#[test]
fn an_id_minted_inside_an_aborted_compound_survives_the_abort() {
    let mut doc = Document::new();
    let token = doc.begin_compound(CompoundMeta {
        label: "Plugin".to_string(),
        origin: HistoryOrigin::Connection("plugin:com.example.p".to_string()),
        coalesce: None,
    });
    a_box(&mut doc);
    // A save while the compound is open mints the id and writes it out.
    let id = doc.ensure_document_id(RANDOM);
    doc.abort_compound(token).unwrap();
    assert_eq!(doc.document_id(), Some(id), "the id is not history");
    assert!(doc.visible_object_ids().is_empty(), "the edit is gone");
}

#[test]
fn a_null_document_id_is_rejected_at_every_version() {
    let mut doc = Document::new();
    a_box(&mut doc);
    let bytes = doc.save();
    for version in [25, 26] {
        let patched = patch_manifest(&bytes, |m| {
            m["format_version"] = version.into();
            m["document_id"] = serde_json::Value::Null;
        });
        assert!(
            matches!(
                Document::load(&patched),
                Err(LoadError::MalformedManifest { .. })
            ),
            "a null document_id at v{version} is not an id"
        );
    }
}
