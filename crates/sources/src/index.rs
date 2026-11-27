//! Parsing and validating `hew.json` (6.1) and `catalog.json` (6.3).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};
use url::Url;

use crate::paths::{check_rel_path_with_suffix, is_commit_hex, is_sha256_hex};
use crate::{
    Catalog, CatalogSource, Category, IndexError, IndexItem, Kind, MAX_INDEX_BYTES, PluginEntry,
    RemovedEntry, SUPPORTED_HEW, Severity, SourceIndex, parse_source_ref,
};

/// Most items an index may list (6.1).
const MAX_ITEMS: usize = 20_000;
/// Longest `name`, in characters (6.1).
const MAX_NAME_CHARS: usize = 200;
/// Most keywords an item may carry, and the longest one, in characters (6.1).
const MAX_KEYWORDS: usize = 32;
const MAX_KEYWORD_CHARS: usize = 64;
/// Largest item `bytes` (6.1): 64 MiB.
const MAX_ITEM_BYTES: u64 = 67_108_864;

fn invalid(reason: impl Into<String>, item: Option<usize>) -> IndexError {
    IndexError::Invalid {
        reason: reason.into(),
        item,
    }
}

/// Checks the size, parses the JSON (the parser's own depth limit refuses
/// pathological nesting), requires an object, and checks `hew` first: a
/// newer document may have a shape this build cannot read, so
/// `index_too_new` wins over every shape error (6.1).
fn parse_document(bytes: &[u8]) -> Result<Map<String, Value>, IndexError> {
    if bytes.len() > MAX_INDEX_BYTES {
        return Err(IndexError::TooLarge);
    }
    let value: Value =
        serde_json::from_slice(bytes).map_err(|e| invalid(format!("not valid JSON: {e}"), None))?;
    let Value::Object(obj) = value else {
        return Err(invalid("the document is not a JSON object", None));
    };
    let hew = match obj.get("hew") {
        Some(Value::Number(n)) => n.as_u64().filter(|n| *n >= 1),
        _ => None,
    };
    match hew {
        Some(n) if n > SUPPORTED_HEW => Err(IndexError::TooNew { found: n }),
        Some(_) => Ok(obj),
        None => Err(invalid("`hew` must be a positive integer", None)),
    }
}

fn str_field(
    obj: &Map<String, Value>,
    key: &str,
    item: Option<usize>,
) -> Result<Option<String>, IndexError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(invalid(format!("`{key}` must be a string"), item)),
    }
}

fn req_str(obj: &Map<String, Value>, key: &str, item: Option<usize>) -> Result<String, IndexError> {
    str_field(obj, key, item)?.ok_or_else(|| invalid(format!("`{key}` is required"), item))
}

fn req_name(obj: &Map<String, Value>, item: Option<usize>) -> Result<String, IndexError> {
    let name = req_str(obj, "name", item)?;
    let n = name.chars().count();
    if n == 0 || n > MAX_NAME_CHARS {
        return Err(invalid(
            format!("`name` must be 1 to {MAX_NAME_CHARS} characters"),
            item,
        ));
    }
    Ok(name)
}

fn req_u64(obj: &Map<String, Value>, key: &str, item: Option<usize>) -> Result<u64, IndexError> {
    obj.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid(format!("`{key}` must be a non-negative integer"), item))
}

fn is_https_url(s: &str) -> bool {
    Url::parse(s).is_ok_and(|u| u.scheme() == "https" && u.host_str().is_some())
}

fn https_field(
    obj: &Map<String, Value>,
    key: &str,
    item: Option<usize>,
) -> Result<Option<String>, IndexError> {
    let v = str_field(obj, key, item)?;
    match &v {
        Some(s) if !is_https_url(s) => Err(invalid(format!("`{key}` must be an https URL"), item)),
        _ => Ok(v),
    }
}

/// True for a lowercase hyphenated UUID (8-4-4-4-12), nil included.
fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == b'-',
            _ => matches!(c, b'0'..=b'9' | b'a'..=b'f'),
        })
}

fn parse_category(s: &str) -> Option<Category> {
    match s {
        "Components" => Some(Category::Components),
        "Materials" => Some(Category::Materials),
        "Models" => Some(Category::Models),
        "Profiles" => Some(Category::Profiles),
        _ => None,
    }
}

