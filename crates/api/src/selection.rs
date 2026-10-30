//! The selection algebra (docs/agents/HEW_API.md §7, "Selection and view
//! state"): how a pick combines with the current selection. The one rule a
//! host applies for `hew.selection.set`, kept here so every host agrees
//! with the app's own `mergeSelection` (`app/src/panels/treeModel.ts`).

use std::collections::BTreeSet;

use crate::host::{SelectMode, SelectionEntry};

/// Combines `picked` with `current` by `mode`, the app's `mergeSelection`:
///
/// - `Replace`: the picked entries.
/// - `Add`: the current entries, then the picked ones not yet selected, in
///   pick order.
/// - `Subtract`: the current entries without the picked ones.
/// - `Toggle`: the current entries without the picked ones, then the picked
///   ones that were not selected before, in pick order.
///
/// Survivors keep their order; index 0 is the primary pick. Entries are
/// equal by kind and id. An entry picked twice counts once (its first
/// occurrence), so no mode ever produces a duplicate.
pub fn merge(
    current: &[SelectionEntry],
    picked: &[SelectionEntry],
    mode: SelectMode,
) -> Vec<SelectionEntry> {
    let mut seen: BTreeSet<SelectionEntry> = BTreeSet::new();
    let picked: Vec<SelectionEntry> = picked.iter().copied().filter(|e| seen.insert(*e)).collect();
    if mode == SelectMode::Replace {
        return picked;
    }
    let picked_set = seen;
    let have: BTreeSet<SelectionEntry> = current.iter().copied().collect();
    let fresh = picked.iter().copied().filter(|e| !have.contains(e));
    let kept = current.iter().copied().filter(|e| !picked_set.contains(e));
    match mode {
        SelectMode::Add => current.iter().copied().chain(fresh).collect(),
        SelectMode::Subtract => kept.collect(),
        SelectMode::Toggle | SelectMode::Replace => kept.chain(fresh).collect(),
    }
}

/// Whether `entry` names a live entity of its kind (undone creations and
/// deleted entities keep their stable ids but are not live).
pub fn entry_live(doc: &kernel::Document, entry: SelectionEntry) -> bool {
    match entry {
        SelectionEntry::Object(id) => doc.object(id).is_some(),
        SelectionEntry::Group(id) => doc.group_members(id).is_some(),
        SelectionEntry::Instance(id) => doc.instance_def(id).is_some(),
        SelectionEntry::Sketch(id) => doc.sketch(id).is_some(),
        SelectionEntry::Guide(id) => doc.guide(id).is_some(),
        SelectionEntry::Annotation(id) => doc.annotation(id).is_some(),
    }
}

/// The public ids of the live entries, in order; an entry whose entity is
/// gone is skipped.
pub fn public_ids(doc: &kernel::Document, entries: &[SelectionEntry]) -> Vec<String> {
    entries
        .iter()
        .filter(|e| entry_live(doc, **e))
        .filter_map(|e| {
            let entity = e.entity();
            doc.sid_of(&entity)
                .map(|sid| crate::ids::public_id(&entity, sid))
        })
        .collect()
}

/// A selection as the bus reports it: `{ ids, primary, generation, epoch }`
/// (`hew.selection.get`/`set`/`clear` and the `selection_changed` event).
pub fn selection_json(doc: &kernel::Document, entries: &[SelectionEntry]) -> serde_json::Value {
    let ids = public_ids(doc, entries);
    let primary = ids.first().cloned();
    serde_json::json!({
        "ids": ids,
        "primary": primary,
        "generation": doc.history_generation(),
        "epoch": doc.document_epoch(),
    })
}
