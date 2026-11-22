//! Executable specs for geometry buffer v10 and the smooth-chain manifest
//! entry (v30) (true-curves.md §3.2; `docs/dev/HEW_FILE_FORMAT.md` §3): the three new
//! surface kinds, crease flags and blend tags round-trip; v8 buffers and a
//! real v8 file still load; v8 content keeps its canonical order; every new
//! decode rule refuses tampered input typed; and sketches persist the E1
//! smooth-chain marker.

use crate::common;
use kernel::trig;

use std::io::{Cursor, Read as _};

use common::curves::*;
use kernel::{
    BlendId, BlendKind, BlendRole, BlendTag, DecodeError, Document, FaceId, KernelOp, LoadError,
    MaterialId, Object, Point3, SurfaceRef, TopologyError, Vec3,
};
use slotmap::{Key, KeyData};

// ------------------------------------------------------------ helpers

fn mat_of(i: u32) -> Option<MaterialId> {
    Some(MaterialId::from(KeyData::from_ffi(
        (1u64 << 32) | (u64::from(i) + 1),
    )))
}

fn dense_of(m: MaterialId) -> u32 {
    ((m.data().as_ffi() & 0xffff_ffff) - 1) as u32
}

fn encode(o: &Object) -> Vec<u8> {
    o.encode(&dense_of)
}

fn decode(b: &[u8]) -> Result<Object, DecodeError> {
    Object::decode(b, &mat_of)
}

/// Byte offsets of one face block in a geometry buffer.
#[derive(Debug, Clone, Copy)]
struct FaceAt {
    start: usize,
    /// The surface kind byte.
    kind: usize,
    /// The v9 face dictionary flag byte (absent below v9).
    attrs: usize,
    /// The v10 crease "any" byte (absent below v10).
    crease: usize,
    /// The v10 blend flag byte (absent below v10).
    blend: usize,
    /// The v11 plane flag byte, right after the hole loops (absent below
    /// v11).
    plane: Option<usize>,
    end: usize,
}

/// Walks a buffer of `version` (read from its header) and returns the
/// header length and every face block's offsets. A test-side reader of the
/// documented layout, independent of the kernel's decoder.
fn walk(b: &[u8]) -> (usize, Vec<FaceAt>) {
    let u32_at = |p: usize| u32::from_le_bytes(b[p..p + 4].try_into().unwrap()) as usize;
    let version = u32_at(4);
    let mut p = 4 + 4 + 1 + 4 + usize::from(version >= 3);
    let vcount = u32_at(p);
    p += 4 + 24 * vcount;
    let fcount = u32_at(p);
    p += 4;
    let header = p;
    let mut faces = Vec::with_capacity(fcount);
    for _ in 0..fcount {
        let start = p;
        p += 4; // material
        if b[p] == 1 {
            p += 64;
        }
        p += 1; // uv flag
        let kind = p;
        p += 1 + 8 * match b[kind] {
            0 => 0,
            1 => 7,
            2 => 4,
            3 => 8,
            4 => 7,
            k => panic!("kind {k}"),
        };
        let k = u32_at(p);
        p += 4 + 4 * k;
        let holes = u32_at(p);
        p += 4;
        let mut edges = k;
        for _ in 0..holes {
            let j = u32_at(p);
            p += 4 + 4 * j;
            edges += j;
        }
        // the face's plane (v11)
        let plane = (version >= 11).then_some(p);
        if version >= 11 {
            p += 1 + if b[p] == 1 { 32 } else { 0 };
        }
        // soft
        let any = b[p];
        p += 1 + if any == 1 { edges } else { 0 };
        // curves, polygons
        for _ in 0..2 {
            let any = b[p];
            p += 1;
            if any == 1 {
                for _ in 0..edges {
                    let flag = b[p];
                    p += 1 + if flag == 1 { 32 } else { 0 };
                }
            }
        }
        // back paint
        let flag = b[p];
        p += 1 + if flag == 1 { 4 } else { 0 };
        // attribute dictionaries (v9): the face's, then its edges'
        let attrs = p;
        if version >= 9 {
            let skip_dict = |p: &mut usize| {
                let flag = b[*p];
                *p += 1;
                if flag == 1 {
                    let len = u32_at(*p);
                    *p += 4 + len;
                }
            };
            skip_dict(&mut p);
            let any = b[p];
            p += 1;
            if any == 1 {
                for _ in 0..edges {
                    skip_dict(&mut p);
                }
            }
        }
        let (crease, blend) = if version >= 10 {
            let crease = p;
            p += 1 + if b[crease] == 1 { edges } else { 0 };
            let blend = p;
            p += 1 + if b[blend] == 1 { 6 } else { 0 };
            (crease, blend)
        } else {
            (p, p)
        };
        faces.push(FaceAt {
            start,
            kind,
            attrs,
            crease,
            blend,
            plane,
            end: p,
        });
    }
    assert_eq!(p, b.len(), "the walker consumed the whole buffer");
    (header, faces)
}

