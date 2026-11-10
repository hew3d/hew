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
    /// Construction line through `origin` with unit `direction`, drawn and
    /// snapped to over `span` (the whole line for a guide Hew draws; a
    /// ray or a segment where a SketchUp file holds one).
    /// `direction` is normalized + non-zero by construction (the Document
    /// constructor rejects a degenerate direction).
    Line {
        origin: Point3,
        direction: Vec3,
        span: GuideSpan,
    },
    /// A single construction point.
    Point { position: Point3 },
}

/// The stretch of a guide line that exists, as distances in meters along
/// its unit direction from its origin: `from` and `to`, each `None` when
/// that end runs on forever. SketchUp's Tape Measure makes a guide that
/// starts at a point (a ray) or one between two points (a segment), and
/// shows only that stretch; a guide Hew makes is the whole line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GuideSpan {
    pub from: Option<f64>,
    pub to: Option<f64>,
}

impl GuideSpan {
    /// The whole line.
    pub const INFINITE: GuideSpan = GuideSpan {
        from: None,
        to: None,
    };

    /// Whether both ends run on forever.
    pub fn is_infinite(&self) -> bool {
        self.from.is_none() && self.to.is_none()
    }

    /// Whether this is a span a guide can have: each given end finite, and
    /// with both ends given, `to` past `from` by more than the merge
    /// tolerance (a guide of no length is a point, not a line).
    pub fn is_valid(&self) -> bool {
        let finite = |e: Option<f64>| e.is_none_or(f64::is_finite);
        if !finite(self.from) || !finite(self.to) {
            return false;
        }
        match (self.from, self.to) {
            (Some(a), Some(b)) => b - a > crate::tol::POINT_MERGE,
            _ => true,
        }
    }

    /// Whether distance `t` along the line lies on the span, within the
    /// merge tolerance of an end.
    pub fn contains(&self, t: f64) -> bool {
        let m = crate::tol::POINT_MERGE;
        self.from.is_none_or(|a| t >= a - m) && self.to.is_none_or(|b| t <= b + m)
    }

    /// `t` moved onto the span: an end it runs past, else `t` itself.
    pub fn clamp(&self, t: f64) -> f64 {
        let t = self.from.map_or(t, |a| t.max(a));
        self.to.map_or(t, |b| t.min(b))
    }

    /// The span after the line's length is scaled by `k` (a positive
    /// factor: a uniform scale, or how much a pose stretches the guide's
    /// direction).
    pub fn scaled(&self, k: f64) -> GuideSpan {
        GuideSpan {
            from: self.from.map(|a| a * k),
            to: self.to.map(|b| b * k),
        }
    }

    /// The span read the other way along the line (a pose that turns the
    /// direction round keeps the same stretch with its ends swapped).
    pub fn reversed(&self) -> GuideSpan {
        GuideSpan {
            from: self.to.map(|b| -b),
            to: self.from.map(|a| -a),
        }
    }
}

/// How a guide line is drawn: a 16-bit on/off pattern, least-significant bit
/// first, one bit per fixed length along the line (the stipple mask a
/// `.skp` guide carries). Any mask is valid and survives save, load and a
/// `.skp` round trip; the named constants are the ones SketchUp's own
/// guides use. The default is [`GuideDash::DASHED`], how Hew has always
/// drawn its guides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GuideDash(pub u16);

impl GuideDash {
    /// An unbroken line.
    pub const SOLID: GuideDash = GuideDash(0xFFFF);
    /// Short dashes (the default).
    pub const DASHED: GuideDash = GuideDash(0x3333);
    /// Dots.
    pub const DOTTED: GuideDash = GuideDash(0x8888);
    /// Long dashes.
    pub const LONG_DASH: GuideDash = GuideDash(0xF0F0);
    /// Dash, dot, dash.
    pub const DASH_DOT: GuideDash = GuideDash(0x1C47);

    /// The mask, bit 0 first along the line.
    pub fn mask(self) -> u16 {
        self.0
    }

    /// Whether this is the default dash (omitted from a saved file).
    pub fn is_default(self) -> bool {
        self == GuideDash::default()
    }
}

impl Default for GuideDash {
    fn default() -> Self {
        GuideDash::DASHED
    }
}
