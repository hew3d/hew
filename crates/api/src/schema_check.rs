//! A deliberately small JSON Schema subset for plugin-provided commands
//! (docs/agents/PLUGINS.md "Commands"): the dispatcher checks a plugin
//! command's params against its declared schema before forwarding them,
//! and the plugin's result after. First-party commands keep their serde
//! validation and never go through this.
//!
//! [`validate_schema`] runs at registration and refuses any schema this
//! checker could not enforce, so a declared schema always means exactly
//! what [`check`] does with it. Supported keywords: `type`, `properties`,
//! `required`, `additionalProperties`, `items`, `enum`, `const`, `minimum`,
//! `maximum`, `minLength`, `maxLength`, `minItems`, `maxItems`, `oneOf`,
//! `anyOf`; `description`, `title`, `default`, `examples`, `$schema`, and
//! `$comment` are accepted and ignored.
//!
//! Pointers in messages are JSON pointers into the checked value (or, for
//! [`validate_schema`], into the schema), with the root written `/`.
//!
//! Reporting order: depth-first, and within an object alphabetically by
//! key (the order `serde_json`'s map keeps; the declared order of
//! `properties` is not kept), then `required`, then extra members. Which
//! of several mismatches is named first therefore depends on key names,
//! not on how the schema lists them.

use serde_json::Value;

/// The type names `type` accepts.
const TYPES: [&str; 7] = [
    "object", "array", "string", "number", "integer", "boolean", "null",
];

/// Keywords accepted and ignored.
const ANNOTATIONS: [&str; 6] = [
    "description",
    "title",
    "default",
    "examples",
    "$schema",
    "$comment",
];

/// Refuse schemas this checker cannot enforce, at registration time.
/// The root must be `"type": "object"` with `"additionalProperties":
/// false`; every keyword anywhere must be one [`check`] enforces (or an
/// ignored annotation), with a value of the right shape.
pub fn validate_schema(schema: &Value) -> Result<(), String> {
    let Some(root) = schema.as_object() else {
        return Err("/: a schema must be a JSON object".into());
    };
    if root.get("type") != Some(&Value::String("object".into())) {
        return Err("/: the root must have \"type\": \"object\"".into());
    }
    if root.get("additionalProperties") != Some(&Value::Bool(false)) {
        return Err("/: the root must have \"additionalProperties\": false".into());
    }
    validate_node(schema, "")
}

fn validate_node(schema: &Value, at: &str) -> Result<(), String> {
    let here = shown(at);
    let Some(object) = schema.as_object() else {
        return Err(format!("{here}: a schema must be a JSON object"));
    };
    for (keyword, value) in object {
        let bad = |what: &str| Err(format!("{here}: \"{keyword}\" {what}"));
        match keyword.as_str() {
            k if ANNOTATIONS.contains(&k) => {}
            "type" => match value {
                Value::String(t) if TYPES.contains(&t.as_str()) => {}
                Value::Array(ts)
                    if !ts.is_empty()
                        && ts
                            .iter()
                            .all(|t| t.as_str().is_some_and(|t| TYPES.contains(&t))) => {}
                _ => return bad("must be a type name or a non-empty array of type names"),
            },
            "properties" => {
                let Some(props) = value.as_object() else {
                    return bad("must be an object of schemas");
                };
                for (name, sub) in props {
                    validate_node(sub, &format!("{at}/properties/{}", escape(name)))?;
                }
            }
            "required" => {
                if !value
                    .as_array()
                    .is_some_and(|names| names.iter().all(Value::is_string))
                {
                    return bad("must be an array of property names");
                }
            }
            "additionalProperties" => match value {
                Value::Bool(false) => {}
                Value::Object(_) => validate_node(value, &format!("{at}/additionalProperties"))?,
                _ => return bad("must be false or a schema"),
            },
            "items" => {
                if !value.is_object() {
                    return bad("must be one schema");
                }
                validate_node(value, &format!("{at}/items"))?;
            }
            "enum" => {
                if !value.as_array().is_some_and(|v| !v.is_empty()) {
                    return bad("must be a non-empty array");
                }
            }
            "const" => {}
            "minimum" | "maximum" => {
                if !value.is_number() {
                    return bad("must be a number");
                }
            }
            "minLength" | "maxLength" | "minItems" | "maxItems" => {
                if count(value).is_none() {
                    return bad("must be a non-negative integer");
                }
            }
            "oneOf" | "anyOf" => {
                let Some(subs) = value.as_array().filter(|v| !v.is_empty()) else {
                    return bad("must be a non-empty array of schemas");
                };
                for (i, sub) in subs.iter().enumerate() {
                    validate_node(sub, &format!("{at}/{keyword}/{i}"))?;
                }
            }
            _ => return Err(format!("{here}: unsupported keyword \"{keyword}\"")),
        }
    }
    Ok(())
}

