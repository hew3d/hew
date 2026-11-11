//! Mend on the real 3DBenchy, through the real import. The file is not in
//! the repository: set `HEW_BENCHY_STL` to the public `3DBenchy.stl` and
//! run with `--ignored`.

use kernel::Document;
use kernel::repair::{MendPlan, default_weld_distance};

#[test]
#[ignore]
fn benchy_mends_to_a_solid_and_the_preview_counts_what_the_mend_does() {
    let path = std::env::var("HEW_BENCHY_STL").expect("set HEW_BENCHY_STL to 3DBenchy.stl");
    let bytes = std::fs::read(path).unwrap();
    let scene = stl_import::import(&bytes, 0.001, Some("3DBenchy")).unwrap();
    let mut doc = Document::new();
    doc.ingest(scene.scene, scene.missing).unwrap();
    let id = doc.visible_object_ids()[0];
    let planes_before = doc.object_drawing_planes(id).len();
    let plan = MendPlan::all(default_weld_distance(doc.object(id).unwrap()));
    let census = doc.mend_census(id, &plan).unwrap();
    let before = doc.state_hash();
    let (report, pieces, _) = doc.mend(id, &plan).unwrap();
    assert!(report.closed && pieces.is_empty(), "{report:?}");
    assert_eq!(census.weld_vertices, report.welded_vertices);
    assert_eq!(census.collapsed_faces, report.collapsed_faces);
    assert_eq!(census.folds, report.folds_removed);
    assert_eq!(census.branching_vertices, report.split_vertices);
    assert_eq!(census.split_edges, report.split_edges);
    assert_eq!(census.islands - 1, report.pieces);
    // The Mend adds no drawing: the same planes carry drawing after.
    assert_eq!(doc.object_drawing_planes(id).len(), planes_before);
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before);
}
