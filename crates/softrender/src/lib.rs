//! The headless render device (docs/design/headless-snapshot.md): a pure
//! software rasterizer over `tessellate`'s renderer-agnostic buffers —
//! the second "device" behind the same primitives the WebGL2 viewport
//! draws. Deterministic by construction (plain f32 arithmetic, fixed
//! traversal order), so rendered bytes are golden-testable artifacts.
//!
//! Outputs both a shaded RGBA image and an **id-buffer**: a per-pixel
//! index into a palette of entity stable ids, so a caller can ask "what
//! object is at pixel (x, y)" without computer vision.

pub mod png;

use kernel::{Point3, Transform, Vec3};

/// Camera projection.
#[derive(Debug, Clone, Copy)]
pub enum Projection {
    /// Vertical field of view, degrees.
    Perspective { fov_y_deg: f64 },
    /// Half the world-space height visible at the target.
    Parallel { half_height: f64 },
}

/// A render camera.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub eye: Point3,
    pub target: Point3,
    pub up: Vec3,
    pub projection: Projection,
}

/// A named standard view (docs/agents/HEW_API.md §7 — `hew.view.snapshot`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardView {
    Iso,
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
}

impl StandardView {
    pub fn from_name(name: &str) -> Option<StandardView> {
        Some(match name {
            "iso" => StandardView::Iso,
            "front" => StandardView::Front,
            "back" => StandardView::Back,
            "left" => StandardView::Left,
            "right" => StandardView::Right,
            "top" => StandardView::Top,
            "bottom" => StandardView::Bottom,
            _ => return None,
        })
    }
}

impl Camera {
    /// The document's saved working camera, as a render camera.
    pub fn from_kernel(state: &kernel::CameraState) -> Camera {
        Camera {
            eye: state.eye,
            target: state.target,
            up: state.up,
            projection: match state.projection {
                kernel::CameraProjection::Perspective => Projection::Perspective {
                    fov_y_deg: state.fov_deg,
                },
                kernel::CameraProjection::Parallel => Projection::Parallel {
                    half_height: state.parallel_height / 2.0,
                },
            },
        }
    }

    /// A standard view fitted to a scene bounding box.
    pub fn standard_view(view: StandardView, bbox: (Point3, Point3)) -> Camera {
        let (min, max) = bbox;
        let center = Point3::new(
            (min.x + max.x) * 0.5,
            (min.y + max.y) * 0.5,
            (min.z + max.z) * 0.5,
        );
        let extent = ((max.x - min.x).powi(2) + (max.y - min.y).powi(2) + (max.z - min.z).powi(2))
            .sqrt()
            .max(1e-6);
        let dist = extent * 1.8;
        let (dir, up) = match view {
            StandardView::Iso => (Vec3::new(-1.0, -1.0, 0.75), Vec3::new(0.0, 0.0, 1.0)),
            StandardView::Front => (Vec3::new(0.0, -1.0, 0.0), Vec3::new(0.0, 0.0, 1.0)),
            StandardView::Back => (Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.0, 1.0)),
            StandardView::Left => (Vec3::new(-1.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0)),
            StandardView::Right => (Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0)),
            StandardView::Top => (Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 1.0, 0.0)),
            StandardView::Bottom => (Vec3::new(0.0, 0.0, -1.0), Vec3::new(0.0, 1.0, 0.0)),
        };
        let dirn = dir.normalized().expect("standard view dirs are non-zero");
        Camera {
            eye: center + dirn * dist,
            target: center,
            up,
            projection: Projection::Perspective { fov_y_deg: 35.0 },
        }
    }
}

/// One scene item: a tessellated mesh at a pose, tagged with the entity
/// stable id its pixels report in the id-buffer.
pub struct RenderItem<'a> {
    pub mesh: &'a tessellate::RenderMesh,
    pub pose: Transform,
    pub sid: u64,
}

/// Why a render was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    /// More items than the u16 id-buffer can index (65535). A scene this
    /// large should be snapshotted in parts or without ids.
    TooManyItems,
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::TooManyItems => {
                write!(
                    f,
                    "more than 65535 renderable items — the id-buffer cannot index them"
                )
            }
        }
    }
}

impl std::error::Error for RenderError {}

/// A finished render.
pub struct Rendered {
    pub width: u32,
    pub height: u32,
    /// RGBA8, row-major, top row first.
    pub rgba: Vec<u8>,
    /// Per-pixel index into `id_palette_sids`; 0 = background.
    pub ids: Vec<u16>,
    /// `id_palette_sids[index - 1]` is the stable id under that pixel.
    pub id_palette_sids: Vec<u64>,
    /// Per-pixel SURFACE depth: distance from the eye along the view axis
    /// (camera-space z, metres), `f32::INFINITY` where nothing was drawn.
    /// Face fills only — the edge overlay's depth bias is not in it.
    pub depth: Vec<f32>,
    /// Per-pixel camera-space surface normal, RGB8, `(n * 0.5 + 0.5) * 255`
    /// per axis with x right, y up, z toward the viewer; background pixels
    /// are `(128, 128, 255)`, a normal facing the viewer straight on.
    pub normals: Vec<u8>,
}

/// A depth image: the PNG and the metric range it encodes.
pub struct DepthImage {
    pub png: Vec<u8>,
    /// Surface depth (metres along the view axis) of the nearest and
    /// farthest drawn pixel; the image's full range maps onto it.
    pub near: f64,
    pub far: f64,
}

impl Rendered {
    /// The depth pass as a grayscale PNG (8 or 16 bits per pixel): NEAR is
    /// WHITE and far is dark — the convention depth-conditioned image
    /// models expect. Background pixels are 0; every drawn surface maps
    /// into `1..=max` linearly between the nearest and farthest drawn
    /// depth, so `depth = far - (v - 1) / (max - 1) * (far - near)`. A
    /// scene whose surfaces are all at one depth (a face-on parallel view)
    /// is a flat `max` — "one depth" meaning within single-precision
    /// interpolation noise (a perspective view of a wall face-on differs by
    /// ~1e-7 relative from pixel to pixel; stretching that over the whole
    /// range would render a flat wall as speckle). A render that drew
    /// nothing is all background with a zero range.
    pub fn depth_image(&self, bits: u8) -> DepthImage {
        let (mut near, mut far) = (f32::INFINITY, f32::NEG_INFINITY);
        for &z in &self.depth {
            if z.is_finite() {
                near = near.min(z);
                far = far.max(z);
            }
        }
        if !near.is_finite() {
            // Nothing was drawn (every triangle clipped): a depth pass is
            // still owed — an all-background image over a zero range.
            near = 0.0;
            far = 0.0;
        }
        let max: f64 = if bits == 16 { 65_535.0 } else { 255.0 };
        let span = (far - near) as f64;
        let flat = span <= 1e-5 * (far.abs() as f64).max(1.0);
        let level = |z: f32| -> f64 {
            if !z.is_finite() {
                return 0.0;
            }
            if flat {
                return max;
            }
            1.0 + ((far - z) as f64 / span) * (max - 1.0)
        };
        let png = if bits == 16 {
            let samples: Vec<u16> = self
                .depth
                .iter()
                .map(|&z| level(z).round() as u16)
                .collect();
            png::encode_gray16(&samples, self.width, self.height)
        } else {
            let samples: Vec<u8> = self.depth.iter().map(|&z| level(z).round() as u8).collect();
            png::encode_gray8(&samples, self.width, self.height)
        };
        DepthImage {
            png,
            near: near as f64,
            far: far as f64,
        }
    }