/// `Err("<json pointer>: <why>")` on the first mismatch, depth-first, in
/// alphabetical key order (see the module comment). The schema is assumed to have passed [`validate_schema`];
/// a keyword with a malformed value is skipped rather than trusted.
pub fn check(schema: &Value, value: &Value) -> Result<(), String> {
    check_at(schema, value, "")
}

fn check_at(schema: &Value, value: &Value, at: &str) -> Result<(), String> {
    let Some(s) = schema.as_object() else {
        return Ok(());
    };
    let fail = |why: String| Err(format!("{}: {why}", shown(at)));

    if let Some(t) = s.get("type") {
        let allowed: Vec<&str> = match t {
            Value::String(one) => vec![one.as_str()],
            Value::Array(many) => many.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        if !allowed.is_empty() && !allowed.iter().any(|t| is_type(value, t)) {
            return fail(format!(
                "expected {}, found {}",
                allowed.join(" or "),
                type_of(value)
            ));
        }
    }
    if let Some(options) = s.get("enum").and_then(Value::as_array)
        && !options.iter().any(|o| json_equal(o, value))
    {
        return fail("not one of the allowed values".into());
    }
    if let Some(constant) = s.get("const")
        && !json_equal(constant, value)
    {
        return fail("not one of the allowed values".into());
    }
    if let Value::Number(n) = value {
        if let Some(Value::Number(min)) = s.get("minimum")
            && compare_numbers(n, min) == Some(std::cmp::Ordering::Less)
        {
            return fail(format!("below minimum {min}"));
        }
        if let Some(Value::Number(max)) = s.get("maximum")
            && compare_numbers(n, max) == Some(std::cmp::Ordering::Greater)
        {
            return fail(format!("above maximum {max}"));
        }
    }
    if let Some(text) = value.as_str() {
        let len = text.chars().count() as u64;
        if let Some(min) = s.get("minLength").and_then(count)
            && len < min
        {
            return fail(format!("shorter than {min}"));
        }
        if let Some(max) = s.get("maxLength").and_then(count)
            && len > max
        {
            return fail(format!("longer than {max}"));
        }
    }
    if let Some(items) = value.as_array() {
        let len = items.len() as u64;
        if let Some(min) = s.get("minItems").and_then(count)
            && len < min
        {
            return fail(format!("fewer than {min} items"));
        }
        if let Some(max) = s.get("maxItems").and_then(count)
            && len > max
        {
            return fail(format!("more than {max} items"));
        }
        if let Some(item_schema) = s.get("items") {
            for (i, item) in items.iter().enumerate() {
                check_at(item_schema, item, &format!("{at}/{i}"))?;
            }
        }
    }
    if let Some(members) = value.as_object() {
        let props = s.get("properties").and_then(Value::as_object);
        if let Some(props) = props {
            for (name, sub) in props {
                if let Some(member) = members.get(name) {
                    check_at(sub, member, &format!("{at}/{}", escape(name)))?;
                }
            }
        }
        if let Some(required) = s.get("required").and_then(Value::as_array) {
            for name in required.iter().filter_map(Value::as_str) {
                if !members.contains_key(name) {
                    return fail(format!("missing required property \"{name}\""));
                }
            }
        }
        match s.get("additionalProperties") {
            Some(Value::Bool(false)) => {
                if let Some(extra) = members
                    .keys()
                    .find(|k| !props.is_some_and(|p| p.contains_key(*k)))
                {
                    return fail(format!("property \"{extra}\" is not allowed"));
                }
            }
            Some(extra_schema @ Value::Object(_)) => {
                for (name, member) in members {
                    if !props.is_some_and(|p| p.contains_key(name)) {
                        check_at(extra_schema, member, &format!("{at}/{}", escape(name)))?;
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(options) = s.get("oneOf").and_then(Value::as_array) {
        let matching = options
            .iter()
            .filter(|o| check_at(o, value, at).is_ok())
            .count();
        if matching == 0 {
            return fail("matches none of oneOf".into());
        }
        if matching > 1 {
            return fail("matches more than one of oneOf".into());
        }
    }
    if let Some(options) = s.get("anyOf").and_then(Value::as_array)
        && !options.iter().any(|o| check_at(o, value, at).is_ok())
    {
        return fail("matches none of anyOf".into());
    }
    Ok(())
}

/// Whether `value` is of the schema type `t`. `integer` is a JSON number
/// with no fractional part; `number` is any JSON number.
fn is_type(value: &Value, t: &str) -> bool {
    match t {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => is_integer(value),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    }
}

fn is_integer(value: &Value) -> bool {
    match value {
        Value::Number(n) => {
            n.is_i64()
                || n.is_u64()
                || n.as_f64()
                    .is_some_and(|f| f.is_finite() && f.fract() == 0.0)
        }
        _ => false,
    }
}

fn type_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// A count keyword's value (`minLength` and the like): a non-negative
/// integer, written as `2` or `2.0`.
fn count(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        let f = value.as_f64()?;
        (f.is_finite() && f >= 0.0 && f.fract() == 0.0 && f < 18_446_744_073_709_551_616.0)
            .then_some(f as u64)
    })
}

/// Compares two JSON numbers by their mathematical value, exactly: an
/// integer against a float is not rounded through `f64`, so integers
/// above 2^53 compare correctly. `None` only for a NaN, which JSON cannot
/// hold.
fn compare_numbers(a: &serde_json::Number, b: &serde_json::Number) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    let int = |n: &serde_json::Number| {
        n.as_i64()
            .map(i128::from)
            .or_else(|| n.as_u64().map(i128::from))
    };
    // An integer `i` against a float `f`.
    let int_vs_float = |i: i128, f: f64| -> Option<Ordering> {
        if f.is_nan() {
            return None;
        }
        // Every i64/u64 lies strictly inside +-2^127.
        if f >= 1.7014118346046923e38 {
            return Some(Ordering::Less);
        }
        if f < -1.7014118346046923e38 {
            return Some(Ordering::Greater);
        }
        let whole = f.trunc();
        match i.cmp(&(whole as i128)) {
            Ordering::Equal => (0.0).partial_cmp(&(f - whole)),
            other => Some(other),
        }
    };
    match (int(a), int(b)) {
        (Some(x), Some(y)) => Some(x.cmp(&y)),
        (Some(x), None) => int_vs_float(x, b.as_f64()?),
        (None, Some(y)) => int_vs_float(y, a.as_f64()?).map(Ordering::reverse),
        (None, None) => a.as_f64()?.partial_cmp(&b.as_f64()?),
    }
}

/// JSON Schema equality: numbers compare by their exact value (`1` equals
/// `1.0`), everything else structurally.
fn json_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            compare_numbers(x, y) == Some(std::cmp::Ordering::Equal)
        }
        (Value::Array(xs), Value::Array(ys)) => {
            xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| json_equal(x, y))
        }
        (Value::Object(xs), Value::Object(ys)) => {
            xs.len() == ys.len()
                && xs
                    .iter()
                    .all(|(k, x)| ys.get(k).is_some_and(|y| json_equal(x, y)))
        }
        _ => a == b,
    }
}

