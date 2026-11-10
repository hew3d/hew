//! What a `.skp` says beyond geometry and structure: its attribute
//! dictionaries (SketchUp's extensions and dynamic components keep their
//! data there), and — in a file Hew wrote — the `hew` dictionary that
//! carries what the format has no field for (`skp-export`'s `dicts`
//! module documents its keys).

use std::collections::BTreeMap;

use kernel::{AttrDict, AttrValue, DisplayState, Point3, SurfaceRef, Vec3};
use skpwright::{Dictionary, Owner, Value};

/// The name of Hew's own dictionary; never imported as an attribute.
pub(crate) const HEW: &str = "hew";

/// Every dictionary of the model, by owner.
pub(crate) struct Dicts {
    by_owner: BTreeMap<OwnerKey, Vec<Dictionary>>,
}

/// `Owner` without a derived `Ord`, as a sortable key.
type OwnerKey = (u8, usize, usize);

fn key(o: Owner) -> OwnerKey {
    match o {
        Owner::Model => (0, 0, 0),
        Owner::Definition(i) => (1, i, 0),
        Owner::Material(i) => (2, i, 0),
        Owner::Layer(i) => (3, i, 0),
        Owner::Face(r, i) => (4, r, i),
        Owner::Edge(r, i) => (5, r, i),
        Owner::Vertex(r, i) => (17, r, i),
        Owner::Curve(r, i) => (6, r, i),
        Owner::Instance(r, i) => (7, r, i),
        Owner::Image(r, i) => (8, r, i),
        Owner::SectionPlane(r, i) => (9, r, i),
        Owner::GuidePoint(r, i) => (10, r, i),
        Owner::Polyline(r, i) => (11, r, i),
        Owner::Guide(i) => (12, i, 0),
        Owner::Text(i) => (13, i, 0),
        Owner::Dimension(i) => (14, i, 0),
        Owner::RadialDimension(i) => (15, i, 0),
        Owner::RenderingOptions => (16, 0, 0),
        Owner::CurrentStyle => (18, 0, 0),
        Owner::Axes => (19, 0, 0),
        Owner::ShadowInfo => (20, 0, 0),
        Owner::LineStyle(i) => (21, i, 0),
        Owner::Scene(i) => (22, i, 0),
        Owner::SceneRendering(i) => (23, i, 0),
        Owner::LayerFolder(i) => (24, i, 0),
        Owner::HatchPattern(i) => (25, i, 0),
        Owner::Style(i) => (26, i, 0),
        Owner::SceneShadowInfo(i) => (27, i, 0),
        Owner::SceneAxes(i) => (28, i, 0),
        Owner::Materials => (29, 0, 0),
        Owner::Layers => (30, 0, 0),
        Owner::Definitions => (31, 0, 0),
        Owner::Scenes => (32, 0, 0),
        Owner::Styles => (33, 0, 0),
        Owner::LineStyles => (34, 0, 0),
        Owner::HatchPatterns => (35, 0, 0),
        Owner::Environments => (36, 0, 0),
    }
}

impl Dicts {
    pub(crate) fn of(model: &skpwright::Model) -> Dicts {
        let mut by_owner: BTreeMap<OwnerKey, Vec<Dictionary>> = BTreeMap::new();
        for a in &model.dictionaries {
            by_owner
                .entry(key(a.owner))
                .or_default()
                .extend(a.dictionaries.iter().cloned());
        }
        Dicts { by_owner }
    }

    /// Hew's dictionary of `owner`, when it has one.
    pub(crate) fn hew(&self, owner: Owner) -> Option<&Dictionary> {
        self.by_owner
            .get(&key(owner))?
            .iter()
            .find(|d| d.name == HEW)
    }

    /// The `hew` entry `name` of `owner` (from any `hew` dictionary it
    /// has: an older writer gave a definition two).
    pub(crate) fn hew_entry(&self, owner: Owner, name: &str) -> Option<&Value> {
        self.by_owner
            .get(&key(owner))?
            .iter()
            .filter(|d| d.name == HEW)
            .find_map(|d| entry(d, name))
    }

    /// `owner`'s dictionaries other than Hew's own, as Hew attributes.
    pub(crate) fn attrs(&self, owner: Owner) -> AttrDict {
        self.by_owner
            .get(&key(owner))
            .into_iter()
            .flatten()
            .filter(|d| d.name != HEW && !d.name.is_empty())
            .map(|d| (d.name.clone(), entries_of(d)))
            .filter(|(_, e)| !e.is_empty())
            .collect()
    }

