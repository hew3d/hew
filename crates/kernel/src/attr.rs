//! Attribute dictionaries (docs/HEW_API.md §8; manifest v14+,
//! HEW_FILE_FORMAT.md): named, namespaced bags of client data the kernel
//! stores, round-trips, and never interprets — the persistence extension
//! point for API clients, plugins, and overlays.
//!
//! A dictionary is addressed `(target, namespace)`; values are read and
//! written at key granularity. The value model is JSON minus the parts a
//! deterministic kernel cannot store: numbers are either exact `i64`s or
//! finite `f64`s (NaN/±Inf are refused typed — DEVELOPMENT.md rule 4
//! posture: reject, never repair), and objects are `BTreeMap`s so every
//! iteration — and therefore every serialization — is deterministic
//! (ARCHITECTURE.md §5.5).

use std::collections::BTreeMap;

/// The maximum nesting depth of an [`AttrValue`] tree (lists and maps
/// count one level each). Deep enough for any honest payload, shallow
/// enough that every recursive walk over a validated value — including
/// the compiler-generated drop glue — stays comfortably inside the stack.
/// A deeper value is refused typed ([`AttrValueError::TooDeep`]), never
/// accepted and never crashed on.
pub const MAX_ATTR_DEPTH: usize = 64;

/// One attribute value: JSON-isomorphic, deterministic, finite.
///
/// Constructed freely (the fields are public); every mutation entry point
/// that stores one ([`crate::Document::attr_set`]) validates it with
/// [`AttrValue::validate`] first, so a non-finite number is refused typed
/// at the boundary rather than poisoning a save.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrValue {
    Null,
    Bool(bool),
    /// An exact integer. JSON integers that fit `i64` load as this
    /// variant, never as a lossy float.
    Int(i64),
    /// A finite double. NaN and ±Inf are invalid everywhere
    /// ([`AttrValue::validate`]).
    Float(f64),
    Text(String),
    List(Vec<AttrValue>),
    Map(BTreeMap<String, AttrValue>),
}

/// Why an [`AttrValue`] (or its JSON encoding) was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttrValueError {
    /// A `Float` is NaN or ±Inf. Non-finite numbers are invalid anywhere
    /// in the model (they cannot round-trip through JSON and compare
    /// unequal to themselves).
    NonFinite,
    /// A JSON number that fits neither `i64` nor `f64` exactly (a `u64`
    /// beyond `i64::MAX`). Storing it lossily would be silent repair.
    UnrepresentableNumber,
    /// The value nests deeper than [`MAX_ATTR_DEPTH`]. Accepting it would
    /// make every later traversal of the stored tree — validation,
    /// serialization, even drop — a stack overflow waiting to happen, so
    /// it is refused typed at the boundary instead.
    TooDeep,
}

impl std::fmt::Display for AttrValueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AttrValueError::NonFinite => {
                write!(f, "attribute numbers must be finite (no NaN or infinity)")
            }
            AttrValueError::UnrepresentableNumber => {
                write!(f, "attribute number is too large to store exactly")
            }
            AttrValueError::TooDeep => write!(
                f,
                "attribute value nests deeper than {MAX_ATTR_DEPTH} levels"
            ),
        }
    }
}

impl std::error::Error for AttrValueError {}

impl AttrValue {
    /// Checks the finiteness and depth invariants. `Ok` for every value
    /// [`AttrValue::from_json`] can produce; `Err` for a hand-built
    /// `Float(NaN)`/`Float(±Inf)` anywhere in the tree, or nesting beyond
    /// [`MAX_ATTR_DEPTH`]. The depth check runs FIRST at each level, so
    /// validation itself never recurses past the bound it enforces.
    pub fn validate(&self) -> Result<(), AttrValueError> {
        self.validate_at(0)
    }

    fn validate_at(&self, depth: usize) -> Result<(), AttrValueError> {
        if depth >= MAX_ATTR_DEPTH {
            return Err(AttrValueError::TooDeep);
        }
        match self {
            AttrValue::Float(x) if !x.is_finite() => Err(AttrValueError::NonFinite),
            AttrValue::List(items) => items.iter().try_for_each(|v| v.validate_at(depth + 1)),
            AttrValue::Map(entries) => entries.values().try_for_each(|v| v.validate_at(depth + 1)),
            _ => Ok(()),
        }
    }

    /// Converts a JSON value into an attribute value. Integers that fit
    /// `i64` stay exact; other numbers become finite floats; a `u64`
    /// beyond `i64::MAX` is refused rather than rounded; nesting beyond
    /// [`MAX_ATTR_DEPTH`] is refused before it can recurse there.
    pub fn from_json(value: &serde_json::Value) -> Result<AttrValue, AttrValueError> {
        AttrValue::from_json_at(value, 0)
    }