/// A current buffer with no v9, v10 or v11 content, rewritten as the v8
/// buffer the previous encoders wrote: header version 8, each face's plane
/// flag (no stored plane) and its four trailing zero bytes (no face or edge
/// dictionary, no creases, no blend tag) stripped.
fn as_v8(b: &[u8]) -> Vec<u8> {
    let (header, faces) = walk(b);
    let mut out = b[..header].to_vec();
    out[4..8].copy_from_slice(&8u32.to_le_bytes());
    for f in faces {
        assert!(b[f.kind] <= 1, "only v8 surface kinds");
        assert_eq!(
            (b[f.attrs], b[f.attrs + 1], b[f.crease], b[f.blend]),
            (0, 0, 0, 0),
            "no v9 or v10 content"
        );
        assert_eq!(f.end, f.attrs + 4);
        match f.plane {
            Some(at) => {
                assert_eq!(b[at], 0, "no stored plane");
                out.extend_from_slice(&b[f.start..at]);
                out.extend_from_slice(&b[at + 1..f.attrs]);
            }
            None => out.extend_from_slice(&b[f.start..f.attrs]),
        }
    }
    out
}

fn corrupt(b: &[u8]) -> &'static str {
    match decode(b) {
        Err(DecodeError::Corrupt { what, .. }) => what,
        other => panic!("expected Corrupt, got {other:?}"),
    }
}

fn tag(id: u32, kind: BlendKind, role: BlendRole) -> BlendTag {
    BlendTag {
        id: BlendId(id),
        kind,
        role,
    }
}

/// The unit box with every face tagged (each kind and role appears) and two
/// creased edges.
fn tagged_creased_box() -> Object {
    let mut o = unit_box();
    let faces: Vec<FaceId> = o.faces().keys().collect();
    let tags = [
        tag(1, BlendKind::Fillet, BlendRole::Strip),
        tag(1, BlendKind::Fillet, BlendRole::Corner),
        tag(2, BlendKind::Chamfer, BlendRole::Runout),
        tag(3, BlendKind::Profile, BlendRole::Strip),
        tag(0, BlendKind::Chamfer, BlendRole::Corner),
    ];
    for (f, t) in faces.iter().zip(tags) {
        o.set_face_blend_for_test(*f, Some(t));
    }
    let e1 = edge_between(&o, p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0));
    let e2 = edge_between(&o, p3(1.0, 1.0, 1.0), p3(0.0, 1.0, 1.0));
    o.set_edge_crease_for_test(e1, true);
    o.set_edge_crease_for_test(e2, true);
    o.validate().unwrap();
    o
}

fn creased_count(o: &Object) -> usize {
    o.edges().values().filter(|e| e.crease).count()
}

fn blend_census(o: &Object) -> Vec<(u32, u8, u8)> {
    let mut v: Vec<(u32, u8, u8)> = o
        .faces()
        .values()
        .filter_map(|f| f.blend)
        .map(|t| (t.id.0, t.kind as u8, t.role as u8))
        .collect();
    v.sort_unstable();
    v
}

fn zip_entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..zip.len())
        .map(|i| {
            let mut e = zip.by_index(i).unwrap();
            let mut buf = Vec::new();
            e.read_to_end(&mut buf).unwrap();
            (e.name().unwrap().to_string(), buf)
        })
        .collect()
}

