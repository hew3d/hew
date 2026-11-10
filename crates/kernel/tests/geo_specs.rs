//! The document's geo-reference (`geo.rs`): set, undo, persistence and its
//! version gate (manifest v25), and adoption on import.

use std::io::{Cursor, Read as _, Write as _};

use kernel::{Document, DocumentError, GeoReference, GeoReferenceError, ImportScene};

fn a_reference() -> GeoReference {
    GeoReference {
        name: "NAD83 / Oregon North".to_string(),
        description: "survey grid".to_string(),
        easting: 2_500_000.5,
        northing: 250_000.25,
        height: 12.75,
        x_axis: [0.0, 1.0],
        scale: 0.9999,
        geodetic_datum: "NAD83".to_string(),
        vertical_datum: "NAVD88".to_string(),
        map_projection: "Lambert Conformal Conic".to_string(),
        map_zone: "3601".to_string(),
        map_unit: "foot".to_string(),
        map_unit_metres: Some(0.3048),
        tgl_id: "EPSG:2913".to_string(),
        calibration_file_name: "site.cal".to_string(),
    }
}

fn manifest_json(bytes: &[u8]) -> serde_json::Value {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut raw = Vec::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_end(&mut raw)
        .unwrap();
    serde_json::from_slice(&raw).unwrap()
}

fn patch_manifest(bytes: &[u8], patch: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let mut manifest = manifest_json(bytes);
    patch(&mut manifest);
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::<u8>::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    out.start_file("manifest.json", opts).unwrap();
    out.write_all(&serde_json::to_vec_pretty(&manifest).unwrap())
        .unwrap();
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        let name = entry.name().unwrap().to_string();
        if name == "manifest.json" {
            continue;
        }
        let mut data = Vec::new();
        entry.read_to_end(&mut data).unwrap();
        out.start_file(&name, opts).unwrap();
        out.write_all(&data).unwrap();
    }
    out.finish().unwrap().into_inner()
}

#[test]
fn a_new_document_has_no_geo_reference_and_writes_no_key() {
    let doc = Document::new();
    assert_eq!(doc.geo_reference(), None);
    let m = manifest_json(&doc.save());
    assert!(m.get("geo").is_none());
    assert_eq!(m["format_version"], 26);
}

#[test]
fn a_geo_reference_survives_save_and_load_field_for_field() {
    let mut doc = Document::new();
    doc.set_geo_reference(Some(a_reference())).unwrap();
    let bytes = doc.save();
    let m = manifest_json(&bytes);
    assert_eq!(m["geo"]["name"], "NAD83 / Oregon North");
    assert_eq!(m["geo"]["map_unit_metres"], 0.3048);
    let loaded = Document::load(&bytes).unwrap();
    assert_eq!(loaded.geo_reference(), Some(&a_reference()));
    assert_eq!(loaded.state_hash(), doc.state_hash());
}

#[test]
fn empty_text_and_an_unrecorded_unit_factor_write_no_keys_and_round_trip() {
    let bare = GeoReference {
        easting: 1.0,
        northing: 2.0,
        scale: 1.0,
        x_axis: [1.0, 0.0],
        ..GeoReference::default()
    };
    let mut doc = Document::new();
    doc.set_geo_reference(Some(bare.clone())).unwrap();
    let bytes = doc.save();
    let geo = &manifest_json(&bytes)["geo"];
    for key in [
        "name",
        "geodetic_datum",
        "map_unit",
        "map_unit_metres",
        "tgl_id",
    ] {
        assert!(geo.get(key).is_none(), "{key} should be omitted");
    }
    assert_eq!(Document::load(&bytes).unwrap().geo_reference(), Some(&bare));
}

