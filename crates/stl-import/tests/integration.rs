//! End-to-end import tests (DESIGN §7): run committed STL fixtures through
//! the full parse → heal → `Document::ingest` pipeline and assert the
//! reconstructed solids are editable, correctly counted, and honestly
//! watertight-or-leaky.

use kernel::{Document, ImportNode, MeshRecipe, Point3, WatertightState};
use stl_import::build::build_scene;
use stl_import::parse::RawTriangles;
use stl_import::{StlError, import};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap_or_else(|e| panic!("reading fixture {name}: {e}"))
}

/// `MeshRecipe`s in `roots` (world meshes only — this crate never emits
/// groups/instances/defs).
fn mesh_recipes(roots: &[ImportNode]) -> Vec<&MeshRecipe> {
    roots
        .iter()
        .map(|n| match n {
            ImportNode::Mesh(r) => r,
            _ => panic!("stl-import only ever emits ImportNode::Mesh"),
        })
        .collect()
}

fn bounding_box(positions: &[Point3]) -> (Point3, Point3) {
    let mut min = Point3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut max = Point3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in positions {
        min = Point3::new(min.x.min(p.x), min.y.min(p.y), min.z.min(p.z));
        max = Point3::new(max.x.max(p.x), max.y.max(p.y), max.z.max(p.z));
    }
    (min, max)
}

/// Signed volume of a recipe's faces (divergence-theorem sum, hole loops wind
/// opposite and subtract). For a reconstructed hollow this is the outer volume
/// minus the cavity volume — the material between the walls, not a filled cube.
fn recipe_signed_volume(recipe: &MeshRecipe) -> f64 {
    let mut v6 = 0.0;
    let loops = recipe.faces.iter().enumerate().flat_map(|(fi, face)| {
        std::iter::once(face.clone()).chain(recipe.face_holes.get(fi).cloned().unwrap_or_default())
    });
    for loop_ in loops {
        for i in 1..loop_.len().saturating_sub(1) {
            let a = recipe.positions[loop_[0]].to_vec();
            let b = recipe.positions[loop_[i]].to_vec();
            let c = recipe.positions[loop_[i + 1]].to_vec();
            v6 += a.dot(b.cross(c));
        }
    }
    v6 / 6.0
}

// ── Binary cube ──────────────────────────────────────────────────────────────

#[test]
fn binary_cube_is_one_watertight_object_with_six_ngon_faces() {
    let bytes = fixture("cube_binary.stl");
    let out = import(&bytes, 1.0, None).expect("parse cube_binary.stl");
    assert!(out.missing.is_empty(), "STL never has external resources");
    assert!(
        out.warnings.is_empty(),
        "a clean watertight cube needs no warnings, got {:?}",
        out.warnings
    );

    let recipes = mesh_recipes(&out.scene.roots);
    assert_eq!(recipes.len(), 1, "one shell in, one Object out");
    assert_eq!(recipes[0].positions.len(), 8, "8 welded corners");

    let mut doc = Document::new();
    let (report, _change) = doc.ingest(out.scene, out.missing).expect("ingest");
    assert_eq!(report.objects_created, 1);
    assert_eq!(report.watertight, 1);
    assert_eq!(report.leaky, 0);

    let oid = doc.visible_object_ids()[0];
    let object = doc.object(oid).expect("object exists");
    assert_eq!(object.watertight(), WatertightState::Watertight);
    assert_eq!(object.vertices().len(), 8);
    assert_eq!(
        object.faces().len(),
        6,
        "the 12 triangles must coplanar-merge back into 6 quads"
    );
}

// ── ASCII cube (same geometry) ───────────────────────────────────────────────