fn manifest_of(bytes: &[u8]) -> serde_json::Value {
    let (_, m) = zip_entries(bytes)
        .into_iter()
        .find(|(n, _)| n == "manifest.json")
        .unwrap();
    serde_json::from_slice(&m).unwrap()
}

fn representative_v8() -> Vec<u8> {
    std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/representative_v8.hew"),
    )
    .unwrap()
}

// ---------------------------------------------------------- 1. classes

#[test]
fn v9_round_trips_every_surface_kind() {
    for (o, f, s) in [
        sphere_facet(),
        torus_facet(),
        cone_facet(),
        cylinder_facet(),
    ] {
        let claimed = with_claim(&o, f, s);
        let bytes = encode(&claimed);
        let back = decode(&bytes).unwrap();
        assert_eq!(
            encode(&back),
            bytes,
            "{s:?}: save, load, save is byte-identical"
        );
        let got = back.faces().values().next().unwrap().surface;
        assert_eq!(got, Some(s), "the claim comes back equal by ==");
    }
}

// ---------------------------------------------- 2. blend tags, creases

#[test]
fn v9_round_trips_blend_tags_and_creases() {
    let o = tagged_creased_box();
    let bytes = encode(&o);
    let back = decode(&bytes).unwrap();
    assert_eq!(encode(&back), bytes);
    assert_eq!(blend_census(&back), blend_census(&o));
    assert_eq!(creased_count(&back), 2);
    for (a, b) in [
        (p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
        (p3(1.0, 1.0, 1.0), p3(0.0, 1.0, 1.0)),
    ] {
        assert!(back.edges()[edge_between(&back, a, b)].crease);
    }
}

// ------------------------------------------------ 3. v8 still loads

#[test]
fn v8_buffers_load_under_v9() {
    // Stamped walls and cap circle claims exercise the surface and v5
    // blocks the walker skips.
    let o = cylinder(0.5, 1.0, 24);
    let v9 = encode(&o);
    let v8 = as_v8(&v9);
    let back = decode(&v8).unwrap();
    assert!(back.edges().values().all(|e| !e.crease));
    assert!(back.faces().values().all(|f| f.blend.is_none()));
    assert_eq!(
        encode(&back),
        v9,
        "a v8 buffer re-saves as the same v9 buffer"
    );
    back.validate().unwrap();
}

// ------------------------------------------------- 4. decode rules

#[test]
fn surface_kind_out_of_range_is_corrupt() {
    let (o, f, s) = sphere_facet();
    let mut b = encode(&with_claim(&o, f, s));
    let (_, faces) = walk(&b);
    b[faces[0].kind] = 5;
    assert_eq!(corrupt(&b), "surface kind out of range");
    // A v8 buffer may hold only kinds 0 and 1.
    let mut v8 = as_v8(&encode(&with_claim(
        &cylinder_facet().0,
        f,
        cylinder_facet().2,
    )));
    let (_, faces) = walk(&v8);
    v8[faces[0].kind] = 2;
    assert_eq!(corrupt(&v8), "surface flag must be 0 or 1");
}

#[test]
fn surface_payloads_out_of_range_are_corrupt() {
    // Each class with one parameter tampered; the payload starts after the
    // kind byte, in wire order.
    type Facet = fn() -> (Object, FaceId, SurfaceRef);
    let cases: [(Facet, usize, f64, &str); 5] = [
        (sphere_facet, 3, 0.0, "sphere radius out of range"),
        (torus_facet, 6, 0.0, "torus major radius out of range"),
        (torus_facet, 7, f64::NAN, "torus minor radius out of range"),
        (torus_facet, 5, 2.0, "torus axis is not a unit vector"),
        (
            cone_facet,
            6,
            std::f64::consts::FRAC_PI_2,
            "cone half-angle out of range",
        ),
    ];
    for (make, slot, value, what) in cases {
        let (o, f, s) = make();
        let mut b = encode(&with_claim(&o, f, s));
        let (_, faces) = walk(&b);
        let at = faces[0].kind + 1 + 8 * slot;
        b[at..at + 8].copy_from_slice(&value.to_le_bytes());
        assert_eq!(corrupt(&b), what);
    }
    // The cone axis gets its own message.
    let (o, f, s) = cone_facet();
    let mut b = encode(&with_claim(&o, f, s));
    let (_, faces) = walk(&b);
    let at = faces[0].kind + 1 + 8 * 5;
    b[at..at + 8].copy_from_slice(&3.0f64.to_le_bytes());
    assert_eq!(corrupt(&b), "cone axis is not a unit vector");
}

#[test]
fn crease_and_blend_tampering_is_rejected() {
    let o = tagged_creased_box();
    let clean = encode(&o);
    let (_, faces) = walk(&clean);

    // A crease flag byte other than 0/1.
    let f = faces.iter().find(|f| clean[f.crease] == 1).unwrap();
    let mut b = clean.clone();
    let first_flag = (f.crease + 1..f.blend).find(|&i| b[i] == 1).unwrap();
    b[first_flag] = 2;
    assert_eq!(corrupt(&b), "crease flag must be 0 or 1");

    // One face of a shared edge drops its crease: the two disagree.
    let mut b = clean.clone();
    b[first_flag] = 0;
    assert_eq!(corrupt(&b), "shared edge has disagreeing crease flags");

    // Blend kind and role outside 0–2.
    let f = faces.iter().find(|f| clean[f.blend] == 1).unwrap();
    let mut b = clean.clone();
    b[f.blend + 5] = 3;
    assert_eq!(corrupt(&b), "blend kind out of range");
    let mut b = clean.clone();
    b[f.blend + 6] = 3;
    assert_eq!(corrupt(&b), "blend role out of range");
    let mut b = clean.clone();
    b[f.blend] = 2;
    assert_eq!(corrupt(&b), "blend flag must be 0 or 1");
}

#[test]
fn crease_with_soft_fails_validation() {
    // A creased wall seam whose soft flag a tampered file turns on: the
    // decoder ORs soft, so the edge loads soft AND creased, and the
    // validator refuses the pair.
    let mut o = cylinder(0.5, 1.0, 24);
    let seam = o
        .edges()
        .iter()
        .find(|(_, e)| {
            let he = &o.half_edges()[e.half_edge];
            let a = o.vertices()[he.origin].position;
            let b = o.vertices()[o.half_edges()[he.next].origin].position;
            (a.z - b.z).abs() > 0.5
        })
        .map(|(id, _)| id)
        .unwrap();
    o.set_edge_crease_for_test(seam, true);
    o.validate().unwrap();
    let mut b = encode(&o);
    // Turn on the soft flag of the creased seam on both of its faces: the
    // faces whose crease block is present carry it at the same per-edge
    // position in their soft block.
    let (_, faces) = walk(&b);
    let f = faces.iter().find(|f| b[f.crease] == 1).unwrap();
    let n = f.blend - (f.crease + 1);
    let k = (0..n).find(|&i| b[f.crease + 1 + i] == 1).unwrap();
    // Rebuild that face's soft block: an "any" of 0 becomes 1 + flags.
    let soft_any = soft_any_offset(&b, f);
    assert_eq!(b[soft_any], 0, "no soft flags on a stamped cylinder");
    let mut flags = vec![0u8; n];
    flags[k] = 1;
    let mut nb = b[..soft_any].to_vec();
    nb.push(1);
    nb.extend_from_slice(&flags);
    nb.extend_from_slice(&b[soft_any + 1..]);
    b = nb;
    match decode(&b) {
        Err(DecodeError::InvalidTopology(TopologyError::EdgeCreaseConflict { .. })) => {}
        other => panic!("expected EdgeCreaseConflict, got {other:?}"),
    }
}

/// The soft-block "any" byte of face `f` (right after its hole loops, and
/// from v11 after its plane flag).
fn soft_any_offset(b: &[u8], f: &FaceAt) -> usize {
    if let Some(at) = f.plane {
        return at + 1 + if b[at] == 1 { 32 } else { 0 };
    }
    let u32_at = |p: usize| u32::from_le_bytes(b[p..p + 4].try_into().unwrap()) as usize;
    let mut p = f.kind;
    p += 1 + 8 * match b[f.kind] {
        0 => 0,
        1 => 7,
        2 => 4,
        3 => 8,
        _ => 7,
    };
    let k = u32_at(p);
    p += 4 + 4 * k;
    let holes = u32_at(p);
    p += 4;
    for _ in 0..holes {
        let j = u32_at(p);
        p += 4 + 4 * j;
    }
    p
}

// --------------------------------------- 5. canonical order of v8 content

#[test]
fn canonical_order_unchanged_for_v8_content() {
    // Every geometry buffer of a real v8 file, decoded and re-encoded by the
    // v9 writer, equals its v8 bytes once the version is set back and the two
    // trailing zero bytes per face are stripped.
    let v8_file = representative_v8();
    let mut seen = 0;
    for (name, bytes) in zip_entries(&v8_file) {
        if !name.starts_with("geometry/") {
            continue;
        }
        let o = decode(&bytes).unwrap();
        let v9 = encode(&o);
        assert_eq!(
            as_v8(&v9),
            bytes,
            "{name}: v8 content sorts and writes as before"
        );
        seen += 1;
    }
    assert!(seen >= 2);
}

#[test]
fn creases_and_tags_reach_the_bytes_deterministically() {
    // Boxes differing only in a blend tag or a crease encode to different
    // bytes, and each encodes deterministically. (The two v9 keys sit after
    // every older key in the canonical face order; two faces that tie on all
    // older keys cannot exist in one valid object, so that order is pinned
    // by `canonical_order_unchanged_for_v8_content`.)
    let plain = unit_box();
    let mut tagged = unit_box();
    let f = face_with_normal(&tagged, Vec3::new(0.0, -1.0, 0.0));
    tagged.set_face_blend_for_test(f, Some(tag(1, BlendKind::Fillet, BlendRole::Strip)));
    let mut creased = unit_box();
    let e = edge_between(&creased, p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0));
    creased.set_edge_crease_for_test(e, true);
    assert_ne!(
        encode(&tagged),
        encode(&plain),
        "the tag is part of the bytes"
    );
    assert_ne!(
        encode(&creased),
        encode(&plain),
        "the crease is part of the bytes"
    );
    assert_ne!(encode(&tagged), encode(&creased));
    assert_eq!(encode(&tagged), encode(&tagged.clone()), "deterministic");
    assert_eq!(encode(&creased), encode(&creased.clone()), "deterministic");
}

