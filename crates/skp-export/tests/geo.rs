//! The geo-reference round trip through a 2026-container file: a fresh
//! write carries the document's record, a save back writes the document's
//! record and keeps the file's own records Hew does not read, and a
//! document that dropped its record writes none.

use std::path::Path;

use kernel::{Document, GeoReference};
use skp_export::{SkpSource, Target, WriteEnv, save};
use skpwright::Model;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../skp-import/tests/fixtures/2027")
            .join(name),
    )
    .unwrap()
}

fn open(bytes: &[u8]) -> (Document, SkpSource) {
    let out = skp_import::import(bytes).unwrap();
    let provenance = out.provenance.clone();
    let mut doc = Document::new();
    let (report, _) = doc.ingest(out.scene, out.textures_missing).unwrap();
    doc.forget_history().unwrap();
    let src = SkpSource::after_open(bytes.to_vec(), &doc, &provenance, &report);
    (doc, src)
}

fn env() -> WriteEnv {
    WriteEnv {
        unix_time: 1_800_000_000,
        seed: 5,
    }
}

fn saved_crs(doc: &Document, src: Option<&SkpSource>) -> Option<skpwright::Crs> {
    let saved = save(doc, Target::Skp2026, env(), src).unwrap();
    Model::parse(&saved.export.bytes).unwrap().features.crs
}

#[test]
fn an_unchanged_save_back_keeps_the_crs() {
    for file in [
        "g-crs.skp",
        "j-crs-datum.skp",
        "j-crs-unit-feet.skp",
        "j-crs-cal.skp",
    ] {
        let bytes = fixture(file);
        let before = Model::parse(&bytes).unwrap().features.crs;
        assert!(before.is_some(), "{file}");
        let (doc, src) = open(&bytes);
        assert_eq!(saved_crs(&doc, Some(&src)), before, "{file}");
    }
}

#[test]
fn an_edited_record_is_written_and_the_rest_of_it_kept() {
    let bytes = fixture("g-crs.skp");
    let before = Model::parse(&bytes).unwrap().features.crs.unwrap();
    let (mut doc, src) = open(&bytes);
    let mut g = doc.geo_reference().unwrap().clone();
    g.easting = 600_000.0;
    g.geodetic_datum = "WGS84".to_string();
    doc.set_geo_reference(Some(g)).unwrap();
    let after = saved_crs(&doc, Some(&src)).unwrap();
    assert_eq!(after.eastings, 600_000.0);
    assert_eq!(after.geodetic_datum, "WGS84");
    assert_eq!(
        skpwright::Crs {
            eastings: before.eastings,
            geodetic_datum: before.geodetic_datum.clone(),
            ..after
        },
        before
    );
}

#[test]
fn a_save_back_keeps_records_in_the_file_that_hew_does_not_read() {
    let bytes = fixture("g-crs.skp");
    let mut m = Model::parse(&bytes).unwrap();
    m.features.crs.as_mut().unwrap().unknown = vec![(0x7ea5, vec![1, 2, 3])];
    let with_unknown = skpwright::Document::from_model(&m)
        .unwrap()
        .write()
        .unwrap();
    let kept = Model::parse(&with_unknown).unwrap().features.crs.unwrap();
    assert_eq!(kept.unknown, vec![(0x7ea5, vec![1, 2, 3])]);

    let (mut doc, src) = open(&with_unknown);
    assert_eq!(saved_crs(&doc, Some(&src)), Some(kept.clone()));
    let mut g = doc.geo_reference().unwrap().clone();
    g.northing += 1.0;
    doc.set_geo_reference(Some(g)).unwrap();
    let after = saved_crs(&doc, Some(&src)).unwrap();
    assert_eq!(after.northings, kept.northings + 1.0);
    assert_eq!(after.unknown, kept.unknown);
}

#[test]
fn clearing_the_record_writes_no_crs_on_a_save_back() {
    let bytes = fixture("g-crs.skp");
    let (mut doc, src) = open(&bytes);
    doc.set_geo_reference(None).unwrap();
    assert_eq!(saved_crs(&doc, Some(&src)), None);
}

#[test]
fn a_fresh_write_carries_the_documents_record() {
    let g = GeoReference {
        name: "Oregon North".to_string(),
        description: "survey grid".to_string(),
        easting: 2_500_000.5,
        northing: 250_000.25,
        height: 12.75,
        x_axis: [0.0, 1.0],
        scale: 0.9999,
        geodetic_datum: "NAD83".to_string(),
        vertical_datum: "NAVD88".to_string(),
        map_projection: "Lambert".to_string(),
        map_zone: "3601".to_string(),
        map_unit: "foot".to_string(),
        map_unit_metres: Some(0.3048),
        tgl_id: "EPSG:2913".to_string(),
        calibration_file_name: "site.cal".to_string(),
    };
    let mut doc = Document::new();
    doc.set_geo_reference(Some(g.clone())).unwrap();
    let crs = saved_crs(&doc, None).expect("a fresh write carries the record");
    assert_eq!(
        (crs.eastings, crs.northings, crs.height, crs.scale),
        (g.easting, g.northing, g.height, g.scale)
    );
    assert_eq!(crs.x_axis, g.x_axis);
    assert_eq!(crs.name, g.name);
    assert_eq!(crs.description, g.description);
    assert_eq!(crs.geodetic_datum, g.geodetic_datum);
    assert_eq!(crs.vertical_datum, g.vertical_datum);
    assert_eq!(crs.map_projection, g.map_projection);
    assert_eq!(crs.map_zone, g.map_zone);
    assert_eq!(crs.map_unit, g.map_unit);
    assert_eq!(crs.map_unit_metres, g.map_unit_metres);
    assert_eq!(crs.tgl_id, g.tgl_id);
    assert_eq!(crs.calibration_file_name, g.calibration_file_name);
    assert!(crs.unknown.is_empty());

    // And it opens again as the same record.
    let saved = save(&doc, Target::Skp2026, env(), None).unwrap();
    let (reopened, _) = open(&saved.export.bytes);
    assert_eq!(reopened.geo_reference(), Some(&g));
}

#[test]
fn a_document_without_a_record_writes_no_crs() {
    assert_eq!(saved_crs(&Document::new(), None), None);
}
