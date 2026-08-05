//! Refusals: the typed "no" the API surfaces verbatim (docs/HEW_API.md
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
}

impl Refusal {
    /// Builds the refusal for a kernel [`DocumentError`].
    pub fn from_document_error(e: &DocumentError) -> Refusal {
        let code = variant_code(e);
        let explanation = ui_copy(&code)
            .map(str::to_string)
            .unwrap_or_else(|| e.to_string());
        Refusal {
            name: snake_case(&code),
            detail: serde_json::json!({}),
            explanation,
        }
    }

    /// A refusal minted by the API layer itself (not a kernel error).
    pub fn api(name: &str, explanation: &str) -> Refusal {
        Refusal {
            name: name.to_string(),
            detail: serde_json::json!({}),
            explanation: explanation.to_string(),
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
fn ui_copy(code: &str) -> Option<&'static str> {
    refusal_copy_gen::ui_copy(code)
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
/// generated API reference (docs/HEW_API.md §9) to publish refusal
/// explanations without hand-duplicating them. Refusals the API layer
/// mints itself (`unimplemented`, `host_capability_missing`, …) are not
/// kernel error variants and legitimately have none.
pub fn explanation_for(refusal_name: &str) -> Option<&'static str> {
    ui_copy(&pascal_case(refusal_name))
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