    /// Every owner that has dictionaries other than Hew's own.
    pub(crate) fn owners(&self) -> impl Iterator<Item = OwnerKey> + '_ {
        self.by_owner
            .iter()
            .filter(|(_, ds)| ds.iter().any(|d| d.name != HEW))
            .map(|(k, _)| *k)
    }
}

/// An owner key back as the owner kinds the importer maps.
pub(crate) enum Mapped {
    Model,
    Definition(usize),
    Material(usize),
    Layer(usize),
    Instance(usize, usize),
    /// Guide lines are expanded per placement on import; their
    /// dictionaries have no single Hew guide to land on.
    Guide,
    Other,
}

pub(crate) fn mapped(k: OwnerKey) -> Mapped {
    match k {
        (0, ..) => Mapped::Model,
        (1, i, _) => Mapped::Definition(i),
        (2, i, _) => Mapped::Material(i),
        (3, i, _) => Mapped::Layer(i),
        (7, r, i) => Mapped::Instance(r, i),
        (12, ..) => Mapped::Guide,
        _ => Mapped::Other,
    }
}

/// A model dictionary SketchUp keeps for itself: its geo-location, its
/// model properties, and the `GSU_`/`SU_` families.
pub(crate) fn sketchup_owned(name: &str) -> bool {
    matches!(name, "ModelProperties" | "GeoReference")
        || name.starts_with("GSU_")
        || name.starts_with("SU_")
}

pub(crate) fn entry<'d>(d: &'d Dictionary, name: &str) -> Option<&'d Value> {
    d.entries.iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