// ------------------------------------------------------ 6. manifest v23

/// A document with one sketch holding an analytic-less chain marked smooth,
/// plus a plain circle; the sketch is never extruded.
fn doc_with_smooth_chain() -> Document {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        let c = sk.begin_curve();
        for i in 0..6 {
            let a = |k: usize| {
                let t = std::f64::consts::PI * k as f64 / 6.0;
                p3(2.0 * trig::cos(t), trig::sin(t), 0.0)
            };
            sk.add_segment(a(i), a(i + 1)).unwrap();
        }
        sk.end_curve();
        sk.mark_curve_smooth_for_test(c);
        add_circle(sk, p3(5.0, 0.0, 0.0), Z, 0.5, 24);
    }
    doc.end_sketch_gesture(s).unwrap();
    doc
}

fn smooth_chains(doc: &Document) -> usize {
    doc.sketch_ids()
        .into_iter()
        .map(|s| {
            let sk = doc.sketch(s).unwrap();
            let curves: std::collections::BTreeSet<_> =
                sk.edges().values().filter_map(|e| e.curve).collect();
            curves
                .into_iter()
                .filter(|&c| sk.curve_is_smooth(c) && sk.curve_analytic(c).is_none())
                .count()
        })
        .sum()
}

#[test]
fn smooth_chain_round_trips_v23() {
    let doc = doc_with_smooth_chain();
    assert_eq!(smooth_chains(&doc), 1);
    let bytes = doc.save();
    let m = manifest_of(&bytes);
    let curves = &m["sketches"][0]["curves"];
    let smooth: Vec<&serde_json::Value> = curves
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["kind"] == "smooth")
        .collect();
    assert_eq!(smooth.len(), 1);
    assert!(smooth[0].get("center").is_none() && smooth[0].get("radius").is_none());
    let back = Document::load(&bytes).unwrap();
    assert_eq!(smooth_chains(&back), 1);
    assert_eq!(back.save(), bytes, "save, load, save is byte-identical");
}

