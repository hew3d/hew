//! Container equivalence: every fixture the corpus authored in SketchUp
//! 2017 also ships as the 2026 container (`tests/fixtures/2026/<stem>.skp`,
//! the same model saved by SketchUp 2026 — the web app's conversion, no
//! edits, no purge). The two must import to the same document: the same
//! definitions, tree, meshes, materials, tags, guides, camera and scenes,
//! including which nodes each scene hides. The 2017 file is fully decoded
//! and pinned against COLLADA upstream, so it is the ground truth for its
//! 2026 twin, and this suite is what lets the importer treat the 2026
//! container as the primary format.
//!
//! What the two containers legitimately do NOT share, and the digest
//! therefore normalises away: the order of a face's vertices (the 2026
//! file starts a ring at a different corner), the order of placements in
//! an entity list (and so of definitions in first-use order), and the
//! persistent-id VALUES themselves (SketchUp renumbers some entities when
//! it converts; the ids stay consistent within each file, so a scene's
//! hidden set is compared by the nodes it resolves to, not by number).
//!
//! Anything else that differs is an Skpwright gap and gets filed upstream,
//! never papered over here (docs/agents/ROADMAP.md).

use std::collections::BTreeMap;
use std::path::Path;

use kernel::{CameraState, Document, ImportGuide, ImportNode, ImportScene, MeshRecipe, Point3};

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture {} unreadable: {e}", path.display()))
}

/// Every stem with a 2026 twin, discovered from the fixture directory so a
/// new pair is covered the moment it lands.
fn stems() -> Vec<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/2026");
    let mut stems: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter_map(|n| n.strip_suffix(".skp").map(str::to_owned))
        .collect();
    stems.sort();
    assert!(stems.len() >= 20, "the 2026 twins are missing: {stems:?}");
    stems
}

// ── A comparable digest of a recipe ───────────────────────────────────────
// Lengths are quantised to a micron so the two readers' float paths (both
// decode the same f64 inches) agree; meshes, node lists and definitions
// are canonicalised as the module docs describe.

#[derive(Debug, PartialEq)]
struct Digest {
    /// Sorted.
    defs: Vec<DefDigest>,
    /// Sorted.
    roots: Vec<NodeDigest>,
    materials: Vec<(String, [u8; 4], bool)>,
    tags: Vec<(Vec<String>, bool)>,
    guides: Vec<([i64; 3], [i64; 3])>,
    camera: Option<CamDigest>,
    scenes: Vec<SceneDigest>,
    section: Option<CutDigest>,
}

type DefDigest = (Option<String>, Vec<NodeDigest>);

/// A face: outer ring, hole rings, material, has a UV frame.
type FaceDigest = (Vec<[i64; 3]>, Vec<Vec<[i64; 3]>>, u32, bool);

/// A section cut: origin, normal, active.
type CutDigest = ([i64; 3], [i64; 3], bool);

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum NodeDigest {
    Mesh {
        name: String,
        /// Sorted.
        faces: Vec<FaceDigest>,
        base_material: u32,
        tags: Vec<Vec<String>>,
    },
    Group {
        name: String,
        /// Sorted.
        children: Vec<NodeDigest>,
        tags: Vec<Vec<String>>,
        hidden: bool,
        keyed: bool,
    },
    Instance {
        /// The placed definition itself, not its index (indices follow
        /// first-use order, which follows placement order).
        def: Box<DefDigest>,
        pose: [i64; 12],
        name: Option<String>,
        tags: Vec<Vec<String>>,
        hidden: bool,
        keyed: bool,
    },
}

#[derive(Debug, PartialEq)]
struct CamDigest {
    perspective: bool,
    fov_deg: i64,
    eye: [i64; 3],
    target: [i64; 3],
    up: [i64; 3],
}

#[derive(Debug, PartialEq)]
struct SceneDigest {
    name: String,
    description: String,
    camera: Option<CamDigest>,
    /// The nodes the scene's hidden keys resolve to, as sorted labels, plus
    /// how many keys resolve to nothing Hew imports (faces, edges, guides).
    hidden: Option<(Vec<String>, usize)>,
    hidden_tags: Option<Vec<Vec<String>>>,
    section: Option<Option<CutDigest>>,
}

