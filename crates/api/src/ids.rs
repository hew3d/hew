//! The api-owned public-id indirection (docs/HEW_API.md §5.1). Public
//! ids are opaque strings a client may cache across saves; underneath,
//! each is a kind prefix plus the entity's kernel stable id (manifest
//! v14) in hex. Clients must not parse them — the format is an
//! implementation detail free to change.
//!
//! Sketch sub-entities (regions, curves) get COMPOUND ids embedding
//! their owning sketch's stable id plus the kernel's session-stable
//! sub-entity key bits: fully self-naming, stable within an open
//! document session, re-resolved by clients after open or attach (§5.1).

use kernel::{Document, EntityRef};
use slotmap::Key as _;
use std::collections::BTreeMap;

/// The public id for an entity, from its kind and stable id.
pub fn public_id(entity: &EntityRef, sid: u64) -> String {
    let prefix = match entity {
        EntityRef::Object(_) => "obj",
        EntityRef::Sketch(_) => "skt",
        EntityRef::Group(_) => "grp",
        EntityRef::Component(_) => "cmp",
        EntityRef::Instance(_) => "ins",
        EntityRef::Guide(_) => "gde",
        EntityRef::Material(_) => "mat",
        EntityRef::Tag(_) => "tag",
    };
    format!("{prefix}_{sid:x}")
}

/// A compound public id for a sketch region: owning sketch + the
/// region's session-stable key bits.
pub fn region_id(sketch_sid: u64, region: kernel::SketchRegionId) -> String {
    format!("rgn_{sketch_sid:x}_{:x}", region.data().as_ffi())
}

/// A compound public id for a sketch curve chain.
pub fn curve_id(sketch_sid: u64, curve: kernel::SketchCurveId) -> String {
    format!("crv_{sketch_sid:x}_{:x}", curve.data().as_ffi())
}

/// The public id for a Scene (docs/HEW_API.md §5.1). Scenes are NOT
/// entities — they carry no [`EntityRef`], do not appear in
/// [`Document::sids`], and are addressed by their own list
/// (`kernel::Document::scenes`) — so this is its own top-level id shape
/// rather than a [`public_id`] prefix, minted straight from the Scene's
/// stable id.
pub fn scene_id(sid: u64) -> String {
    format!("scene_{sid:x}")
}

/// Parses a Scene public id back to its stable id — `None` for anything
/// that isn't well-formed `scene_<hex>`. Does not check the id is a LIVE
/// Scene in any particular document; callers do that with
/// `Document::scene`, which is what turns a since-deleted Scene's id into
/// the same typed `unknown_scene` refusal a malformed one gets.
pub fn resolve_scene_id(public: &str) -> Option<u64> {
    let hex = public.strip_prefix("scene_")?;
    u64::from_str_radix(hex, 16).ok()
}

/// A compound public id for a sketch edge — minted exactly like
/// [`region_id`]/[`curve_id`] (docs/HEW_API.md §5.2): the owning sketch's
/// stable id plus the edge's session-stable slotmap key bits. Unlike a
/// SOLID edge (never addressable — §5.2), a sketch edge is durable,
/// user-visible scaffolding a client draws, queries, and later trims, so
/// it gets the same compound-id treatment as a region or curve chain.
pub fn edge_id(sketch_sid: u64, edge: kernel::SketchEdgeId) -> String {
    format!("edg_{sketch_sid:x}_{:x}", edge.data().as_ffi())
}

/// Resolves public id strings against one document state. Built per
/// envelope — construction walks the sid table once, so resolution
/// inside the envelope is a map lookup.
#[derive(Debug)]
pub struct IdResolver {
    by_sid: BTreeMap<u64, EntityRef>,
}

impl IdResolver {
    pub fn new(doc: &Document) -> IdResolver {
        IdResolver {
            by_sid: doc.sids().map(|(e, s)| (s, e.clone())).collect(),
        }
    }

    /// The public id of a document entity, if it carries a stable id.
    pub fn public_of(&self, doc: &Document, entity: &EntityRef) -> Option<String> {
        doc.sid_of(entity).map(|sid| public_id(entity, sid))
    }

    /// The entity a public id names, if its prefix and sid resolve to an
    /// entity of that kind in this document.
    pub fn resolve(&self, public: &str) -> Option<EntityRef> {
        let (prefix, sid_hex) = public.split_once('_')?;
        let sid = u64::from_str_radix(sid_hex, 16).ok()?;
        let entity = self.by_sid.get(&sid)?;
        let matches = matches!(
            (prefix, entity),
            ("obj", EntityRef::Object(_))
                | ("skt", EntityRef::Sketch(_))
                | ("grp", EntityRef::Group(_))
                | ("cmp", EntityRef::Component(_))
                | ("ins", EntityRef::Instance(_))
                | ("gde", EntityRef::Guide(_))
                | ("mat", EntityRef::Material(_))
                | ("tag", EntityRef::Tag(_))
        );
        matches.then(|| entity.clone())
    }

    /// A sketch region's compound id → (owning sketch, region key), if
    /// the sketch part resolves. The region key itself is validated by
    /// the kernel lookup at use (a stale key is a lookup miss, never an
    /// aliased region — generational keys).
    pub fn resolve_region(
        &self,
        public: &str,
    ) -> Option<(kernel::SketchId, kernel::SketchRegionId)> {
        let (sketch, bits) = self.resolve_compound(public, "rgn_")?;
        Some((
            sketch,
            kernel::SketchRegionId::from(slotmap::KeyData::from_ffi(bits)),
        ))
    }