#[test]
fn smooth_entry_in_v29_manifest_is_rejected() {
    let bytes = doc_with_smooth_chain().save();
    let patched = with_patched_manifest(&bytes, |m| m["format_version"] = 29.into());
    match Document::load(&patched) {
        Err(LoadError::MalformedManifest { what }) => {
            assert!(
                what.contains("smooth curve entry in a pre-v30 manifest"),
                "{what}"
            );
        }
        other => panic!("expected MalformedManifest, got {other:?}"),
    }
    // A smooth entry carrying geometry, and a circle entry missing it, are
    // malformed at any version.
    for patch in [
        (|m: &mut serde_json::Value| {
            let c = m["sketches"][0]["curves"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|c| c["kind"] == "smooth")
                .unwrap();
            c["radius"] = 1.0.into();
        }) as fn(&mut serde_json::Value),
        |m: &mut serde_json::Value| {
            let c = m["sketches"][0]["curves"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|c| c.get("kind").is_none())
                .unwrap();
            c.as_object_mut().unwrap().remove("center");
        },
    ] {
        assert!(matches!(
            Document::load(&with_patched_manifest(&bytes, patch)),
            Err(LoadError::MalformedManifest { .. })
        ));
    }
}

#[test]
fn v22_document_saves_byte_identical_sketch_entries() {
    // The representative v8/v22 file's sketches re-save with identical JSON.
    let v8_file = representative_v8();
    let doc = Document::load(&v8_file).unwrap();
    let before = manifest_of(&v8_file);
    let after = manifest_of(&doc.save());
    assert_eq!(after["format_version"], 33);
    assert_eq!(after["sketches"], before["sketches"]);
}

