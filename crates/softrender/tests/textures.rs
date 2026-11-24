//! KB-37 / issue #61: the headless renderer shows a material's image, not
//! only its base color. A unit box's top face carries a textured material;
//! a straight-down parallel view then reads texels one-to-one.

use kernel::{Document, ImageFormat, Material, Point3, Rgba8, Texture, Vec3};
use softrender::{Camera, DocumentItem, Projection, Rendered};

const SIZE: u32 = 100;

/// Quadrants of the fixture image (top row first): red, green / blue, yellow.
const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const YELLOW: [u8; 3] = [255, 255, 0];

fn quadrant_png() -> Vec<u8> {
    let mut rgba = Vec::new();
    for c in [RED, GREEN, BLUE, YELLOW] {
        rgba.extend([c[0], c[1], c[2], 255]);
    }
    softrender::png::encode(&rgba, 2, 2)
}

fn quadrant_jpeg() -> Vec<u8> {
    include_bytes!("fixtures/quadrants.jpg").to_vec()
}

/// A unit box whose top face carries `material`; returns the document.
fn box_with_top(material: Material) -> Document {
    let mut doc = Document::new();
    let plane = kernel::Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let sk = doc.add_sketch(plane);
    doc.begin_sketch_gesture(sk).unwrap();
    for (a, b) in [
        ((0.0, 0.0), (1.0, 0.0)),
        ((1.0, 0.0), (1.0, 1.0)),
        ((1.0, 1.0), (0.0, 1.0)),
        ((0.0, 1.0), (0.0, 0.0)),
    ] {
        doc.sketch_mut(sk)
            .unwrap()
            .add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
            .unwrap();
    }
    doc.end_sketch_gesture(sk).unwrap();
    let region = doc.extrudable_regions(sk).unwrap()[0];
    let (obj, _) = doc.extrude_region(sk, region, 1.0).unwrap();
    let mid = doc.add_material(material);
    let top = doc
        .object(obj)
        .unwrap()
        .faces()
        .iter()
        .find(|(_, f)| {
            f.plane
                .normal()
                .approx_eq(Vec3::new(0.0, 0.0, 1.0), kernel::tol::NORMAL_DIRECTION)
        })
        .map(|(id, _)| id)
        .unwrap();
    doc.paint_face(obj, top, Some(mid)).unwrap();
    doc
}

fn textured(color: Rgba8, image: Vec<u8>, format: ImageFormat) -> Material {
    Material::textured(
        "tex",
        color,
        Texture {
            image,
            format,
            world_size: [1.0, 1.0],
        },
    )
}

fn top_view() -> Camera {
    Camera {
        eye: Point3::new(0.5, 0.5, 5.0),
        target: Point3::new(0.5, 0.5, 1.0),
        up: Vec3::new(0.0, 1.0, 0.0),
        projection: Projection::Parallel { half_height: 0.6 },
    }
}

fn render(doc: &Document) -> (Rendered, Vec<DocumentItem>) {
    let items = softrender::document_items(doc);
    let ritems: Vec<_> = items.iter().map(DocumentItem::as_render_item).collect();
    let r = softrender::render(&ritems, &top_view(), SIZE, SIZE).unwrap();
    drop(ritems);
    (r, items)
}

/// The pixel under world point `(x, y)` on the top face (z = 1).
fn pixel_at(r: &Rendered, x: f64, y: f64) -> [u8; 3] {
    let focal = SIZE as f64 * 0.5 / 0.6;
    let px = (SIZE as f64 * 0.5 + (x - 0.5) * focal) as usize;
    let py = (SIZE as f64 * 0.5 - (y - 0.5) * focal) as usize;
    let i = (py * SIZE as usize + px) * 4;
    [r.rgba[i], r.rgba[i + 1], r.rgba[i + 2]]
}

/// Which image quadrant tessellation's UVs put under world point `(x, y)`
/// of the top face: the UVs are affine over the axis-aligned unit square.
fn expected_quadrant(doc: &Document, x: f64, y: f64) -> (usize, usize) {
    let obj = doc.visible_object_ids()[0];
    let mesh = tessellate::tessellate(doc.object(obj).unwrap(), doc.materials()).unwrap();
    let uv_of = |cx: f32, cy: f32| -> [f32; 2] {
        for i in 0..mesh.positions.len() / 3 {
            let p = &mesh.positions[i * 3..i * 3 + 3];
            if (p[0] - cx).abs() < 1e-6 && (p[1] - cy).abs() < 1e-6 && (p[2] - 1.0).abs() < 1e-6 {
                return [mesh.uvs[i * 2], mesh.uvs[i * 2 + 1]];
            }
        }
        panic!("no top vertex at ({cx}, {cy})");
    };
    let (o, ux, uy) = (uv_of(0.0, 0.0), uv_of(1.0, 0.0), uv_of(0.0, 1.0));
    let u = o[0] as f64 + (ux[0] - o[0]) as f64 * x + (uy[0] - o[0]) as f64 * y;
    let v = o[1] as f64 + (ux[1] - o[1]) as f64 * x + (uy[1] - o[1]) as f64 * y;
    // v = 0 is the image bottom; u wraps.
    let col = ((u - u.floor()) * 2.0) as usize;
    let row = ((1.0 - (v - v.floor())) * 2.0) as usize;
    (row.min(1), col.min(1))
}