#[test]
fn ascii_cube_matches_binary_topology() {
    let binary = import(&fixture("cube_binary.stl"), 1.0, None).expect("parse binary");
    let ascii = import(&fixture("cube_ascii.stl"), 1.0, None).expect("parse ascii");

    let binary_recipes = mesh_recipes(&binary.scene.roots);
    let ascii_recipes = mesh_recipes(&ascii.scene.roots);
    assert_eq!(binary_recipes.len(), ascii_recipes.len());
    let (b, a) = (binary_recipes[0], ascii_recipes[0]);
    assert_eq!(b.positions.len(), a.positions.len());
    assert_eq!(b.faces.len(), a.faces.len());

    // Both encodings must reconstruct byte-identical GEOMETRY, not merely equal
    // counts: the binary and ASCII cube fixtures carry the same coordinates, so
    // detection + both parsers + the shared heal pipeline must agree vertex for
    // vertex and face for face (winding included). Coordinates are exact powers
    // of ten here (no f32 rounding), so exact equality is the right bar.
    let key = |pt: Point3| (pt.x.to_bits(), pt.y.to_bits(), pt.z.to_bits());
    let mut b_pts: Vec<_> = b.positions.iter().map(|&p| key(p)).collect();
    let mut a_pts: Vec<_> = a.positions.iter().map(|&p| key(p)).collect();
    b_pts.sort_unstable();
    a_pts.sort_unstable();
    assert_eq!(
        b_pts, a_pts,
        "welded vertex sets must match coordinate-for-coordinate"
    );

    // Faces compared as sets of world-space corner loops (each recipe may order
    // its own vertex array differently, so compare by resolved coordinates, and
    // canonicalize each loop's rotation while PRESERVING winding direction).
    let face_key = |recipe: &MeshRecipe| -> Vec<Vec<(u64, u64, u64)>> {
        let mut faces: Vec<Vec<(u64, u64, u64)>> = recipe
            .faces
            .iter()
            .map(|f| {
                let loop_pts: Vec<(u64, u64, u64)> =
                    f.iter().map(|&vi| key(recipe.positions[vi])).collect();
                // Rotate so the lexicographically smallest corner is first
                // (winding preserved — no reversal).
                let start = (0..loop_pts.len())
                    .min_by_key(|&i| &loop_pts[i])
                    .unwrap_or(0);
                loop_pts[start..]
                    .iter()
                    .chain(&loop_pts[..start])
                    .copied()
                    .collect()
            })
            .collect();
        faces.sort();
        faces
    };
    assert_eq!(
        face_key(b),
        face_key(a),
        "face loops must match coordinate-for-coordinate, winding included"
    );

    let mut doc = Document::new();
    let (report, _) = doc
        .ingest(ascii.scene, ascii.missing)
        .expect("ingest ascii");
    assert_eq!(report.objects_created, 1);
    assert_eq!(report.watertight, 1);
    assert_eq!(report.leaky, 0);
    let oid = doc.visible_object_ids()[0];
    let object = doc.object(oid).unwrap();
    assert_eq!(object.vertices().len(), 8);
    assert_eq!(object.faces().len(), 6);
}

/// A minimal ASCII STL well under the 84-byte binary-header floor, exercising
/// detection step 1 (`buffer.len() < 84` → try ASCII directly) rather than
/// the exact-size-identity or `facet`-token branches the other fixtures hit.
#[test]
fn tiny_ascii_under_84_bytes_is_detected() {
    let text = b"vertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\n";
    assert!(
        text.len() < 84,
        "fixture must exercise the short-buffer path"
    );
    let out = import(text, 1.0, None).expect("parse tiny ascii");
    let recipes = mesh_recipes(&out.scene.roots);
    assert_eq!(recipes.len(), 1);
    assert_eq!(recipes[0].positions.len(), 3);
}

// ── Two-part file ─────────────────────────────────────────────────────────────

#[test]
fn two_part_file_yields_two_watertight_objects() {
    let bytes = fixture("two_cubes.stl");
    let out = import(&bytes, 1.0, None).expect("parse two_cubes.stl");
    let recipes = mesh_recipes(&out.scene.roots);
    assert_eq!(
        recipes.len(),
        2,
        "two disjoint shells must stay two Objects"
    );
    for r in &recipes {
        assert_eq!(
            r.positions.len(),
            8,
            "each cube welds to 8 corners independently"
        );
    }
    // Names distinguish the pieces per DESIGN §3: "Imported", "Imported (2)".
    let names: Vec<&str> = recipes.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"Imported"));
    assert!(names.contains(&"Imported (2)"));

    let mut doc = Document::new();
    let (report, _) = doc.ingest(out.scene, out.missing).expect("ingest");
    assert_eq!(report.objects_created, 2);
    assert_eq!(report.watertight, 2);
    assert_eq!(report.leaky, 0);
}

// ── Naming from the file stem ─────────────────────────────────────────────────

