//! End-to-end import tests: build a self-contained `.gltf` (JSON + a `data:`
//! base64 buffer) in memory, run it through the full parse → heal → ingest
//! pipeline, and assert the reconstructed solid is editable + watertight.

use gltf_import::buffers::base64_decode;
use kernel::Document;

// ── base64 (encode here; the crate only ships a decoder) ─────────────────────

fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(ALPHABET[(n >> 18 & 63) as usize] as char);
        out.push(ALPHABET[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[test]
fn base64_roundtrip() {
    for s in [
        &b""[..],
        &b"f"[..],
        &b"fo"[..],
        &b"foo"[..],
        &b"foob"[..],
        &b"hello, glTF \x00\x01\x02\xff"[..],
    ] {
        let enc = base64_encode(s);
        assert_eq!(base64_decode(&enc).as_deref(), Some(s), "roundtrip {s:?}");
    }
}

// ── A unit cube as a triangle-soup .gltf ─────────────────────────────────────

/// Build a minimal `.gltf` for a unit cube: 8 corners, 12 triangles (mixed
/// winding on purpose — the heal pass must orient it outward), POSITION as
/// FLOAT/VEC3 and indices as UNSIGNED_INT, all in one data-URI buffer.
fn cube_gltf() -> Vec<u8> {
    let corners: [[f32; 3]; 8] = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 1.0, 1.0],
        [0.0, 1.0, 1.0],
    ];
    // 6 faces × 2 triangles. Winding is deliberately not all-outward.
    let tris: [[u32; 3]; 12] = [
        [0, 1, 2],
        [0, 2, 3], // z=0
        [4, 5, 6],
        [4, 6, 7], // z=1
        [0, 1, 5],
        [0, 5, 4], // y=0
        [3, 2, 6],
        [3, 6, 7], // y=1
        [0, 3, 7],
        [0, 7, 4], // x=0
        [1, 2, 6],
        [1, 6, 5], // x=1
    ];

    let mut buf = Vec::new();
    for c in &corners {
        for v in c {
            buf.extend_from_slice(&v.to_le_bytes());
        }
    }
    let pos_len = buf.len(); // 96
    for t in &tris {
        for i in t {
            buf.extend_from_slice(&i.to_le_bytes());
        }
    }
    let idx_len = buf.len() - pos_len; // 144
    let total = buf.len();

    let b64 = base64_encode(&buf);
    let json = format!(
        r#"{{
  "asset": {{ "version": "2.0" }},
  "scene": 0,
  "scenes": [{{ "nodes": [0] }}],
  "nodes": [{{ "mesh": 0, "name": "Cube" }}],
  "meshes": [{{ "name": "Cube", "primitives": [
      {{ "attributes": {{ "POSITION": 0 }}, "indices": 1, "mode": 4 }}
  ]}}],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5126, "count": 8, "type": "VEC3", "min": [0,0,0], "max": [1,1,1] }},
    {{ "bufferView": 1, "componentType": 5125, "count": 36, "type": "SCALAR" }}
  ],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": 0, "byteLength": {pos_len}, "target": 34962 }},
    {{ "buffer": 0, "byteOffset": {pos_len}, "byteLength": {idx_len}, "target": 34963 }}
  ],
  "buffers": [{{ "byteLength": {total}, "uri": "data:application/octet-stream;base64,{b64}" }}]
}}"#
    );
    json.into_bytes()
}

