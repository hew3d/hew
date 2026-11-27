//! Refusals: the typed "no" the API surfaces verbatim (docs/agents/HEW_API.md
//! §2, §4.4). A refusal carries a stable machine name drawn from the
//! kernel's typed error inventory, refusal-specific `detail`, and a
//! plain-language `explanation`.
//!
//! Machine names are the kernel error variant names in snake_case —
//! derived mechanically (the same variant-name convention the wasm
//! boundary uses, recased), so a new kernel variant gets a stable name
//! with no hand-maintained table. Explanations prefer the UI's own copy
//! (`app/src/kernelErrors.ts` is the authoritative table) and fall back
//! to the kernel's `Display` text for codes it has none for.
//!
//! The UI copy itself is not hand-mirrored here: `refusal_copy_gen`
//! (`refusal_copy.gen.rs`) is generated from `kernelErrors.ts`'s
//! `DESCRIPTIONS` table by `app/src/kernelErrorsDump.test.ts`, covering
//! every described code rather than a hand-picked subset. Regenerate with
//! `REGENERATE_REFUSAL_COPY=1 pnpm --dir app exec vitest run
//! src/kernelErrorsDump.test.ts` after changing `kernelErrors.ts`; a plain
//! `pnpm --dir app test` fails on drift.

use kernel::DocumentError;

#[path = "refusal_copy.gen.rs"]
mod refusal_copy_gen;

/// One refusal, ready to become the canonical §4.4 `error.data` payload.
#[derive(Debug, Clone)]
pub struct Refusal {
    /// Stable machine name (`push_pull_obstructed` style).
    pub name: String,
    /// Refusal-specific structured data.
    pub detail: serde_json::Value,
    /// Plain-language explanation — the UI's copy where mirrored,
    /// otherwise the kernel's own terse sentence.
    pub explanation: String,
    /// The locked node a `locked_node` refusal names, still in kernel
    /// terms: [`Refusal::with_document`] turns it into the public id in
    /// `detail.node`, once the document it lives in is at hand. Boxed: a
    /// refusal travels inside every transaction error, so it stays small.
    pub(crate) locked_cause: Option<Box<kernel::NodeId>>,
}

impl Refusal {
    /// Builds the refusal for a kernel [`DocumentError`].
    pub fn from_document_error(e: &DocumentError) -> Refusal {
        let code = variant_code(e);
        // The indirect lock has copy of its own (docs/agents/HEW_API.md
        // §7, Lock): the shape is also used inside something locked.
        let copy_code = match e {
            DocumentError::LockedNode {
                via_shared_shape: true,
                ..
            } => LOCKED_VIA_SHARED_SHAPE,
            _ => code.as_str(),
        };
        let explanation = ui_copy(copy_code)
            .map(str::to_string)
            .unwrap_or_else(|| e.to_string());
        let locked_cause = match e {
            DocumentError::LockedNode { node, .. } => Some(Box::new(*node)),
            _ => None,
        };
        Refusal {
            name: snake_case(&code),
            detail: refusal_detail(e),
            explanation,
            locked_cause,
        }
    }

    /// Resolves what a refusal names in kernel terms against `doc`, the
    /// document it was refused in: a `locked_node` refusal's cause becomes
    /// its public id in `detail.node`. Run where a command's refusal meets
    /// the dispatcher (the document is untouched by a refusal, and a
    /// transaction's abort keeps every stable id), so every path a client
    /// sees carries it.
    pub fn with_document(mut self, doc: &kernel::Document) -> Refusal {
        if let Some(node) = self.locked_cause.take() {
            let entity = match *node {
                kernel::NodeId::Object(o) => kernel::EntityRef::Object(o),
                kernel::NodeId::Group(g) => kernel::EntityRef::Group(g),
                kernel::NodeId::Instance(i) => kernel::EntityRef::Instance(i),
            };
            if let (Some(sid), Some(map)) = (doc.sid_of(&entity), self.detail.as_object_mut()) {
                map.insert(
                    "node".to_string(),
                    serde_json::Value::String(crate::ids::public_id(&entity, sid)),
                );
            }
        }
        self
    }

