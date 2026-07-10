//! Byte-mutation fuzz (DEVELOPMENT.md rule 3): random corruptions of valid
//! .glb fixtures must never panic or hang the importer — malformed input
//! fails typed (or heals into a reportable scene), never crashes.

use proptest::prelude::*;

const FIXTURES: &[&[u8]] = &[
    include_bytes!("fixtures/box.glb"),
    include_bytes!("fixtures/countertop.glb"),
    include_bytes!("fixtures/instances.glb"),
];

proptest! {
    #[test]
    fn mutated_glb_never_panics(
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
        // Typed errors and healed-with-warnings scenes are both fine;
        // panics and hangs are the failure mode under test.
        let _ = gltf_import::import(&bytes);
    }
}