/// A `name_hint` (the UI passes the file stem) names the Objects: a single
/// Object takes the bare stem, multiples get " (2)", " (3)" suffixes.
#[test]
fn name_hint_names_objects_from_the_file_stem() {
    // Single-object file → bare stem.
    let single = import(&fixture("cube_binary.stl"), 1.0, Some("bunny")).expect("import");
    let names: Vec<&str> = mesh_recipes(&single.scene.roots)
        .iter()
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(names, vec!["bunny"]);

    // Multi-object file → stem + numbered suffixes.
    let multi = import(&fixture("two_cubes.stl"), 1.0, Some("plate")).expect("import");
    let names: Vec<&str> = mesh_recipes(&multi.scene.roots)
        .iter()
        .map(|r| r.name.as_str())
        .collect();
    assert!(names.contains(&"plate"));
    assert!(names.contains(&"plate (2)"));

    // None or a blank hint falls back to "Imported".
    for hint in [None, Some(""), Some("   ")] {
        let out = import(&fixture("cube_binary.stl"), 1.0, hint).expect("import");
        assert_eq!(mesh_recipes(&out.scene.roots)[0].name, "Imported");
    }
}

// ── Open mesh (leaky, honest) ────────────────────────────────────────────────

#[test]
fn open_mesh_arrives_leaky_not_refused() {
    let bytes = fixture("cube_open.stl");
    let out = import(&bytes, 1.0, None).expect("an open mesh must still import (rule 4)");
    // The leaky state is reported authoritatively by ImportReport.leaky (the
    // badge the UI shows), NOT by a crate warning — the crate never emits a
    // second, possibly-divergent leaky count of its own. A clean single open
    // shell therefore produces no warnings (no non-manifold split, no
    // enclosure); the honesty lives entirely in report.leaky below.
    assert!(
        out.warnings.is_empty(),
        "an open shell needs no crate warning (leaky is an ImportReport count), got {:?}",
        out.warnings
    );

    let recipes = mesh_recipes(&out.scene.roots);
    assert_eq!(recipes.len(), 1);

    let mut doc = Document::new();
    let (report, _) = doc.ingest(out.scene, out.missing).expect("ingest");
    assert_eq!(report.objects_created, 1);
    assert_eq!(
        report.watertight, 0,
        "missing a facet must NOT read as closed"
    );
    assert_eq!(report.leaky, 1);
    assert!(
        report.skipped.is_empty(),
        "the open shell must be imported, never skipped"
    );

    let oid = doc.visible_object_ids()[0];
    let object = doc.object(oid).unwrap();
    assert_eq!(object.watertight(), WatertightState::Open);
    // Rule 4: the gap is real and visible, never patched shut. Only one
    // facet (one triangle) was dropped from the fixture, so 5 of the 6 cube
    // faces still coplanar-merge into quads; the 6th keeps its lone
    // surviving triangle (its former partner is the missing facet) —
    // 5 quads + 1 triangle = 6 faces, with a real boundary gap where the
    // triangle's missing neighbor used to be.
    assert_eq!(object.faces().len(), 6);
}

// ── Detection edge case ──────────────────────────────────────────────────────

/// A binary file whose 80-byte header begins with the literal ASCII text
/// "solid" must still be detected as binary via the size identity, not
/// misread as ASCII because of the leading keyword (DESIGN §2).
#[test]
fn solid_prefixed_header_still_detected_as_binary() {
    let bytes = fixture("solid_header_binary.stl");
    assert!(
        bytes.starts_with(b"solid"),
        "fixture must actually start with the misleading ASCII keyword"
    );
    let out = import(&bytes, 1.0, None).expect("parse solid_header_binary.stl");
    let recipes = mesh_recipes(&out.scene.roots);
    assert_eq!(recipes.len(), 1);
    assert_eq!(
        recipes[0].positions.len(),
        8,
        "detected as the 8-corner binary cube, not garbled ASCII"
    );

    let mut doc = Document::new();
    let (report, _) = doc.ingest(out.scene, out.missing).expect("ingest");
    assert_eq!(report.watertight, 1);
    assert_eq!(report.leaky, 0);
}

// ── Unit scaling ──────────────────────────────────────────────────────────────

#[test]
fn unit_scale_0_001_on_10_unit_cube_yields_10mm_bounding_box() {
    let bytes = fixture("cube_binary.stl"); // a 10-unit cube
    let out = import(&bytes, 0.001, None).expect("parse with mm unit scale");
    let recipes = mesh_recipes(&out.scene.roots);
    let (min, max) = bounding_box(&recipes[0].positions);
    let extent = max - min;
    const TOL: f64 = 1e-9;
    assert!((extent.x - 0.01).abs() < TOL, "x extent = {}", extent.x);
    assert!((extent.y - 0.01).abs() < TOL, "y extent = {}", extent.y);
    assert!((extent.z - 0.01).abs() < TOL, "z extent = {}", extent.z);
}

