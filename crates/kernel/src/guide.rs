//! Construction-geometry guides (ARCHITECTURE.md): non-solid, non-sketch helper
//! geometry for alignment. Guides carry no topology, no fill, and never affect
//! watertightness; the inference engine will treat them as snap targets (a
//! later slice — not wired up here).

use crate::math::{Point3, Vec3};

/// A construction-geometry guide: non-solid, non-sketch helper geometry
/// for alignment. Carries no topology and never affects watertightness; the
/// inference engine treats guides as snap targets (a later slice).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Guide {
    /// Infinite construction line through `origin` with unit `direction`.
    /// `direction` is normalized + non-zero by construction (the Document
    /// constructor rejects a degenerate direction).
    Line { origin: Point3, direction: Vec3 },
    /// A single construction point.
    Point { position: Point3 },
}