    fn from_json_at(value: &serde_json::Value, depth: usize) -> Result<AttrValue, AttrValueError> {
        if depth >= MAX_ATTR_DEPTH {
            return Err(AttrValueError::TooDeep);
        }
        Ok(match value {
            serde_json::Value::Null => AttrValue::Null,
            serde_json::Value::Bool(b) => AttrValue::Bool(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    AttrValue::Int(i)
                } else if n.is_u64() {
                    // A u64 beyond i64::MAX: `as_f64` would hand back a
                    // silently LOSSY float — refuse instead.
                    return Err(AttrValueError::UnrepresentableNumber);
                } else if let Some(x) = n.as_f64() {
                    // serde_json numbers are finite by grammar; belt and
                    // suspenders for float_roundtrip edge cases.
                    if !x.is_finite() {
                        return Err(AttrValueError::NonFinite);
                    }
                    AttrValue::Float(x)
                } else {
                    return Err(AttrValueError::UnrepresentableNumber);
                }
            }
            serde_json::Value::String(s) => AttrValue::Text(s.clone()),
            serde_json::Value::Array(items) => AttrValue::List(
                items
                    .iter()
                    .map(|v| AttrValue::from_json_at(v, depth + 1))
                    .collect::<Result<_, _>>()?,
            ),
            serde_json::Value::Object(entries) => AttrValue::Map(
                entries
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), AttrValue::from_json_at(v, depth + 1)?)))
                    .collect::<Result<_, _>>()?,
            ),
        })
    }

    /// Converts back to JSON — the wire form in `manifest.json` and on the
    /// API. Total for every valid value ([`AttrValue::validate`]); a
    /// non-finite float would panic, which is why validation happens at
    /// the mutation boundary.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            AttrValue::Null => serde_json::Value::Null,
            AttrValue::Bool(b) => serde_json::Value::Bool(*b),
            AttrValue::Int(i) => serde_json::Value::Number((*i).into()),
            AttrValue::Float(x) => serde_json::Value::Number(
                serde_json::Number::from_f64(*x).expect("validated attribute floats are finite"),
            ),
            AttrValue::Text(s) => serde_json::Value::String(s.clone()),
            AttrValue::List(items) => {
                serde_json::Value::Array(items.iter().map(AttrValue::to_json).collect())
            }
            AttrValue::Map(entries) => serde_json::Value::Object(
                entries
                    .iter()
                    .map(|(k, v)| (k.clone(), v.to_json()))
                    .collect(),
            ),
        }
    }
}

/// One entity's (or the document's) full attribute store:
/// namespace → key → value. `BTreeMap` end to end — deterministic
/// iteration is what makes saves byte-deterministic.
pub type AttrDict = BTreeMap<String, BTreeMap<String, AttrValue>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip_is_exact() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"n": null, "b": true, "i": -42, "x": 0.5,
                "s": "hi", "a": [1, "two"], "o": {"k": 3}}"#,
        )
        .unwrap();
        let value = AttrValue::from_json(&json).unwrap();
        value.validate().unwrap();
        assert_eq!(value.to_json(), json);
    }

    #[test]
    fn integers_stay_exact() {
        let json = serde_json::json!(9_007_199_254_740_993_i64); // 2^53 + 1
        let value = AttrValue::from_json(&json).unwrap();
        assert_eq!(value, AttrValue::Int(9_007_199_254_740_993));
        assert_eq!(value.to_json(), json);
    }

    #[test]
    fn oversized_u64_is_refused() {
        let json = serde_json::json!(u64::MAX);
        assert_eq!(
            AttrValue::from_json(&json),
            Err(AttrValueError::UnrepresentableNumber)
        );
    }

    #[test]
    fn too_deep_values_are_refused_not_crashed_on() {
        let mut v = AttrValue::Null;
        for _ in 0..MAX_ATTR_DEPTH {
            v = AttrValue::List(vec![v]);
        }
        assert_eq!(v.validate(), Err(AttrValueError::TooDeep));
        assert_eq!(
            AttrValue::from_json(&v_to_json_shallow(MAX_ATTR_DEPTH)),
            Err(AttrValueError::TooDeep)
        );
        // One level inside the bound is fine.
        let mut ok = AttrValue::Null;
        for _ in 0..(MAX_ATTR_DEPTH - 1) {
            ok = AttrValue::List(vec![ok]);
        }
        ok.validate().unwrap();
    }

    /// A JSON array nested `depth` levels, built iteratively (the point of
    /// the bound is that we can't lean on recursion for deep shapes).
    fn v_to_json_shallow(depth: usize) -> serde_json::Value {
        let mut v = serde_json::Value::Null;
        for _ in 0..depth {
            v = serde_json::Value::Array(vec![v]);
        }
        v
    }

    #[test]
    fn hand_built_nan_fails_validation() {
        let v = AttrValue::List(vec![AttrValue::Float(f64::NAN)]);
        assert_eq!(v.validate(), Err(AttrValueError::NonFinite));
    }
}