// ── Empty / garbage ───────────────────────────────────────────────────────────

#[test]
fn empty_binary_file_is_empty_error() {
    let bytes = fixture("empty_binary.stl");
    assert_eq!(
        bytes.len(),
        84,
        "an 84-byte file is exactly header+count, zero triangles"
    );
    match import(&bytes, 1.0, None) {
        Err(StlError::Empty) => {}
        Err(StlError::Parse(msg)) => panic!("expected StlError::Empty, got Parse({msg})"),
        Ok(_) => panic!("expected StlError::Empty, got Ok"),
    }
}

#[test]
fn empty_ascii_file_is_empty_error() {
    let bytes = fixture("empty_ascii.stl");
    match import(&bytes, 1.0, None) {
        Err(StlError::Empty) => {}
        Err(StlError::Parse(msg)) => panic!("expected StlError::Empty, got Parse({msg})"),
        Ok(_) => panic!("expected StlError::Empty, got Ok"),
    }
}

#[test]
fn garbage_file_is_parse_error() {
    let bytes = fixture("garbage.stl");
    assert!(
        bytes.len() < 84,
        "fixture must exercise the short-buffer/ASCII-fails path"
    );
    match import(&bytes, 1.0, None) {
        Err(StlError::Parse(_)) => {}
        Err(StlError::Empty) => panic!("expected StlError::Parse, got Empty"),
        Ok(_) => panic!("expected StlError::Parse, got Ok"),
    }
}

/// A failed import must never partially populate a document — `import()`
/// returns before any `Document::ingest` call happens, so this is really
/// asserting the crate's own contract (Err means no `StlScene` at all), but
/// it's worth pinning explicitly since it's the whole reason `import` is a
/// pure function with no side effects.
#[test]
fn garbage_does_not_produce_a_scene() {
    let bytes = fixture("garbage.stl");
    assert!(import(&bytes, 1.0, None).is_err());
}

// ── Truncated / trailing-junk binary (parse lenient fallback) ─────────────────

/// A binary STL that declares more triangles than its bytes contain (a
/// truncated download) must import the complete records that ARE present,
/// loudly (a `warnings` note), rather than failing — DEVELOPMENT.md rule 4.
#[test]
fn truncated_binary_imports_available_triangles() {
    // Chop the last two triangle records (100 bytes) off the intact cube; the
    // header's declared count now overstates what remains.
    let mut bytes = fixture("cube_binary.stl");
    bytes.truncate(bytes.len() - 100);
    let out =
        import(&bytes, 1.0, None).expect("a truncated binary STL still imports what survived");
    assert!(
        out.warnings.iter().any(|w| w.contains("truncated")),
        "truncation must be reported, got {:?}",
        out.warnings
    );
    // 10 of the 12 cube triangles remain — an open shell now, but imported.
    let mut doc = Document::new();
    let (report, _) = doc.ingest(out.scene, out.missing).expect("ingest");
    assert!(report.objects_created >= 1);
    assert_eq!(
        report.watertight, 0,
        "a cube missing two facets is not closed"
    );
}

/// Trailing junk after the declared triangle count (a common exporter quirk) is
/// tolerated: the declared triangles import, the extra bytes are ignored with a
/// warning, never a parse failure.
#[test]
fn trailing_junk_binary_imports_and_warns() {
    let mut bytes = fixture("cube_binary.stl");
    bytes.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x11]); // 6 stray bytes
    let out = import(&bytes, 1.0, None).expect("trailing junk must not fail the parse");
    assert!(!out.warnings.is_empty(), "trailing junk must be reported");
    let mut doc = Document::new();
    let (report, _) = doc.ingest(out.scene, out.missing).expect("ingest");
    assert_eq!(report.objects_created, 1, "the full cube still imports");
    assert_eq!(report.watertight, 1);
}

// ── Hollow / nested shells reconstruct as ONE Object with a cavity ────────────

