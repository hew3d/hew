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
}