    /// The normals pass as an RGB PNG (see [`Rendered::normals`]).
    pub fn normals_png(&self) -> Vec<u8> {
        png::encode_rgb(&self.normals, self.width, self.height)
    }
}

const BACKGROUND: [u8; 4] = [245, 246, 248, 255];
/// The background pixel a render fills untouched pixels with (public so a
/// print composer can recolour it to paper white).
pub const BACKGROUND_RGBA: [u8; 4] = BACKGROUND;
const AMBIENT: f32 = 0.35;
const EDGE_COLOR: [u8; 4] = [40, 44, 52, 255];
/// Depth bias pulling edges toward the viewer so they win the z-test
/// against their own faces.
const EDGE_DEPTH_BIAS: f32 = 2e-3;

/// Perspective near plane, metres from the eye: geometry nearer than this is
/// cut away (and a triangle straddling it is clipped to it).
const NEAR_PLANE: f64 = 0.01;

struct Frame {
    view: [[f64; 4]; 3], // world → camera (rotation+translation rows)
    proj: Projection,
    width: u32,
    height: u32,
    focal: f64, // pixels per unit at z=1 (perspective) or per world unit (parallel)
}

impl Frame {
    fn new(camera: &Camera, width: u32, height: u32) -> Frame {
        let f = (camera.target - camera.eye)
            .normalized()
            .unwrap_or(Vec3::new(0.0, -1.0, 0.0));
        let up0 = camera.up;
        let r = f
            .cross(up0)
            .normalized()
            .unwrap_or(Vec3::new(1.0, 0.0, 0.0));
        let u = r.cross(f);
        let e = camera.eye.to_vec();
        // Camera space: x right, y up, z forward (into the scene).
        let view = [
            [r.x, r.y, r.z, -r.dot(e)],
            [u.x, u.y, u.z, -u.dot(e)],
            [f.x, f.y, f.z, -f.dot(e)],
        ];
        let focal = match camera.projection {
            Projection::Perspective { fov_y_deg } => {
                (height as f64 * 0.5) / (fov_y_deg.to_radians() * 0.5).tan()
            }
            Projection::Parallel { half_height } => (height as f64 * 0.5) / half_height.max(1e-9),
        };
        Frame {
            view,
            proj: camera.projection,
            width,
            height,
            focal,
        }
    }

    /// World point → camera space (x right, y up, z forward).
    fn camera(&self, p: Point3) -> [f64; 3] {
        let v = [p.x, p.y, p.z, 1.0];
        [
            dot4(&self.view[0], &v),
            dot4(&self.view[1], &v),
            dot4(&self.view[2], &v),
        ]
    }

    /// Camera-space point (already at or beyond the near plane under
    /// perspective) → (screen x, screen y, camera-space depth).
    fn screen(&self, c: [f64; 3]) -> (f32, f32, f32) {
        let (sx, sy) = match self.proj {
            Projection::Perspective { .. } => {
                let z = c[2].max(NEAR_PLANE);
                (c[0] / z * self.focal, c[1] / z * self.focal)
            }
            Projection::Parallel { .. } => (c[0] * self.focal, c[1] * self.focal),
        };
        (
            (self.width as f64 * 0.5 + sx) as f32,
            (self.height as f64 * 0.5 - sy) as f32,
            c[2] as f32,
        )
    }

    /// The part of a camera-space triangle in front of the near plane, as
    /// 0, 1 or 2 triangles (Sutherland–Hodgman against `z >= NEAR_PLANE`,
    /// then a fan). A triangle with a vertex behind the eye — a long
    /// ceiling sheet in a close view — is CUT at the plane, not dropped:
    /// dropping it made whatever lay behind it show through. Parallel
    /// projection has no behind-the-eye, so nothing is clipped there.
    fn clip_triangle(&self, tri: [[f64; 3]; 3]) -> Vec<[[f64; 3]; 3]> {
        if matches!(self.proj, Projection::Parallel { .. })
            || tri.iter().all(|v| v[2] >= NEAR_PLANE)
        {
            return vec![tri];
        }
        let mut poly: Vec<[f64; 3]> = Vec::with_capacity(4);
        for i in 0..3 {
            let (cur, next) = (tri[i], tri[(i + 1) % 3]);
            let (cur_in, next_in) = (cur[2] >= NEAR_PLANE, next[2] >= NEAR_PLANE);
            if cur_in {
                poly.push(cur);
            }
            if cur_in != next_in {
                poly.push(lerp_to_plane(cur, next));
            }
        }
        match poly.len() {
            3 => vec![[poly[0], poly[1], poly[2]]],
            4 => vec![[poly[0], poly[1], poly[2]], [poly[0], poly[2], poly[3]]],
            _ => Vec::new(),
        }
    }

    /// The part of a camera-space segment in front of the near plane.
    fn clip_segment(&self, a: [f64; 3], b: [f64; 3]) -> Option<([f64; 3], [f64; 3])> {
        if matches!(self.proj, Projection::Parallel { .. }) {
            return Some((a, b));
        }
        match (a[2] >= NEAR_PLANE, b[2] >= NEAR_PLANE) {
            (true, true) => Some((a, b)),
            (true, false) => Some((a, lerp_to_plane(a, b))),
            (false, true) => Some((lerp_to_plane(a, b), b)),
            (false, false) => None,
        }
    }
}

/// Where the segment `a → b` crosses `z = NEAR_PLANE`.
fn lerp_to_plane(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    let t = (NEAR_PLANE - a[2]) / (b[2] - a[2]);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        NEAR_PLANE,
    ]
}

fn dot4(row: &[f64; 4], v: &[f64; 4]) -> f64 {
    row[0] * v[0] + row[1] * v[1] + row[2] * v[2] + row[3] * v[3]
}

/// Renders the items. Deterministic: identical inputs give identical
/// bytes, whatever the platform.
pub fn render(
    items: &[RenderItem],
    camera: &Camera,
    width: u32,
    height: u32,
) -> Result<Rendered, RenderError> {
    render_with_background(items, camera, width, height, BACKGROUND)
}