/// One entry of `items`, validated against the 6.1 field table. `pos` is its
/// 0-based position, attached to every error.
fn parse_item(v: &Value, pos: usize) -> Result<IndexItem, IndexError> {
    let at = Some(pos);
    let Value::Object(obj) = v else {
        return Err(invalid("item is not an object", at));
    };
    let id = req_str(obj, "id", at)?;
    if !is_uuid(&id) {
        return Err(invalid("`id` must be a lowercase hyphenated UUID", at));
    }
    let path = req_str(obj, "path", at)?;
    check_rel_path_with_suffix(&path, ".hew").map_err(|e| invalid(format!("`path`: {e}"), at))?;
    let name = req_name(obj, at)?;
    let category = parse_category(&req_str(obj, "category", at)?).ok_or_else(|| {
        invalid(
            "`category` must be Components, Materials, Models or Profiles",
            at,
        )
    })?;
    let sha256 = req_str(obj, "sha256", at)?;
    if !is_sha256_hex(&sha256) {
        return Err(invalid("`sha256` must be 64 lowercase hex characters", at));
    }
    let bytes = req_u64(obj, "bytes", at)?;
    if !(1..=MAX_ITEM_BYTES).contains(&bytes) {
        return Err(invalid(
            format!("`bytes` must be 1 to {MAX_ITEM_BYTES}"),
            at,
        ));
    }
    let keywords = parse_keywords(obj, at)?;
    let thumbnail = str_field(obj, "thumbnail", at)?;
    if let Some(t) = &thumbnail {
        check_rel_path_with_suffix(t, ".png")
            .map_err(|e| invalid(format!("`thumbnail`: {e}"), at))?;
    }
    Ok(IndexItem {
        id,
        path,
        name,
        category,
        keywords,
        collection: str_field(obj, "collection", at)?,
        sha256,
        bytes,
        thumbnail,
        license: str_field(obj, "license", at)?,
    })
}

fn parse_keywords(obj: &Map<String, Value>, at: Option<usize>) -> Result<Vec<String>, IndexError> {
    let list = match obj.get("keywords") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(a)) => a,
        Some(_) => return Err(invalid("`keywords` must be an array", at)),
    };
    if list.len() > MAX_KEYWORDS {
        return Err(invalid(
            format!("`keywords` holds more than {MAX_KEYWORDS}"),
            at,
        ));
    }
    list.iter()
        .map(|k| match k {
            Value::String(s) if s.chars().count() <= MAX_KEYWORD_CHARS => Ok(s.clone()),
            Value::String(_) => Err(invalid(
                format!("a keyword is longer than {MAX_KEYWORD_CHARS} characters"),
                at,
            )),
            _ => Err(invalid("a keyword is not a string", at)),
        })
        .collect()
}

/// One `plugins` entry (section 9); `None` when it has the wrong shape.
fn parse_plugin(v: &Value) -> Option<PluginEntry> {
    let obj = v.as_object()?;
    let s = |k: &str| obj.get(k)?.as_str().map(str::to_owned);
    let n = |k: &str| obj.get(k)?.as_u64();
    Some(PluginEntry {
        id: s("id")?,
        version: s("version")?,
        package: s("package")?,
        sha256: s("sha256")?,
        bytes: n("bytes")?,
        hew: n("hew")?,
        signature: s("signature")?,
        key: s("key")?,
    })
}

/// Parses and validates a `hew.json` (rules of 6.1).
///
/// Size is checked before parsing; `hew` before everything else. Unknown
/// keys are ignored. Optional fields of the wrong JSON type are violations;
/// only `plugins` is lenient (a wrong-shaped entry, or a `plugins` that is
/// not an array, is dropped). Item positions are 0-based; a duplicate id or
/// path is reported at the later item.
pub fn parse_index(bytes: &[u8]) -> Result<SourceIndex, IndexError> {
    let obj = parse_document(bytes)?;
    let name = req_name(&obj, None)?;
    let description = str_field(&obj, "description", None)?;
    let author = str_field(&obj, "author", None)?;
    let homepage = https_field(&obj, "homepage", None)?;
    let license = str_field(&obj, "license", None)?;
    let license_url = https_field(&obj, "licenseUrl", None)?;
    let Some(Value::Array(raw_items)) = obj.get("items") else {
        return Err(invalid("`items` must be an array", None));
    };
    if raw_items.len() > MAX_ITEMS {
        return Err(invalid(format!("more than {MAX_ITEMS} items"), None));
    }
    let mut ids = BTreeSet::new();
    let mut paths = BTreeSet::new();
    let mut items = Vec::with_capacity(raw_items.len());
    for (pos, raw) in raw_items.iter().enumerate() {
        let item = parse_item(raw, pos)?;
        if !ids.insert(item.id.clone()) {
            return Err(invalid("duplicate `id`", Some(pos)));
        }
        if !paths.insert(item.path.clone()) {
            return Err(invalid("duplicate `path`", Some(pos)));
        }
        items.push(item);
    }
    let plugins = match obj.get("plugins") {
        Some(Value::Array(a)) => a.iter().filter_map(parse_plugin).collect(),
        _ => Vec::new(),
    };
    Ok(SourceIndex {
        hew: SUPPORTED_HEW,
        name,
        description,
        author,
        homepage,
        license,
        license_url,
        items,
        plugins,
    })
}