    /// A refusal minted by the API layer itself (not a kernel error).
    pub fn api(name: &str, explanation: &str) -> Refusal {
        Refusal {
            name: name.to_string(),
            detail: serde_json::json!({}),
            explanation: explanation.to_string(),
            locked_cause: None,
        }
    }

    /// Attaches structured detail.
    pub fn with_detail(mut self, detail: serde_json::Value) -> Refusal {
        self.detail = detail;
        self
    }

    /// The canonical §4.4 `error.data` shape.
    pub fn into_data(self, failed_index: usize, failed_method: &str) -> serde_json::Value {
        serde_json::json!({
            "refusal": self.name,
            "failed_index": failed_index,
            "failed_method": failed_method,
            "detail": self.detail,
            "explanation": self.explanation,
        })
    }
}

/// The structured payload of a refusal that carries numbers the caller can
/// act on (01-conventions §3): a limit, a maximum, a location. `{}` for the
/// rest.
///
/// `operand_face_not_flat` names the face: `operand` is `"a"` or `"b"`
/// (the first or second operand of the pairwise boolean that refused, or
/// the 1-based place of a later one, as a string) and `face` is the
/// kernel's key for the face in that operand, an opaque hex string like
/// the key part of a region or edge id. A solid face has no public id
/// (§5.2), so the key is for reports and logs, not for addressing it.
fn refusal_detail(e: &DocumentError) -> serde_json::Value {
    use kernel::{BlendError, ShapeError};
    use slotmap::Key as _;
    let point = |p: kernel::Point3| serde_json::json!([p.x, p.y, p.z]);
    match e {
        DocumentError::Shape(ShapeError::SelfIntersecting { limit: Some(l) }) => {
            serde_json::json!({ "limit": l })
        }
        DocumentError::Shape(ShapeError::SmoothTooDense { faces, cap }) => {
            serde_json::json!({ "faces": faces, "cap": cap })
        }
        DocumentError::Shape(ShapeError::WallTooThick { at })
        | DocumentError::Shape(ShapeError::UnsupportedShellCorner { at })
        | DocumentError::Blend(BlendError::UnsupportedVertex { at })
        | DocumentError::Blend(BlendError::BlendFacesFlush { at })
        | DocumentError::Blend(BlendError::Obstructed { at }) => {
            serde_json::json!({ "at": point(*at) })
        }
        DocumentError::Blend(BlendError::RadiusTooLarge { max, limited_by }) => {
            serde_json::json!({ "max": max, "limited_by": limited_by.name() })
        }
        DocumentError::Sketch(kernel::SketchError::CornerRadiusTooLarge { max }) => {
            serde_json::json!({ "max": max })
        }
        DocumentError::Blend(BlendError::ProfileTooWide { needed, available }) => {
            serde_json::json!({ "needed": needed, "available": available })
        }
        DocumentError::Boolean(kernel::BooleanError::NonPlanarInput { at })
        | DocumentError::Boolean(kernel::BooleanError::DegenerateInput { at }) => {
            serde_json::json!({ "at": point(*at) })
        }
        DocumentError::Boolean(kernel::BooleanError::OperandFaceNotFlat { which, face }) => {
            let operand = match which {
                kernel::Operand::A => "a".to_string(),
                kernel::Operand::B => "b".to_string(),
                kernel::Operand::Nth(n) => n.to_string(),
            };
            serde_json::json!({
                "operand": operand,
                "face": format!("{:x}", face.data().as_ffi()),
            })
        }
        // `node` (the locked node's public id) is added by
        // `Refusal::with_document`, which has the document.
        DocumentError::LockedNode {
            via_shared_shape, ..
        } => serde_json::json!({ "via_shared_shape": via_shared_shape }),
        _ => serde_json::json!({}),
    }
}

/// The copy-table key of the indirect lock's explanation
/// (`app/src/kernelErrors.ts`): not a kernel variant, a second sentence
/// for [`DocumentError::LockedNode`] when `via_shared_shape` is set.
const LOCKED_VIA_SHARED_SHAPE: &str = "LockedNodeViaSharedShape";

