//! Physically based material properties: the part of a Hew material the
//! `.skp` material record holds in its `pbrMR` block.
//!
//! Hew interprets the factors, the normal and occlusion strengths and the
//! four maps ([`skp_import::pbr_of`] says how a record reads). Everything
//! else in a record (workflow, promotion state, the enable flags of maps
//! Hew does not hold, the normal-map style, the base colour factor,
//! attributes the reader does not type) is kept as the file had it. Only
//! what the document changed is written onto a record, so an untouched
//! material saves back byte for byte.

use kernel::{Pbr, PbrMap};
use skpwright::{PbrMaterial, PbrTexture};

/// Which slot of a record a map belongs to.
#[derive(Clone, Copy)]
enum Slot {
    Metallic,
    Roughness,
    Normal,
    Occlusion,
}

const SLOTS: [(Slot, &str); 4] = [
    (Slot::Metallic, "metallic"),
    (Slot::Roughness, "roughness"),
    (Slot::Normal, "normal"),
    (Slot::Occlusion, "occlusion"),
];

impl Slot {
    fn texture(self, r: &mut PbrMaterial) -> &mut Option<PbrTexture> {
        match self {
            Slot::Metallic => &mut r.metallic,
            Slot::Roughness => &mut r.roughness,
            Slot::Normal => &mut r.normal,
            Slot::Occlusion => &mut r.occlusion,
        }
    }

    fn enabled(self, r: &mut PbrMaterial) -> &mut bool {
        match self {
            Slot::Metallic => &mut r.enable_metalness,
            Slot::Roughness => &mut r.enable_roughness,
            Slot::Normal => &mut r.enable_normal,
            Slot::Occlusion => &mut r.enable_occlusion,
        }
    }

    fn doc_map(self, p: &Pbr) -> &Option<PbrMap> {
        match self {
            Slot::Metallic => &p.metallic_map,
            Slot::Roughness => &p.roughness_map,
            Slot::Normal => &p.normal_map,
            Slot::Occlusion => &p.occlusion_map,
        }
    }
}

/// Puts the document's properties onto `rec`. With `fresh` (a record made
/// now) every value is written; otherwise only the values that differ from
/// what the record already reads as, so a value Hew does not interpret
/// differently stays as the file had it.
pub(crate) fn apply(rec: &mut PbrMaterial, doc: Option<&Pbr>, fresh: bool) {
    let doc = doc.cloned().unwrap_or(Pbr::PLAIN);
    let cur = skp_import::pbr_of(rec).unwrap_or(Pbr::PLAIN);
    if fresh || cur.metallic != doc.metallic {
        rec.metallic_factor = doc.metallic;
    }
    if fresh || cur.roughness != doc.roughness {
        rec.roughness_factor = doc.roughness;
    }
    if cur.normal_scale != doc.normal_scale {
        rec.normal_scale = doc.normal_scale;
    }
    if cur.occlusion_strength != doc.occlusion_strength {
        rec.occlusion_strength = doc.occlusion_strength;
    }
    let mut changed_map = false;
    for (slot, name) in SLOTS {
        let want = slot.doc_map(&doc);
        if slot.doc_map(&cur) == want {
            continue;
        }
        changed_map = true;
        match want {
            // A record's images are PNG; any other codec cannot be held.
            Some(m) if m.format == kernel::ImageFormat::Png => {
                *slot.texture(rec) = Some(PbrTexture {
                    id: 0,
                    path: format!("./{name}.png"),
                    file_name: format!("{name}.png"),
                    png: m.image.clone(),
                    other_attrs: Vec::new(),
                });
                *slot.enabled(rec) = true;
            }
            _ => {
                *slot.texture(rec) = None;
                *slot.enabled(rec) = false;
            }
        }
    }
    if changed_map {
        // The ids count the slots that hold an image, in slot order; a
        // material with a map is metallic-roughness (what SketchUp does).
        let mut next = 1;
        for (slot, _) in SLOTS {
            if let Some(t) = slot.texture(rec) {
                t.id = next;
                next += 1;
            }
        }
        if next > 1 {
            rec.workflow = 1;
        }
    }
}