fn q(v: f64) -> i64 {
    (v * 1e6).round() as i64
}

fn qp(p: Point3) -> [i64; 3] {
    [q(p.x), q(p.y), q(p.z)]
}

fn qv(v: kernel::Vec3) -> [i64; 3] {
    [q(v.x), q(v.y), q(v.z)]
}

fn cam(c: &CameraState) -> CamDigest {
    CamDigest {
        perspective: matches!(c.projection, kernel::CameraProjection::Perspective),
        fov_deg: q(c.fov_deg),
        eye: qp(c.eye),
        target: qp(c.target),
        up: qv(c.up),
    }
}

fn section(s: &kernel::SectionPlaneState) -> CutDigest {
    (qp(s.origin), qv(s.normal), s.active)
}

/// A ring as quantised positions, rotated to start at its smallest corner
/// (winding preserved).
fn ring(m: &MeshRecipe, ring: &[usize]) -> Vec<[i64; 3]> {
    let pts: Vec<[i64; 3]> = ring.iter().map(|&vi| qp(m.positions[vi])).collect();
    let Some(start) = (0..pts.len()).min_by_key(|&i| pts[i]) else {
        return pts;
    };
    pts[start..].iter().chain(&pts[..start]).copied().collect()
}

fn mesh(m: &MeshRecipe) -> NodeDigest {
    let mut faces: Vec<FaceDigest> = (0..m.faces.len())
        .map(|fi| {
            let mut holes: Vec<Vec<[i64; 3]>> =
                m.face_holes[fi].iter().map(|h| ring(m, h)).collect();
            holes.sort();
            (
                ring(m, &m.faces[fi]),
                holes,
                m.face_materials[fi],
                m.face_uv_frames[fi].is_some(),
            )
        })
        .collect();
    faces.sort();
    NodeDigest::Mesh {
        name: m.name.clone(),
        faces,
        base_material: m.base_material,
        tags: m.tags.clone(),
    }
}

fn def(scene: &ImportScene, di: usize) -> DefDigest {
    let d = &scene.defs[di];
    (d.name.clone(), nodes(scene, &d.children))
}

fn nodes(scene: &ImportScene, list: &[ImportNode]) -> Vec<NodeDigest> {
    let mut out: Vec<NodeDigest> = list
        .iter()
        .map(|n| match n {
            ImportNode::Mesh(m) => mesh(m),
            ImportNode::Group {
                name,
                children,
                tags,
                hidden,
                key,
            } => NodeDigest::Group {
                name: name.clone(),
                children: nodes(scene, children),
                tags: tags.clone(),
                hidden: *hidden,
                keyed: key.is_some(),
            },
            ImportNode::Instance {
                def: di,
                pose,
                name,
                tags,
                hidden,
                key,
            } => NodeDigest::Instance {
                def: Box::new(def(scene, *di)),
                pose: pose.to_affine().map(q),
                name: name.clone(),
                tags: tags.clone(),
                hidden: *hidden,
                keyed: key.is_some(),
            },
        })
        .collect();
    out.sort();
    out
}