#[test]
fn setting_is_one_undo_step_and_undo_redo_restore_the_record_verbatim() {
    let mut doc = Document::new();
    doc.set_geo_reference(Some(a_reference())).unwrap();
    let mut other = a_reference();
    other.easting = 7.0;
    doc.set_geo_reference(Some(other.clone())).unwrap();
    doc.set_geo_reference(None).unwrap();
    assert_eq!(doc.geo_reference(), None);
    doc.undo().unwrap();
    assert_eq!(doc.geo_reference(), Some(&other));
    doc.undo().unwrap();
    assert_eq!(doc.geo_reference(), Some(&a_reference()));
    doc.undo().unwrap();
    assert_eq!(doc.geo_reference(), None);
    doc.redo().unwrap();
    doc.redo().unwrap();
    assert_eq!(doc.geo_reference(), Some(&other));
    doc.redo().unwrap();
    assert_eq!(doc.geo_reference(), None);
    assert_eq!(
        doc.history_entries().undo.last().unwrap().label,
        "Set geo-location"
    );
}

#[test]
fn setting_the_record_already_held_records_nothing() {
    let mut doc = Document::new();
    doc.set_geo_reference(None).unwrap();
    assert!(doc.undo().is_err());
    doc.set_geo_reference(Some(a_reference())).unwrap();
    let depth = doc.history_entries().undo.len();
    doc.set_geo_reference(Some(a_reference())).unwrap();
    assert_eq!(doc.history_entries().undo.len(), depth);
}

#[test]
fn a_bad_record_is_refused_and_leaves_the_document_alone() {
    let mut doc = Document::new();
    let mut bad = a_reference();
    bad.easting = f64::NAN;
    assert_eq!(
        doc.set_geo_reference(Some(bad)),
        Err(DocumentError::InvalidGeoReference(
            GeoReferenceError::NonFinite
        ))
        .map(|_: ()| unreachable!())
    );
    let mut bad = a_reference();
    bad.map_unit_metres = Some(0.0);
    assert!(matches!(
        doc.set_geo_reference(Some(bad)),
        Err(DocumentError::InvalidGeoReference(
            GeoReferenceError::NonPositiveUnit
        ))
    ));
    assert_eq!(doc.geo_reference(), None);
    assert!(doc.undo().is_err());
}

#[test]
fn the_x_axis_angle_is_none_for_a_zero_vector() {
    let mut g = a_reference();
    assert!((g.x_axis_angle().unwrap() - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    g.x_axis = [0.0, 0.0];
    assert_eq!(g.x_axis_angle(), None);
}

#[test]
fn geo_is_gated_at_v25_and_checked_at_load() {
    let mut doc = Document::new();
    doc.set_geo_reference(Some(a_reference())).unwrap();
    let bytes = doc.save();
    let smuggled = patch_manifest(&bytes, |m| m["format_version"] = serde_json::json!(24));
    assert!(Document::load(&smuggled).is_err());
    let bad_unit = patch_manifest(&bytes, |m| {
        m["geo"]["map_unit_metres"] = serde_json::json!(-1.0)
    });
    assert!(Document::load(&bad_unit).is_err());
    let unknown = patch_manifest(&bytes, |m| m["geo"]["north"] = serde_json::json!(1.0));
    assert!(Document::load(&unknown).is_err());
    // A v24 file without `geo` still loads.
    let older = patch_manifest(&Document::new().save(), |m| {
        m["format_version"] = serde_json::json!(24)
    });
    assert_eq!(Document::load(&older).unwrap().geo_reference(), None);
}

fn scene_with(geo: Option<GeoReference>) -> ImportScene {
    ImportScene {
        materials: Vec::new(),
        defs: Vec::new(),
        roots: Vec::new(),
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        geo,
        extra: Default::default(),
    }
}

#[test]
fn an_import_into_an_empty_document_adopts_the_geo_reference_without_history() {
    let mut doc = Document::new();
    doc.ingest(scene_with(Some(a_reference())), Vec::new())
        .unwrap();
    assert_eq!(doc.geo_reference(), Some(&a_reference()));
    doc.undo().unwrap();
    // Like the camera and the view style, the adopted record is not undone
    // with the import.
    assert_eq!(doc.geo_reference(), Some(&a_reference()));
}

#[test]
fn an_import_does_not_adopt_a_geo_reference_that_fails_validation() {
    let mut bad = a_reference();
    bad.scale = f64::INFINITY;
    let mut doc = Document::new();
    doc.ingest(scene_with(Some(bad)), Vec::new()).unwrap();
    assert_eq!(doc.geo_reference(), None);
}
