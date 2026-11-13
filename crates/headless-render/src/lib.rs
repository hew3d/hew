//! The headless renderers behind `hew.view.snapshot` and
//! `hew.view.line_drawing`, shared by every host that owns a `Document`:
//! `hew-cli`'s `CliHost` and the app's `LiveHost`. Pure over `&Document`;
//! no I/O (a `path` is the host's business, through
//! `Host::write_snapshot`). `hew.print.pdf` stays in `hew-cli`, which
//! imports the shared view helpers below.

mod line_drawing;
mod snapshot;
mod view;

pub use line_drawing::line_drawing;
pub use snapshot::snapshot;
pub use view::{
    View, basis, cross, dot, hlr_error, hlr_section, kind_name, projected_extent, resolve_view,
    scene_and_hidden,
};