/// Importer key -> a label for the node carrying it, over the whole scene
/// (definition members included).
fn key_labels(scene: &ImportScene) -> BTreeMap<u64, String> {
    fn visit(scene: &ImportScene, list: &[ImportNode], out: &mut BTreeMap<u64, String>) {
        for n in list {
            match n {
                ImportNode::Mesh(_) => {}
                ImportNode::Group {
                    name,
                    children,
                    key,
                    ..
                } => {
                    if let Some(k) = key {
                        out.insert(*k, format!("group {name:?}"));
                    }
                    visit(scene, children, out);
                }
                ImportNode::Instance { def, name, key, .. } => {
                    if let Some(k) = key {
                        let label = name
                            .clone()
                            .or_else(|| scene.defs[*def].name.clone())
                            .unwrap_or_default();
                        out.insert(*k, format!("instance {label:?}"));
                    }
                }
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(scene, &scene.roots, &mut out);
    for d in &scene.defs {
        visit(scene, &d.children, &mut out);
    }
    out
}

fn digest(scene: &ImportScene) -> Digest {
    let labels = key_labels(scene);
    let mut defs: Vec<DefDigest> = (0..scene.defs.len()).map(|di| def(scene, di)).collect();
    defs.sort();
    Digest {
        defs,
        roots: nodes(scene, &scene.roots),
        materials: scene
            .materials
            .iter()
            .map(|m| {
                let c = m.color;
                (m.name.clone(), [c.r, c.g, c.b, c.a], m.texture.is_some())
            })
            .collect(),
        tags: scene
            .tags
            .iter()
            .map(|t| (t.path.clone(), t.hidden))
            .collect(),
        guides: scene
            .guides
            .iter()
            .map(|g| match g {
                ImportGuide::Line {
                    origin, direction, ..
                } => (qp(*origin), qv(*direction)),
                ImportGuide::Point { position, .. } => (qp(*position), [0, 0, 0]),
            })
            .collect(),
        camera: scene.camera.as_ref().map(cam),
        scenes: scene
            .scenes
            .iter()
            .map(|s| SceneDigest {
                name: s.name.clone(),
                description: s.description.clone(),
                camera: s.camera.as_ref().map(cam),
                hidden: s.hidden_keys.as_ref().map(|keys| {
                    let mut resolved: Vec<String> =
                        keys.iter().filter_map(|k| labels.get(k).cloned()).collect();
                    resolved.sort();
                    let unresolved = keys.len() - resolved.len();
                    (resolved, unresolved)
                }),
                hidden_tags: s.hidden_tags.clone(),
                section: s.section.as_ref().map(|o| o.as_ref().map(section)),
            })
            .collect(),
        section: scene.section.as_ref().map(section),
    }
}

/// The first places two digests differ, as `path: 2017 vs 2026` lines —
/// a whole-digest dump of a house is unreadable.
fn diff_digest(a: &Digest, b: &Digest) -> Vec<String> {
    let mut out = Vec::new();
    fn field<T: PartialEq + std::fmt::Debug>(out: &mut Vec<String>, path: &str, a: &T, b: &T) {
        if a != b {
            out.push(format!("{path}: {a:?} vs {b:?}"));
        }
    }
    fn list<T: PartialEq + std::fmt::Debug>(out: &mut Vec<String>, path: &str, a: &[T], b: &[T]) {
        if a.len() != b.len() {
            out.push(format!("{path}: {} vs {} entries", a.len(), b.len()));
        }
        for (i, (x, y)) in a.iter().zip(b).enumerate() {
            field(out, &format!("{path}[{i}]"), x, y);
        }
    }
    fn kind(n: &NodeDigest) -> String {
        match n {
            NodeDigest::Mesh { name, faces, .. } => {
                format!("Mesh {name:?} ({} faces)", faces.len())
            }
            NodeDigest::Group { name, children, .. } => {
                format!("Group {name:?} ({} children)", children.len())
            }
            NodeDigest::Instance { def, name, .. } => {
                format!("Instance {name:?} of {:?}", def.0)
            }
        }
    }
    fn nodes(out: &mut Vec<String>, path: &str, a: &[NodeDigest], b: &[NodeDigest]) {
        if a.len() != b.len() {
            out.push(format!("{path}: {} vs {} nodes", a.len(), b.len()));
        }
        for (i, (x, y)) in a.iter().zip(b).enumerate() {
            let p = format!("{path}[{i}]");
            match (x, y) {
                (
                    NodeDigest::Mesh {
                        name: na,
                        faces: fa,
                        base_material: ma,
                        tags: ta,
                    },
                    NodeDigest::Mesh {
                        name: nb,
                        faces: fb,
                        base_material: mb,
                        tags: tb,
                    },
                ) => {
                    field(out, &format!("{p}.name"), na, nb);
                    field(out, &format!("{p}.base_material"), ma, mb);
                    field(out, &format!("{p}.tags"), ta, tb);
                    if fa.len() != fb.len() {
                        out.push(format!("{p}.faces: {} vs {}", fa.len(), fb.len()));
                    }
                    if let Some(j) = (0..fa.len().min(fb.len())).find(|&j| fa[j] != fb[j]) {
                        out.push(format!("{p}.faces[{j}]: {:?} vs {:?}", fa[j], fb[j]));
                    }
                }
                (
                    NodeDigest::Group {
                        name: na,
                        children: ca,
                        tags: ta,
                        hidden: ha,
                        keyed: ka,
                    },
                    NodeDigest::Group {
                        name: nb,
                        children: cb,
                        tags: tb,
                        hidden: hb,
                        keyed: kb,
                    },
                ) => {
                    field(out, &format!("{p}.name"), na, nb);
                    field(out, &format!("{p}.tags"), ta, tb);
                    field(out, &format!("{p}.hidden"), ha, hb);
                    field(out, &format!("{p}.keyed"), ka, kb);
                    nodes(out, &format!("{p}.children"), ca, cb);
                }
                (
                    NodeDigest::Instance {
                        def: da,
                        pose: pa,
                        name: na,
                        tags: ta,
                        hidden: ha,
                        keyed: ka,
                    },
                    NodeDigest::Instance {
                        def: db,
                        pose: pb,
                        name: nb,
                        tags: tb,
                        hidden: hb,
                        keyed: kb,
                    },
                ) => {
                    field(out, &format!("{p}.def.name"), &da.0, &db.0);
                    nodes(out, &format!("{p}.def"), &da.1, &db.1);
                    field(out, &format!("{p}.pose"), pa, pb);
                    field(out, &format!("{p}.name"), na, nb);
                    field(out, &format!("{p}.tags"), ta, tb);
                    field(out, &format!("{p}.hidden"), ha, hb);
                    field(out, &format!("{p}.keyed"), ka, kb);
                }
                _ => out.push(format!("{p}: {} vs {}", kind(x), kind(y))),
            }
        }
    }
    list(&mut out, "materials", &a.materials, &b.materials);
    list(&mut out, "tags", &a.tags, &b.tags);
    list(&mut out, "guides", &a.guides, &b.guides);
    field(&mut out, "camera", &a.camera, &b.camera);
    list(&mut out, "scenes", &a.scenes, &b.scenes);
    field(&mut out, "section", &a.section, &b.section);
    if a.defs.len() != b.defs.len() {
        out.push(format!("defs: {} vs {}", a.defs.len(), b.defs.len()));
    }
    for (i, ((na, ca), (nb, cb))) in a.defs.iter().zip(&b.defs).enumerate() {
        field(&mut out, &format!("defs[{i}].name"), na, nb);
        nodes(&mut out, &format!("defs[{i}]"), ca, cb);
    }
    nodes(&mut out, "roots", &a.roots, &b.roots);
    out
}

fn import(name: &str) -> skp_import::SkpScene {
    let out = skp_import::import(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert!(
        !out.warnings
            .iter()
            .any(|w| w.contains("parser recovered") || w.contains("reader skipped")),
        "{name}: a clean corpus file reads without recovery warnings; got {:?}",
        out.warnings
    );
    out
}

// ── The same recipe from either container ─────────────────────────────────

#[test]
fn every_2026_twin_imports_to_the_same_recipe_as_its_2017_original() {
    let mut failures = Vec::new();
    for stem in stems() {
        let a = import(&format!("{stem}.skp"));
        let b = import(&format!("2026/{stem}.skp"));
        let (da, db) = (digest(&a.scene), digest(&b.scene));
        if da != db {
            let lines = diff_digest(&da, &db);
            let shown: Vec<&str> = lines.iter().take(12).map(String::as_str).collect();
            failures.push(format!(
                "{stem}: recipe differs ({} difference(s), 2017 vs 2026):\n  {}",
                lines.len(),
                shown.join("\n  ")
            ));
        }
        if a.textures_missing != b.textures_missing {
            failures.push(format!("{stem}: textures_missing differ"));
        }
        let mut wa = a.warnings.clone();
        let mut wb = b.warnings.clone();
        wa.sort();
        wb.sort();
        if wa != wb {
            failures.push(format!(
                "{stem}: converter warnings differ: {wa:?} vs {wb:?}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "the 2026 container imports differently from the 2017 original:\n{}",
        failures.join("\n")
    );
}

// ── …and the same document ────────────────────────────────────────────────

#[test]
fn every_2026_twin_ingests_to_the_same_document_as_its_2017_original() {
    for stem in stems() {
        let mut docs = Vec::new();
        for name in [format!("{stem}.skp"), format!("2026/{stem}.skp")] {
            let out = import(&name);
            let mut doc = Document::new();
            let (report, _) = doc
                .ingest(out.scene, out.textures_missing)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            docs.push((report, doc));
        }
        let (ra, da) = &docs[0];
        let (rb, db) = &docs[1];
        assert_eq!(ra.objects_created, rb.objects_created, "{stem}: objects");
        assert_eq!(ra.watertight, rb.watertight, "{stem}: watertight");
        assert_eq!(ra.leaky, rb.leaky, "{stem}: leaky");
        assert_eq!(ra.skipped.len(), rb.skipped.len(), "{stem}: skipped");
        assert_eq!(da.instance_ids().len(), db.instance_ids().len(), "{stem}");
        assert_eq!(da.group_ids().len(), db.group_ids().len(), "{stem}");
        assert_eq!(da.component_ids().len(), db.component_ids().len(), "{stem}");
        assert_eq!(da.guide_ids().len(), db.guide_ids().len(), "{stem}");
        assert_eq!(
            da.user_hidden_nodes().len(),
            db.user_hidden_nodes().len(),
            "{stem}"
        );
        assert_eq!(da.scenes().len(), db.scenes().len(), "{stem}");
        assert_eq!(
            da.camera_state().is_some(),
            db.camera_state().is_some(),
            "{stem}"
        );
        let ta: Vec<_> = da.tag_meta().map(|(p, h)| (p.to_vec(), h)).collect();
        let tb: Vec<_> = db.tag_meta().map(|(p, h)| (p.to_vec(), h)).collect();
        assert_eq!(ta, tb, "{stem}: tags");
        assert_eq!(
            da.materials().len(),
            db.materials().len(),
            "{stem}: palette"
        );
        // Both documents round-trip through the native format.
        for (name, doc) in [("2017", da), ("2026", db)] {
            let bytes = doc.save();
            Document::load(&bytes).unwrap_or_else(|e| panic!("{stem} ({name}): {e}"));
        }
    }
}

// ── The 2026 container is read on its own terms ───────────────────────────

#[test]
fn a_2026_file_is_identified_and_read_as_the_zip_container() {
    let bytes = fixture("2026/box.skp");
    let (version, _) = skp_import::skpwright::header_info(&bytes).expect("a SketchUp header");
    assert!(version.starts_with("{26."), "got {version}");
    assert_eq!(
        skp_import::skpwright::detect_container(&bytes),
        skp_import::skpwright::Container::Zip
    );
    let out = import("2026/box.skp");
    let mut doc = Document::new();
    let (report, _) = doc.ingest(out.scene, out.textures_missing).unwrap();
    assert_eq!(report.objects_created, 1);
    assert_eq!(report.watertight, 1);
}

#[test]
fn scenes_and_the_saved_view_arrive_from_the_2026_container() {
    // two-scenes.skp: the same assertions the 2017 spec makes, on the twin.
    let out = import("2026/two-scenes.skp");
    let mut doc = Document::new();
    doc.ingest(out.scene, out.textures_missing).unwrap();
    let cam = doc.camera_state().expect("saved view adopted");
    let inch = skp_import::skpwright::INCH;
    assert!((cam.eye.x - 326.1769714 / inch).abs() < 1e-6);
    assert!((cam.eye.y - -250.1695099 / inch).abs() < 1e-6);
    assert!((cam.eye.z - 154.6123047 / inch).abs() < 1e-6);
    let scenes = doc.scenes();
    let names: Vec<&str> = scenes.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Scene 1", "Scene 2"]);
    assert_eq!(scenes[0].hidden_tags.as_ref().map(|t| t.len()), Some(1));
    assert!(scenes[1].camera.is_some());
}

#[test]
fn scene_hidden_sets_and_section_cuts_arrive_from_both_containers() {
    // feature-pack.skp: two scenes, each cutting on the same active section
    // plane and hiding two entities. Those two are a face and an edge, which
    // Hew has no node for, so both containers must carry the same two
    // unresolvable ids (the recipe test already holds them equal) and the
    // cut must reach every scene identically.
    for name in ["feature-pack.skp", "2026/feature-pack.skp"] {
        let out = import(name);
        assert!(
            !key_labels(&out.scene).is_empty(),
            "{name}: placed objects carry keys"
        );
        assert_eq!(out.scene.scenes.len(), 2, "{name}");
        for s in &out.scene.scenes {
            let keys = s.hidden_keys.as_ref().expect("hidden set captured");
            assert_eq!(keys.len(), 2, "{name} / {}: two hidden entities", s.name);
            let cut = s
                .section
                .as_ref()
                .expect("section captured")
                .as_ref()
                .expect("an active cut");
            assert!(cut.active, "{name} / {}", s.name);
            assert!(
                (cut.normal.y.abs() - 1.0).abs() < 1e-9,
                "{name}: cut normal along Y, got {:?}",
                cut.normal
            );
        }
        let mut doc = Document::new();
        doc.ingest(out.scene, out.textures_missing).unwrap();
        assert_eq!(doc.scenes().len(), 2, "{name}");
        assert!(
            doc.scenes()
                .iter()
                .all(|s| matches!(s.section, Some(Some(ref c)) if c.active)),
            "{name}: each scene keeps its cut"
        );
    }
}

#[test]
fn a_model_level_cut_arrives_cutting_from_both_containers() {
    // section-plane.skp: one active section plane at the root, no scenes.
    // Both containers record which root plane is active, so the cut
    // arrives cutting from either, at the same place.
    let old = import("section-plane.skp");
    let new = import("2026/section-plane.skp");
    let a = old.scene.section.expect("2017: the cut is adopted");
    let b = new.scene.section.expect("2026: the cut is adopted");
    assert!(a.active && b.active, "cutting from both containers");
    assert_eq!(qp(a.origin), qp(b.origin));
    assert_eq!(qv(a.normal), qv(b.normal));
    assert!(
        (a.origin.z - 0.5).abs() < 1e-9,
        "the plane sits at 0.5 m: {:?}",
        a.origin
    );
    // SketchUp keeps the box's lower half (the file's own thumbnail shows
    // it), and a Hew cut's normal points at the half it removes: up.
    assert!(
        (a.normal.z - 1.0).abs() < 1e-9,
        "cutting across Z, removing the upper half: {:?}",
        a.normal
    );
    for (name, out) in [("2017", old), ("2026", new)] {
        let mut doc = Document::new();
        doc.ingest(out.scene, out.textures_missing).unwrap();
        let plane = doc
            .section_plane()
            .unwrap_or_else(|| panic!("{name}: plane adopted"));
        assert!(plane.active, "{name}: the empty document takes the cut");
    }
}

#[test]
fn a_deactivated_plane_yields_to_the_active_one() {
    // section-plane-deactivated.skp (2017): the plane above, deactivated,
    // plus a second plane made active. The active one becomes the cut; the
    // deactivated plane is not it, and is not imported (Hew has one plane).
    let out = import("section-plane-deactivated.skp");
    let cut = out.scene.section.expect("the active plane is adopted");
    assert!(cut.active);
    assert!(
        (cut.normal.x.abs() - 1.0).abs() < 1e-9,
        "the second plane cuts across X: {:?}",
        cut.normal
    );
    assert!(
        (cut.origin.x - 0.635).abs() < 1e-3,
        "about 25 in along X: {:?}",
        cut.origin
    );
}