/// A cube at metre scale with every triangle carrying its own (slightly
/// perturbed) copy of each corner — the shape three.js' flat-shaded export
/// produces, where f32 quantisation leaves coincident corners microns apart.
/// `jitter` (metres) is the per-copy offset; with it larger than the kernel's
/// 1 nm weld but smaller than the scale-aware glTF weld, only the fix
/// merges the seams back into a watertight solid.
fn cube_perface_gltf(scale: f32, jitter: f32) -> Vec<u8> {
    let c: [[f32; 3]; 8] = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 1.0, 1.0],
        [0.0, 1.0, 1.0],
    ];
    let tris: [[usize; 3]; 12] = [
        [0, 1, 2],
        [0, 2, 3],
        [4, 5, 6],
        [4, 6, 7],
        [0, 1, 5],
        [0, 5, 4],
        [3, 2, 6],
        [3, 6, 7],
        [0, 3, 7],
        [0, 7, 4],
        [1, 2, 6],
        [1, 6, 5],
    ];

    // Flatten to 36 per-corner vertices, each x nudged by a deterministic
    // sub-weld jitter so shared corners are *near* (not exactly) coincident.
    let mut verts: Vec<[f32; 3]> = Vec::new();
    for (ti, t) in tris.iter().enumerate() {
        for (ci, &corner) in t.iter().enumerate() {
            let j = (((ti * 3 + ci) % 5) as f32 - 2.0) * jitter; // [-2j, 2j]
            verts.push([
                c[corner][0] * scale + j,
                c[corner][1] * scale,
                c[corner][2] * scale,
            ]);
        }
    }

    let mut buf = Vec::new();
    for v in &verts {
        for x in v {
            buf.extend_from_slice(&x.to_le_bytes());
        }
    }
    let pos_len = buf.len();
    for i in 0..verts.len() as u32 {
        buf.extend_from_slice(&i.to_le_bytes());
    }
    let idx_len = buf.len() - pos_len;
    let total = buf.len();
    let count = verts.len();
    let hi = scale + 2.0 * jitter;

    let b64 = base64_encode(&buf);
    format!(
        r#"{{
  "asset": {{ "version": "2.0" }},
  "scene": 0,
  "scenes": [{{ "nodes": [0] }}],
  "nodes": [{{ "mesh": 0, "name": "Cube" }}],
  "meshes": [{{ "primitives": [{{ "attributes": {{ "POSITION": 0 }}, "indices": 1, "mode": 4 }}] }}],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5126, "count": {count}, "type": "VEC3", "min": [-1,0,0], "max": [{hi},{scale},{scale}] }},
    {{ "bufferView": 1, "componentType": 5125, "count": {count}, "type": "SCALAR" }}
  ],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": 0, "byteLength": {pos_len}, "target": 34962 }},
    {{ "buffer": 0, "byteOffset": {pos_len}, "byteLength": {idx_len}, "target": 34963 }}
  ],
  "buffers": [{{ "byteLength": {total}, "uri": "data:application/octet-stream;base64,{b64}" }}]
}}"#
    )
    .into_bytes()
}

#[test]
fn f32_scale_cube_welds_to_a_watertight_solid() {
    // 30 m cube, per-corner copies ~5 µm apart — far above the 1 nm native weld,
    // so without the scale-aware glTF weld this imports as a leaky triangle soup.
    let bytes = cube_perface_gltf(30.0, 2.5e-6);
    let (scene, missing) = gltf_import::import(&bytes).expect("import scaled cube");

    let mut doc = Document::new();
    let (report, _) = doc.ingest(scene, missing).expect("ingest");
    assert_eq!(report.objects_created, 1, "one object");
    assert_eq!(report.watertight, 1, "f32-scale cube must weld watertight");
    assert_eq!(report.leaky, 0, "no leaky shell");
}

