//! Generational handles into kernel element storage.
//!
//! Backed by `slotmap` keys: a stale handle after a deletion is a lookup miss,
//! never an aliased element.

use slotmap::new_key_type;

new_key_type! {
    /// Handle to a [`crate::topo::Vertex`].
    pub struct VertexId;
    /// Handle to a [`crate::topo::HalfEdge`].
    pub struct HalfEdgeId;
    /// Handle to an [`crate::topo::Edge`].
    pub struct EdgeId;
    /// Handle to a [`crate::topo::Loop`].
    pub struct LoopId;
    /// Handle to a [`crate::topo::Face`].
    pub struct FaceId;
    /// Handle to a [`crate::topo::Shell`].
    pub struct ShellId;
    /// Handle to an [`crate::topo::Object`] in a [`crate::document::Document`].
    pub struct ObjectId;
    /// Handle to a [`crate::sketch::Sketch`] in a [`crate::document::Document`].
    pub struct SketchId;
    /// Handle to a merge group (non-destructive container) in a
    /// [`crate::document::Document`].
    pub struct GroupId;
    /// Handle to a component definition (shared geometry, a library entry — not
    /// a tree node) in a [`crate::document::Document`] (ARCHITECTURE.md).
    pub struct ComponentId;
    /// Handle to a component instance (a tree node placing a
    /// [`ComponentId`] at a per-instance pose) in a
    /// [`crate::document::Document`] (ARCHITECTURE.md).
    pub struct InstanceId;
    /// Handle to a [`crate::material::Material`] in a
    /// [`crate::document::Document`]'s palette (ARCHITECTURE.md). A
    /// [`crate::topo::Face`] references one (or `None` = default material).
    pub struct MaterialId;
    /// Handle to a [`crate::guide::Guide`] construction-geometry entity in a
    /// [`crate::document::Document`] (ARCHITECTURE.md).
    pub struct GuideId;
    /// Handle to a [`crate::annotation::Annotation`] (dimension or
    /// leader-text entity) in a [`crate::document::Document`]
    /// (ARCHITECTURE.md).
    pub struct AnnotationId;
}

/// Serde for a list of slotmap keys as decimal strings of their FFI value
/// (`KeyData::as_ffi`): strings, not numbers, because JSON numbers lose
/// precision above 2^53 in JavaScript. The test harness and the wasm
/// boundary pass handles as decimal strings the same way.
pub mod serde_ffi_vec {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use slotmap::{Key, KeyData};

    /// Writes each key as its decimal FFI string.
    pub fn serialize<K: Key, S: Serializer>(keys: &[K], s: S) -> Result<S::Ok, S::Error> {
        keys.iter()
            .map(|k| k.data().as_ffi().to_string())
            .collect::<Vec<String>>()
            .serialize(s)
    }

    /// Reads decimal FFI strings back into keys.
    pub fn deserialize<'de, K: Key, D: Deserializer<'de>>(d: D) -> Result<Vec<K>, D::Error> {
        let raw: Vec<String> = Vec::deserialize(d)?;
        raw.iter()
            .map(|s| {
                s.parse::<u64>()
                    .map(|v| K::from(KeyData::from_ffi(v)))
                    .map_err(serde::de::Error::custom)
            })
            .collect()
    }
}

/// Serde for an optional slotmap key as a decimal FFI string (see
/// [`serde_ffi_vec`]).
pub mod serde_ffi_opt {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use slotmap::{Key, KeyData};

    /// Writes the key as its decimal FFI string, or `null`.
    pub fn serialize<K: Key, S: Serializer>(key: &Option<K>, s: S) -> Result<S::Ok, S::Error> {
        key.map(|k| k.data().as_ffi().to_string()).serialize(s)
    }

    /// Reads an optional decimal FFI string back into a key.
    pub fn deserialize<'de, K: Key, D: Deserializer<'de>>(d: D) -> Result<Option<K>, D::Error> {
        let raw: Option<String> = Option::deserialize(d)?;
        raw.map(|s| {
            s.parse::<u64>()
                .map(|v| K::from(KeyData::from_ffi(v)))
                .map_err(serde::de::Error::custom)
        })
        .transpose()
    }
}