/// A JSON pointer reference token (`~` → `~0`, `/` → `~1`).
fn escape(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

/// The pointer as messages show it: the root is `/`.
fn shown(at: &str) -> &str {
    if at.is_empty() { "/" } else { at }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn closed(props: Value, required: &[&str]) -> Value {
        json!({ "type": "object", "properties": props, "required": required, "additionalProperties": false })
    }

    /// One row per keyword and per message text.
    #[test]
    fn schema_check_cases() {
        let one = |s: Value| closed(json!({ "v": s }), &["v"]);
        let rows: Vec<(Value, Value, Option<&str>)> = vec![
            // type, every name, and a type list
            (one(json!({"type": "number"})), json!({"v": 1.5}), None),
            (
                one(json!({"type": "number"})),
                json!({"v": "x"}),
                Some("/v: expected number, found string"),
            ),
            (one(json!({"type": "integer"})), json!({"v": 2}), None),
            (one(json!({"type": "integer"})), json!({"v": 2.0}), None),
            (
                one(json!({"type": "integer"})),
                json!({"v": 2.5}),
                Some("/v: expected integer, found number"),
            ),
            (
                one(json!({"type": "string"})),
                json!({"v": true}),
                Some("/v: expected string, found boolean"),
            ),
            (
                one(json!({"type": "boolean"})),
                json!({"v": null}),
                Some("/v: expected boolean, found null"),
            ),
            (
                one(json!({"type": "null"})),
                json!({"v": []}),
                Some("/v: expected null, found array"),
            ),
            (
                one(json!({"type": "array"})),
                json!({"v": {}}),
                Some("/v: expected array, found object"),
            ),
            (
                one(json!({"type": "object"})),
                json!({"v": 1}),
                Some("/v: expected object, found number"),
            ),
            (
                one(json!({"type": ["string", "null"]})),
                json!({"v": null}),
                None,
            ),
            (
                one(json!({"type": ["string", "null"]})),
                json!({"v": 3}),
                Some("/v: expected string or null, found number"),
            ),
            // properties, required, additionalProperties
            (
                one(json!({"type": "number"})),
                json!({}),
                Some("/: missing required property \"v\""),
            ),
            (
                one(json!({"type": "number"})),
                json!({"v": 1, "w": 2}),
                Some("/: property \"w\" is not allowed"),
            ),
            (
                one(
                    json!({"type": "object", "properties": {}, "additionalProperties": {"type": "string"}}),
                ),
                json!({"v": {"x": 1}}),
                Some("/v/x: expected string, found number"),
            ),
            (
                json!({"type": "object", "properties": {"a~b/c": {"type": "string"}}, "additionalProperties": false}),
                json!({"a~b/c": 1}),
                Some("/a~0b~1c: expected string, found number"),
            ),
            // nested reporting is depth-first, alphabetically by key
            (
                closed(
                    json!({"b": {"type": "number"}, "a": {"type": "number"}}),
                    &["b", "a"],
                ),
                json!({"a": "x", "b": "y"}),
                Some("/a: expected number, found string"),
            ),
            (
                closed(
                    json!({"a": closed(json!({"b": {"type": "string"}}), &["b"]), "c": {"type": "string"}}),
                    &["a", "c"],
                ),
                json!({"a": {"b": 1}, "c": 2}),
                Some("/a/b: expected string, found number"),
            ),
            // items, with the index in the pointer
            (
                one(json!({"type": "array", "items": {"type": "integer"}})),
                json!({"v": [1, 2, "x"]}),
                Some("/v/2: expected integer, found string"),
            ),
            // enum and const (numbers compare by value)
            (
                one(json!({"enum": ["a", "b"]})),
                json!({"v": "c"}),
                Some("/v: not one of the allowed values"),
            ),
            (one(json!({"enum": [1, 2]})), json!({"v": 1.0}), None),
            (
                one(json!({"const": {"k": [1]}})),
                json!({"v": {"k": [1]}}),
                None,
            ),
            (
                one(json!({"const": 3})),
                json!({"v": 4}),
                Some("/v: not one of the allowed values"),
            ),
            // exact comparison above 2^53, and integer-valued float counts
            (
                one(json!({"enum": [9007199254740992.0]})),
                json!({"v": 9007199254740993i64}),
                Some("/v: not one of the allowed values"),
            ),
            (
                one(json!({"enum": [9007199254740992.0]})),
                json!({"v": 9007199254740992i64}),
                None,
            ),
            (
                one(json!({"maximum": 9007199254740992.0})),
                json!({"v": 9007199254740993u64}),
                Some("/v: above maximum 9007199254740992.0"),
            ),
            (
                one(json!({"minimum": 0.5})),
                json!({"v": 0}),
                Some("/v: below minimum 0.5"),
            ),
            (one(json!({"minimum": -0.5})), json!({"v": 0}), None),
            (
                one(json!({"type": "string", "minLength": 2.0})),
                json!({"v": "a"}),
                Some("/v: shorter than 2"),
            ),
            // minimum, maximum
            (
                one(json!({"type": "number", "minimum": 0})),
                json!({"v": -1}),
                Some("/v: below minimum 0"),
            ),
            (
                one(json!({"type": "number", "minimum": 0})),
                json!({"v": 0}),
                None,
            ),
            (
                one(json!({"type": "number", "maximum": 2.5})),
                json!({"v": 3}),
                Some("/v: above maximum 2.5"),
            ),
            // minLength, maxLength (in characters)
            (
                one(json!({"type": "string", "minLength": 2})),
                json!({"v": "é"}),
                Some("/v: shorter than 2"),
            ),
            (
                one(json!({"type": "string", "maxLength": 2})),
                json!({"v": "éé"}),
                None,
            ),
            (
                one(json!({"type": "string", "maxLength": 2})),
                json!({"v": "abc"}),
                Some("/v: longer than 2"),
            ),
            // minItems, maxItems
            (
                one(json!({"type": "array", "minItems": 1})),
                json!({"v": []}),
                Some("/v: fewer than 1 items"),
            ),
            (
                one(json!({"type": "array", "maxItems": 1})),
                json!({"v": [1, 2]}),
                Some("/v: more than 1 items"),
            ),
            // oneOf, anyOf
            (
                one(json!({"oneOf": [{"type": "string"}, {"type": "integer"}]})),
                json!({"v": true}),
                Some("/v: matches none of oneOf"),
            ),
            (
                one(json!({"oneOf": [{"type": "number"}, {"type": "integer"}]})),
                json!({"v": 1}),
                Some("/v: matches more than one of oneOf"),
            ),
            (
                one(json!({"oneOf": [{"type": "number"}, {"type": "integer"}]})),
                json!({"v": 1.5}),
                None,
            ),
            (
                one(json!({"anyOf": [{"type": "string"}, {"type": "null"}]})),
                json!({"v": 1}),
                Some("/v: matches none of anyOf"),
            ),
            (
                one(json!({"anyOf": [{"type": "string"}, {"type": "number"}]})),
                json!({"v": 1}),
                None,
            ),
            // ignored annotations
            (
                one(
                    json!({"type": "string", "description": "d", "title": "t", "default": "x", "examples": ["y"], "$comment": "c"}),
                ),
                json!({"v": "ok"}),
                None,
            ),
        ];
        assert!(rows.len() >= 25);
        for (i, (schema, value, expected)) in rows.iter().enumerate() {
            validate_schema(schema).unwrap_or_else(|e| panic!("row {i}: schema refused: {e}"));
            let got = check(schema, value);
            match expected {
                None => assert_eq!(got, Ok(()), "row {i}"),
                Some(message) => assert_eq!(got, Err(message.to_string()), "row {i}"),
            }
        }
    }

    #[test]
    fn validate_schema_refuses_unsupported_keywords() {
        let refused = [
            (
                json!({"type": "object", "properties": {"a": {"$ref": "#/x"}}, "additionalProperties": false}),
                "/properties/a: unsupported keyword \"$ref\"",
            ),
            (
                json!({"type": "object", "patternProperties": {}, "additionalProperties": false}),
                "/: unsupported keyword \"patternProperties\"",
            ),
            (
                json!({"type": "object", "properties": {"a": {"type": "string", "format": "uri"}}, "additionalProperties": false}),
                "/properties/a: unsupported keyword \"format\"",
            ),
            (
                json!({"type": "object", "properties": {}}),
                "/: the root must have \"additionalProperties\": false",
            ),
            (
                json!({"type": "array", "additionalProperties": false}),
                "/: the root must have \"type\": \"object\"",
            ),
            (
                json!({"type": "object", "properties": {"a": {"type": "decimal"}}, "additionalProperties": false}),
                "/properties/a: \"type\" must be a type name or a non-empty array of type names",
            ),
            (
                json!({"type": "object", "properties": {"a": {"oneOf": [{"not": {}}]}}, "additionalProperties": false}),
                "/properties/a/oneOf/0: unsupported keyword \"not\"",
            ),
        ];
        for (schema, why) in refused {
            assert_eq!(validate_schema(&schema), Err(why.to_string()), "{schema}");
        }
        validate_schema(
            &json!({"type": "object", "properties": {}, "additionalProperties": false}),
        )
        .expect("the empty closed object is valid");
    }
}