// ---------------------------------------------- 7. a real v8 file

#[test]
fn real_v8_file_loads_and_resaves() {
    let v8_file = representative_v8();
    assert_eq!(manifest_of(&v8_file)["geometry_version"], 8);
    let doc = Document::load(&v8_file).unwrap();
    let resaved = doc.save();
    assert_eq!(manifest_of(&resaved)["geometry_version"], 11);
    let again = Document::load(&resaved).unwrap();
    let ids = doc.visible_object_ids();
    assert_eq!(ids.len(), again.visible_object_ids().len());
    for (a, b) in ids.iter().zip(again.visible_object_ids()) {
        let (x, y) = (doc.object(*a).unwrap(), again.object(b).unwrap());
        assert!(objects_equivalent(x, y), "geometry survives the resave");
        assert_eq!(claim_census(x), claim_census(y));
    }
}

// ------------------------------------------------- 9. propagation

#[test]
fn crease_survives_edge_split_and_undo() {
    // The bottom front edge is creased; a cut across the front face from its
    // midpoint splits it. Both fragments keep the crease; undo restores one
    // creased edge.
    let mut o = unit_box();
    let e = edge_between(&o, p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0));
    o.set_edge_crease_for_test(e, true);
    o.validate().unwrap();
    let front = face_with_normal(&o, Vec3::new(0.0, -1.0, 0.0));
    let mut h = kernel::History::new();
    h.apply(
        &mut o,
        KernelOp::SplitFace {
            face: front,
            path: vec![p3(0.5, 0.0, 0.0), p3(0.5, 0.0, 1.0)],
            restore: None,
            curves: vec![],
        },
    )
    .unwrap();
    o.validate().unwrap();
    let left = edge_between(&o, p3(0.0, 0.0, 0.0), p3(0.5, 0.0, 0.0));
    let right = edge_between(&o, p3(0.5, 0.0, 0.0), p3(1.0, 0.0, 0.0));
    assert!(o.edges()[left].crease && o.edges()[right].crease);
    assert_eq!(creased_count(&o), 2);
    h.undo(&mut o).unwrap();
    o.validate().unwrap();
    assert_eq!(creased_count(&o), 1);
    assert!(o.edges()[edge_between(&o, p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0))].crease);
}