const QUADRANTS: [[[u8; 3]; 2]; 2] = [[RED, GREEN], [BLUE, YELLOW]];
const PROBES: [(f64, f64); 4] = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)];

fn near(a: [u8; 3], b: [u8; 3], tol: i32) -> bool {
    (0..3).all(|i| (a[i] as i32 - b[i] as i32).abs() <= tol)
}

#[test]
fn a_png_texture_shows_its_image_not_the_base_color() {
    let doc = box_with_top(textured(
        Rgba8::rgb(255, 255, 255),
        quadrant_png(),
        ImageFormat::Png,
    ));
    let (r, items) = render(&doc);
    assert!(items[0].textures.undecodable.is_empty());
    let mut seen = Vec::new();
    for (x, y) in PROBES {
        let (row, col) = expected_quadrant(&doc, x, y);
        let got = pixel_at(&r, x, y);
        assert_eq!(got, QUADRANTS[row][col], "pixel at ({x}, {y})");
        seen.push(got);
    }
    // All four quadrants are distinct, so the image really is on the face.
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 4);
}

#[test]
fn a_jpeg_texture_shows_its_image() {
    let doc = box_with_top(textured(
        Rgba8::rgb(255, 255, 255),
        quadrant_jpeg(),
        ImageFormat::Jpeg,
    ));
    let (r, items) = render(&doc);
    assert!(items[0].textures.undecodable.is_empty());
    for (x, y) in PROBES {
        let (row, col) = expected_quadrant(&doc, x, y);
        let got = pixel_at(&r, x, y);
        assert!(
            near(got, QUADRANTS[row][col], 12),
            "pixel at ({x}, {y}) is {got:?}, want about {:?}",
            QUADRANTS[row][col]
        );
    }
}

#[test]
fn the_material_color_modulates_the_image() {
    // A half-intensity red tint: red stays 128, green and blue texels go to 0.
    let doc = box_with_top(textured(
        Rgba8::rgb(128, 0, 0),
        quadrant_png(),
        ImageFormat::Png,
    ));
    let (r, _) = render(&doc);
    for (x, y) in PROBES {
        let (row, col) = expected_quadrant(&doc, x, y);
        let texel = QUADRANTS[row][col];
        let want = [(texel[0] as f32 * 128.0 / 255.0) as u8, 0, 0];
        assert!(
            near(pixel_at(&r, x, y), want, 2),
            "pixel at ({x}, {y}) is {:?}, want {want:?}",
            pixel_at(&r, x, y)
        );
    }
}

#[test]
fn an_undecodable_image_keeps_the_base_color_and_is_noted() {
    let doc = box_with_top(textured(
        Rgba8::rgb(200, 100, 50),
        b"not an image".to_vec(),
        ImageFormat::Png,
    ));
    let (r, items) = render(&doc);
    assert_eq!(items[0].textures.undecodable, vec!["tex".to_string()]);
    for (x, y) in PROBES {
        assert!(
            near(pixel_at(&r, x, y), [200, 100, 50], 1),
            "pixel at ({x}, {y}) is {:?}",
            pixel_at(&r, x, y)
        );
    }
}

#[test]
fn a_textured_render_is_deterministic() {
    let doc = box_with_top(textured(
        Rgba8::rgb(255, 255, 255),
        quadrant_png(),
        ImageFormat::Png,
    ));
    let (a, _) = render(&doc);
    let (b, _) = render(&doc);
    assert_eq!(a.rgba, b.rgba);
}

#[test]
fn a_tilted_perspective_view_still_samples_the_image() {
    // Perspective-correct UVs through the near-plane clip: the eye sits just
    // above the face, so the face's far corners are clipped.
    let doc = box_with_top(textured(
        Rgba8::rgb(255, 255, 255),
        quadrant_png(),
        ImageFormat::Png,
    ));
    let items = softrender::document_items(&doc);
    let ritems: Vec<_> = items.iter().map(DocumentItem::as_render_item).collect();
    // The eye hovers over the near part of the face looking along +y, so the
    // face's near edge is behind the eye and the triangles are clipped.
    let camera = Camera {
        eye: Point3::new(0.5, 0.2, 1.05),
        target: Point3::new(0.5, 1.2, 1.0),
        up: Vec3::new(0.0, 0.0, 1.0),
        projection: Projection::Perspective { fov_y_deg: 80.0 },
    };
    let r = softrender::render(&ritems, &camera, 120, 120).unwrap();
    // The face is shaded (a grazing view), so match each quadrant by its
    // hue pattern, not its exact value.
    let hue = |p: &[u8]| -> Option<usize> {
        let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
        let m = r.max(g).max(b);
        if m < 60 {
            return None;
        }
        let on = |c: i32| c > m * 3 / 4;
        let off = |c: i32| c < m / 8;
        match (on(r), on(g), on(b)) {
            (true, false, false) if off(g) && off(b) => Some(0),
            (false, true, false) if off(r) && off(b) => Some(1),
            (false, false, true) if off(r) && off(g) => Some(2),
            (true, true, false) if off(b) => Some(3),
            _ => None,
        }
    };
    let mut seen = [false; 4];
    for p in r.rgba.as_chunks::<4>().0 {
        if let Some(h) = hue(p) {
            seen[h] = true;
        }
    }
    assert!(
        seen.iter().filter(|&&s| s).count() >= 3,
        "the image's quadrants show on the clipped face: {seen:?}"
    );
}
