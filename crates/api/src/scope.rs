//! `scope: "own" | "all"` on the part reads and the render commands
//! (docs/agents/PLUGINS.md "Reading a container's own Objects"). Pure over
//! a [`Document`].
//!
//! A container's own Objects are its direct Object members and the Objects
//! of its direct Groups, recursively through Groups only, never the members
//! of a nested instance. A container is a Group or an Instance (for an
//! Instance, its definition's direct members). On an Object both scopes are
//! the same.

use crate::commands::CmdError;
use kernel::Transform;
use kernel::{Document, DocumentError, InstanceId, NodeId, ObjectId, Occurrence, RenderRoot};
use serde_json::Value;

/// Which leaves a read or a render takes under a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    /// Every leaf placement under the container, nested instances included.
    #[default]
    All,
    /// The container's own Objects: through Groups, never through instances.
    Own,
}

impl Scope {
    /// Reads the `scope` member of a command's params: absent is
    /// [`Scope::All`]; an unknown string or a non-string is `-32602`.
    pub fn from_params(params: &Value) -> Result<Scope, CmdError> {
        match params.get("scope") {
            None | Some(Value::Null) => Ok(Scope::All),
            Some(Value::String(v)) => Scope::from_name(Some(v)),
            Some(_) => Err(CmdError::Params("scope is \"own\" or \"all\"".into())),
        }
    }

    /// Parses an already-extracted `scope` value (`None` is the default).
    pub fn from_name(name: Option<&str>) -> Result<Scope, CmdError> {
        match name {
            None | Some("all") => Ok(Scope::All),
            Some("own") => Ok(Scope::Own),
            Some(other) => Err(CmdError::Params(format!(
                "unknown scope \"{other}\"; expected own or all"
            ))),
        }
    }
}

/// How many instances a leaf's path may hold under `root` and still be the
/// root's own: the root's own path, plus the root itself when it is an
/// Instance (its definition's direct members sit one instance deeper). A
/// definition root's members sit at depth zero.
fn own_depth(root: &RenderRoot) -> usize {
    match root {
        RenderRoot::Occurrence(occ) => {
            occ.path.len() + usize::from(matches!(occ.node, NodeId::Instance(_)))
        }
        RenderRoot::Definition(_) => 0,
    }
}

/// Every leaf placement under `root` (hidden ones included), with its world
/// pose and path, narrowed to the root's own Objects under [`Scope::Own`].
///
/// # Errors
/// [`DocumentError::TooManyOccurrences`] — the walk is past the cap.
pub fn scoped_leaves(
    doc: &Document,
    root: &RenderRoot,
    scope: Scope,
) -> Result<Vec<(ObjectId, Transform, Vec<InstanceId>)>, DocumentError> {
    let leaves = doc.render_root_leaves(root, true)?;
    Ok(match scope {
        Scope::All => leaves,
        Scope::Own => {
            let depth = own_depth(root);
            leaves
                .into_iter()
                .filter(|(_, _, path)| path.len() == depth)
                .collect()
        }
    })
}

/// [`scoped_leaves`] for one placement, without the paths.
///
/// # Errors
/// As [`scoped_leaves`].
pub fn leaves_for(
    doc: &Document,
    occ: &Occurrence,
    scope: Scope,
) -> Result<Vec<(ObjectId, Transform)>, DocumentError> {
    Ok(
        scoped_leaves(doc, &RenderRoot::Occurrence(occ.clone()), scope)?
            .into_iter()
            .map(|(o, t, _)| (o, t))
            .collect(),
    )
}

/// The placement's own Objects at their world poses: from
/// `render_root_leaves`, a leaf whose path adds no instance below the root
/// (and, for an Instance root, none below the root's own id).
///
/// # Errors
/// As [`scoped_leaves`].
pub fn own_leaves(
    doc: &Document,
    occ: &Occurrence,
) -> Result<Vec<(ObjectId, Transform)>, DocumentError> {
    leaves_for(doc, occ, Scope::Own)
}

