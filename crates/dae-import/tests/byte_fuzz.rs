//! Byte-mutation fuzz (DEVELOPMENT.md rule 3): random corruptions of valid
//! .dae fixtures must never panic or hang the importer — malformed input
//! fails typed (or heals into a reportable scene), never crashes.

use dae_import::ImageMap;
use proptest::prelude::*;

const FIXTURES: &[&[u8]] = &[
    include_bytes!("fixtures/box_closed.dae"),
    include_bytes!("fixtures/polygons_with_hole.dae"),
    include_bytes!("fixtures/group_and_instance.dae"),
];

proptest! {
    #[test]
    fn mutated_dae_never_panics(
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
        let _ = dae_import::import(&bytes, &ImageMap::default());
    }
}
