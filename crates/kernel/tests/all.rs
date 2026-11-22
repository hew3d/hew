//! The kernel's integration tests, built as ONE test binary.
//!
//! Each `tests/*.rs` file used to be its own binary: well over a hundred
//! links on every kernel change, and a test run that went through them one
//! at a time. As modules of a single binary they link once and the whole
//! suite runs in parallel across every core.
//!
//! To add a spec file, create `tests/<name>.rs` and declare it below. The
//! test at the bottom fails if a `.rs` file directly in this directory is
//! not declared, so a new spec file cannot sit here unbuilt and unrun. (It
//! does not look inside subdirectories; those are helper modules, reached
//! through the `mod` that owns them.) `crates/kernel/Cargo.toml`
//! turns target auto-discovery off for the same reason.
//!
//! Run one file's tests with its module path as the filter:
//!
//! ```text
//! cargo test -p kernel --test all deform_specs::
//! ```
//!
//! The filter is a substring match, so `document_specs::` also runs
//! `blend_document_specs::`. Add `--exact` with a full test path when only
//! one test is wanted.
//!
//! Shared helpers (`common`, `face_arrangement_support`) are declared once
//! here; a spec file imports them with `use crate::common;`.
//!
//! This file must not be named `main.rs` or `lib.rs`: proptest looks for
//! either next to a source file to decide where failure seeds live, and the
//! `<name>.proptest-regressions` files beside each spec depend on it finding
//! neither.

mod common;
mod face_arrangement_support;
mod lock_oracle;

mod arc_on_face_specs;
mod asset_lifecycle_specs;
mod attr_specs;
mod blend_corner_specs;
mod blend_document_specs;
mod blend_edit_specs;
mod blend_ext_specs;
mod blend_props;
mod blend_record_specs;
mod blend_rim_specs;
mod blend_specs;
mod blend_swept_edit_specs;
mod blend_swept_limit_specs;
mod blend_swept_symmetry_specs;
mod boolean_imported_planarity_specs;
mod boolean_many_specs;
mod boolean_near_coplanar_repro;
mod boolean_sliver_specs;
mod boolean_t_junction_specs;
mod boolean_tidy_specs;
mod canonical_uv_order_spec;
mod cell_solid_specs;
mod changes;
mod circle_on_face_edge_specs;
mod clipboard_drawing_specs;
mod component_edit_k1_specs;
mod component_edit_k2_specs;
mod compound;
mod compute_specs;
mod curves_identity_specs;
mod def_member_transform_specs;
mod definition_delete_undo_specs;
mod deform_specs;
mod detached_sketch_specs;
mod determinism;
mod doc_fuzz_repro_union_imprint_merge;
mod doc_replay_diverge_repro;
mod doc_save_load_hash_repro;
mod document_fuzz;
mod document_fuzz_repro_boolean_staple;
mod document_id;
mod document_specs;
mod draft_specs;
mod edge_imprint_and_push_specs;
mod effective_hidden;
mod explode_session_specs;
mod face_arrangement_specs;
mod face_arrangement_stage2_specs;
mod face_arrangement_stage3_specs;
mod face_arrangement_stage4_specs;
mod face_arrangement_stage4b_specs;
mod face_edge_attrs;
mod face_plane_specs;
mod faceted_carrier_booleans;
mod flip_specs;
mod follow_me_twist_specs;
mod format_v9_specs;
mod geo_specs;
mod golden_file;
mod group_session_specs;
mod group_sketch_specs;
mod health_specs;
mod history_budget_specs;
mod import_extra_specs;
mod import_specs;
mod imported_mark_specs;
mod imprint_edit_specs;
mod insert_definition;
mod instance_materials;
mod kb25_specs;
mod library_specs;
mod library_update_specs;
mod linked_copies_specs;
mod lock_enforcement_specs;
mod lock_specs;
mod loft_specs;
mod make_solid_faces_specs;
mod make_unique_by_path_specs;
mod mend_specs;
mod nested_component_specs;
mod nested_serialize_specs;
mod object_measure;
mod occurrences;
mod offset_specs;
mod op_fuzz;
mod op_fuzz_repro_guard_regime_undo;
mod op_fuzz_repro_imprint_neighbor_push_undo;
mod op_fuzz_repro_imprint_pushpull_undo;
mod op_fuzz_repro_merge_heal_redo;
mod op_fuzz_repro_nonquad_wall_undo;
mod op_fuzz_repro_pushpull_split_split;
mod op_fuzz_repro_replay_drift;
mod op_fuzz_repro_ring_outruns_boundary;
mod op_fuzz_repro_split_pushpull;
mod op_fuzz_repro_split_pushpull_split;
mod op_fuzz_repro_split_split;
mod op_fuzz_repro_ulp_edge_undo;
mod op_specs;
mod owned_sketch_specs;
mod patch_specs;
mod path_array_specs;
mod plane_units_scale_specs;
mod project_specs;
mod props;
mod push_pull_fold_guard_specs;
mod rebase;
mod reflection_specs;
mod region_consume_undo_reversal_property_specs;
mod region_consume_undo_reversal_specs;
mod rejoin_solid_specs;
mod repair_fuzz;
mod repair_specs;
mod reshape_specs;
mod revolve_specs;
mod saved_mark_specs;
mod scan_seam_specs;
mod scenes_specs;
mod serialize_specs;
mod session_draft_scope_specs;
mod session_fuzz;
mod shell_specs;
mod sid_specs;
mod sketch_corner_specs;
mod sketch_solid_model_specs;
mod sketch_specs;
mod smooth_specs;
mod smoothness_specs;
mod soften_specs;
mod split_face_hole_reassign_repro;
mod split_face_lens_orientation_repro;
mod split_face_notch_chord_repro;
mod split_face_same_edge_repro;
mod station_specs;
mod subdiv_specs;
mod surface_class_specs;
mod surface_ref_specs;
mod tag_many_specs;
mod tag_rename_specs;
mod takes_part_specs;
mod tangent_loop_specs;
mod transaction_specs;
mod transform_exact_specs;
mod triangulate_props;
mod undo_attr_restore_specs;
mod unit_key_specs;
mod uv_frame_transform_specs;
mod wire_faces_specs;

/// Every `tests/*.rs` file is declared above. Auto-discovery is off, so a
/// file that is not declared would never be compiled, and its tests would
/// pass by not existing.
#[test]
fn every_spec_file_in_this_directory_is_declared() {
    let declared: std::collections::BTreeSet<&str> = include_str!("all.rs")
        .lines()
        .filter_map(|line| line.strip_prefix("mod ")?.strip_suffix(';'))
        .collect();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut undeclared = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            let name = path.file_stem().unwrap().to_str().unwrap().to_owned();
            if name != "all" && !declared.contains(name.as_str()) {
                undeclared.push(name);
            }
        }
    }
    undeclared.sort();
    assert!(
        undeclared.is_empty(),
        "tests/{{{}}}.rs exist but are not declared in tests/all.rs, so they never run",
        undeclared.join(",")
    );
}
