//! The camera vocabulary shared by `hew.view.snapshot` (doc.rs) and
//! `hew.view.camera` (view.rs) — docs/agents/HEW_API.md §7 is explicit that
//! there is exactly ONE camera spec in the protocol, not two: an explicit
//! `{eye, target, up?, projection?, fov_deg?}` or a named `view`, mutually
//! exclusive. This module is that spec's single parsing implementation,
//! extracted from `hew.view.snapshot`'s own (pre-existing) inline parsing
//! so the two commands can never drift into two dialects of "where is the
//! camera."

use super::CmdError;
use crate::host::{SnapshotCamera, SnapshotProjection, StandardView};
use serde::Deserialize;

/// The wire shape of `camera` before validation: `up`, `projection`, and
/// `fov_deg` stay optional all the way to the host, which resolves their
/// defaults (identity up, perspective, 35°) when it builds its own
/// camera.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawCamera {
    pub eye: [f64; 3],
    pub target: [f64; 3],
    #[serde(default)]
    pub up: Option<[f64; 3]>,
    #[serde(default)]
    pub projection: Option<String>,
    #[serde(default)]
    pub fov_deg: Option<f64>,
}

impl RawCamera {
    fn validate(self) -> Result<SnapshotCamera, CmdError> {
        let projection = match self.projection.as_deref() {
            None => None,
            Some("perspective") => Some(SnapshotProjection::Perspective),
            Some("parallel") => Some(SnapshotProjection::Parallel),
            Some(other) => {
                return Err(CmdError::Params(format!(
                    "unknown camera projection \"{other}\" (expected \"perspective\" or \"parallel\")"
                )));
            }
        };
        Ok(SnapshotCamera {
            eye: self.eye,
            target: self.target,
            up: self.up,
            projection,
            fov_deg: self.fov_deg,
        })
    }
}

/// Converts a validated explicit camera into the kernel's own
/// `CameraState`, resolving the same defaults every other consumer of
/// this vocabulary does (identity up `[0,0,1]`, perspective, 35°) —
/// `hew.scenes.add`/`update`'s path, which captures a camera into
/// document state rather than rendering it or moving a live viewport.
pub(crate) fn to_kernel_camera(cam: &SnapshotCamera) -> kernel::CameraState {
    let up = cam.up.unwrap_or([0.0, 0.0, 1.0]);
    let projection = match cam.projection.unwrap_or(SnapshotProjection::Perspective) {
        SnapshotProjection::Perspective => kernel::CameraProjection::Perspective,
        SnapshotProjection::Parallel => kernel::CameraProjection::Parallel,
    };
    kernel::CameraState {
        projection,
        fov_deg: cam.fov_deg.unwrap_or(35.0),
        eye: kernel::Point3::new(cam.eye[0], cam.eye[1], cam.eye[2]),
        target: kernel::Point3::new(cam.target[0], cam.target[1], cam.target[2]),
        up: kernel::Vec3::new(up[0], up[1], up[2]),
    }
}

/// Validates the shared `camera`/`view` vocabulary: mutually exclusive, an
/// unknown standard-view name refused, an unknown camera projection
/// refused. Used by both `hew.view.snapshot` and `hew.view.camera` so
/// there is exactly one validation path for "where is the camera."
pub(crate) fn parse_camera_or_view(
    camera: Option<RawCamera>,
    view: Option<String>,
) -> Result<(Option<SnapshotCamera>, Option<StandardView>), CmdError> {
    if camera.is_some() && view.is_some() {
        return Err(CmdError::Params(
            "camera and view are mutually exclusive".to_string(),
        ));
    }
    let camera = camera.map(RawCamera::validate).transpose()?;
    let view = view
        .map(|name| {
            StandardView::from_name(&name)
                .ok_or_else(|| CmdError::Params(format!("unknown standard view \"{name}\"")))
        })
        .transpose()?;
    Ok((camera, view))
}
