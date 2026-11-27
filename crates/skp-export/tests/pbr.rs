//! Physically based materials through a 2026-container file: a save back
//! writes what the document changed and keeps every field of the file's
//! record Hew does not interpret; a fresh write carries the document's
//! properties and maps.

use std::path::Path;

use kernel::{Document, ImageFormat, Material, Pbr, PbrMap, Rgba8};
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

fn saved(doc: &Document, src: Option<&SkpSource>) -> Model {
    let saved = save(doc, Target::Skp2026, env(), src).unwrap();
    Model::parse(&saved.export.bytes).unwrap()
}

fn record<'a>(m: &'a Model, name: &str) -> &'a skpwright::PbrMaterial {
    let i = m
        .materials
        .iter()
        .position(|x| matches!(x, skpwright::Material::Solid { name: n, .. } | skpwright::Material::Textured { name: n, .. } if n == name))
        .unwrap_or_else(|| panic!("no material {name}"));
    &m.features
        .material_pbr
        .iter()
        .find(|(j, _)| *j == i)
        .unwrap_or_else(|| panic!("no record for {name}"))
        .1
}

fn id_of(doc: &Document, name: &str) -> kernel::MaterialId {
    doc.material_ids()
        .into_iter()
        .find(|&id| doc.material(id).unwrap().name == name)
        .unwrap()
}

#[test]
fn an_unchanged_save_back_keeps_the_whole_record() {
    for (file, name) in [
        ("f-material-pbr.skp", "Pbr"),
        ("g-pbr-full.skp", "PbrFull"),
        ("m-normal_style_opengl.skp", "P"),
    ] {
        let bytes = fixture(file);
        let before = Model::parse(&bytes).unwrap();
        let (doc, src) = open(&bytes);
        assert_eq!(
            record(&saved(&doc, Some(&src)), name),
            record(&before, name),
            "{file}"
        );
    }
}

#[test]
fn an_edit_changes_the_factors_and_keeps_what_hew_does_not_read() {
    let bytes = fixture("g-pbr-full.skp");
    let before = Model::parse(&bytes).unwrap();
    let (mut doc, src) = open(&bytes);
    let id = id_of(&doc, "PbrFull");
    doc.set_material_pbr(id, 0.1, 0.9).unwrap();
    let after = saved(&doc, Some(&src));
    let (b, a) = (record(&before, "PbrFull"), record(&after, "PbrFull"));
    assert_eq!((a.metallic_factor, a.roughness_factor), (0.1, 0.9));
    assert_eq!(
        skpwright::PbrMaterial {
            metallic_factor: b.metallic_factor,
            roughness_factor: b.roughness_factor,
            ..a.clone()
        },
        *b,
        "workflow, flags, strengths, style and maps stay as the file had them"
    );
}

#[test]
fn a_material_made_in_hew_carries_its_properties_and_maps() {
    let map_bytes = {
        let m = Model::parse(&fixture("g-pbr-full.skp")).unwrap();
        m.features.material_pbr[0]
            .1
            .normal
            .as_ref()
            .unwrap()
            .png
            .clone()
    };
    let mut doc = Document::new();
    let mut mat = Material::solid("Steel", Rgba8::rgb(180, 180, 190));
    mat.pbr = Some(Pbr {
        metallic: 0.95,
        roughness: 0.2,
        normal_map: Some(PbrMap {
            image: map_bytes.clone(),
            format: ImageFormat::Png,
        }),
        ..Pbr::PLAIN
    });
    doc.add_material(mat);
    doc.add_material(Material::solid("Plain", Rgba8::rgb(1, 2, 3)));
    let m = saved(&doc, None);
    let r = record(&m, "Steel");
    assert_eq!((r.metallic_factor, r.roughness_factor), (0.95, 0.2));
    assert_eq!(r.workflow, 1);
    assert!(r.enable_normal && !r.enable_metalness);
    assert_eq!(r.normal.as_ref().unwrap().png, map_bytes);
    assert_eq!(
        m.features.material_pbr.len(),
        1,
        "a plain material has none"
    );

    // And it opens again as the same properties.
    let (reopened, _) = open(
        &save(&doc, Target::Skp2026, env(), None)
            .unwrap()
            .export
            .bytes,
    );
    let back = reopened.material(id_of(&reopened, "Steel")).unwrap();
    assert_eq!(back.pbr, doc.material(id_of(&doc, "Steel")).unwrap().pbr);
}
