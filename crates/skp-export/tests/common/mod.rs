//! The canonical digest of a Hew document both round-trip suites compare:
//! see `roundtrip.rs` for what it covers and normalizes away.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use kernel::{AttrTarget, Document, EntityRef, NodeId, Object, Point3};

// ── Targets ───────────────────────────────────────────────────────────────

/// Every container and every stream release whose layout carries all of
/// the content these suites compare (the 2026 container, then releases 5
/// to 2020). Releases 3 and 4 have no layer or attribute records, and
/// release 3 neither hidden state nor radial dimensions: they are covered
/// by `native.rs`'s every-release test, which checks what they do hold.
pub fn all_targets() -> Vec<skp_export::Target> {
    std::iter::once(skp_export::Target::Skp2026)
        .chain(
            skpwright::Release::ALL
                .into_iter()
                .filter(|r| *r >= skpwright::Release::R5)
                .map(skp_export::Target::Stream),
        )
        .collect()
}

// ── Digest ────────────────────────────────────────────────────────────────

pub type P = [i64; 3];

/// Positions per meter a digest resolves: a micron. A coarser digest
/// (`set_quantum`) tells a vertex a micron off from a real difference
/// when that micron reorders siblings.
static QUANTUM: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1_000_000);

/// Set the digest's resolution (positions per meter) for this process.
pub fn set_quantum(per_meter: u64) {
    QUANTUM.store(per_meter, std::sync::atomic::Ordering::Relaxed);
}

fn q(v: f64) -> i64 {
    (v * QUANTUM.load(std::sync::atomic::Ordering::Relaxed) as f64).round() as i64
}

fn qp(p: Point3) -> P {
    [q(p.x), q(p.y), q(p.z)]
}