/// A hollow single-part STL — an outer wall plus a fully-enclosed, vertex-
/// disjoint inner wall — reconstructs into ONE watertight Object whose inner
/// shell is a cavity (a void in the material), NOT two independent solids and
/// NOT a filled cube. The signed volume proves it: it is the shell-between
/// (outer − cavity), not the outer's full volume.
#[test]
fn hollow_shell_reconstructs_as_one_object_with_a_cavity() {
    // Fixture: outer cube edge 20, inner cube edge 8 (at 6,6,6) — both closed.
    let bytes = fixture("nested_cubes.stl");
    let out = import(&bytes, 1.0, None).expect("hollow cube imports");
    assert!(
        out.warnings.is_empty(),
        "a reconstructed hollow is correct behavior, not a warning: {:?}",
        out.warnings
    );

    let recipes = mesh_recipes(&out.scene.roots);
    assert_eq!(recipes.len(), 1, "outer + cavity are ONE Object");

    // Signed volume = 20³ − 8³ = 8000 − 512 = 7488 (the material between the
    // walls), NOT the solid 8000 a filled outer cube would give.
    let vol = recipe_signed_volume(recipes[0]);
    assert!(
        (vol - 7488.0).abs() < 1e-6,
        "hollow signed volume must be outer − cavity (7488), got {vol}"
    );

    let mut doc = Document::new();
    let (report, _) = doc.ingest(out.scene, out.missing).expect("ingest");
    assert_eq!(report.objects_created, 1);
    assert_eq!(
        report.watertight, 1,
        "a closed shell-with-cavity is watertight"
    );
    assert_eq!(report.leaky, 0);
    assert!(report.skipped.is_empty());
}

/// A cube-in-a-cavity-in-a-cube: the outer solid gets its cavity (odd nesting
/// depth), and the solid ISLAND floating inside that cavity (even depth) splits
/// out as its own discrete solid Object. Two Objects: one hollow, one solid.
#[test]
fn island_inside_cavity_is_its_own_solid() {
    // Fixture: outer edge 40, cavity edge 24 (at 8), island edge 8 (at 16).
    let bytes = fixture("cube_in_cavity_in_cube.stl");
    let out = import(&bytes, 1.0, None).expect("nested triple imports");
    let recipes = mesh_recipes(&out.scene.roots);
    assert_eq!(
        recipes.len(),
        2,
        "outer-with-cavity + island = two Objects, got {}",
        recipes.len()
    );

    // One recipe is the hollow (outer − cavity = 40³ − 24³ = 64000 − 13824 =
    // 50176); the other is the solid island (8³ = 512).
    let mut vols: Vec<f64> = recipes.iter().map(|r| recipe_signed_volume(r)).collect();
    vols.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert!(
        (vols[0] - 512.0).abs() < 1e-6,
        "island solid volume 512, got {}",
        vols[0]
    );
    assert!(
        (vols[1] - 50176.0).abs() < 1e-6,
        "hollow volume 50176, got {}",
        vols[1]
    );

    let mut doc = Document::new();
    let (report, _) = doc.ingest(out.scene, out.missing).expect("ingest");
    assert_eq!(report.objects_created, 2);
    assert_eq!(
        report.watertight, 2,
        "both the hollow and the island are closed"
    );
    assert_eq!(report.leaky, 0);
    assert!(report.skipped.is_empty());
}

/// Two side-by-side hollow parts stay TWO hollow Objects (disjoint bboxes, so
/// neither's cavity is mis-assigned to the other's outer shell).
#[test]
fn two_side_by_side_hollows_stay_two_hollow_objects() {
    let bytes = fixture("two_hollow_parts.stl");
    let out = import(&bytes, 1.0, None).expect("two hollows import");
    let recipes = mesh_recipes(&out.scene.roots);
    assert_eq!(recipes.len(), 2, "two hollow parts → two Objects");
    for r in &recipes {
        let vol = recipe_signed_volume(r);
        assert!(
            (vol - 7488.0).abs() < 1e-6,
            "each part is a hollow (7488), not a solid 8000: got {vol}"
        );
    }

    let mut doc = Document::new();
    let (report, _) = doc.ingest(out.scene, out.missing).expect("ingest");
    assert_eq!(report.objects_created, 2);
    assert_eq!(report.watertight, 2);
    assert_eq!(report.leaky, 0);
    assert!(report.skipped.is_empty());
}