#[test]
fn blend_tag_inherited_by_split_children() {
    let mut o = unit_box();
    let top = face_with_normal(&o, Z);
    let t = tag(4, BlendKind::Fillet, BlendRole::Strip);
    o.set_face_blend_for_test(top, Some(t));
    let report = o
        .split_face(top, &[p3(0.5, 0.0, 1.0), p3(0.5, 1.0, 1.0)])
        .unwrap();
    o.validate().unwrap();
    let tagged: Vec<_> = o.faces().values().filter(|f| f.blend == Some(t)).collect();
    assert_eq!(tagged.len(), 2, "both halves of the split carry the tag");
    let _ = report;
    assert_eq!(o.max_blend_id(), Some(BlendId(4)));
    assert_eq!(o.next_blend_id(), BlendId(5));
    assert_eq!(unit_box().next_blend_id(), BlendId(1));
}

#[test]
fn ordinary_push_pull_drops_the_tag_of_the_moved_face_only() {
    let mut cube = unit_box();
    let ids: Vec<FaceId> = cube.faces().keys().collect();
    let t = tag(1, BlendKind::Chamfer, BlendRole::Strip);
    for f in &ids {
        cube.set_face_blend_for_test(*f, Some(t));
    }
    let top = face_with_normal(&cube, Z);
    cube.push_pull(top, 0.5).unwrap();
    let top_now = face_with_normal(&cube, Z);
    assert_eq!(cube.faces()[top_now].blend, None);
    assert_eq!(
        cube.faces().values().filter(|f| f.blend == Some(t)).count(),
        5
    );
}

#[test]
fn similarity_keeps_tags_and_creases_but_a_stretch_drops_the_tag() {
    use kernel::Transform;
    let mut cube = unit_box();
    let f = face_with_normal(&cube, Vec3::new(0.0, -1.0, 0.0));
    let t = tag(1, BlendKind::Fillet, BlendRole::Strip);
    cube.set_face_blend_for_test(f, Some(t));
    let e = edge_between(&cube, p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0));
    cube.set_edge_crease_for_test(e, true);
    cube.validate().unwrap();

    let mut scaled = cube.clone();
    scaled
        .apply_transform(&Transform::uniform_scale(2.0))
        .unwrap();
    assert_eq!(scaled.faces()[f].blend, Some(t), "similarity keeps the tag");
    assert_eq!(creased_count(&scaled), 1);

    let mut stretched = cube.clone();
    stretched
        .apply_transform(&Transform::scale(Vec3::new(2.0, 1.0, 1.0)))
        .unwrap();
    assert_eq!(stretched.faces()[f].blend, None, "a stretch drops the tag");
    assert_eq!(
        creased_count(&stretched),
        1,
        "a crease is about the edge, not its shape: it survives"
    );
}

// ------------------------------------------------------ shared helpers

fn objects_equivalent(x: &Object, y: &Object) -> bool {
    let rings = |o: &Object| -> Vec<Vec<Point3>> {
        o.faces()
            .values()
            .map(|f| o.loop_positions(f.outer_loop).collect())
            .collect()
    };
    let cyclic = |a: &[Point3], b: &[Point3]| {
        a.len() == b.len()
            && (0..a.len()).any(|off| {
                (0..a.len())
                    .all(|i| a[i].approx_eq(b[(i + off) % a.len()], kernel::tol::POINT_MERGE))
            })
    };
    let xs = rings(x);
    let mut ys = rings(y);
    if xs.len() != ys.len() {
        return false;
    }
    for r in &xs {
        match ys.iter().position(|c| cyclic(r, c)) {
            Some(i) => {
                ys.swap_remove(i);
            }
            None => return false,
        }
    }
    true
}

fn with_patched_manifest(bytes: &[u8], patch: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    use std::io::Write as _;
    let entries = zip_entries(bytes);
    let mut manifest: serde_json::Value = manifest_of(bytes);
    patch(&mut manifest);
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::<u8>::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    for (name, data) in entries {
        out.start_file(name.as_str(), opts).unwrap();
        if name == "manifest.json" {
            out.write_all(&serde_json::to_vec_pretty(&manifest).unwrap())
                .unwrap();
        } else {
            out.write_all(&data).unwrap();
        }
    }
    out.finish().unwrap().into_inner()
}