/// [`render`] with a chosen fill for untouched pixels. Only the color
/// buffer sees `background`; the id buffer (0 = background), depth, and
/// normals passes are unchanged.
pub fn render_with_background(
    items: &[RenderItem],
    camera: &Camera,
    width: u32,
    height: u32,
    background: [u8; 4],
) -> Result<Rendered, RenderError> {
    if items.len() > u16::MAX as usize {
        // One past this and `palette.len() as u16` would wrap to the
        // background sentinel — refuse typed instead.
        return Err(RenderError::TooManyItems);
    }
    let n = (width as usize) * (height as usize);
    let mut rgba = Vec::with_capacity(n * 4);
    for _ in 0..n {
        rgba.extend_from_slice(&background);
    }
    let mut depth = vec![f32::INFINITY; n];
    let mut surface_depth = vec![f32::INFINITY; n];
    let mut normals: Vec<u8> = [128u8, 128, 255].repeat(n);
    let mut ids = vec![0u16; n];
    let mut palette: Vec<u64> = Vec::with_capacity(items.len());
    let frame = Frame::new(camera, width, height);

    for item in items {
        palette.push(item.sid);
        let id_index = palette.len() as u16; // 0 is background
        let mesh = item.mesh;

        // Pose every vertex into camera space once.
        let mut cams: Vec<[f64; 3]> = Vec::with_capacity(mesh.positions.len() / 3);
        let mut world_normals: Vec<Vec3> = Vec::with_capacity(mesh.normals.len() / 3);
        for i in (0..mesh.positions.len()).step_by(3) {
            let p = item.pose.apply_point(Point3::new(
                mesh.positions[i] as f64,
                mesh.positions[i + 1] as f64,
                mesh.positions[i + 2] as f64,
            ));
            cams.push(frame.camera(p));
        }
        for i in (0..mesh.normals.len()).step_by(3) {
            // Inverse-transpose, so a group or instance posed with a
            // non-uniform scale keeps normals perpendicular to its faces.
            let v = item.pose.apply_normal(Vec3::new(
                mesh.normals[i] as f64,
                mesh.normals[i + 1] as f64,
                mesh.normals[i + 2] as f64,
            ));
            world_normals.push(v.normalized().unwrap_or(Vec3::new(0.0, 0.0, 1.0)));
        }
        let view_dir = (camera.target - camera.eye)
            .normalized()
            .unwrap_or(Vec3::new(0.0, -1.0, 0.0));
        let perspective = matches!(frame.proj, Projection::Perspective { .. });

        for tri in mesh.front_triangles() {
            let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            let clipped = frame.clip_triangle([cams[a], cams[b], cams[c]]);
            if clipped.is_empty() {
                continue; // wholly behind the near plane
            }
            // Two-sided flat shade from the face normal (headlight).
            let nrm = world_normals[a];
            let lambert = nrm.dot(view_dir).abs() as f32;
            let shade = AMBIENT + (1.0 - AMBIENT) * lambert.clamp(0.0, 1.0);
            let color = [
                (mesh.colors[a * 3] * shade * 255.0).clamp(0.0, 255.0) as u8,
                (mesh.colors[a * 3 + 1] * shade * 255.0).clamp(0.0, 255.0) as u8,
                (mesh.colors[a * 3 + 2] * shade * 255.0).clamp(0.0, 255.0) as u8,
                255,
            ];
            // Camera-space normal for the normals pass: x right, y up, z
            // toward the viewer, flipped to face the viewer like the
            // two-sided shading above.
            let (mut nx, mut ny, mut nz) = (
                frame.view[0][0] * nrm.x + frame.view[0][1] * nrm.y + frame.view[0][2] * nrm.z,
                frame.view[1][0] * nrm.x + frame.view[1][1] * nrm.y + frame.view[1][2] * nrm.z,
                -(frame.view[2][0] * nrm.x + frame.view[2][1] * nrm.y + frame.view[2][2] * nrm.z),
            );
            if nz < 0.0 {
                (nx, ny, nz) = (-nx, -ny, -nz);
            }
            let to_byte = |v: f64| ((v * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
            let normal_rgb = [to_byte(nx), to_byte(ny), to_byte(nz)];
            for piece in &clipped {
                let (pa, pb, pc) = (
                    frame.screen(piece[0]),
                    frame.screen(piece[1]),
                    frame.screen(piece[2]),
                );
                fill_triangle(
                    pa,
                    pb,
                    pc,
                    color,
                    id_index,
                    perspective,
                    width,
                    height,
                    &mut rgba,
                    &mut depth,
                    &mut ids,
                    &mut surface_depth,
                    &mut normals,
                    normal_rgb,
                );
            }
        }

        // Hard-edge overlay (soft edges deliberately suppressed).
        for seg in item_edges(mesh) {
            let (p0, p1) = seg;
            let c0 = frame.camera(item.pose.apply_point(p0));
            let c1 = frame.camera(item.pose.apply_point(p1));
            if let Some((c0, c1)) = frame.clip_segment(c0, c1) {
                draw_line(
                    frame.screen(c0),
                    frame.screen(c1),
                    perspective,
                    width,
                    height,
                    &mut rgba,
                    &mut depth,
                );
            }
        }
    }

    Ok(Rendered {
        width,
        height,
        rgba,
        ids,
        id_palette_sids: palette,
        depth: surface_depth,
        normals,
    })
}

fn item_edges(mesh: &tessellate::RenderMesh) -> impl Iterator<Item = (Point3, Point3)> + '_ {
    mesh.edge_positions.chunks_exact(6).map(|s| {
        (
            Point3::new(s[0] as f64, s[1] as f64, s[2] as f64),
            Point3::new(s[3] as f64, s[4] as f64, s[5] as f64),
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn fill_triangle(
    a: (f32, f32, f32),
    b: (f32, f32, f32),
    c: (f32, f32, f32),
    color: [u8; 4],
    id_index: u16,
    perspective: bool,
    width: u32,
    height: u32,
    rgba: &mut [u8],
    depth: &mut [f32],
    ids: &mut [u16],
    surface_depth: &mut [f32],
    normals: &mut [u8],
    normal_rgb: [u8; 3],
) {
    let min_x = a.0.min(b.0).min(c.0).floor().max(0.0) as i64;
    let max_x = (a.0.max(b.0).max(c.0).ceil() as i64).min(width as i64 - 1);
    let min_y = a.1.min(b.1).min(c.1).floor().max(0.0) as i64;
    let max_y = (a.1.max(b.1).max(c.1).ceil() as i64).min(height as i64 - 1);
    if min_x > max_x || min_y > max_y {
        return;
    }
    let area = edge_fn(a, b, (c.0, c.1));
    if area.abs() < 1e-12 {
        return;
    }
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let p = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = edge_fn(b, c, p) / area;
            let w1 = edge_fn(c, a, p) / area;
            let w2 = edge_fn(a, b, p) / area;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            // Screen-space barycentric weights are affine in 1/z, not z,
            // under perspective (linear z here provably misorders steeply
            // tilted triangles); parallel projection is affine in z itself.
            let z = if perspective {
                1.0 / (w0 / a.2 + w1 / b.2 + w2 / c.2)
            } else {
                w0 * a.2 + w1 * b.2 + w2 * c.2
            };
            let idx = (y as usize) * (width as usize) + (x as usize);
            if z < depth[idx] {
                depth[idx] = z;
                ids[idx] = id_index;
                rgba[idx * 4..idx * 4 + 4].copy_from_slice(&color);
                surface_depth[idx] = z;
                normals[idx * 3..idx * 3 + 3].copy_from_slice(&normal_rgb);
            }
        }
    }
}

fn edge_fn(a: (f32, f32, f32), b: (f32, f32, f32), p: (f32, f32)) -> f32 {
    (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
}

fn draw_line(
    a: (f32, f32, f32),
    b: (f32, f32, f32),
    perspective: bool,
    width: u32,
    height: u32,
    rgba: &mut [u8],
    depth: &mut [f32],
) {
    let steps = (b.0 - a.0).abs().max((b.1 - a.1).abs()).ceil().max(1.0) as usize;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = a.0 + (b.0 - a.0) * t;
        let y = a.1 + (b.1 - a.1) * t;
        let z_surface = if perspective {
            1.0 / ((1.0 - t) / a.2 + t / b.2)
        } else {
            a.2 + (b.2 - a.2) * t
        };
        let z = z_surface - EDGE_DEPTH_BIAS * (1.0 + z_surface.abs());
        if x < 0.0 || y < 0.0 || x >= width as f32 || y >= height as f32 {
            continue;
        }
        let idx = (y as usize) * (width as usize) + (x as usize);
        if z <= depth[idx] {
            depth[idx] = z;
            rgba[idx * 4..idx * 4 + 4].copy_from_slice(&EDGE_COLOR);
        }
    }
}

// ------------------------------------------------------------ document scenes

/// One document scene item, OWNING its tessellated mesh —
/// [`RenderItem`] only borrows one, so this is what lives long enough to
/// build a render list from. Shared by every headless caller that renders a
/// whole [`kernel::Document`] (`hew-cli`'s snapshot host, the library
/// thumbnail path) so the traversal cannot drift between them.
pub struct DocumentItem {
    pub mesh: tessellate::RenderMesh,
    pub pose: Transform,
    pub sid: u64,
}

/// Whether a node, or any group above it, is user-hidden (SketchUp "Hide" —
/// view state a headless render must respect, both for pixels and for view
/// fitting).
fn node_or_ancestor_hidden(doc: &kernel::Document, node: kernel::NodeId) -> bool {
    if doc.node_user_hidden(node) {
        return true;
    }
    let mut cursor = doc.node_parent(node);
    while let Some(group) = cursor {
        if doc.node_user_hidden(kernel::NodeId::Group(group)) {
            return true;
        }
        cursor = doc.node_parent(kernel::NodeId::Group(group));
    }
    false
}

/// The hidden leaf sets a render skips: object handles and instance
/// handles, as `kernel::Document::hidden_leaves` (the document's own view
/// state — user-hidden nodes AND hidden tags) or a Scene's
/// `ResolvedScene` (docs/design/scenes.md §7) produce them.
#[derive(Debug, Clone, Default)]
pub struct HiddenLeaves {
    pub objects: std::collections::BTreeSet<kernel::ObjectId>,
    pub instances: std::collections::BTreeSet<kernel::InstanceId>,
}

impl HiddenLeaves {
    /// The document's current hidden state.
    pub fn of_document(doc: &kernel::Document) -> HiddenLeaves {
        let (objects, instances) = doc.hidden_leaves();
        HiddenLeaves {
            objects: objects.into_iter().collect(),
            instances: instances.into_iter().collect(),
        }
    }

    /// From explicit lists (a resolved Scene's leaf sets).
    pub fn from_lists(
        objects: &[kernel::ObjectId],
        instances: &[kernel::InstanceId],
    ) -> HiddenLeaves {
        HiddenLeaves {
            objects: objects.iter().copied().collect(),
            instances: instances.iter().copied().collect(),
        }
    }
}

/// Tessellates every visible scene item of `doc`: each visible world object
/// at identity pose, and each placed instance's definition members at the
/// instance's own pose — tagged with the INSTANCE's stable id, so pixels
/// report the instance, not the shared definition. Hidden geometry — the
/// document's user-hidden nodes (and nodes under user-hidden groups) AND
/// nodes under a hidden tag, exactly the set the app's own viewport hides —
/// is skipped; an object that fails to tessellate is skipped, not fatal,
/// so one bad object cannot blank a whole render.
pub fn document_items(doc: &kernel::Document) -> Vec<DocumentItem> {
    document_items_hiding(doc, &HiddenLeaves::of_document(doc))
}

/// [`document_items`] with an explicit hidden set — a Scene's resolved
/// leaf sets (docs/design/scenes.md §7) instead of the document's own.
pub fn document_items_hiding(doc: &kernel::Document, hidden: &HiddenLeaves) -> Vec<DocumentItem> {
    let mut items: Vec<DocumentItem> = Vec::new();
    for id in doc.visible_object_ids() {
        if hidden.objects.contains(&id) || node_or_ancestor_hidden(doc, kernel::NodeId::Object(id))
        {
            continue;
        }
        let Some(obj) = doc.object(id) else { continue };
        let Ok(mesh) = tessellate::tessellate(obj, doc.materials()) else {
            continue;
        };
        let Some(sid) = doc.sid_of(&kernel::EntityRef::Object(id)) else {
            continue;
        };
        items.push(DocumentItem {
            mesh,
            pose: Transform::IDENTITY,
            sid,
        });
    }
    for instance in doc.instance_ids() {
        if hidden.instances.contains(&instance)
            || node_or_ancestor_hidden(doc, kernel::NodeId::Instance(instance))
        {
            continue;
        }
        let (Some(def), Some(pose)) = (doc.instance_def(instance), doc.instance_pose(instance))
        else {
            continue;
        };
        let Some(sid) = doc.sid_of(&kernel::EntityRef::Instance(instance)) else {
            continue;
        };
        // Nested definitions expand recursively: every leaf placement
        // composes its def-local pose with this instance's own — still
        // tagged with the OUTERMOST instance's sid.
        // Each leaf also carries the material its placements pass down
        // (an unpainted member face shows the nearest painted placement).
        let shown = doc.expanded_def_placements_shown(def);
        let inherited = doc.instance_expanded_inherited(instance);
        for (((member, local), shown), inherited) in doc
            .expanded_def_placements(def)
            .into_iter()
            .zip(shown)
            .zip(inherited)
        {
            // A user-hidden definition member is hidden in every instance
            // (the per-node flag's documented semantics), and so is one
            // under a hidden member or tag.
            if !shown || hidden.objects.contains(&member) {
                continue;
            }
            let Some(obj) = doc.object(member) else {
                continue;
            };
            let Ok(mesh) = tessellate::tessellate_with(obj, doc.materials(), inherited) else {
                continue;
            };
            items.push(DocumentItem {
                mesh,
                pose: local.then(&pose),
                sid,
            });
        }
    }
    items
}

/// The items a `selection` draws (`hew.view.line_drawing`'s `selection`):
/// each root's visible leaf placements at their world poses — a
/// definition root at the identity pose — minus anything `hidden` holds.
/// A definition root ignores `hidden` (tags, instance visibility) and
/// honors only its members' own user-hidden flags, as
/// [`kernel::Document::definition_preview`] does.
/// Segments report the outermost placement, as [`document_items_hiding`]'s
/// do: the world instance a leaf sits under, else the leaf object itself,
/// or the definition for a definition root. A leaf placement two roots
/// reach (a group and one of its members, an id named twice) is drawn once.
///
/// # Errors
/// [`kernel::DocumentError::TooManyOccurrences`] — a root holds more leaf
/// placements than one occurrence walk lists.
pub fn selection_items_hiding(
    doc: &kernel::Document,
    roots: &[kernel::RenderRoot],
    hidden: &HiddenLeaves,
) -> Result<Vec<DocumentItem>, kernel::DocumentError> {
    let mut items = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for root in roots {
        let marker = match root {
            kernel::RenderRoot::Definition(def) => Some(*def),
            kernel::RenderRoot::Occurrence(_) => None,
        };
        // A definition root shows what `Document::definition_preview`
        // shows: tags and instance visibility are ignored, a member's own
        // user-hidden flag (and its groups' and nested instances') is
        // honored. An occurrence root follows `hidden`.
        for (object, pose, path) in doc.render_root_leaves(root, marker.is_some())? {
            if !seen.insert((marker, path.clone(), object)) {
                continue;
            }
            let skipped = if marker.is_some() {
                node_or_ancestor_hidden(doc, kernel::NodeId::Object(object))
                    || path
                        .iter()
                        .any(|&i| doc.node_user_hidden(kernel::NodeId::Instance(i)))
            } else {
                hidden.objects.contains(&object)
                    || path.first().is_some_and(|i| hidden.instances.contains(i))
            };
            if skipped {
                continue;
            }
            let owner = match (root, path.first()) {
                (kernel::RenderRoot::Definition(def), _) => kernel::EntityRef::Component(*def),
                (_, Some(&i)) => kernel::EntityRef::Instance(i),
                (_, None) => kernel::EntityRef::Object(object),
            };
            let Some(sid) = doc.sid_of(&owner) else {
                continue;
            };
            let Some(obj) = doc.object(object) else {
                continue;
            };
            let inherited = doc.inherited_material(&path);
            let Ok(mesh) = tessellate::tessellate_with(obj, doc.materials(), inherited) else {
                continue;
            };
            items.push(DocumentItem { mesh, pose, sid });
        }
    }
    Ok(items)
}

/// The world-space bounds of `items`' vertices, or `None` when empty.
pub fn items_bbox(items: &[DocumentItem]) -> Option<(Point3, Point3)> {
    let mut acc: Option<(Point3, Point3)> = None;
    for it in items {
        for v in it.mesh.positions.chunks_exact(3) {
            let p = it.pose.apply_point(Point3::new(
                f64::from(v[0]),
                f64::from(v[1]),
                f64::from(v[2]),
            ));
            acc = Some(match acc {
                None => (p, p),
                Some((min, max)) => (
                    Point3::new(min.x.min(p.x), min.y.min(p.y), min.z.min(p.z)),
                    Point3::new(max.x.max(p.x), max.y.max(p.y), max.z.max(p.z)),
                ),
            });
        }
    }
    acc
}

/// The axis-aligned bounds of everything [`document_items`] would render
/// (exact object vertices, not tessellation output — cheaper, same box).
/// Falls back to a unit box about the origin for an empty scene, so view
/// fitting always has something to frame.
pub fn document_bbox(doc: &kernel::Document) -> (Point3, Point3) {
    document_bbox_hiding(doc, &HiddenLeaves::of_document(doc))
}

/// [`document_bbox`] with an explicit hidden set (a resolved Scene's).
pub fn document_bbox_hiding(doc: &kernel::Document, hidden: &HiddenLeaves) -> (Point3, Point3) {
    let mut acc: Option<(Point3, Point3)> = None;
    let mut extend = |p: Point3| {
        acc = Some(match acc {
            None => (p, p),
            Some((min, max)) => (
                Point3::new(min.x.min(p.x), min.y.min(p.y), min.z.min(p.z)),
                Point3::new(max.x.max(p.x), max.y.max(p.y), max.z.max(p.z)),
            ),
        });
    };
    for id in doc.visible_object_ids() {
        if hidden.objects.contains(&id) || node_or_ancestor_hidden(doc, kernel::NodeId::Object(id))
        {
            continue;
        }
        if let Some(obj) = doc.object(id) {
            for v in obj.vertices().values() {
                extend(v.position);
            }
        }
    }
    for instance in doc.instance_ids() {
        if hidden.instances.contains(&instance)
            || node_or_ancestor_hidden(doc, kernel::NodeId::Instance(instance))
        {
            continue;
        }
        let (Some(def), Some(pose)) = (doc.instance_def(instance), doc.instance_pose(instance))
        else {
            continue;
        };
        let shown = doc.expanded_def_placements_shown(def);
        for ((member, local), shown) in doc.expanded_def_placements(def).into_iter().zip(shown) {
            if !shown || hidden.objects.contains(&member) {
                continue;
            }
            let composed = local.then(&pose);
            if let Some(obj) = doc.object(member) {
                for v in obj.vertices().values() {
                    extend(composed.apply_point(v.position));
                }
            }
        }
    }
    acc.unwrap_or((Point3::new(-0.5, -0.5, -0.5), Point3::new(0.5, 0.5, 0.5)))
}

/// Renders `doc` to a square PNG thumbnail from a fitted isometric view —
/// the library's offline thumbnail recipe. `None` when the document has
/// nothing visible to render (an honest "no thumbnail", never a
/// background-only image passed off as one) or when the scene exceeds the
/// renderer's item budget.
pub fn render_document_thumbnail(doc: &kernel::Document, size: u32) -> Option<Vec<u8>> {
    let items = document_items(doc);
    if items.is_empty() {
        return None;
    }
    let render_items: Vec<RenderItem> = items
        .iter()
        .map(|it| RenderItem {
            mesh: &it.mesh,
            pose: it.pose,
            sid: it.sid,
        })
        .collect();
    let camera = Camera::standard_view(StandardView::Iso, document_bbox(doc));
    let rendered = render(&render_items, &camera, size, size).ok()?;
    Some(png::encode(&rendered.rgba, rendered.width, rendered.height))
}

#[cfg(test)]
mod tests {
    use super::perspective_tests::tri_mesh;
    use super::*;

    fn box_mesh() -> (tessellate::RenderMesh, kernel::MaterialPalette) {
        let mut doc = kernel::Document::new();
        let plane = kernel::Plane::from_polygon(&[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ])
        .unwrap();
        let s = doc.add_sketch(plane);
        doc.begin_sketch_gesture(s).unwrap();
        {
            let sk = doc.sketch_mut(s).unwrap();
            for (a, b) in [
                (Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0)),
                (Point3::new(1.0, 0.0, 0.0), Point3::new(1.0, 1.0, 0.0)),
                (Point3::new(1.0, 1.0, 0.0), Point3::new(0.0, 1.0, 0.0)),
                (Point3::new(0.0, 1.0, 0.0), Point3::new(0.0, 0.0, 0.0)),
            ] {
                sk.add_segment(a, b).unwrap();
            }
        }
        doc.end_sketch_gesture(s).unwrap();
        let region = doc.extrudable_regions(s).unwrap()[0];
        let (obj, _) = doc.extrude_region(s, region, 1.0).unwrap();
        let mesh = tessellate::tessellate(doc.object(obj).unwrap(), doc.materials()).unwrap();
        (mesh, doc.materials().clone())
    }

    fn box_at(doc: &mut kernel::Document, x0: f64) -> kernel::ObjectId {
        let plane = kernel::Plane::from_polygon(&[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ])
        .unwrap();
        let s = doc.add_sketch(plane);
        doc.begin_sketch_gesture(s).unwrap();
        {
            let sk = doc.sketch_mut(s).unwrap();
            for (a, b) in [
                (Point3::new(x0, 0.0, 0.0), Point3::new(x0 + 1.0, 0.0, 0.0)),
                (
                    Point3::new(x0 + 1.0, 0.0, 0.0),
                    Point3::new(x0 + 1.0, 1.0, 0.0),
                ),
                (Point3::new(x0 + 1.0, 1.0, 0.0), Point3::new(x0, 1.0, 0.0)),
                (Point3::new(x0, 1.0, 0.0), Point3::new(x0, 0.0, 0.0)),
            ] {
                sk.add_segment(a, b).unwrap();
            }
        }
        doc.end_sketch_gesture(s).unwrap();
        let region = doc.extrudable_regions(s).unwrap()[0];
        doc.extrude_region(s, region, 1.0).unwrap().0
    }

    /// Headless renders hide what the app's own viewport hides: user-hidden
    /// nodes AND tag-hidden nodes (docs/design/scenes.md §7) — and a Scene's
    /// resolved leaf sets override the document's own state.
    #[test]
    fn document_items_skip_tag_hidden_and_honor_a_scene_override() {
        let mut doc = kernel::Document::new();
        let a = box_at(&mut doc, 0.0);
        let b = box_at(&mut doc, 3.0);
        let c = box_at(&mut doc, 6.0);
        doc.add_node_tag(kernel::NodeId::Object(b), vec!["Hardware".to_string()])
            .unwrap();
        let sid_of =
            |doc: &kernel::Document, id| doc.sid_of(&kernel::EntityRef::Object(id)).unwrap();
        assert_eq!(document_items(&doc).len(), 3);

        doc.set_tag_hidden(vec!["Hardware".to_string()], true);
        let sids: Vec<u64> = document_items(&doc).iter().map(|i| i.sid).collect();
        assert_eq!(
            sids,
            vec![sid_of(&doc, a), sid_of(&doc, c)],
            "tag-hidden b is skipped"
        );
        let (lo, hi) = document_bbox(&doc);
        assert!(lo.x >= -1e-9 && hi.x <= 7.0 + 1e-9);

        // A Scene captured with c hidden and the tag visible: rendering
        // through its resolution shows a and b, not c — regardless of the
        // document's current tag state.
        doc.set_tag_hidden(vec!["Hardware".to_string()], false);
        doc.set_node_user_hidden(kernel::NodeId::Object(c), true);
        let scene = doc
            .add_scene(None, kernel::SceneProps::ALL, None, None, None)
            .unwrap();
        doc.set_node_user_hidden(kernel::NodeId::Object(c), false);
        doc.set_tag_hidden(vec!["Hardware".to_string()], true);
        let resolved = doc.resolve_scene(scene).unwrap();
        let hidden = HiddenLeaves::from_lists(
            resolved.hidden_object_ids.as_deref().unwrap_or(&[]),
            resolved.hidden_instance_ids.as_deref().unwrap_or(&[]),
        );
        let sids: Vec<u64> = document_items_hiding(&doc, &hidden)
            .iter()
            .map(|i| i.sid)
            .collect();
        assert_eq!(sids, vec![sid_of(&doc, a), sid_of(&doc, b)]);
        let (_, hi) = document_bbox_hiding(&doc, &hidden);
        assert!(hi.x <= 4.0 + 1e-9, "bbox excludes the Scene-hidden c");
    }

    fn render_box(view: StandardView) -> Rendered {
        let (mesh, _) = box_mesh();
        let camera = Camera::standard_view(
            view,
            (Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 1.0, 1.0)),
        );
        render(
            &[RenderItem {
                mesh: &mesh,
                pose: Transform::IDENTITY,
                sid: 1,
            }],
            &camera,
            128,
            128,
        )
        .expect("renders")
    }

    #[test]
    fn the_depth_pass_is_near_white_far_dark_with_a_metric_range() {
        let r = render_box(StandardView::Iso);
        let img = r.depth_image(8);
        assert!(
            img.near > 0.0 && img.near < img.far,
            "{} {}",
            img.near,
            img.far
        );
        // Read the PNG itself back: the encoder's output, not a copy of its
        // formula, must put near at 255, far at 1 and the background at 0.
        let pixels = decode_gray(&img.png, r.width, r.height, 8);
        let nearest = r.depth.iter().copied().fold(f32::INFINITY, f32::min);
        let farthest = r
            .depth
            .iter()
            .copied()
            .filter(|z| z.is_finite())
            .fold(f32::NEG_INFINITY, f32::max);
        let at = |target: f32| {
            let i = r.depth.iter().position(|&z| z == target).unwrap();
            pixels[i]
        };
        assert_eq!(at(nearest), 255, "nearest is white");
        assert_eq!(at(farthest), 1, "farthest is dark but not background");
        assert_eq!(pixels[0], 0, "the corner is background");
        assert!(
            pixels
                .iter()
                .zip(&r.depth)
                .all(|(&v, z)| (v == 0) == !z.is_finite()),
            "only background is 0"
        );
        // And the documented inverse recovers the metric depth.
        let i = r.depth.iter().position(|&z| z == farthest).unwrap();
        let recovered = img.far - (pixels[i] as f64 - 1.0) / 254.0 * (img.far - img.near);
        assert!(
            (recovered - farthest as f64).abs() < 1e-3,
            "{recovered} vs {farthest}"
        );
        // 16-bit uses the same mapping at finer steps.
        let wide = r.depth_image(16);
        assert_eq!(&wide.png[24..26], &[16, 0]);
        assert!((wide.near - img.near).abs() < 1e-9 && (wide.far - img.far).abs() < 1e-9);
        let wide_pixels = decode_gray(&wide.png, r.width, r.height, 16);
        let j = r.depth.iter().position(|&z| z == nearest).unwrap();
        assert_eq!(wide_pixels[j], 65_535);
    }

    /// Decodes a gray PNG this crate's encoder wrote (stored deflate,
    /// filter 0) back into samples — enough to test what a viewer would see.
    fn decode_gray(png: &[u8], width: u32, height: u32, bits: u8) -> Vec<u16> {
        let mut i = 8;
        let mut idat = Vec::new();
        while i < png.len() {
            let len = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
            if &png[i + 4..i + 8] == b"IDAT" {
                idat.extend_from_slice(&png[i + 8..i + 8 + len]);
            }
            i += 12 + len;
        }
        let mut raw = Vec::new();
        let mut z = &idat[2..]; // past the zlib header
        loop {
            let last = z[0] & 1 == 1;
            let n = u16::from_le_bytes([z[1], z[2]]) as usize;
            raw.extend_from_slice(&z[5..5 + n]);
            z = &z[5 + n..];
            if last {
                break;
            }
        }
        let bytes = if bits == 16 { 2 } else { 1 };
        let stride = width as usize * bytes;
        let mut out = Vec::new();
        for row in raw.chunks_exact(stride + 1).take(height as usize) {
            assert_eq!(row[0], 0, "filter 0");
            for px in row[1..].chunks_exact(bytes) {
                out.push(if bits == 16 {
                    u16::from_be_bytes([px[0], px[1]])
                } else {
                    px[0] as u16
                });
            }
        }
        out
    }

    fn ceiling_camera() -> Camera {
        // Standing under a ceiling, looking forward and up at it.
        Camera {
            eye: Point3::new(0.0, 0.0, 1.5),
            target: Point3::new(0.0, 5.0, 2.5),
            up: Vec3::new(0.0, 0.0, 1.0),
            projection: Projection::Perspective { fov_y_deg: 60.0 },
        }
    }

    /// A long ceiling sheet (two triangles reaching 100 m in every
    /// direction, so vertices lie behind the eye) at z = 2.5.
    fn ceiling_sheet() -> [tessellate::RenderMesh; 2] {
        [
            tri_mesh([
                (-100.0, -100.0, 2.5),
                (100.0, -100.0, 2.5),
                (100.0, 100.0, 2.5),
            ]),
            tri_mesh([
                (-100.0, -100.0, 2.5),
                (100.0, 100.0, 2.5),
                (-100.0, 100.0, 2.5),
            ]),
        ]
    }

    #[test]
    fn a_sheet_reaching_behind_the_eye_is_clipped_not_dropped() {
        let sheet = ceiling_sheet();
        // Joists ABOVE the ceiling (farther along the view): they must stay
        // hidden wherever the ceiling covers them.
        let joists = tri_mesh([(-100.0, 0.5, 3.0), (100.0, 0.5, 3.0), (0.0, 100.0, 3.0)]);
        let items = [
            RenderItem {
                mesh: &joists,
                pose: Transform::IDENTITY,
                sid: 9,
            },
            RenderItem {
                mesh: &sheet[0],
                pose: Transform::IDENTITY,
                sid: 1,
            },
            RenderItem {
                mesh: &sheet[1],
                pose: Transform::IDENTITY,
                sid: 1,
            },
        ];
        let r = render(&items, &ceiling_camera(), 128, 128).expect("renders");
        // The view centre looks at (0, 5, 2.5), on the ceiling: 26^0.5 m
        // along the view axis (the sheet meets the view at a shallow angle,
        // so half a pixel off-centre moves the depth by a tenth of a metre).
        let centre = 64 * 128 + 64;
        assert_eq!(
            r.id_palette_sids[r.ids[centre] as usize - 1],
            1,
            "the ceiling is there, and the joists above it are not"
        );
        assert!(
            (r.depth[centre] as f64 - 26f64.sqrt()).abs() < 0.5,
            "{}",
            r.depth[centre]
        );
        // Every row that looks up at the ceiling is covered, right to the
        // top of the frame (rays pointing below the horizon never meet a
        // ceiling, so the lower rows are legitimately background).
        for row in [10, 40, 70, 80] {
            assert!(r.ids[row * 128 + 64] != 0, "row {row} sees the ceiling");
        }
    }

    #[test]
    fn an_edge_with_an_endpoint_behind_the_eye_is_cut_at_the_near_plane() {
        let mut mesh = tri_mesh([(50.0, 50.0, 50.0), (51.0, 50.0, 50.0), (50.0, 51.0, 50.0)]);
        // A line along the ceiling from behind the camera to far ahead.
        mesh.edge_positions = vec![0.0, -50.0, 2.5, 0.0, 50.0, 2.5];
        let items = [RenderItem {
            mesh: &mesh,
            pose: Transform::IDENTITY,
            sid: 1,
        }];
        let r = render(&items, &ceiling_camera(), 128, 128).expect("renders");
        let ink = [EDGE_COLOR[0], EDGE_COLOR[1], EDGE_COLOR[2], 255];
        assert!(
            r.rgba.chunks_exact(4).any(|px| px == ink),
            "the edge is drawn, not dropped with its behind-the-eye endpoint"
        );
    }

    #[test]
    fn a_triangle_wholly_behind_the_eye_still_draws_nothing() {
        let behind = tri_mesh([(-1.0, -5.0, 2.0), (1.0, -5.0, 2.0), (0.0, -9.0, 2.0)]);
        let items = [RenderItem {
            mesh: &behind,
            pose: Transform::IDENTITY,
            sid: 1,
        }];
        let r = render(&items, &ceiling_camera(), 64, 64).expect("renders");
        assert!(r.ids.iter().all(|&i| i == 0));
    }

    #[test]
    fn a_face_on_perspective_view_of_a_flat_wall_is_one_flat_depth_not_speckle() {
        let (mesh, _) = box_mesh();
        let camera = Camera {
            eye: Point3::new(0.5, 0.5, 10.0),
            target: Point3::new(0.5, 0.5, 0.0),
            up: Vec3::new(0.0, 1.0, 0.0),
            projection: Projection::Perspective { fov_y_deg: 40.0 },
        };
        let r = render(
            &[RenderItem {
                mesh: &mesh,
                pose: Transform::IDENTITY,
                sid: 1,
            }],
            &camera,
            64,
            64,
        )
        .expect("renders");
        let img = r.depth_image(8);
        let pixels = decode_gray(&img.png, 64, 64, 8);
        let drawn: Vec<u16> = pixels.iter().copied().filter(|&v| v != 0).collect();
        assert!(!drawn.is_empty());
        assert!(
            drawn.iter().all(|&v| v == 255),
            "every pixel of a single-depth face is the flat max, not noise"
        );
    }

    #[test]
    fn an_empty_render_still_owes_a_depth_image() {
        let camera = Camera::standard_view(
            StandardView::Iso,
            (Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 1.0, 1.0)),
        );
        let r = render(&[], &camera, 16, 16).expect("renders nothing");
        let img = r.depth_image(8);
        assert_eq!((img.near, img.far), (0.0, 0.0));
        assert!(decode_gray(&img.png, 16, 16, 8).iter().all(|&v| v == 0));
    }

    #[test]
    fn a_face_on_parallel_view_is_one_flat_depth() {
        let (mesh, _) = box_mesh();
        let camera = Camera {
            eye: Point3::new(0.5, 0.5, 10.0),
            target: Point3::new(0.5, 0.5, 0.0),
            up: Vec3::new(0.0, 1.0, 0.0),
            projection: Projection::Parallel { half_height: 1.0 },
        };
        let r = render(
            &[RenderItem {
                mesh: &mesh,
                pose: Transform::IDENTITY,
                sid: 1,
            }],
            &camera,
            64,
            64,
        )
        .expect("renders");
        let img = r.depth_image(8);
        // The box's top (z = 1) is 9 m from an eye at z = 10.
        assert!((img.near - 9.0).abs() < 1e-3 && (img.far - 9.0).abs() < 1e-3);
        // Its normal, seen from above, faces the viewer straight on.
        let centre = (32 * 64 + 32) * 3;
        assert_eq!(&r.normals[centre..centre + 3], &[128, 128, 255]);
    }

    #[test]
    fn the_normals_pass_distinguishes_the_visible_faces_and_fills_the_background() {
        let r = render_box(StandardView::Iso);
        assert_eq!(
            &r.normals[0..3],
            &[128, 128, 255],
            "background faces the viewer"
        );
        let mut distinct: Vec<[u8; 3]> = r
            .normals
            .chunks_exact(3)
            .zip(&r.depth)
            .filter(|(_, z)| z.is_finite())
            .map(|(n, _)| [n[0], n[1], n[2]])
            .collect();
        distinct.sort();
        distinct.dedup();
        assert_eq!(
            distinct.len(),
            3,
            "an iso view of a box shows three faces: {distinct:?}"
        );
        // Every drawn normal faces the viewer (z at or above the midpoint).
        assert!(distinct.iter().all(|n| n[2] >= 127), "{distinct:?}");
    }

    #[test]
    fn a_box_renders_with_ids_and_background() {
        let (mesh, _) = box_mesh();
        let camera = Camera::standard_view(
            StandardView::Iso,
            (Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 1.0, 1.0)),
        );
        let r = render(
            &[RenderItem {
                mesh: &mesh,
                pose: Transform::IDENTITY,
                sid: 42,
            }],
            &camera,
            128,
            128,
        )
        .expect("renders");
        assert_eq!(r.id_palette_sids, vec![42]);
        let center = r.ids[64 * 128 + 64];
        assert_eq!(center, 1, "the box covers the view center");
        assert_eq!(r.ids[0], 0, "the corner is background");
        assert_eq!(&r.rgba[0..4], &BACKGROUND, "background color at corner");
        // Some pixel got edge ink.
        assert!(
            r.rgba
                .chunks_exact(4)
                .any(|px| px == [EDGE_COLOR[0], EDGE_COLOR[1], EDGE_COLOR[2], 255]),
            "hard edges drawn"
        );
    }

    #[test]
    fn a_transparent_background_leaves_the_shading_alone() {
        let (mesh, _) = box_mesh();
        let camera = Camera::standard_view(
            StandardView::Iso,
            (Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 1.0, 1.0)),
        );
        let item = RenderItem {
            mesh: &mesh,
            pose: Transform::IDENTITY,
            sid: 42,
        };
        let opaque = render(&[item], &camera, 128, 128).expect("renders");
        let item = RenderItem {
            mesh: &mesh,
            pose: Transform::IDENTITY,
            sid: 42,
        };
        let clear =
            render_with_background(&[item], &camera, 128, 128, [0, 0, 0, 0]).expect("renders");
        assert_eq!(&clear.rgba[0..4], &[0, 0, 0, 0], "corner is transparent");
        assert_eq!(clear.ids[0], 0, "the id buffer still reads background");
        let c = (64 * 128 + 64) * 4;
        assert_eq!(&clear.rgba[c..c + 4], &opaque.rgba[c..c + 4]);
        assert_eq!(clear.ids, opaque.ids);
        assert_eq!(clear.depth_image(8).png, opaque.depth_image(8).png);
        assert_eq!(clear.normals_png(), opaque.normals_png());
        // Every pixel the box touches is opaque; none is half-covered.
        for (px, id) in clear.rgba.chunks_exact(4).zip(&clear.ids) {
            assert_eq!(
                px[3],
                if *id == 0 && px == [0, 0, 0, 0] {
                    0
                } else {
                    255
                }
            );
        }
    }

    #[test]
    fn rendering_is_deterministic() {
        let (mesh, _) = box_mesh();
        let camera = Camera::standard_view(
            StandardView::Iso,
            (Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 1.0, 1.0)),
        );
        let items = [RenderItem {
            mesh: &mesh,
            pose: Transform::IDENTITY,
            sid: 1,
        }];
        let a = render(&items, &camera, 96, 96).expect("renders");
        let b = render(&items, &camera, 96, 96).expect("renders");
        assert_eq!(a.rgba, b.rgba);
        assert_eq!(a.ids, b.ids);
    }

    #[test]
    fn nearer_object_wins_the_id_buffer() {
        let (mesh, _) = box_mesh();
        let camera = Camera {
            eye: Point3::new(0.5, -4.0, 0.5),
            target: Point3::new(0.5, 0.5, 0.5),
            up: Vec3::new(0.0, 0.0, 1.0),
            projection: Projection::Perspective { fov_y_deg: 35.0 },
        };
        // Same box twice: one two units behind the other.
        let items = [
            RenderItem {
                mesh: &mesh,
                pose: Transform::translation(Vec3::new(0.0, 2.0, 0.0)),
                sid: 7,
            },
            RenderItem {
                mesh: &mesh,
                pose: Transform::IDENTITY,
                sid: 9,
            },
        ];
        let r = render(&items, &camera, 96, 96).expect("renders");
        let center = r.ids[48 * 96 + 48] as usize;
        assert_eq!(r.id_palette_sids[center - 1], 9, "front box owns the pixel");
    }
}