/// A plain solid (no nesting) is unchanged by the reconstruction pass: one
/// watertight Object at its full volume.
#[test]
fn plain_solid_is_unchanged_by_reconstruction() {
    let bytes = fixture("cube_binary.stl"); // a single 10-edge solid cube
    let out = import(&bytes, 1.0, None).expect("solid cube imports");
    let recipes = mesh_recipes(&out.scene.roots);
    assert_eq!(recipes.len(), 1);
    let vol = recipe_signed_volume(recipes[0]);
    assert!(
        (vol - 1000.0).abs() < 1e-6,
        "a filled 10-cube has volume 1000, got {vol}"
    );
    assert!(out.warnings.is_empty());
}

/// An OPEN (leaky) shell nested inside a closed solid must NOT be flipped and
/// merged as a cavity — that would silently absorb the good outer solid and
/// mislabel it leaky, hiding the actually-broken inner part. The containment
/// gate requires the CANDIDATE to be closed too: the open inner imports as its
/// own leaky Object, the outer stays a watertight solid.
#[test]
fn open_nested_shell_is_not_merged_as_a_cavity() {
    // Fixture: closed outer cube [0,10]³ + nested OPEN box [3,7]³ (missing a facet).
    let bytes = fixture("open_nested_in_solid.stl");
    let out = import(&bytes, 1.0, None).expect("import");
    let recipes = mesh_recipes(&out.scene.roots);
    assert_eq!(
        recipes.len(),
        2,
        "outer solid + open inner stay TWO Objects, never one merged cavity"
    );

    let mut doc = Document::new();
    let (report, _) = doc.ingest(out.scene, out.missing).expect("ingest");
    assert_eq!(report.objects_created, 2);
    assert_eq!(
        report.watertight, 1,
        "the outer solid must stay watertight, not be corrupted by the open inner"
    );
    assert_eq!(
        report.leaky, 1,
        "the open inner is its own honestly-leaky Object"
    );
    assert!(report.skipped.is_empty());
}

/// A shell whose bounding box fits inside a curved container but whose geometry
/// STRADDLES the container's true boundary (they intersect) must NOT be merged:
/// a single-probe containment test would fuse two intersecting shells into a
/// nonsense self-intersecting "watertight" solid. The all-vertices gate leaves
/// them as two separate Objects — the honest, conservative result.
#[test]
fn straddling_shell_in_curved_container_is_not_merged() {
    // Fixture: octahedron r=10 (bbox [-10,10]³, but true volume far smaller) +
    // cube [2,5]³ — its bbox is inside the octahedron's, but corner (5,5,5)
    // has x+y+z = 15 > 10, so it pokes out through the +++ face (straddles).
    let bytes = fixture("octahedron_straddle.stl");
    let out = import(&bytes, 1.0, None).expect("import");
    let recipes = mesh_recipes(&out.scene.roots);
    assert_eq!(
        recipes.len(),
        2,
        "the straddling cube must NOT merge into the octahedron; two Objects"
    );

    let mut doc = Document::new();
    let (report, _) = doc.ingest(out.scene, out.missing).expect("ingest");
    assert_eq!(report.objects_created, 2);
    assert_eq!(
        report.watertight, 2,
        "both stay their own closed Objects — no fabricated self-intersecting hollow"
    );
    assert!(report.skipped.is_empty());
}

// ── Near-degenerate sliver does not skip the whole object (3DBenchy blocker) ──

