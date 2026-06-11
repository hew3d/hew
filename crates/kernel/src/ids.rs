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
    /// Handle to an [`crate::topo::Object`] in a future Document scene graph.
    pub struct ObjectId;
}
