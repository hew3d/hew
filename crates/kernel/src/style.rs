//! How a document asks to be shown: the backdrop and the axes
//! (docs/dev/HEW_FILE_FORMAT.md §4.15). A SketchUp model carries a style —
//! background and sky colors, whether the ground is drawn, whether the
//! model axes show — and a file opened from one keeps looking the way it
//! did there. A document without a style is shown Hew's own way (the
//! theme's backdrop, the ground grid, the axes).
//!
//! View state like the camera (`camera.rs`): persisted with the document,
//! never part of undo history. The small set here is what Hew can honor
//! today; edge and face styles wait for a styles feature of its own.

/// A document's backdrop and axes settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewStyle {
    /// The color behind everything, sRGB.
    pub background: [u8; 3],
    /// The sky's color above the horizon when a sky is drawn; `None` draws
    /// the background there.
    pub sky: Option<[u8; 3]>,
    /// The ground plane's color below the horizon when a ground is drawn;
    /// `None` leaves the plane out (Hew's grid lines still draw on it).
    pub ground: Option<[u8; 3]>,
    /// Whether the model axes are shown.
    pub axes: bool,
    /// The section cut's look and the guides' (a `.skp` style's own).
    /// Every one is `None` for a style Hew makes; a value is carried and
    /// saved, and the viewport draws the cut's color and width.
    pub section: SectionLook,
    /// The guides' color, sRGB; `None` is Hew's own.
    pub guides_color: Option<[u8; 3]>,
    /// Whether the style hides guides (View > Guides off); `None` is not
    /// said. The editor's own guides toggle is not driven by it.
    pub guides_hidden: Option<bool>,
}

/// How a section cut is drawn, as a style states it. All optional: `None`
/// leaves Hew's own look (a black cut outline, no fill).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SectionLook {
    /// The outline where a plane cuts a solid, sRGB.
    pub cut_color: Option<[u8; 3]>,
    /// The cut outline's width in pixels.
    pub cut_width: Option<u32>,
    /// Whether a cut face is filled; carried, not drawn.
    pub fill: Option<bool>,
    /// The fill's color, sRGB; carried, not drawn.
    pub fill_color: Option<[u8; 3]>,
    /// The color of the plane that is cutting, sRGB; carried, not drawn.
    pub active_color: Option<[u8; 3]>,
}

impl SectionLook {
    /// Whether nothing is said (Hew's own look).
    pub fn is_unset(&self) -> bool {
        *self == SectionLook::default()
    }
}