/// A single near-degenerate sliver triangle in an otherwise-good mesh must NOT
/// skip the ENTIRE object at ingest — the real-world blocker a 3DBenchy-class
/// STL (~110k faces, routinely a few slivers) hits. `mesh_heal`'s sliver gate
/// is a HEIGHT test scaled to the source's coincidence precision (`weld_tol`),
/// but the kernel's `Plane::from_polygon` degeneracy gate is an ABSOLUTE
/// Newell-normal floor (`tol::NORMALIZE_MIN_LENGTH`); a micro-sliver can clear
/// the height gate yet trip the kernel's, and `Document::ingest` is
/// all-or-nothing per object — so one bad face would drop the whole model.
/// The healer now removes any kernel-degenerate face itself.
///
/// Fixture: a watertight edge-20 cube sharing its origin corner with a ~5µm
/// sliver whose 2·area is below `NORMALIZE_MIN_LENGTH` at mm import scale but
/// whose height exceeds the weld-tolerance floor — the exact mismatch.
#[test]
fn near_degenerate_sliver_does_not_skip_the_whole_object() {
    let bytes = fixture("sliver_degenerate.stl");

    // The sliver really IS kernel-degenerate at the mm import scale — proving
    // the fixture exercises the exact gate (a `Plane::from_polygon` rejection),
    // not some other filter.
    let (raw, _w) = stl_import::parse::parse(&bytes).expect("parse");
    let kernel_degenerate = raw
        .faces
        .iter()
        .filter(|f| {
            let pts: Vec<Point3> = f
                .iter()
                .map(|&i| {
                    let p = raw.positions[i];
                    Point3::new(p.x * 0.001, p.y * 0.001, p.z * 0.001)
                })
                .collect();
            kernel::Plane::from_polygon(&pts).is_err()
        })
        .count();
    assert_eq!(
        kernel_degenerate, 1,
        "the fixture must contain exactly one kernel-degenerate sliver"
    );

    // Import at mm: the object comes through, the sliver is dropped, nothing
    // skipped. The cube stays watertight (dropping a vertex-only sliver leaves
    // the cube's edges intact).
    let out = import(&bytes, 0.001, Some("part")).expect("import");
    let mut doc = Document::new();
    let (report, _) = doc.ingest(out.scene, out.missing).expect("ingest");
    assert!(
        report.objects_created >= 1,
        "one bad sliver must not skip the whole object; got 0 created"
    );
    assert!(
        report.skipped.is_empty(),
        "the degenerate sliver is dropped at the import boundary, not skipped at ingest: {:?}",
        report
            .skipped
            .iter()
            .map(|s| s.reason.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        report.watertight, 1,
        "the cube survives intact and watertight"
    );

    // The drop is never silent: even though the cube stays watertight here
    // (the sliver only shared a vertex, so no shell opened), the import warns
    // that sliver geometry was removed.
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("degenerate sliver face")),
        "removing a kernel-degenerate face must be reported in the import warnings: {:?}",
        out.warnings
    );
}

/// A DISCONNECTED shell made entirely of kernel-degenerate slivers must still
/// report its drop — the shell heals to nothing and is filtered out, but its
/// count reaches the warning instead of the whole shell vanishing without a
/// trace (the count used to be discarded when a fully-degenerate shell
/// returned empty from heal).
///
/// Geometry (import scale 1.0, all near the origin so f32 holds the tiny
/// offsets and the per-shell weld tolerance sits at its 1e-7 floor): an
/// isolated triangle with base 8e-6 and height 1.2e-7 — effective height
/// ABOVE the weld-scaled zero-area gate (1.2e-7 > 1e-7) so it survives to the
/// kernel-degeneracy filter, whose absolute Newell floor it trips
/// (2·area = 9.6e-13 < NORMALIZE_MIN_LENGTH 1e-12). A small watertight cube
/// (edge 0.02, disjoint from the sliver) rides along to prove the good
/// geometry is unaffected.
#[test]
fn fully_degenerate_isolated_shell_still_reports_its_drop() {
    fn tri(bytes: &mut Vec<u8>, a: [f32; 3], b: [f32; 3], c: [f32; 3]) {
        bytes.extend_from_slice(&[0u8; 12]); // normal (ignored by the parser)
        for v in [a, b, c] {
            for x in v {
                bytes.extend_from_slice(&x.to_le_bytes());
            }
        }
        bytes.extend_from_slice(&[0u8; 2]); // attribute byte count
    }

    // Cube corners (edge 0.02, offset well away from the sliver).
    let lo = 0.02f32;
    let hi = 0.04f32;
    let p = |x: f32, y: f32, z: f32| [x, y, z];
    let mut body: Vec<u8> = Vec::new();
    let mut count = 0u32;
    // 12 cube triangles (outward wound; heal re-orients regardless).
    let corners = [
        p(lo, lo, lo),
        p(hi, lo, lo),
        p(hi, hi, lo),
        p(lo, hi, lo),
        p(lo, lo, hi),
        p(hi, lo, hi),
        p(hi, hi, hi),
        p(lo, hi, hi),
    ];
    let quads: [[usize; 4]; 6] = [
        [0, 3, 2, 1], // bottom
        [4, 5, 6, 7], // top
        [0, 1, 5, 4], // front
        [2, 3, 7, 6], // back
        [1, 2, 6, 5], // right
        [3, 0, 4, 7], // left
    ];
    for q in quads {
        tri(&mut body, corners[q[0]], corners[q[1]], corners[q[2]]);
        tri(&mut body, corners[q[0]], corners[q[2]], corners[q[3]]);
        count += 2;
    }
    // The isolated kernel-degenerate sliver near the origin.
    tri(
        &mut body,
        p(0.0, 0.0, 0.0),
        p(8e-6, 0.0, 0.0),
        p(4e-6, 1.2e-7, 0.0),
    );
    count += 1;

    let mut bytes: Vec<u8> = vec![0u8; 80];
    bytes.extend_from_slice(&count.to_le_bytes());
    bytes.extend_from_slice(&body);

    let out = import(&bytes, 1.0, Some("part")).expect("import");
    let mut doc = Document::new();
    let (report, _) = doc.ingest(out.scene, out.missing).expect("ingest");
    assert_eq!(report.objects_created, 1, "only the cube imports");
    assert_eq!(report.watertight, 1, "the cube is untouched and watertight");
    assert!(report.skipped.is_empty());
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("1 degenerate sliver face")),
        "a fully-degenerate shell must still report its dropped face: {:?}",
        out.warnings
    );
}

