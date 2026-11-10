//! A 2026-container file's coordinate reference system opens as the
//! document's geo-reference, field for field.

use std::path::Path;

fn open(name: &str) -> Option<kernel::GeoReference> {
    let bytes = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/2027")
            .join(name),
    )
    .unwrap();
    skp_import::import(&bytes).unwrap().scene.geo
}

#[test]
fn a_crs_opens_with_every_field() {
    let g = open("g-crs.skp").expect("g-crs holds a system");
    assert_eq!(g.name, "TestCRS");
    assert_eq!(g.description, "a crs");
    assert_eq!(g.easting, 500_000.0);
    assert_eq!(g.northing, 4_000_000.0);
    assert_eq!(g.height, 12.5);
    assert_eq!(g.scale, 0.9996);
    assert_eq!(g.x_axis, [0.6, 0.8]);
    assert!((g.x_axis_angle().unwrap() - 0.8_f64.atan2(0.6)).abs() < 1e-12);
    assert_eq!(g.map_zone, "Zone 10");
    assert_eq!(g.geodetic_datum, "");
    assert_eq!(g.map_unit, "");
    assert_eq!(g.map_unit_metres, None);
}

type Check = fn(&kernel::GeoReference) -> bool;

#[test]
fn each_one_field_crs_file_opens_with_that_field() {
    let cases: [(&str, Check); 7] = [
        ("datum", |g| g.geodetic_datum == "NAD83"),
        ("vert", |g| g.vertical_datum == "NAVD88"),
        ("proj", |g| g.map_projection == "UTM"),
        ("tgl", |g| g.tgl_id == "tgl-1"),
        ("cal", |g| g.calibration_file_name == "cal.txt"),
        ("unit-feet", |g| {
            g.map_unit == "foot" && g.map_unit_metres == Some(0.3048)
        }),
        ("unit-us_survey_feet", |g| {
            g.map_unit == "US survey foot"
                && g.map_unit_metres
                    .is_some_and(|m| (m - 0.3048006096).abs() < 1e-9)
        }),
    ];
    for (file, check) in cases {
        let g = open(&format!("j-crs-{file}.skp")).unwrap_or_else(|| panic!("{file}: no system"));
        // The base system the pairs share.
        assert_eq!(
            (g.easting, g.northing, g.height, g.scale),
            (1.0, 2.0, 3.0, 1.0)
        );
        assert!(check(&g), "{file}: {g:?}");
    }
}

#[test]
fn a_file_without_a_crs_opens_without_a_geo_reference() {
    assert_eq!(open("f-layer-folder.skp"), None);
}