    /// A sketch curve's compound id → (owning sketch, curve key).
    pub fn resolve_curve(&self, public: &str) -> Option<(kernel::SketchId, kernel::SketchCurveId)> {
        let (sketch, bits) = self.resolve_compound(public, "crv_")?;
        Some((
            sketch,
            kernel::SketchCurveId::from(slotmap::KeyData::from_ffi(bits)),
        ))
    }

    /// A sketch edge's compound id → (owning sketch, edge key). Same
    /// generational-key caveat as [`IdResolver::resolve_region`]: a
    /// syntactically valid id whose edge key is stale (already erased) is
    /// not caught here — the kernel lookup at use is what turns that into
    /// a typed `unknown_edge` refusal, never a silently aliased edge.
    pub fn resolve_edge(&self, public: &str) -> Option<(kernel::SketchId, kernel::SketchEdgeId)> {
        let (sketch, bits) = self.resolve_compound(public, "edg_")?;
        Some((
            sketch,
            kernel::SketchEdgeId::from(slotmap::KeyData::from_ffi(bits)),
        ))
    }

    fn resolve_compound(&self, public: &str, prefix: &str) -> Option<(kernel::SketchId, u64)> {
        let rest = public.strip_prefix(prefix)?;
        let (sketch_hex, key_hex) = rest.split_once('_')?;
        let sketch_sid = u64::from_str_radix(sketch_hex, 16).ok()?;
        let bits = u64::from_str_radix(key_hex, 16).ok()?;
        let EntityRef::Sketch(sketch) = self.by_sid.get(&sketch_sid)? else {
            return None;
        };
        Some((*sketch, bits))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_ids_round_trip_through_the_resolver() {
        let mut doc = Document::new();
        let plane = kernel::Plane::from_polygon(&[
            kernel::Point3::new(0.0, 0.0, 0.0),
            kernel::Point3::new(1.0, 0.0, 0.0),
            kernel::Point3::new(0.0, 1.0, 0.0),
        ])
        .unwrap();
        let sketch = doc.add_sketch(plane);
        let entity = EntityRef::Sketch(sketch);
        let sid = doc.sid_of(&entity).expect("minted");

        let resolver = IdResolver::new(&doc);
        let public = public_id(&entity, sid);
        assert!(public.starts_with("skt_"));
        assert_eq!(resolver.resolve(&public), Some(entity.clone()));
        assert_eq!(resolver.public_of(&doc, &entity), Some(public.clone()));

        // Wrong prefix for the right sid refuses to resolve.
        let forged = public.replace("skt_", "obj_");
        assert_eq!(resolver.resolve(&forged), None);
        assert_eq!(resolver.resolve("obj_zzz"), None);
    }

    #[test]
    fn scene_ids_round_trip_and_reject_malformed_input() {
        assert_eq!(scene_id(0x2a), "scene_2a");
        assert_eq!(resolve_scene_id("scene_2a"), Some(0x2a));
        assert_eq!(resolve_scene_id("scene_"), None);
        assert_eq!(resolve_scene_id("scene_zz"), None);
        assert_eq!(resolve_scene_id("obj_2a"), None);
        assert_eq!(resolve_scene_id("2a"), None);
    }

    #[test]
    fn edge_ids_round_trip_and_refuse_a_forged_or_stale_key() {
        let mut doc = Document::new();
        let plane = kernel::Plane::from_polygon(&[
            kernel::Point3::new(0.0, 0.0, 0.0),
            kernel::Point3::new(1.0, 0.0, 0.0),
            kernel::Point3::new(0.0, 1.0, 0.0),
        ])
        .unwrap();
        let sketch = doc.add_sketch(plane);
        let sketch_sid = doc.sid_of(&EntityRef::Sketch(sketch)).expect("minted");
        let edge = doc
            .sketch_mut(sketch)
            .expect("sketch is live")
            .add_segment(
                kernel::Point3::new(0.0, 0.0, 0.0),
                kernel::Point3::new(1.0, 0.0, 0.0),
            )
            .expect("segment")
            .new_edges[0];

        let resolver = IdResolver::new(&doc);
        let public = edge_id(sketch_sid, edge);
        assert!(public.starts_with("edg_"));
        assert_eq!(resolver.resolve_edge(&public), Some((sketch, edge)));

        // Wrong prefix on an otherwise well-formed compound id refuses:
        // `resolve_edge` only strips an "edg_" prefix, so a region- or
        // curve-shaped string (even one carrying these exact sid/key
        // bits) never resolves as an edge.
        let forged = public.replace("edg_", "rgn_");
        assert_eq!(resolver.resolve_edge(&forged), None);

        // A sketch part that does not resolve to a live sketch refuses,
        // regardless of what the key bits say.
        assert_eq!(resolver.resolve_edge("edg_ffffff_1"), None);

        // A syntactically valid id naming a since-erased edge key is not
        // caught by the resolver (generational keys — see
        // `IdResolver::resolve_edge`'s doc comment): it resolves to a
        // key the sketch's own slotmap no longer holds, which is exactly
        // what the kernel lookup at use turns into a typed refusal.
        doc.sketch_mut(sketch)
            .expect("sketch is live")
            .remove_edge(edge)
            .expect("erase the only edge");
        let (resolved_sketch, resolved_edge) =
            resolver.resolve_edge(&public).expect("id still parses");
        assert_eq!(resolved_sketch, sketch);
        assert_eq!(resolved_edge, edge);
        assert!(
            doc.sketch(sketch)
                .expect("sketch itself survives an edge erase")
                .edges()
                .get(resolved_edge)
                .is_none(),
            "the resolved key is stale in the live sketch"
        );
    }
}
