//! Physically based materials open as the material's properties, and the
//! open report names what Hew keeps but cannot show.

use std::path::Path;

fn fixture(dir: &str, name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(dir)
            .join(name),
    )
    .unwrap()
}

fn pbr_of_named(file: &str, material: &str) -> Option<kernel::Pbr> {
    let out = skp_import::import(&fixture("2027", file)).unwrap();
    out.scene
        .materials
        .iter()
        .find(|m| m.name == material)
        .unwrap_or_else(|| panic!("{file}: no material {material}"))
        .pbr
        .clone()
}

#[test]
fn factors_open_as_the_materials_properties() {
    let p = pbr_of_named("f-material-pbr.skp", "Pbr").expect("properties");
    assert_eq!((p.metallic, p.roughness), (0.7, 0.3));
    assert!(p.metallic_map.is_none() && p.normal_map.is_none());
}

#[test]
fn an_untouched_metallic_default_is_not_read_as_a_full_metal() {
    // Only the roughness was set: the metallic factor is the file's default
    // 1 with the classic workflow, which is not a metal.
    let out = skp_import::import(&fixture("2027", "m-rough.skp")).unwrap();
    let rough = out
        .scene
        .materials
        .iter()
        .find_map(|m| m.pbr.as_ref().filter(|p| p.roughness == 0.75))
        .expect("the rough material");
    assert_eq!(rough.metallic, 0.0);
}

#[test]
fn the_maps_and_strengths_open_with_the_material() {
    let p = pbr_of_named("g-pbr-full.skp", "PbrFull").expect("properties");
    assert_eq!((p.metallic, p.roughness), (0.7, 0.3));
    assert_eq!((p.normal_scale, p.occlusion_strength), (0.6, 0.4));
    for map in p.maps() {
        let m = map.as_ref().expect("all four slots hold an image");
        assert!(m.image.starts_with(&[0x89, b'P', b'N', b'G']));
    }
}

#[test]
fn a_plain_file_has_no_properties_and_no_kept_note() {
    for (dir, file) in [("", "box.skp"), ("2027", "g-crs.skp")] {
        let out = skp_import::import(&fixture(dir, file)).unwrap();
        assert!(
            out.scene.materials.iter().all(|m| m.pbr.is_none()),
            "{file}"
        );
        assert!(
            !out.warnings.iter().any(|w| w.starts_with("Kept but")),
            "{file}: {:?}",
            out.warnings
        );
    }
}

fn kept_note(file: &str) -> Option<String> {
    skp_import::import(&fixture("2027", file))
        .unwrap()
        .warnings
        .into_iter()
        .find(|w| w.starts_with("Kept but not shown or editable"))
}

#[test]
fn the_open_report_counts_each_kind_it_keeps_but_cannot_show() {
    let has = |file: &str, part: &str| {
        let note = kept_note(file).unwrap_or_else(|| panic!("{file}: no note"));
        assert!(note.contains(part), "{file}: {note}");
    };
    has("f-hatch-pattern.skp", "hatch pattern");
    has("g-page-env.skp", "environment");
    has("j-snap-two.skp", "2 snaps");
    has("proc-solid-union.skp", "procedure");
    has("f-layer-linestyle.skp", "layer line style");
    has("f-layer-pagebehavior.skp", "layer page behaviour");
}

#[test]
fn the_note_is_one_line_and_pluralises() {
    let note = kept_note("j-snap-two.skp").unwrap();
    assert!(!note.contains('\n'));
    assert!(note.contains("2 snaps"), "{note}");
    let one = kept_note("h-snap.skp").unwrap();
    assert!(one.contains("1 snap") && !one.contains("1 snaps"), "{one}");
}