// ── Non-manifold decomposition survives to the kernel (BLOCKER regression) ────

/// A non-manifold single part must decompose into pieces that ALL reach the
/// kernel — none silently dropped. This guards the original BLOCKER: healing a
/// shell BEFORE (never after) `split_non_manifold` means the split's output —
/// including its coincident-vertex pinch fix — is handed to `Document::ingest`
/// verbatim, never re-welded. A re-weld after the split collapses coincident
/// duplicates back together, recreates the non-manifold edge, and gets pieces
/// silently skipped (a rule-4 geometry-loss violation).
///
/// The fixture is two solid tetrahedra glued at a shared face with the SAME
/// winding — a genuinely non-manifold "bowtie" join (its three shared edges are
/// each traversed four times). `mesh-heal`'s own `pinch_falls_back_to_vertex_split`
/// fixture is a pair of self-intersecting (degenerate) quads the kernel rejects
/// on geometry regardless of any weld, so it cannot serve as an "imported"
/// fixture; this valid solid bowtie is the closest importable analog that
/// exercises `split_non_manifold`'s Some-path end-to-end.
#[test]
fn non_manifold_part_decomposes_and_no_piece_is_dropped() {
    // Two tets over a shared triangle (0,1,2), apex 3 above and apex 4 below,
    // with the shared base face present in BOTH tets wound the same way.
    let positions = vec![
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
        Point3::new(0.3, 0.3, 1.0),
        Point3::new(0.3, 0.3, -1.0),
    ];
    let tris = [
        [0, 1, 2],
        [0, 3, 1],
        [1, 3, 2],
        [2, 3, 0], // upper tet
        [0, 1, 2],
        [0, 4, 1],
        [1, 4, 2],
        [2, 4, 0], // lower tet (base wound the same way)
    ];
    let raw = RawTriangles {
        positions,
        faces: tris.iter().map(|t| t.to_vec()).collect(),
    };
    let (scene, warnings) = build_scene(raw, 1.0, None);
    assert!(
        warnings.iter().any(|w| w.contains("non-manifold")),
        "the split must be reported, got {warnings:?}"
    );

    // Total faces the recipes carry BEFORE ingest — none may be lost at ingest.
    let recipe_faces: usize = scene
        .roots
        .iter()
        .map(|n| match n {
            ImportNode::Mesh(r) => r.faces.len(),
            _ => 0,
        })
        .sum();
    assert!(recipe_faces >= 1);

    let mut doc = Document::new();
    let (report, _) = doc.ingest(scene, Vec::new()).expect("ingest");
    // Every decomposed piece is imported — nothing skipped (the BLOCKER).
    let skipped_reasons: Vec<&str> = report.skipped.iter().map(|s| s.reason.as_str()).collect();
    assert_eq!(
        report.skipped.len(),
        0,
        "a split piece was silently dropped: {skipped_reasons:?}"
    );
    assert!(
        report.objects_created >= 2,
        "the non-manifold part must decompose into multiple Objects, got {}",
        report.objects_created
    );
    // Open shells (the cut seams are honest boundaries), never fabricated solid.
    assert_eq!(report.leaky, report.objects_created);
    assert_eq!(report.watertight, 0);

    // Face conservation: the kernel Objects hold exactly the faces the recipes
    // handed over — ingest dropped none.
    let object_faces: usize = doc
        .visible_object_ids()
        .iter()
        .filter_map(|&id| doc.object(id))
        .map(|o| o.faces().len())
        .sum();
    assert_eq!(
        object_faces, recipe_faces,
        "ingest must import every split-piece face, none dropped"
    );
}