/// The error's variant code: the leading alphanumeric run of its `Debug`
/// form — the innermost variant name for delegating variants is NOT
/// unwrapped here; the kernel's `DocumentError` wrappers (`Sketch(..)`,
/// `Extrude(..)`, …) delegate to their inner variant name below.
fn variant_code(e: &DocumentError) -> String {
    // Delegating variants surface the INNERMOST error's variant name,
    // exactly like the wasm boundary's `doc_err` (its CODE convention —
    // B3). `Op` nests twice (`Op(PushPull(..))` / `Op(Sticky(..))`), so it
    // unwraps twice — a one-level unwrap here once collapsed every
    // push/pull refusal into one generic name.
    let debug = match e {
        DocumentError::Sketch(inner) => format!("{inner:?}"),
        DocumentError::Extrude(inner) => format!("{inner:?}"),
        DocumentError::FollowMe(inner) => format!("{inner:?}"),
        DocumentError::Boolean(inner) => format!("{inner:?}"),
        DocumentError::Slice(inner) => format!("{inner:?}"),
        DocumentError::Transform(inner) => format!("{inner:?}"),
        DocumentError::Op(kernel::KernelOpError::PushPull(inner)) => format!("{inner:?}"),
        DocumentError::Op(kernel::KernelOpError::Sticky(inner)) => format!("{inner:?}"),
        DocumentError::InvalidAxesFrame(inner) => format!("{inner:?}"),
        DocumentError::Repair(inner) => format!("{inner:?}"),
        DocumentError::Shape(inner) => format!("{inner:?}"),
        DocumentError::Blend(inner) => format!("{inner:?}"),
        other => format!("{other:?}"),
    };
    debug
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect()
}

/// `PushPullObstructed` → `push_pull_obstructed`.
pub fn snake_case(pascal: &str) -> String {
    let mut out = String::with_capacity(pascal.len() + 8);
    for (i, c) in pascal.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// UI copy for a kernel error code — delegates to the generated table
/// (see the module doc comment above). Codes the UI table has none for
/// fall back to the kernel `Display` text.
/// In a production build, copy that would point at a Preview feature is
/// replaced by the app's production wording ([`crate::preview::production_copy`]).
fn ui_copy(code: &str) -> Option<&'static str> {
    crate::preview::production_copy(code).or_else(|| refusal_copy_gen::ui_copy(code))
}

/// The inverse of [`snake_case`]: `push_pull_obstructed` →
/// `PushPullObstructed`. Used to look a registry refusal *name* (§4.4,
/// always snake_case) back up in the UI copy table, which is keyed by the
/// kernel's own PascalCase variant names.
pub fn pascal_case(snake: &str) -> String {
    let mut out = String::with_capacity(snake.len());
    let mut capitalize_next = true;
    for c in snake.chars() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            out.extend(c.to_uppercase());
            capitalize_next = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// The plain-language explanation for a registry refusal *name* (e.g.
/// `"distance_too_small"`), if the UI table has one — used by the
/// generated API reference (docs/agents/HEW_API.md §9) to publish refusal
/// explanations without hand-duplicating them. Refusals the API layer
/// mints itself (`unimplemented`, `host_capability_missing`, …) are not
/// kernel error variants and legitimately have none.
pub fn explanation_for(refusal_name: &str) -> Option<&'static str> {
    ui_copy(&pascal_case(refusal_name))
}

