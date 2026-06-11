//! Versioned binary geometry encoding — the kernel's half of the native file
//! format (ARCHITECTURE.md).
//!
//! The kernel is I/O-free (DEVELOPMENT.md rule 1): bytes in, bytes out. The shell
//! layer owns files, the zip container, and the JSON manifest; this module
//! owns only the geometry buffer an Object becomes inside that container.
//!
//! Contract with `docs/HEW_FILE_FORMAT.md` (DEVELOPMENT.md rule: the spec is updated
//! in the SAME commit as any serialization change — that starts with the
//! commit that first implements these stubs, which must create the geometry-
//! buffer section of the spec):
//!
//! - Little-endian throughout; explicit format version header.
//! - **Deterministic**: encoding the same Object twice yields identical
//!   bytes (handles are canonicalized to a stable ordering first), so
//!   golden-file tests and content-addressed storage work.
//! - **Validating**: decode runs the full topology validator and rejects
//!   rather than repairs (rule 4) — a corrupt or hand-tampered file produces
//!   a typed error, never a quietly-broken Object.
//!
//! M0 status: contracts only; bodies are `todo!()` pending M3.

use crate::error::TopologyError;
use crate::topo::Object;

/// Version of the geometry buffer layout. Bump on any layout change and
/// extend `docs/HEW_FILE_FORMAT.md` plus the golden-file tests in the same
/// commit.
pub const GEOMETRY_FORMAT_VERSION: u32 = 1;

/// Typed failures of [`Object::decode`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// The buffer announces a version this build does not read.
    UnsupportedVersion {
        /// The version found in the header.
        found: u32,
    },
    /// The buffer ends before its announced contents do.
    Truncated,
    /// A structurally unreadable buffer (bad magic, impossible counts,
    /// out-of-range indices).
    Corrupt {
        /// Byte offset of the first inconsistency.
        offset: usize,
        /// What was wrong there.
        what: &'static str,
    },
    /// The buffer parsed, but the geometry it describes fails the topology
    /// validator. Surfaced verbatim; nothing is repaired.
    InvalidTopology(TopologyError),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::UnsupportedVersion { found } => {
                write!(
                    f,
                    "geometry buffer version {found} is not supported \
                     (this build reads {GEOMETRY_FORMAT_VERSION})"
                )
            }
            DecodeError::Truncated => write!(f, "geometry buffer is truncated"),
            DecodeError::Corrupt { offset, what } => {
                write!(f, "geometry buffer corrupt at byte {offset}: {what}")
            }
            DecodeError::InvalidTopology(e) => {
                write!(f, "decoded geometry is invalid: {e}")
            }
        }
    }
}

impl std::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DecodeError::InvalidTopology(e) => Some(e),
            _ => None,
        }
    }
}

impl Object {
    /// Encodes this Object's geometry as a self-contained, versioned,
    /// deterministic binary buffer (see module docs for the obligations).
    #[allow(unused_variables)] // contract stub: implementation lands in M3
    pub fn encode(&self) -> Vec<u8> {
        todo!("M3: deterministic geometry encoding (spec it in docs/HEW_FILE_FORMAT.md, same commit)")
    }

    /// Decodes a buffer produced by [`Object::encode`] (any supported
    /// version), validating fully before returning.
    ///
    /// Roundtrip property (see `tests/op_specs.rs`):
    /// `decode(encode(o))` equals `o` topologically and geometrically, and
    /// `encode` is deterministic.
    #[allow(unused_variables)] // contract stub: implementation lands in M3
    pub fn decode(bytes: &[u8]) -> Result<Object, DecodeError> {
        todo!("M3: validating geometry decoding")
    }
}