/// The render roots `scope` selects under `roots`: unchanged for
/// [`Scope::All`]; under [`Scope::Own`], each placement root replaced by
/// one Object occurrence per own leaf, so a host renders exactly those and
/// fits its camera to them. A root with no own leaf contributes nothing,
/// and so does a definition root that places a nested instance (its own
/// objects cannot be named apart from it).
///
/// # Errors
/// As [`scoped_leaves`].
pub fn scoped_render_roots(
    doc: &Document,
    roots: Vec<RenderRoot>,
    scope: Scope,
) -> Result<Vec<RenderRoot>, DocumentError> {
    if scope == Scope::All {
        return Ok(roots);
    }
    let mut out = Vec::new();
    for root in &roots {
        if let RenderRoot::Definition(_) = root {
            // A definition's members have no world placement to name one
            // by one; its own objects are all of them when it places no
            // instance, and the caller refuses otherwise.
            if scoped_leaves(doc, root, Scope::Own)?.len()
                == scoped_leaves(doc, root, Scope::All)?.len()
                && !out.contains(root)
            {
                out.push(root.clone());
            }
            continue;
        }
        for (o, _, path) in scoped_leaves(doc, root, Scope::Own)? {
            let node = NodeId::Object(o);
            if let Some(occ) = doc.occurrence_at(&path, node)
                && !out.contains(&RenderRoot::Occurrence(occ.clone()))
            {
                out.push(RenderRoot::Occurrence(occ));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::{Plane, Point3, Vec3};

    fn ground() -> Plane {
        Plane::from_polygon(&[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ])
        .unwrap()
    }

    fn cube(doc: &mut Document, x: f64) -> ObjectId {
        let s = doc.add_sketch(ground());
        let c = [
            Point3::new(x, 0.0, 0.0),
            Point3::new(x + 0.1, 0.0, 0.0),
            Point3::new(x + 0.1, 0.1, 0.0),
            Point3::new(x, 0.1, 0.0),
        ];
        let sk = doc.sketch_mut(s).unwrap();
        for k in 0..4 {
            sk.add_segment(c[k], c[(k + 1) % 4]).unwrap();
        }
        let r = doc.extrudable_regions(s).unwrap();
        doc.extrude_region(s, r[0], 0.1).unwrap().0
    }

    /// A group (inner group + loose object + a placement) inside a group:
    /// own takes both groups' objects, never the placement's.
    #[test]
    fn own_leaves_goes_through_groups_but_not_instances() {
        let mut doc = Document::new();
        let a = cube(&mut doc, 0.0);
        let b = cube(&mut doc, 1.0);
        let c = cube(&mut doc, 2.0);
        let (_, inst, _) = doc.make_component(&[NodeId::Object(c)]).unwrap();
        let (inner, _) = doc.group_nodes(&[NodeId::Object(b)]).unwrap();
        let (outer, _) = doc
            .group_nodes(&[
                NodeId::Object(a),
                NodeId::Group(inner),
                NodeId::Instance(inst),
            ])
            .unwrap();
        let occ = doc.occurrence_at(&[], NodeId::Group(outer)).unwrap();
        let own: Vec<ObjectId> = own_leaves(&doc, &occ)
            .unwrap()
            .into_iter()
            .map(|(o, _)| o)
            .collect();
        assert_eq!(own, vec![a, b]);
        let all = leaves_for(&doc, &occ, Scope::All).unwrap();
        assert_eq!(all.len(), 3);
    }

    /// An instance root's own leaves are its definition's direct objects
    /// at the placement's pose; a nested instance's objects are not.
    #[test]
    fn own_leaves_of_an_instance_are_its_definitions_direct_objects() {
        let mut doc = Document::new();
        let plate = cube(&mut doc, 0.0);
        let pin = cube(&mut doc, 1.0);
        let (_, inner, _) = doc.make_component(&[NodeId::Object(pin)]).unwrap();
        let (_, outer, _) = doc
            .make_component(&[NodeId::Object(plate), NodeId::Instance(inner)])
            .unwrap();
        doc.transform_instance(outer, &Transform::translation(Vec3::new(0.0, 0.0, 5.0)))
            .unwrap();
        let occ = doc.occurrence_at(&[], NodeId::Instance(outer)).unwrap();
        let own = own_leaves(&doc, &occ).unwrap();
        assert_eq!(own.len(), 1);
        assert_eq!(own[0].0, plate);
        let p = own[0].1.apply_point(Point3::new(0.0, 0.0, 0.0));
        assert!((p.z - 5.0).abs() < 1e-12);
        assert_eq!(leaves_for(&doc, &occ, Scope::All).unwrap().len(), 2);
    }

    /// On an Object both scopes are the same.
    #[test]
    fn own_leaves_of_an_object_is_the_object() {
        let mut doc = Document::new();
        let a = cube(&mut doc, 0.0);
        let occ = doc.occurrence_at(&[], NodeId::Object(a)).unwrap();
        assert_eq!(
            own_leaves(&doc, &occ).unwrap(),
            leaves_for(&doc, &occ, Scope::All).unwrap()
        );
    }

    #[test]
    fn own_leaves_scope_names_parse() {
        assert_eq!(
            Scope::from_params(&serde_json::json!({})).unwrap(),
            Scope::All
        );
        assert_eq!(
            Scope::from_params(&serde_json::json!({ "scope": "own" })).unwrap(),
            Scope::Own
        );
        assert!(Scope::from_params(&serde_json::json!({ "scope": "mine" })).is_err());
        assert!(Scope::from_params(&serde_json::json!({ "scope": 3 })).is_err());
    }
}