/// [`explanation_for`] as the published reference states it: the UI table's
/// copy whatever the build, so the generated API reference is the same in a
/// preview and a production build.
pub fn reference_explanation_for(refusal_name: &str) -> Option<&'static str> {
    refusal_copy_gen::ui_copy(&pascal_case(refusal_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_names_are_snake_case_variant_names() {
        let r = Refusal::from_document_error(&DocumentError::NothingToUndo);
        assert_eq!(r.name, "nothing_to_undo");
        assert_eq!(r.explanation, "Nothing to undo.");
    }

    /// A lock's refusal (OW-1): `locked_node`, its detail saying whether
    /// the lock reaches the edit only through a shared shape (with copy of
    /// its own, offering Make Unique), and the locked node's public id
    /// once the document is at hand.
    #[test]
    fn a_locked_node_refusal_names_the_node_and_the_indirect_case() {
        let mut doc = kernel::Document::new();
        let s = doc.add_sketch(
            kernel::Plane::from_point_normal(
                kernel::Point3::new(0.0, 0.0, 0.0),
                kernel::Vec3::new(0.0, 0.0, 1.0),
            )
            .unwrap(),
        );
        let sk = doc.sketch_mut(s).unwrap();
        let c = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            sk.add_segment(
                kernel::Point3::new(a.0, a.1, 0.0),
                kernel::Point3::new(b.0, b.1, 0.0),
            )
            .unwrap();
        }
        let region = doc.extrudable_regions(s).unwrap()[0];
        let (o, _) = doc.extrude_region(s, region, 1.0).unwrap();
        let node = kernel::NodeId::Object(o);
        let id = crate::ids::public_id(
            &kernel::EntityRef::Object(o),
            doc.sid_of(&kernel::EntityRef::Object(o)).unwrap(),
        );
        for via in [false, true] {
            let r = Refusal::from_document_error(&DocumentError::LockedNode {
                node,
                via_shared_shape: via,
            })
            .with_document(&doc);
            assert_eq!(r.name, "locked_node");
            assert_eq!(r.detail["via_shared_shape"], via);
            assert_eq!(r.detail["node"], id.as_str());
            let copy = if via {
                "This shape is also used inside a locked group. Make this copy unique to change it."
            } else {
                "Locked. Unlock it to change it."
            };
            assert_eq!(r.explanation, copy);
        }
    }

    #[test]
    fn delegating_variants_surface_the_inner_name() {
        let r = Refusal::from_document_error(&DocumentError::Extrude(
            kernel::ExtrudeError::DistanceTooSmall,
        ));
        assert_eq!(r.name, "distance_too_small");
    }

    /// `ui_copy` reaches the generated table for a sampling of codes
    /// spanning several op families — not exhaustive (that's
    /// `app/src/kernelErrorsDump.test.ts`'s job, which regenerates and
    /// diffs the ENTIRE table against `kernelErrors.ts` on every run),
    /// just a guard that the generated module is actually wired up and
    /// returns real UI copy rather than silently falling through to the
    /// kernel `Display` fallback.
    #[test]
    fn ui_copy_covers_a_sampling_of_known_codes() {
        for code in [
            "DistanceTooSmall",
            "ObjectNotSolid",
            "WouldVanish",
            "NonManifoldResult",
            "DegenerateSegment",
            "PointOffPlane",
            "MalformedRegion",
            "DegenerateCurve",
            "NothingToUndo",
            "NothingToRedo",
            "UnknownTag",
            "ExplodeSessionOpen",
            "BooleanOperandEmpty",
        ] {
            assert!(
                ui_copy(code).is_some(),
                "expected generated UI copy for {code}"
            );
        }
    }

    /// The import-boolean refusals name the offending face's position.
    #[test]
    fn import_boolean_refusals_carry_the_face_position() {
        let at = kernel::Point3::new(1.0, 2.0, 3.0);
        for (e, name) in [
            (
                kernel::BooleanError::NonPlanarInput { at },
                "non_planar_input",
            ),
            (
                kernel::BooleanError::DegenerateInput { at },
                "degenerate_input",
            ),
        ] {
            let e = DocumentError::Boolean(e);
            assert_eq!(snake_case(&variant_code(&e)), name);
            assert_eq!(
                refusal_detail(&e),
                serde_json::json!({ "at": [1.0, 2.0, 3.0] })
            );
        }
    }

    #[test]
    fn ui_copy_is_none_for_an_unknown_code() {
        assert_eq!(ui_copy("TotallyMadeUpCode"), None);
    }

    #[test]
    fn pascal_case_is_the_inverse_of_snake_case() {
        for pascal in [
            "PushPullObstructed",
            "DistanceTooSmall",
            "NonManifoldResult",
            "A",
        ] {
            assert_eq!(pascal_case(&snake_case(pascal)), pascal);
        }
    }

    #[test]
    fn explanation_for_finds_kernel_backed_refusal_names() {
        assert_eq!(
            explanation_for("distance_too_small"),
            Some(
                "That distance is too small to build anything. Drag further, or type an exact length."
            )
        );
        assert_eq!(explanation_for("nothing_to_undo"), Some("Nothing to undo."));
    }

    #[test]
    fn explanation_for_is_none_for_an_api_minted_refusal() {
        // "unimplemented" and friends are minted by the api layer, not a
        // kernel DocumentError variant, so the UI table has no entry.
        assert_eq!(explanation_for("unimplemented"), None);
    }
}
