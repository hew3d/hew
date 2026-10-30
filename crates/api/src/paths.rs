//! Path parameters (docs/agents/HEW_API.md §10): the request fields that
//! name a file on the host's disk. A restricted grant (a plugin or a
//! connected app) may not set any of them; it reaches files only through
//! the user's own pickers. First-party connections are unaffected.

/// (method, JSON pointer into params) pairs a restricted grant may not set.
/// `hew.library.save` has no path parameter (it writes inside the
/// library), so it is not listed.
pub const PATH_PARAMS: &[(&str, &str)] = &[
    ("hew.doc.open", "/path"),
    ("hew.doc.save", "/path"),
    ("hew.doc.export", "/path"),
    ("hew.doc.import", "/path"),
    ("hew.view.snapshot", "/path"),
    ("hew.view.line_drawing", "/path"),
    ("hew.print.pdf", "/path"),
    ("hew.material.create", "/texture/path"),
];

/// `Some(pointer)` when `params` sets a listed pointer for `method`. A
/// member present with any value, `null` included, counts as set, and so
/// does a transaction `$ref` object standing at the pointer or at any of
/// its parents (the root included), since substitution could fill it.
pub fn path_param_set(method: &str, params: &serde_json::Value) -> Option<&'static str> {
    PATH_PARAMS
        .iter()
        .find(|(m, pointer)| *m == method && sets(params, pointer))
        .map(|(_, pointer)| *pointer)
}

/// Commands whose `document` parameter names a window to act on. A
/// restricted grant without `documents.other` may not name one: it acts
/// on its own document by leaving the parameter out (or `null`).
/// `hew.doc.attach`'s parameter is guarded by attach itself.
pub const DOCUMENT_PARAMS: &[&str] = &["hew.doc.close", "hew.doc.revert", "hew.doc.focus"];

/// The `document` value a listed command's `params` names, when it names
/// one: any non-null value, or a transaction `$ref` at the parameter or at
/// the params root.
pub fn document_param_set(method: &str, params: &serde_json::Value) -> Option<serde_json::Value> {
    if !DOCUMENT_PARAMS.contains(&method) {
        return None;
    }
    match params.pointer("/document") {
        Some(serde_json::Value::Null) => None,
        Some(v) => Some(v.clone()),
        None if params.as_object().is_some_and(|o| o.contains_key("$ref")) => Some(params.clone()),
        None => None,
    }
}

fn sets(params: &serde_json::Value, pointer: &str) -> bool {
    if params.pointer(pointer).is_some() {
        return true;
    }
    let is_ref = |v: &serde_json::Value| v.as_object().is_some_and(|o| o.contains_key("$ref"));
    let mut prefix = String::new();
    if params.pointer("").is_some_and(is_ref) {
        return true;
    }
    for segment in pointer.trim_start_matches('/').split('/') {
        prefix.push('/');
        prefix.push_str(segment);
        if params.pointer(&prefix).is_some_and(is_ref) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Registry;
    use serde_json::json;

    /// Every listed pointer's first segment is a declared parameter of its
    /// command, so the list cannot name a field that does not exist.
    #[test]
    fn path_params_name_real_fields() {
        let reg = Registry::protocol_1();
        for (method, pointer) in PATH_PARAMS {
            let decl = reg
                .get(method)
                .unwrap_or_else(|| panic!("{method} is not registered"));
            let first = pointer
                .trim_start_matches('/')
                .split('/')
                .next()
                .expect("non-empty pointer");
            assert!(
                decl.params_schema
                    .pointer(&format!("/properties/{first}"))
                    .is_some(),
                "{method}'s params schema has no `{first}` property"
            );
        }
    }

    #[test]
    fn document_params_name_real_fields() {
        let reg = Registry::protocol_1();
        for method in DOCUMENT_PARAMS {
            let decl = reg.get(method).unwrap();
            assert!(
                decl.params_schema.pointer("/properties/document").is_some(),
                "{method} has no document parameter"
            );
        }
    }

    #[test]
    fn document_param_set_cases() {
        assert_eq!(
            document_param_set(
                "hew.doc.close",
                &json!({"document": "doc_2", "discard": true})
            ),
            Some(json!("doc_2"))
        );
        assert_eq!(
            document_param_set("hew.doc.close", &json!({"document": null})),
            None
        );
        assert_eq!(document_param_set("hew.doc.revert", &json!({})), None);
        assert_eq!(
            document_param_set("hew.doc.focus", &json!({"document": {"$ref": "a#/x"}})),
            Some(json!({"$ref": "a#/x"}))
        );
        assert_eq!(
            document_param_set("hew.doc.close", &json!({"$ref": "a#/x"})),
            Some(json!({"$ref": "a#/x"}))
        );
        assert_eq!(
            document_param_set("hew.query.scene", &json!({"document": "doc_2"})),
            None
        );
    }

    #[test]
    fn path_param_set_cases() {
        assert_eq!(
            path_param_set("hew.doc.export", &json!({"format": "stl", "path": "/x"})),
            Some("/path")
        );
        assert_eq!(
            path_param_set("hew.doc.export", &json!({"format": "stl"})),
            None
        );
        assert_eq!(
            path_param_set("hew.doc.save", &json!({"path": null})),
            Some("/path")
        );
        assert_eq!(
            path_param_set(
                "hew.material.create",
                &json!({"texture": {"path": "/t.png"}})
            ),
            Some("/texture/path")
        );
        assert_eq!(
            path_param_set(
                "hew.material.create",
                &json!({"texture": {"bytes": "AA=="}})
            ),
            None
        );
        assert_eq!(
            path_param_set("hew.query.scene", &json!({"path": "/x"})),
            None
        );
        assert_eq!(
            path_param_set("hew.doc.save", &serde_json::Value::Null),
            None
        );
        assert_eq!(
            path_param_set("hew.material.create", &json!({"texture": {"$ref": "a#/t"}})),
            Some("/texture/path")
        );
        assert_eq!(
            path_param_set("hew.print.pdf", &json!({"$ref": "a#/x"})),
            Some("/path")
        );
    }
}
