//! OpenSKP materials -> the kernel palette, plus the archive-slot -> dense
//! index mapping face/instance material references resolve through.

use std::collections::BTreeMap;

use kernel::{ImageFormat, Material, Rgba8, Texture};

/// Dense palette + slot lookup for one model.
pub(crate) struct MatTable {
    /// Palette entries; dense index i = `model.materials[i]`.
    pub materials: Vec<Material>,
    /// Archive store-map slot -> dense index (from `Model::material_links`).
    slot_to_dense: BTreeMap<u16, u32>,
    /// Texture names whose embedded image bytes were absent (falls back to
    /// the material's average color); surfaced in the import report.
    pub textures_missing: Vec<String>,
    /// Applied texture size in inches per dense index (for UV projection).
    applied_size_in: Vec<Option<(f64, f64)>>,
}

impl MatTable {
    pub fn build(model: &openskp::Model) -> MatTable {
        let mut materials = Vec::new();
        let mut textures_missing = Vec::new();
        let mut applied_size_in = Vec::new();
        let mut slot_to_dense = BTreeMap::new();

        // Only REFERENCED materials enter the palette: `material_links` lists
        // exactly the store-map slots faces/instances use (SKP_FORMATn).
        // Everything else is template baggage — every `.skp` carries an
        // unreferenced "Default" white, which must not pollute Hew's palette.
        // (SketchUp's "Default" paint is matref 0 = no material, so it can
        // never be referenced.)
        for &(slot, idx) in &model.material_links {
            let Some(m) = model.materials.get(idx) else {
                continue;
            };
            slot_to_dense.insert(slot, materials.len() as u32);
            match m {
                openskp::Material::Solid {
                    name,
                    rgba,
                    opacity,
                } => {
                    // SketchUp stores opacity separately from the color; Hew
                    // folds it into the palette alpha (same as dae-import's
                    // COLLADA transparency channel).
                    let a = (opacity.clamp(0.0, 1.0) * f64::from(rgba[3])).round() as u8;
                    materials.push(Material::solid(
                        name.clone(),
                        Rgba8::rgba(rgba[0], rgba[1], rgba[2], a),
                    ));
                    applied_size_in.push(None);
                }
                openskp::Material::Textured {
                    name,
                    texture,
                    applied_size_in: size_in,
                    image_bytes,
                    avg_rgba,
                    opacity,
                } => {
                    let avg = avg_rgba.unwrap_or([200, 200, 200, 255]);
                    let opacity = opacity.clamp(0.0, 1.0);
                    match image_bytes {
                        Some(bytes) if !bytes.is_empty() => {
                            let format = sniff_format(bytes);
                            // Tile size: applied size is inches; Texture wants
                            // meters. Non-positive/absent -> 1 m tiles (the
                            // per-face UV frames carry the real mapping).
                            let world_size = match size_in {
                                Some((w, h)) if *w > 0.0 && *h > 0.0 => {
                                    [w / openskp::INCH, h / openskp::INCH]
                                }
                                _ => [1.0, 1.0],
                            };
                            // Opacity folds into the tint alpha, same as the
                            // solid arm (the tint's color stays white: the
                            // image is authoritative, matching dae-import).
                            let a = (opacity * 255.0).round() as u8;
                            materials.push(Material::textured(
                                name.clone(),
                                Rgba8::rgba(255, 255, 255, a),
                                Texture {
                                    image: bytes.clone(),
                                    format,
                                    world_size,
                                },
                            ));
                        }
                        _ => {
                            // No embedded image: keep the material usable as
                            // its average color and report the texture. Rare
                            // since OpenSKP resolves shared-texture back-refs
                            // to the owning material's bytes, but stays as
                            // the honest fallback (rule 4: loud, not silent).
                            let label = texture.clone().unwrap_or_else(|| name.clone());
                            if !textures_missing.contains(&label) {
                                textures_missing.push(label);
                            }
                            let a = (opacity * f64::from(avg[3])).round() as u8;
                            materials.push(Material::solid(
                                name.clone(),
                                Rgba8::rgba(avg[0], avg[1], avg[2], a),
                            ));
                        }
                    }
                    applied_size_in.push(*size_in);
                }
            }
        }

        MatTable {
            materials,
            slot_to_dense,
            textures_missing,
            applied_size_in,
        }
    }

    /// Dense index for an archive slot; `NO_MATERIAL` when the slot is 0/unlinked.
    pub fn dense(&self, slot: u16) -> u32 {
        if slot == 0 {
            return kernel::NO_MATERIAL;
        }
        self.slot_to_dense
            .get(&slot)
            .copied()
            .unwrap_or(kernel::NO_MATERIAL)
    }

    /// The applied texture size (inches) behind an archive slot, when the
    /// linked material is textured with a positive size.
    pub fn applied_size_in(&self, slot: u16) -> Option<(f64, f64)> {
        let dense = self.dense(slot);
        if dense == kernel::NO_MATERIAL {
            return None;
        }
        self.applied_size_in
            .get(dense as usize)
            .copied()
            .flatten()
            .filter(|&(w, h)| w > 0.0 && h > 0.0)
    }

    /// Whether the linked material carries an image texture.
    pub fn is_textured(&self, slot: u16) -> bool {
        let dense = self.dense(slot);
        dense != kernel::NO_MATERIAL
            && self
                .materials
                .get(dense as usize)
                .is_some_and(|m| m.texture.is_some())
    }
}

/// PNG vs JPEG from the payload magic (OpenSKP embeds only these two).
fn sniff_format(bytes: &[u8]) -> ImageFormat {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        ImageFormat::Png
    } else {
        ImageFormat::Jpeg
    }
}