fn parse_kind(s: &str) -> Option<Kind> {
    match s {
        "github" => Some(Kind::Github),
        "gitlab" => Some(Kind::Gitlab),
        "forgejo" => Some(Kind::Forgejo),
        "web" => Some(Kind::Web),
        _ => None,
    }
}

/// A `source` string that must already be in canonical form: it parses to
/// exactly one reference whose canonical text is the string itself (Q14).
fn require_canonical(s: &str, at: Option<usize>) -> Result<(), IndexError> {
    match parse_source_ref(s) {
        Ok(v) if matches!(v.as_slice(), [only] if only.canonical == s) => Ok(()),
        _ => Err(invalid(
            format!("{s:?} is not a canonical source reference"),
            at,
        )),
    }
}

fn parse_catalog_source(v: &Value, pos: usize) -> Result<CatalogSource, IndexError> {
    let at = Some(pos);
    let Value::Object(obj) = v else {
        return Err(invalid("source entry is not an object", at));
    };
    let source = req_str(obj, "source", at)?;
    require_canonical(&source, at)?;
    let kind = parse_kind(&req_str(obj, "kind", at)?)
        .ok_or_else(|| invalid("`kind` must be github, gitlab, forgejo or web", at))?;
    let commit = str_field(obj, "commit", at)?;
    if commit.as_deref().is_some_and(|c| !is_commit_hex(c)) {
        return Err(invalid(
            "`commit` must be 40 or 64 lowercase hex characters",
            at,
        ));
    }
    if kind == Kind::Web && commit.is_some() {
        return Err(invalid("a web source has no `commit`", at));
    }
    let snapshot = req_str(obj, "snapshot", at)?;
    check_rel_path_with_suffix(&snapshot, ".json")
        .map_err(|e| invalid(format!("`snapshot`: {e}"), at))?;
    let Some(Value::Object(cats)) = obj.get("categories") else {
        return Err(invalid("`categories` must be an object", at));
    };
    let categories = cats
        .iter()
        .map(|(k, n)| {
            if parse_category(k).is_none() {
                return Err(invalid(
                    "a `categories` key must be Components, Materials, Models or Profiles",
                    at,
                ));
            }
            n.as_u64()
                .map(|n| (k.clone(), n))
                .ok_or_else(|| invalid("a `categories` count is not a non-negative integer", at))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let Some(Value::Bool(stale)) = obj.get("stale") else {
        return Err(invalid("`stale` must be a boolean", at));
    };
    Ok(CatalogSource {
        source,
        kind,
        commit,
        name: req_str(obj, "name", at)?,
        description: str_field(obj, "description", at)?,
        author: str_field(obj, "author", at)?,
        homepage: https_field(obj, "homepage", at)?,
        license: str_field(obj, "license", at)?,
        license_url: https_field(obj, "licenseUrl", at)?,
        items: req_u64(obj, "items", at)?,
        bytes: req_u64(obj, "bytes", at)?,
        categories,
        stale: *stale,
        snapshot,
    })
}

fn parse_removed(v: &Value, pos: usize) -> Result<RemovedEntry, IndexError> {
    let at = Some(pos);
    let Value::Object(obj) = v else {
        return Err(invalid("removed entry is not an object", at));
    };
    let source = req_str(obj, "source", at)?;
    require_canonical(&source, at)?;
    let severity = match req_str(obj, "severity", at)?.as_str() {
        "removed" => Severity::Removed,
        "critical" => Severity::Critical,
        _ => return Err(invalid("`severity` must be removed or critical", at)),
    };
    Ok(RemovedEntry {
        source,
        severity,
        reason: req_str(obj, "reason", at)?,
        link: https_field(obj, "link", at)?,
    })
}

/// Parses a `catalog.json` (6.3), with the same size, depth and `hew` rules
/// as [`parse_index`]. Every `source` must be a canonical reference (Q14),
/// every `snapshot` a safe relative `.json` path the app follows without
/// deriving it, every `commit` 40 or 64 lowercase hex (and absent on a `web`
/// source), every `categories` key one of the four categories, and every
/// `link`, `homepage` and `licenseUrl` an `https` URL (Q15).
pub fn parse_catalog(bytes: &[u8]) -> Result<Catalog, IndexError> {
    let obj = parse_document(bytes)?;
    let built = req_str(&obj, "built", None)?;
    let Some(Value::Array(raw_sources)) = obj.get("sources") else {
        return Err(invalid("`sources` must be an array", None));
    };
    let Some(Value::Array(raw_removed)) = obj.get("removed") else {
        return Err(invalid("`removed` must be an array", None));
    };
    Ok(Catalog {
        hew: SUPPORTED_HEW,
        built,
        sources: raw_sources
            .iter()
            .enumerate()
            .map(|(i, v)| parse_catalog_source(v, i))
            .collect::<Result<_, _>>()?,
        removed: raw_removed
            .iter()
            .enumerate()
            .map(|(i, v)| parse_removed(v, i))
            .collect::<Result<_, _>>()?,
    })
}
