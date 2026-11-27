//! The view and hidden-line helpers behind `hew.view.line_drawing` and
//! `hew-cli`'s `hew.print.pdf`: a camera resolved to a view, the Scene's
//! hidden set, the section plane, and the projected extent of a box.
//! Shared so the print path and the line-drawing path can never drift.

use api::print_layout::RectM;
use api::{Refusal, SnapshotCamera, SnapshotProjection, StandardView};
use kernel::{Document, Point3, Vec3};
/// A resolved print view: unit direction (eye → target), up, and whether
/// the request was perspective.
pub struct View {
    pub dir: Vec3,
    pub up: Vec3,
    pub perspective: bool,
    /// Explicit eye/target when the caller gave a camera (Standard mode
    /// keeps them; Scaled only keeps the direction).
    pub eye: Point3,
    pub target: Point3,
    pub fov_deg: f64,
}

pub fn dot(a: Vec3, b: Vec3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}
pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

/// The view-plane basis `hlr::Frame` and the app's `viewPlaneBasis` use.
pub fn basis(dir: Vec3, up: Vec3) -> (Vec3, Vec3) {
    let mut right = cross(dir, up);
    if right.length() < 1e-9 {
        // Same degenerate rule as hlr::Frame / the app's print pass.
        let seed = if dir.x.abs() < 0.9 {
            Vec3::new(1.0, 0.0, 0.0)
        } else {
            Vec3::new(0.0, 1.0, 0.0)
        };
        right = seed - dir * dot(seed, dir);
    }
    let right = right * (1.0 / right.length());
    let upv = cross(right, dir);
    (right, upv)
}

pub fn resolve_view(
    doc: &Document,
    camera: &Option<SnapshotCamera>,
    view: Option<StandardView>,
    scene: Option<&kernel::ResolvedScene>,
    bbox: (Point3, Point3),
) -> Result<View, Refusal> {
    let from_points = |eye: Point3,
                       target: Point3,
                       up: Vec3,
                       perspective: bool,
                       fov: f64|
     -> Result<View, Refusal> {
        let d = target - eye;
        if d.length() < 1e-12 {
            return Err(Refusal::api(
                "nothing_to_render",
                "The camera's eye sits on its target.",
            ));
        }
        let dir = d * (1.0 / d.length());
        Ok(View {
            dir,
            up,
            perspective,
            eye,
            target,
            fov_deg: fov,
        })
    };
    if let Some(cam) = camera {
        let eye = Point3::new(cam.eye[0], cam.eye[1], cam.eye[2]);
        let target = Point3::new(cam.target[0], cam.target[1], cam.target[2]);
        let up = cam
            .up
            .map(|u| Vec3::new(u[0], u[1], u[2]))
            .unwrap_or(Vec3::new(0.0, 0.0, 1.0));
        let perspective = !matches!(cam.projection, Some(SnapshotProjection::Parallel));
        return from_points(eye, target, up, perspective, cam.fov_deg.unwrap_or(35.0));
    }
    if let Some(v) = view {
        // Standard views: the app's print directions (world up +Z; top and
        // bottom are exact axis views — the basis rule reads x → right).
        let (eye_dir, up): ([f64; 3], [f64; 3]) = match v {
            StandardView::Top => ([0.0, 0.0, 1.0], [0.0, 0.0, 1.0]),
            StandardView::Bottom => ([0.0, 0.0, -1.0], [0.0, 0.0, 1.0]),
            StandardView::Front => ([0.0, -1.0, 0.0], [0.0, 0.0, 1.0]),
            StandardView::Back => ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
            StandardView::Right => ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            StandardView::Left => ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            StandardView::Iso => ([1.0, -1.0, 1.0], [0.0, 0.0, 1.0]),
        };
        let center = Point3::new(
            (bbox.0.x + bbox.1.x) / 2.0,
            (bbox.0.y + bbox.1.y) / 2.0,
            (bbox.0.z + bbox.1.z) / 2.0,
        );
        let ed = Vec3::new(eye_dir[0], eye_dir[1], eye_dir[2]);
        let extent = (bbox.1 - bbox.0).length().max(1e-6);
        let eye = center + ed * (1.0 / ed.length()) * (extent * 1.8);
        return from_points(eye, center, Vec3::new(up[0], up[1], up[2]), false, 35.0);
    }
    if let Some(resolved) = scene
        && let Some(cam) = resolved.camera
    {
        return from_points(
            cam.eye,
            cam.target,
            cam.up,
            matches!(cam.projection, kernel::CameraProjection::Perspective),
            cam.fov_deg,
        );
    }
    match doc.camera_state() {
        Some(state) => from_points(
            state.eye,
            state.target,
            state.up,
            matches!(state.projection, kernel::CameraProjection::Perspective),
            state.fov_deg,
        ),
        None => {
            let center = Point3::new(
                (bbox.0.x + bbox.1.x) / 2.0,
                (bbox.0.y + bbox.1.y) / 2.0,
                (bbox.0.z + bbox.1.z) / 2.0,
            );
            let ed = Vec3::new(1.0, -1.0, 1.0);
            let extent = (bbox.1 - bbox.0).length().max(1e-6);
            let eye = center + ed * (1.0 / ed.length()) * (extent * 1.8);
            from_points(eye, center, Vec3::new(0.0, 0.0, 1.0), false, 35.0)
        }
    }
}