/// Rotate a ring so it starts at its smallest point (rings keep their
/// direction — winding is meaningful).
fn canon_ring(mut r: Vec<P>) -> Vec<P> {
    if let Some(i) = r
        .iter()
        .enumerate()
        .min_by_key(|(_, p)| **p)
        .map(|(i, _)| i)
    {
        r.rotate_left(i);
    }
    r
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub struct FaceD {
    outer: Vec<P>,
    holes: Vec<Vec<P>>,
    material: Option<String>,
    /// Textured faces only: the UV of each outer corner (1e-4).
    uv: Vec<[i64; 2]>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub enum NodeD {
    Object {
        name: Option<String>,
        tags: Vec<Vec<String>>,
        hidden: bool,
        faces: Vec<FaceD>,
        /// Soft edges, circle-claimed edges, faces on a curved surface.
        claims: [usize; 3],
        attrs: String,
    },
    Group {
        name: Option<String>,
        tags: Vec<Vec<String>>,
        hidden: bool,
        children: Vec<NodeD>,
        attrs: String,
    },
    Instance {
        def: Box<DefD>,
        pose: Vec<i64>,
        name: Option<String>,
        tags: Vec<Vec<String>>,
        hidden: bool,
        attrs: String,
    },
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub struct DefD {
    name: Option<String>,
    members: Vec<NodeD>,
    sketches: Vec<String>,
    attrs: String,
}

struct Digester<'d> {
    doc: &'d Document,
}

impl Digester<'_> {
    fn mat_name(&self, m: Option<kernel::MaterialId>) -> Option<String> {
        let m = self.doc.materials().get(m?)?;
        Some(format!(
            "{}:{:?}:{}",
            m.name,
            [m.color.r, m.color.g, m.color.b, m.color.a],
            m.texture.is_some()
        ))
    }

    fn attrs(&self, e: EntityRef) -> String {
        format!(
            "{:?}",
            self.doc.attr_get(&AttrTarget::Entity(e)).ok().flatten()
        )
    }

    fn faces(&self, obj: &Object) -> Vec<FaceD> {
        let he = obj.half_edges();
        let v = obj.vertices();
        let ring = |l| -> Vec<P> {
            canon_ring(
                obj.loop_half_edges(l)
                    .map(|h| qp(v[he[h].origin].position))
                    .collect(),
            )
        };
        let mut faces: Vec<FaceD> = obj
            .faces()
            .iter()
            .map(|(_, f)| {
                let mut holes: Vec<Vec<P>> = f.inner_loops.iter().map(|&l| ring(l)).collect();
                holes.sort();
                let effective = f.material.or(obj.default_material());
                // The UV each outer corner renders with, in the outer
                // ring's canonical order: the face's frame, else Hew's
                // planar projection of the texture.
                let tex = effective
                    .and_then(|m| self.doc.materials().get(m))
                    .and_then(|m| m.texture.as_ref());
                let pts: Vec<Point3> = obj
                    .loop_half_edges(f.outer_loop)
                    .map(|h| v[he[h].origin].position)
                    .collect();
                let mut uv: Vec<[i64; 2]> = match tex {
                    Some(t) => {
                        let n = f.plane.normal();
                        let (a, b) = tessellate::plane_basis(n);
                        pts.iter()
                            .map(|&p| {
                                let w = match f.uv_frame {
                                    Some(fr) => fr.apply(p),
                                    None => [
                                        p.to_vec().dot(a) / t.world_size[0],
                                        p.to_vec().dot(b) / t.world_size[1],
                                    ],
                                };
                                [(w[0] * 1e4).round() as i64, (w[1] * 1e4).round() as i64]
                            })
                            .collect()
                    }
                    None => Vec::new(),
                };
                let raw: Vec<P> = pts.iter().map(|&p| qp(p)).collect();
                let start = raw
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, p)| **p)
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                if !uv.is_empty() {
                    uv.rotate_left(start);
                }
                FaceD {
                    outer: ring(f.outer_loop),
                    holes,
                    material: self.mat_name(effective),
                    uv,
                }
            })
            .collect();
        faces.sort();
        faces
    }

    fn name(&self, n: Option<&str>) -> Option<String> {
        n.filter(|s| !s.is_empty()).map(str::to_string)
    }

    fn tags(&self, node: NodeId) -> Vec<Vec<String>> {
        self.doc.node_tags(node).to_vec()
    }

    fn node(&self, node: NodeId) -> NodeD {
        let hidden = self.doc.node_user_hidden(node);
        let tags = self.tags(node);
        match node {
            NodeId::Object(id) => {
                let obj = self.doc.object(id).unwrap();
                NodeD::Object {
                    name: self.name(self.doc.object_name(id)),
                    tags,
                    hidden,
                    faces: self.faces(obj),
                    claims: effective_claims(obj),
                    attrs: self.attrs(EntityRef::Object(id)),
                }
            }
            NodeId::Group(id) => {
                let mut children: Vec<NodeD> = self
                    .doc
                    .group_members(id)
                    .unwrap()
                    .into_iter()
                    .map(|m| self.node(m))
                    .collect();
                children.sort();
                NodeD::Group {
                    name: self.name(self.doc.group_name(id)),
                    tags,
                    hidden,
                    children,
                    attrs: self.attrs(EntityRef::Group(id)),
                }
            }
            NodeId::Instance(id) => {
                let def = self.doc.instance_def(id).unwrap();
                let mut members: Vec<NodeD> = self
                    .doc
                    .def_member_nodes(def)
                    .unwrap()
                    .into_iter()
                    .map(|m| self.node(m))
                    .collect();
                members.sort();
                let mut sketches: Vec<String> = self
                    .doc
                    .def_member_sketches(def)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|s| sketch_digest(self.doc, s))
                    .collect();
                sketches.sort();
                NodeD::Instance {
                    def: Box::new(DefD {
                        name: self.name(self.doc.component_name(def)),
                        members,
                        sketches,
                        attrs: self.attrs(EntityRef::Component(def)),
                    }),
                    pose: self
                        .doc
                        .instance_pose(id)
                        .unwrap()
                        .to_affine()
                        .iter()
                        .map(|&x| q(x))
                        .collect(),
                    name: self.name(self.doc.instance_name(id)),
                    tags,
                    hidden,
                    attrs: self.attrs(EntityRef::Instance(id)),
                }
            }
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct DocD {
    pub roots: Vec<NodeD>,
    pub tags: BTreeMap<String, bool>,
    pub guides: Vec<String>,
    pub section: Option<String>,
    pub camera: Option<String>,
    /// Name, camera, hidden node digests, hidden tag paths, section,
    /// display toggles.
    pub scenes: Vec<String>,
    /// World sketches.
    pub sketches: Vec<String>,
    /// The document's own attributes, and each material's and tag's.
    pub attrs: Vec<String>,
    /// Dimensions and leader texts, positions to a micron.
    pub annotations: Vec<String>,
}

/// A sketch: its plane, its segments (unordered, each end-sorted), the
/// regions it closes, and each welded curve's members and claim.
fn sketch_digest(doc: &Document, id: kernel::SketchId) -> String {
    let sk = doc.sketch(id).unwrap();
    let pl = sk.plane();
    let n = pl.normal();
    let mut segs: Vec<[P; 2]> = sk
        .edges()
        .iter()
        .map(|(_, e)| {
            let mut s = [
                qp(sk.vertices()[e.from].position),
                qp(sk.vertices()[e.to].position),
            ];
            s.sort();
            s
        })
        .collect();
    segs.sort();
    let mut chains: BTreeMap<kernel::SketchCurveId, Vec<[P; 2]>> = BTreeMap::new();
    for (_, e) in sk.edges().iter() {
        if let Some(c) = e.curve {
            let mut s = [
                qp(sk.vertices()[e.from].position),
                qp(sk.vertices()[e.to].position),
            ];
            s.sort();
            chains.entry(c).or_default().push(s);
        }
    }
    let mut curves: Vec<String> = chains
        .into_iter()
        .map(|(c, mut m)| {
            m.sort();
            let claim = sk
                .curve_analytic(c)
                .map(|a| (a.kind, qp(a.geom.center), q(a.geom.radius)));
            format!("{m:?} {claim:?}")
        })
        .collect();
    curves.sort();
    format!(
        "plane {:?} d {} segs {segs:?} regions {} curves {curves:?} tags {:?}",
        [q(n.x), q(n.y), q(n.z)],
        q(pl.signed_distance(Point3::ORIGIN)),
        sk.regions().len(),
        doc.sketch_tags(id)
    )
}

/// An Object's soft edges, circle-claimed edges and curved faces, as the
/// `.skp` writer reads them: a seam between two facets of one surface is
/// soft, and the rim where a cylinder wall meets a face off it is a circle,
/// whether or not the Object stores the claim itself (Hew's own extrusions
/// keep the surface on the faces only).
fn effective_claims(obj: &Object) -> [usize; 3] {
    let he = obj.half_edges();
    let v = obj.vertices();
    let surface = |h: kernel::HalfEdgeId| obj.faces()[obj.loops()[he[h].loop_id].face].surface;
    let (mut soft, mut claimed) = (0, 0);
    for (_, e) in obj.edges().iter() {
        let near = surface(e.half_edge);
        let far = e.twin_half_edge.and_then(surface);
        let seam = matches!((near, far), (Some(x), Some(y)) if x.same_surface(&y));
        if e.soft || seam {
            soft += 1;
        }
        let rim = || {
            let w = match (near, far) {
                (Some(w), other) | (other, Some(w))
                    if other.is_none_or(|o| !o.same_surface(&w)) =>
                {
                    w
                }
                _ => return false,
            };
            let kernel::SurfaceRef::Cylinder {
                axis_point,
                axis,
                radius,
            } = w;
            let (pa, pb) = (
                v[he[e.half_edge].origin].position,
                v[he[he[e.half_edge].next].origin].position,
            );
            let h = |p: Point3| (p - axis_point).dot(axis);
            let r = |p: Point3| {
                let d = p - axis_point;
                (d - axis * d.dot(axis)).length()
            };
            let tol = 1e-6 * radius.max(1.0);
            (h(pa) - h(pb)).abs() <= tol
                && (r(pa) - radius).abs() <= tol
                && (r(pb) - radius).abs() <= tol
        };
        if e.curve.is_some() || (!seam && rim()) {
            claimed += 1;
        }
    }
    let curved = obj
        .faces()
        .iter()
        .filter(|(_, f)| f.surface.is_some())
        .count();
    [soft, claimed, curved]
}

pub fn digest(doc: &Document) -> DocD {
    let d = Digester { doc };
    let mut roots: Vec<NodeD> = doc
        .top_level_nodes()
        .into_iter()
        .map(|n| d.node(n))
        .collect();
    roots.sort();
    let tags = doc.tag_meta().map(|(p, h)| (p.join("|"), h)).collect();
    let mut sketches: Vec<String> = doc
        .sketch_ids()
        .into_iter()
        .map(|s| sketch_digest(doc, s))
        .collect();
    sketches.sort();
    let mut attrs: Vec<String> = vec![format!(
        "doc {:?}",
        doc.attr_get(&AttrTarget::Document).ok().flatten()
    )];
    for (id, m) in doc.materials().iter() {
        let a = d.attrs(EntityRef::Material(id));
        if a != "None" {
            attrs.push(format!("material {} {a}", m.name));
        }
    }
    for (p, _) in doc.tag_meta() {
        let a = d.attrs(EntityRef::Tag(p.to_vec()));
        if a != "None" {
            attrs.push(format!("tag {p:?} {a}"));
        }
    }
    attrs.sort();
    let qv = |v: kernel::Vec3| [q(v.x), q(v.y), q(v.z)];
    let mut annotations: Vec<String> = doc
        .annotation_ids()
        .into_iter()
        .filter_map(|id| doc.annotation(id).cloned())
        .map(|a| match a {
            kernel::Annotation::LinearDimension {
                a,
                b,
                offset,
                plane,
                text_override,
            } => format!(
                "linear {:?} {:?} {:?} {:?} {:?}",
                qp(a.point),
                qp(b.point),
                qv(offset),
                qv(plane.normal()),
                text_override
            ),
            kernel::Annotation::LeaderText {
                anchor,
                offset,
                text,
            } => {
                format!("text {:?} {:?} {text}", qp(anchor.point), qv(offset))
            }
            kernel::Annotation::RadialDimension {
                anchor,
                kind,
                curve,
                leader_dir,
                text_override,
            } => {
                format!(
                    "radial {:?} {kind:?} {:?} {} {:?} {:?} {:?}",
                    qp(anchor.point),
                    qp(curve.center),
                    q(curve.radius),
                    qv(curve.plane.normal()),
                    qv(leader_dir),
                    text_override
                )
            }
        })
        .collect();
    annotations.sort();
    let mut guides: Vec<String> = doc
        .guide_ids()
        .into_iter()
        .filter_map(|g| doc.guide(g).copied())
        .map(|g| match g {
            kernel::Guide::Line {
                origin, direction, ..
            } => format!(
                "line {:?} {:?}",
                qp(origin),
                [q(direction.x), q(direction.y), q(direction.z)]
            ),
            kernel::Guide::Point { position } => format!("point {:?}", qp(position)),
        })
        .collect();
    guides.sort();
    let section = doc.section_plane().map(|s| {
        format!(
            "{:?} {:?} {}",
            qp(s.origin),
            [q(s.normal.x), q(s.normal.y), q(s.normal.z)],
            s.active
        )
    });
    let cam = |c: &kernel::CameraState| {
        // The view as it shows: eye, direction, up square to it, and the
        // lens. Not the target's depth: a `.skp` camera's target lands on
        // the model at the next open (`Document::ingest`), and a parallel
        // view's height is what it spans, so that is compared instead.
        let view = (c.target - c.eye).normalized().unwrap();
        let up = (c.up - view * c.up.dot(view)).normalized().unwrap();
        let height = match c.projection {
            kernel::CameraProjection::Parallel => q(c.parallel_height),
            kernel::CameraProjection::Perspective => 0,
        };
        format!(
            "{:?} {:?} {:?} {:?} {} {}",
            c.projection,
            qp(c.eye),
            [q(view.x), q(view.y), q(view.z)],
            [q(up.x), q(up.y), q(up.z)],
            q(c.fov_deg),
            height
        )
    };
    // Stable id -> a label of what it names, so scenes compare by content.
    let sid_label: BTreeMap<u64, String> = doc
        .sids()
        .filter_map(|(e, sid)| {
            // Live entities only: the id table keeps undone ones.
            let node = match e {
                EntityRef::Object(id) if doc.object(*id).is_some() => NodeId::Object(*id),
                EntityRef::Group(id) if doc.group_members(*id).is_some() => NodeId::Group(*id),
                EntityRef::Instance(id) if doc.instance_def(*id).is_some() => NodeId::Instance(*id),
                EntityRef::Tag(p) => return Some((sid, format!("tag {}", p.join("/")))),
                _ => return None,
            };
            Some((sid, format!("{:?}", Digester { doc }.node(node))))
        })
        .collect();
    let mut scenes: Vec<String> = doc
        .scenes()
        .iter()
        .map(|s| {
            let set = |ids: &Option<BTreeSet<u64>>| {
                ids.as_ref().map(|ids| {
                    let mut v: Vec<&str> = ids
                        .iter()
                        .filter_map(|i| sid_label.get(i).map(String::as_str))
                        .collect();
                    v.sort();
                    v.join(" | ")
                })
            };
            format!(
                "{} / {} / cam {:?} / hidden {:?} / tags {:?} / section {:?} / display {:?} / style {:?}",
                s.name,
                s.description,
                s.camera.as_ref().map(cam),
                set(&s.hidden_nodes),
                set(&s.hidden_tags),
                s.section.map(|o| o.map(|p| (qp(p.origin), p.active))),
                s.display,
                s.view_style
            )
        })
        .collect();
    scenes.sort();
    DocD {
        roots,
        tags,
        guides,
        section,
        camera: doc.camera_state().as_ref().map(cam),
        scenes,
        sketches,
        attrs,
        annotations,
    }
}

/// Two digests that differ only by a micron in some coordinates: a vertex
/// that crosses a rounding boundary when a placement's pose is baked in
/// and taken out again. Every other token must match exactly.
pub fn near(a: &str, b: &str) -> bool {
    let split = |s: &str| -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = String::new();
        for c in s.chars() {
            if c.is_ascii_digit() || c == '-' {
                cur.push(c);
            } else {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                out.push(c.to_string());
            }
        }
        if !cur.is_empty() {
            out.push(cur);
        }
        out
    };
    let (ta, tb) = (split(a), split(b));
    ta.len() == tb.len() && ta.iter().zip(&tb).all(|(x, y)| {
        x == y
            || matches!((x.parse::<i64>(), y.parse::<i64>()), (Ok(p), Ok(q)) if (p - q).abs() <= 1)
    })
}