/// Two triangles forming a quad, in two primitives that reference *different*
/// glTF materials which nonetheless share the same color + texture image. The
/// importer must deduplicate them to a single kernel material (so the image is
/// stored once, not per-material — the fix for the OOM on large SketchUp→glTF
/// models where hundreds of materials shared a handful of images).
fn shared_texture_gltf() -> Vec<u8> {
    // Quad in the XY plane.
    let pos: [[f32; 3]; 4] = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ];
    let idx_a: [u32; 3] = [0, 1, 2];
    let idx_b: [u32; 3] = [0, 2, 3];
    let image: [u8; 16] = *b"\x89PNG\r\n\x1a\n_faketex"; // bytes are opaque to the kernel

    let mut buf = Vec::new();
    for p in &pos {
        for v in p {
            buf.extend_from_slice(&v.to_le_bytes());
        }
    }
    let off_a = buf.len();
    for i in &idx_a {
        buf.extend_from_slice(&i.to_le_bytes());
    }
    let off_b = buf.len();
    for i in &idx_b {
        buf.extend_from_slice(&i.to_le_bytes());
    }
    let off_img = buf.len();
    buf.extend_from_slice(&image);
    let total = buf.len();
    let b64 = base64_encode(&buf);

    let tex_mat = r#"{ "pbrMetallicRoughness": { "baseColorFactor": [1,1,1,1], "baseColorTexture": { "index": 0 } } }"#;
    format!(
        r#"{{
  "asset": {{ "version": "2.0" }},
  "scene": 0,
  "scenes": [{{ "nodes": [0] }}],
  "nodes": [{{ "mesh": 0 }}],
  "meshes": [{{ "primitives": [
    {{ "attributes": {{ "POSITION": 0 }}, "indices": 1, "material": 0, "mode": 4 }},
    {{ "attributes": {{ "POSITION": 0 }}, "indices": 2, "material": 1, "mode": 4 }}
  ] }}],
  "materials": [{tex_mat}, {tex_mat}, {tex_mat}],
  "textures": [{{ "source": 0 }}],
  "images": [{{ "bufferView": 3, "mimeType": "image/png" }}],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3", "min": [0,0,0], "max": [1,1,0] }},
    {{ "bufferView": 1, "componentType": 5125, "count": 3, "type": "SCALAR" }},
    {{ "bufferView": 2, "componentType": 5125, "count": 3, "type": "SCALAR" }}
  ],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": 0, "byteLength": {off_a} }},
    {{ "buffer": 0, "byteOffset": {off_a}, "byteLength": 12 }},
    {{ "buffer": 0, "byteOffset": {off_b}, "byteLength": 12 }},
    {{ "buffer": 0, "byteOffset": {off_img}, "byteLength": 16 }}
  ],
  "buffers": [{{ "byteLength": {total}, "uri": "data:application/octet-stream;base64,{b64}" }}]
}}"#
    )
    .into_bytes()
}

#[test]
fn materials_sharing_a_texture_are_deduplicated() {
    let bytes = shared_texture_gltf();
    let (scene, _missing) = gltf_import::import(&bytes).expect("import");

    // Three glTF materials, one shared image → a single kernel material.
    assert_eq!(
        scene.materials.len(),
        1,
        "shared color+image deduped to one material"
    );
    assert!(
        scene.materials[0].has_texture(),
        "the deduped material keeps its texture"
    );

    // Both triangles (different glTF materials, remapped to the same one) merge
    // into the single quad face.
    let kernel::ImportNode::Mesh(recipe) = &scene.roots[0] else {
        panic!("expected a Mesh root");
    };
    assert_eq!(
        recipe.faces.len(),
        1,
        "coplanar same-material triangles merge to one face"
    );
    assert_eq!(
        recipe.face_materials,
        vec![0],
        "face references the deduped material 0"
    );
}

#[test]
fn cube_imports_as_one_watertight_object_with_six_faces() {
    let bytes = cube_gltf();
    let (scene, missing) = gltf_import::import(&bytes).expect("import cube");
    assert!(missing.is_empty(), "no missing resources");

    // One root, a single Mesh (single-use mesh → baked world Object).
    assert_eq!(scene.roots.len(), 1, "one root node");
    let kernel::ImportNode::Mesh(recipe) = &scene.roots[0] else {
        panic!("expected a Mesh root, got a group/instance");
    };
    // Heal's coplanar-triangle merge must reassemble the 12 triangles back into
    // the cube's 6 editable quad faces.
    assert_eq!(recipe.faces.len(), 6, "12 triangles merged into 6 faces");

    // Ingest: the reconstructed solid must be watertight.
    let mut doc = Document::new();
    let (report, _) = doc.ingest(scene, missing).expect("ingest");
    assert_eq!(report.objects_created, 1, "one object");
    assert_eq!(report.watertight, 1, "watertight");
    assert_eq!(report.leaky, 0, "not leaky");
    assert!(report.skipped.is_empty(), "nothing skipped");
}