/// Scene resolution shared with `snapshot`: the Scene's hidden sets
/// (when it captures them) else the document's own.
pub fn scene_and_hidden(
    doc: &Document,
    scene: Option<u64>,
) -> Result<(Option<kernel::ResolvedScene>, softrender::HiddenLeaves), Refusal> {
    let scene = match scene {
        Some(sid) => Some(
            doc.resolve_scene(sid)
                .map_err(|e| Refusal::from_document_error(&e))?,
        ),
        None => None,
    };
    let hidden = match &scene {
        Some(resolved) => match (&resolved.hidden_object_ids, &resolved.hidden_instance_ids) {
            (Some(objects), Some(instances)) => {
                softrender::HiddenLeaves::from_lists(objects, instances)
            }
            _ => softrender::HiddenLeaves::of_document(doc),
        },
        None => softrender::HiddenLeaves::of_document(doc),
    };
    Ok((scene, hidden))
}

pub fn hlr_section(doc: &Document, scene: Option<&kernel::ResolvedScene>) -> Option<hlr::Section> {
    // A Scene's captured section plane (Some(None) = captured as "no
    // plane") wins; else the document's live one.
    let plane = match scene.and_then(|s| s.section) {
        Some(captured) => captured,
        None => doc.section_plane(),
    };
    match plane {
        Some(p) if p.active => Some(hlr::Section {
            origin: p.origin,
            normal: p.normal,
        }),
        _ => None,
    }
}

pub fn hlr_error(e: hlr::HlrError) -> Refusal {
    match e {
        hlr::HlrError::TooComplex { pairs, budget } => Refusal::api("too_complex", &format!("This model is too complex for a vector line drawing ({pairs} candidate/occluder pairs, budget {budget}). Use a shaded print instead."))
            .with_detail(serde_json::json!({ "pairs": pairs, "budget": budget })),
        hlr::HlrError::DegenerateCamera => Refusal::api("nothing_to_render", "The camera is degenerate (eye on target)."),
    }
}

pub fn kind_name(k: hlr::Kind) -> &'static str {
    match k {
        hlr::Kind::Hard => "hard",
        hlr::Kind::Silhouette => "silhouette",
        hlr::Kind::Soft => "soft",
        hlr::Kind::Section => "section",
        hlr::Kind::Hidden => "hidden",
    }
}

/// Project the world bbox onto the view plane about `center`; returns the
/// rect (metres, y up) and the depth range along `dir`.
pub fn projected_extent(
    bbox: (Point3, Point3),
    center: Point3,
    dir: Vec3,
    right: Vec3,
    upv: Vec3,
) -> (RectM, (f64, f64)) {
    let (mut minx, mut maxx, mut miny, mut maxy, mut mind, mut maxd) = (
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    for i in 0..8 {
        let p = Point3::new(
            if i & 1 == 1 { bbox.1.x } else { bbox.0.x },
            if i & 2 == 2 { bbox.1.y } else { bbox.0.y },
            if i & 4 == 4 { bbox.1.z } else { bbox.0.z },
        );
        let rel = p - center;
        let x = dot(rel, right);
        let y = dot(rel, upv);
        let d = dot(rel, dir);
        minx = minx.min(x);
        maxx = maxx.max(x);
        miny = miny.min(y);
        maxy = maxy.max(y);
        mind = mind.min(d);
        maxd = maxd.max(d);
    }
    (
        RectM {
            x: minx,
            y: miny,
            w: (maxx - minx).max(1e-6),
            h: (maxy - miny).max(1e-6),
        },
        (mind, maxd),
    )
}
