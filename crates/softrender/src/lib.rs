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

/// A named standard view (docs/HEW_API.md §7 — `hew.view.snapshot`).
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
                    // The saved state keeps fov even under parallel; derive
                    // a half-height from the eye distance and that fov so
                    // the framing matches what the app showed.
                    half_height: {
                        let d = (state.target - state.eye).length();
                        d * (state.fov_deg.to_radians() * 0.5).tan()
                    },
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
}

const BACKGROUND: [u8; 4] = [245, 246, 248, 255];
const AMBIENT: f32 = 0.35;
const EDGE_COLOR: [u8; 4] = [40, 44, 52, 255];
/// Depth bias pulling edges toward the viewer so they win the z-test
/// against their own faces.
const EDGE_DEPTH_BIAS: f32 = 2e-3;

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

    /// World point → (screen x, screen y, camera-space depth). `None`
    /// behind the eye under perspective.
    fn project(&self, p: Point3) -> Option<(f32, f32, f32)> {
        let v = [p.x, p.y, p.z, 1.0];
        let cx = dot4(&self.view[0], &v);
        let cy = dot4(&self.view[1], &v);
        let cz = dot4(&self.view[2], &v);
        let (sx, sy) = match self.proj {
            Projection::Perspective { .. } => {
                if cz <= 1e-9 {
                    return None;
                }
                (cx / cz * self.focal, cy / cz * self.focal)
            }
            Projection::Parallel { .. } => (cx * self.focal, cy * self.focal),
        };
        Some((
            (self.width as f64 * 0.5 + sx) as f32,
            (self.height as f64 * 0.5 - sy) as f32,
            cz as f32,
        ))
    }
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
    if items.len() > u16::MAX as usize {
        // One past this and `palette.len() as u16` would wrap to the
        // background sentinel — refuse typed instead.
        return Err(RenderError::TooManyItems);
    }
    let n = (width as usize) * (height as usize);
    let mut rgba = Vec::with_capacity(n * 4);
    for _ in 0..n {
        rgba.extend_from_slice(&BACKGROUND);
    }
    let mut depth = vec![f32::INFINITY; n];
    let mut ids = vec![0u16; n];
    let mut palette: Vec<u64> = Vec::with_capacity(items.len());
    let frame = Frame::new(camera, width, height);

    for item in items {
        palette.push(item.sid);
        let id_index = palette.len() as u16; // 0 is background
        let mesh = item.mesh;

        // Pose + project every vertex once.
        let mut proj: Vec<Option<(f32, f32, f32)>> = Vec::with_capacity(mesh.positions.len() / 3);
        let mut world_normals: Vec<Vec3> = Vec::with_capacity(mesh.normals.len() / 3);
        for i in (0..mesh.positions.len()).step_by(3) {
            let p = item.pose.apply_point(Point3::new(
                mesh.positions[i] as f64,
                mesh.positions[i + 1] as f64,
                mesh.positions[i + 2] as f64,
            ));
            proj.push(frame.project(p));
        }
        for i in (0..mesh.normals.len()).step_by(3) {
            let v = item.pose.apply_vector(Vec3::new(
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

        for tri in mesh.indices.chunks_exact(3) {
            let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            let (Some(pa), Some(pb), Some(pc)) = (proj[a], proj[b], proj[c]) else {
                continue; // behind the eye — clip whole triangle (v0 posture)
            };
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
            );
        }

        // Hard-edge overlay (soft edges deliberately suppressed).
        for seg in item_edges(mesh) {
            let (p0, p1) = seg;
            let w0 = item.pose.apply_point(p0);
            let w1 = item.pose.apply_point(p1);
            if let (Some(a), Some(b)) = (frame.project(w0), frame.project(w1)) {
                draw_line(a, b, perspective, width, height, &mut rgba, &mut depth);
            }
        }
    }

    Ok(Rendered {
        width,
        height,
        rgba,
        ids,
        id_palette_sids: palette,
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

#[cfg(test)]
mod tests {
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

    fn tri_mesh(verts: [(f32, f32, f32); 3]) -> tessellate::RenderMesh {
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
