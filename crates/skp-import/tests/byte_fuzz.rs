//! Byte-mutation fuzz (DEVELOPMENT.md rule 3): random corruptions of valid
//! .skp fixtures must never panic or hang the importer — malformed input
//! fails typed (or heals into a reportable scene), never crashes.

use proptest::prelude::*;

const FIXTURES: &[&[u8]] = &[
    include_bytes!("fixtures/box-group.skp"),
    include_bytes!("fixtures/box-component.skp"),
    include_bytes!("fixtures/box-two-materials.skp"),
];

proptest! {
    #[test]
    fn mutated_skp_never_panics(
        seed in 0usize..3,
        mutations in proptest::collection::vec((any::<usize>(), any::<u8>()), 1..48),
        truncate in proptest::option::of(any::<usize>()),
    ) {
        let mut bytes = FIXTURES[seed].to_vec();
        for &(pos, val) in &mutations {
            let i = pos % bytes.len();
            bytes[i] = val;
        }
        if let Some(t) = truncate {
            bytes.truncate(t % (bytes.len() + 1));
        }
        let _ = skp_import::import(&bytes);
    }
}