/// A dictionary's entries as Hew values. The keys `hew_json` lists hold
/// JSON text Hew wrote for a value SketchUp has no type for; they come back
/// as the value. A value Hew cannot hold (an undecoded 8-byte value) is
/// left out.
fn entries_of(d: &Dictionary) -> BTreeMap<String, AttrValue> {
    let json_keys: Vec<&str> = match entry(d, "hew_json") {
        Some(Value::Array(keys)) => keys
            .iter()
            .filter_map(|k| match k {
                Value::Text(s) => Some(s.as_str()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    d.entries
        .iter()
        .filter(|(k, _)| k != "hew_json" || json_keys.is_empty())
        .filter_map(|(k, v)| {
            let value = match v {
                Value::Text(s) if json_keys.contains(&k.as_str()) => json_value(s)?,
                v => attr_value(v)?,
            };
            Some((k.clone(), value))
        })
        .collect()
}

fn attr_value(v: &Value) -> Option<AttrValue> {
    Some(match v {
        Value::Nil => AttrValue::Null,
        Value::Int(i) => AttrValue::Int(i64::from(*i)),
        Value::Float(f) => AttrValue::Float(f64::from(*f)).finite()?,
        Value::Double(f) => AttrValue::Float(*f).finite()?,
        Value::Bool(b) => AttrValue::Bool(*b),
        Value::Time(t) => AttrValue::Int(i64::from(*t)),
        Value::Text(s) => AttrValue::Text(s.clone()),
        Value::Array(items) => {
            AttrValue::List(items.iter().map(attr_value).collect::<Option<_>>()?)
        }
        Value::Vector(v) | Value::Direction(v) => AttrValue::List(
            v.iter()
                .map(|x| AttrValue::Float(*x).finite())
                .collect::<Option<_>>()?,
        ),
        Value::Bytes8(_) => return None,
    })
}

trait Finite: Sized {
    fn finite(self) -> Option<Self>;
}

impl Finite for AttrValue {
    fn finite(self) -> Option<AttrValue> {
        match &self {
            AttrValue::Float(f) if !f.is_finite() => None,
            _ => Some(self),
        }
    }
}

fn json_value(s: &str) -> Option<AttrValue> {
    fn conv(v: serde_json::Value) -> Option<AttrValue> {
        Some(match v {
            serde_json::Value::Null => AttrValue::Null,
            serde_json::Value::Bool(b) => AttrValue::Bool(b),
            serde_json::Value::Number(n) => match n.as_i64() {
                Some(i) => AttrValue::Int(i),
                None => AttrValue::Float(n.as_f64()?),
            },
            serde_json::Value::String(s) => AttrValue::Text(s),
            serde_json::Value::Array(a) => {
                AttrValue::List(a.into_iter().map(conv).collect::<Option<_>>()?)
            }
            serde_json::Value::Object(m) => AttrValue::Map(
                m.into_iter()
                    .map(|(k, v)| Some((k, conv(v)?)))
                    .collect::<Option<_>>()?,
            ),
        })
    }
    conv(serde_json::from_str(s).ok()?)
}

/// What the model's `hew` dictionary says, when Hew wrote the file.
#[derive(Default)]
pub(crate) struct HewModel {
    /// The file was written by Hew (a `hew` dictionary of a version this
    /// reader knows).
    pub written_by_hew: bool,
    /// Layer name -> the tag path it was written from.
    pub tag_paths: BTreeMap<String, Vec<String>>,
    /// The file's layer folders are the tags' hierarchy: a layer's folders
    /// stand over its recorded path, which a SketchUp edit leaves stale.
    pub folders: bool,
    /// The model's loose geometry is drawn sketches.
    pub loose_sketches: bool,
    /// The model's loose geometry is as Hew built it (no repair needed).
    pub loose_built: bool,
    /// The display settings were written from a Hew view style; absent
    /// in a Hew-written file, the document had none (Hew's own look).
    pub view_style: bool,
    /// Scene name -> display toggles.
    pub scene_display: BTreeMap<String, DisplayState>,
}

impl HewModel {
    pub(crate) fn of(dicts: &Dicts) -> HewModel {
        let Some(d) = dicts.hew(Owner::Model) else {
            return HewModel::default();
        };
        if entry(d, "version") != Some(&Value::Int(1)) {
            return HewModel::default();
        }
        let mut out = HewModel {
            written_by_hew: true,
            ..HewModel::default()
        };
        if let Some(Value::Array(items)) = entry(d, "tag_paths") {
            for item in items {
                if let Value::Array(pair) = item
                    && let [Value::Text(name), path] = pair.as_slice()
                    && let Some(path) = path_of(path)
                {
                    out.tag_paths.insert(name.clone(), path);
                }
            }
        }
        out.folders = entry(d, "folders") == Some(&Value::Bool(true));
        out.loose_sketches = entry(d, "loose") == Some(&Value::Text("sketches".into()));
        out.loose_built = entry(d, "loose_built") == Some(&Value::Bool(true));
        out.view_style = entry(d, "view_style") == Some(&Value::Bool(true));
        if let Some(Value::Array(items)) = entry(d, "scene_display") {
            for item in items {
                if let Value::Array(v) = item
                    && let [
                        Value::Text(name),
                        Value::Bool(grid),
                        Value::Bool(axes),
                        Value::Bool(guides),
                    ] = v.as_slice()
                {
                    out.scene_display.insert(
                        name.clone(),
                        DisplayState {
                            grid: *grid,
                            axes: *axes,
                            guides: *guides,
                        },
                    );
                }
            }
        }
        out
    }
}

/// A `hew` tag path value: an array of non-empty text segments.
pub(crate) fn path_of(v: &Value) -> Option<Vec<String>> {
    let Value::Array(segs) = v else { return None };
    let path: Vec<String> = segs
        .iter()
        .map(|s| match s {
            Value::Text(t) if !t.is_empty() => Some(t.clone()),
            _ => None,
        })
        .collect::<Option<_>>()?;
    (!path.is_empty()).then_some(path)
}

/// A `hew` face `surface` value: `[axis point xyz, axis xyz, radius]`.
pub(crate) fn surface_of(v: &Value) -> Option<SurfaceRef> {
    let Value::Array(items) = v else { return None };
    let n: Vec<f64> = items
        .iter()
        .map(|x| match x {
            Value::Double(d) => Some(*d),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let [px, py, pz, dx, dy, dz, r] = n.as_slice() else {
        return None;
    };
    let all = [*px, *py, *pz, *dx, *dy, *dz, *r];
    if !all.iter().all(|x| x.is_finite()) {
        return None;
    }
    Some(SurfaceRef::Cylinder {
        axis_point: Point3::new(*px, *py, *pz),
        axis: Vec3::new(*dx, *dy, *dz),
        radius: *r,
    })
}

/// A Hew annotation written by `skp-export` (its `encode_annotation`), back
/// from its `hew` value; `None` for anything malformed.
pub(crate) fn decode_annotation(v: &Value) -> Option<kernel::Annotation> {
    use kernel::{Anchor, Annotation, CapturedCurve, Plane, RadialKind};
    let Value::Array(items) = v else { return None };
    let Some(Value::Text(kind)) = items.first() else {
        return None;
    };
    // A cursor over the fields after the kind.
    let mut at = 1usize;
    let mut num = || -> Option<f64> {
        let v = match items.get(at)? {
            Value::Double(d) if d.is_finite() => *d,
            _ => return None,
        };
        at += 1;
        Some(v)
    };
    let mut point = || -> Option<Point3> { Some(Point3::new(num()?, num()?, num()?)) };
    let free = |p: Point3| Anchor {
        node: None,
        point: p,
    };
    let text_of = |v: Option<&Value>| match v {
        Some(Value::Text(t)) => Some(Some(t.clone())),
        Some(Value::Nil) => Some(None),
        _ => None,
    };
    match kind.as_str() {
        "linear" => {
            let (a, b, o, po, pn) = (point()?, point()?, point()?, point()?, point()?);
            Some(Annotation::LinearDimension {
                a: free(a),
                b: free(b),
                offset: o.to_vec(),
                plane: Plane::from_point_normal(po, pn.to_vec()).ok()?,
                text_override: text_of(items.get(16))?,
            })
        }
        "text" => {
            let (a, o) = (point()?, point()?);
            let Some(Value::Text(t)) = items.get(7) else {
                return None;
            };
            Some(Annotation::LeaderText {
                anchor: free(a),
                offset: o.to_vec(),
                text: t.clone(),
            })
        }
        "radial" => {
            let (a, d, c) = (point()?, point()?, point()?);
            let r = match items.get(10)? {
                Value::Double(x) if x.is_finite() => *x,
                _ => return None,
            };
            let rest = &items[11..];
            let get = |i: usize| match rest.get(i) {
                Some(Value::Double(x)) if x.is_finite() => Some(*x),
                _ => None,
            };
            let po = Point3::new(get(0)?, get(1)?, get(2)?);
            let pn = Point3::new(get(3)?, get(4)?, get(5)?);
            let kind = match rest.get(6) {
                Some(Value::Text(k)) if k == "radius" => RadialKind::Radius,
                Some(Value::Text(k)) if k == "diameter" => RadialKind::Diameter,
                _ => return None,
            };
            Some(Annotation::RadialDimension {
                anchor: free(a),
                kind,
                curve: CapturedCurve {
                    center: c,
                    radius: r,
                    plane: Plane::from_point_normal(po, pn.to_vec()).ok()?,
                },
                leader_dir: d.to_vec(),
                text_override: text_of(rest.get(7))?,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_keys_come_back_as_their_values() {
        let d = Dictionary {
            name: "shop".into(),
            entries: vec![
                (
                    "meta".into(),
                    Value::Text(r#"{"count":3,"grain":"x"}"#.into()),
                ),
                ("n".into(), Value::Int(4)),
                ("v".into(), Value::Vector([1.0, 2.0, 3.0])),
                ("sun".into(), Value::Direction([0.0, 0.0, 1.0])),
                ("when".into(), Value::Time(1_024_666_200)),
                (
                    "hew_json".into(),
                    Value::Array(vec![Value::Text("meta".into())]),
                ),
            ],
            flag: 0,
        };
        let e = entries_of(&d);
        assert!(!e.contains_key("hew_json"));
        assert_eq!(e["n"], AttrValue::Int(4));
        let AttrValue::Map(m) = &e["meta"] else {
            panic!("{:?}", e["meta"])
        };
        assert_eq!(m["count"], AttrValue::Int(3));
        assert_eq!(
            e["v"],
            AttrValue::List(vec![
                AttrValue::Float(1.0),
                AttrValue::Float(2.0),
                AttrValue::Float(3.0)
            ])
        );
        assert_eq!(
            e["sun"],
            AttrValue::List(vec![
                AttrValue::Float(0.0),
                AttrValue::Float(0.0),
                AttrValue::Float(1.0)
            ])
        );
        assert_eq!(e["when"], AttrValue::Int(1_024_666_200));
    }
}