#[cfg(test)]
mod perspective_tests {
    use super::*;

    pub(crate) fn tri_mesh(verts: [(f32, f32, f32); 3]) -> tessellate::RenderMesh {
        let mut positions = Vec::new();
        for v in verts {
            positions.extend_from_slice(&[v.0, v.1, v.2]);
        }
        tessellate::RenderMesh {
            positions,
            normals: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            indices: vec![0, 1, 2],
            colors: vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            uvs: vec![0.0; 6],
            groups: vec![tessellate::MaterialGroup {
                material: None,
                back: false,
                start: 0,
                count: 3,
            }],
            edge_positions: vec![],
            soft_edge_positions: vec![],
            face_ranges: vec![],
        }
    }

    /// The review's hand-computed misorder case: a steeply depth-spanning
    /// triangle (true surface depth ≈2.93 at the tested pixel) must beat a
    /// flat triangle at depth 8 there. Linear screen-space z said 14.67
    /// and lost; perspective-correct 1/z interpolation wins.
    #[test]
    fn steep_triangle_occludes_a_farther_flat_one() {
        let near_steep = tri_mesh([(1.0, -1.0, 2.0), (-1.0, -1.0, 2.0), (0.0, 1.0, 40.0)]);
        let far_flat = tri_mesh([(-50.0, -50.0, 8.0), (50.0, -50.0, 8.0), (0.0, 50.0, 8.0)]);
        let camera = Camera {
            eye: Point3::new(0.0, 0.0, 0.0),
            target: Point3::new(0.0, 0.0, 1.0),
            up: Vec3::new(0.0, 1.0, 0.0),
            projection: Projection::Perspective { fov_y_deg: 90.0 },
        };
        let items = [
            RenderItem {
                mesh: &near_steep,
                pose: Transform::IDENTITY,
                sid: 111,
            },
            RenderItem {
                mesh: &far_flat,
                pose: Transform::IDENTITY,
                sid: 222,
            },
        ];
        let r = render(&items, &camera, 400, 400).expect("renders");
        // The projected-vertex centroid of the steep triangle (screen
        // mean), where the linear interpolation was provably wrong.
        let (px, py) = (200usize, 265usize);
        let idx = r.ids[py * 400 + px] as usize;
        assert_eq!(
            r.id_palette_sids[idx - 1],
            111,
            "the truly nearer steep triangle owns the pixel"
        );
    }
}
