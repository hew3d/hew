//! What the webview asks the save panel for and what comes back, and the
//! two rules every platform's panel shares. Free of Tauri and of any one
//! platform's toolkit, so each platform's panel compiles on its own.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One entry of the panel's format list.
#[derive(Debug, Clone, Deserialize)]
pub struct PanelFormat {
    /// What the webview calls it (`"skp2017"`), handed back in the result.
    pub id: String,
    /// What the panel shows (`"SketchUp 2017 (.skp)"`).
    pub label: String,
    /// The file extension, without the dot.
    pub extension: String,
    /// The format's own options, shown while it is the chosen one.
    #[serde(default)]
    pub options: Vec<PanelOption>,
}

/// One option control: a choice list or a checkbox, with its current value.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PanelOption {
    Select {
        id: String,
        label: String,
        choices: Vec<PanelChoice>,
        value: String,
    },
    Check {
        id: String,
        label: String,
        value: bool,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct PanelChoice {
    pub value: String,
    pub label: String,
}

/// What the user confirmed.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PanelResult {
    pub path: String,
    pub format: String,
    pub options: BTreeMap<String, serde_json::Value>,
}

/// What the webview asked for.
#[derive(Debug, Clone)]
pub struct PanelRequest {
    pub title: String,
    pub default_name: String,
    pub formats: Vec<PanelFormat>,
    /// The index of the format chosen when the panel opens.
    pub initial: usize,
}

impl PanelOption {
    pub fn id(&self) -> &str {
        match self {
            PanelOption::Select { id, .. } | PanelOption::Check { id, .. } => id,
        }
    }

    pub fn value(&self) -> serde_json::Value {
        match self {
            PanelOption::Select { value, .. } => serde_json::Value::String(value.clone()),
            PanelOption::Check { value, .. } => serde_json::Value::Bool(*value),
        }
    }
}

/// The result for `format` at `path`, with its options' current values.
pub fn result_for(path: String, format: &PanelFormat) -> PanelResult {
    PanelResult {
        path,
        format: format.id.clone(),
        options: format
            .options
            .iter()
            .map(|o| (o.id().to_string(), o.value()))
            .collect(),
    }
}

/// `name` with the extension of the format left (`from`) swapped for the
/// one chosen (`to`); a name ending in neither keeps what the user typed.
pub fn rename_for(name: &str, from: &str, to: &str) -> String {
    let dotted = format!(".{from}");
    if name.len() > dotted.len() && name.to_lowercase().ends_with(&dotted.to_lowercase()) {
        format!("{}.{to}", &name[..name.len() - dotted.len()])
    } else {
        name.to_string()
    }
}

/// `path` with the chosen format's `extension`: as given when it already
/// ends in it (any case), else with it appended — a name typed with none,
/// or with another format's, still gets the file the format was written
/// as (`Model` → `Model.skp`, `Model.v2` → `Model.v2.skp`). A name that
/// ends in the extension twice (`Model.hew.hew`, which the panel makes when
/// its prefilled name already carried the extension) collapses to one.
pub fn with_extension(path: &str, extension: &str) -> String {
    let path = collapse_doubled_extension(path, extension);
    let path = path.as_str();
    let has = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(extension));
    if has {
        path.to_string()
    } else {
        format!("{path}.{extension}")
    }
}

/// Why a save at `renamed` must not go ahead: the extension rule changed
/// the name the user `typed`, and a file already sits at the new name. The
/// platform panel asked about replacing `typed`, never `renamed`, so the
/// user is told instead of overwritten. `None` when the name is as typed
/// or the new name is free.
pub fn unasked_overwrite(
    typed: &str,
    renamed: &str,
    exists: impl Fn(&str) -> bool,
) -> Option<String> {
    if renamed == typed || !exists(renamed) {
        return None;
    }
    let name = std::path::Path::new(renamed)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| renamed.to_string());
    Some(format!(
        "{name} already exists. Choose another name, or delete it first."
    ))
}

/// `path` with a doubled trailing `.<ext>.<ext>` (any case) reduced to one.
fn collapse_doubled_extension(path: &str, extension: &str) -> String {
    let dot_ext = format!(".{extension}");
    let mut out = path;
    while let Some(stem) = strip_suffix_ignore_case(out, &dot_ext) {
        // Only a repeat: what remains must itself end in the extension, and
        // must keep a name in front of it (`.hew` alone is just a dotfile).
        match strip_suffix_ignore_case(stem, &dot_ext) {
            Some(name) if !name.is_empty() && !name.ends_with(['/', '\\']) => out = stem,
            _ => break,
        }
    }
    out.to_string()
}

fn strip_suffix_ignore_case<'a>(s: &'a str, suffix: &str) -> Option<&'a str> {
    let cut = s.len().checked_sub(suffix.len())?;
    if !s.is_char_boundary(cut) || !s[cut..].eq_ignore_ascii_case(suffix) {
        return None;
    }
    Some(&s[..cut])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_format_change_swaps_only_the_extension_it_gave() {
        assert_eq!(rename_for("Model.hew", "hew", "skp"), "Model.skp");
        assert_eq!(rename_for("Model.HEW", "hew", "skp"), "Model.skp");
        assert_eq!(rename_for("Model.skp", "skp", "skp"), "Model.skp");
        // A name the user typed without it, or with another, is theirs.
        assert_eq!(rename_for("Model", "hew", "skp"), "Model");
        assert_eq!(rename_for("Model.v2", "hew", "skp"), "Model.v2");
        assert_eq!(rename_for(".hew", "hew", "skp"), ".hew");
    }

    #[test]
    fn a_saved_path_always_ends_in_its_formats_extension() {
        assert_eq!(with_extension("/d/Model.skp", "skp"), "/d/Model.skp");
        assert_eq!(with_extension("/d/Model.SKP", "skp"), "/d/Model.SKP");
        assert_eq!(with_extension("/d/Model", "skp"), "/d/Model.skp");
        assert_eq!(with_extension("/d/Model.v2", "skp"), "/d/Model.v2.skp");
        assert_eq!(with_extension("/d/x.glb", "stl"), "/d/x.glb.stl");
    }

    #[test]
    fn a_doubled_extension_collapses_to_one() {
        assert_eq!(with_extension("/d/Model.hew.hew", "hew"), "/d/Model.hew");
        assert_eq!(with_extension("/d/Model.HEW.hew", "hew"), "/d/Model.HEW");
        assert_eq!(
            with_extension("/d/Model.hew.hew.hew", "hew"),
            "/d/Model.hew"
        );
        assert_eq!(
            with_extension("C:\\d\\Model.skp.SKP", "skp"),
            "C:\\d\\Model.skp"
        );
        // Another format's extension in front is a name, not a repeat.
        assert_eq!(
            with_extension("/d/Model.hew.skp", "skp"),
            "/d/Model.hew.skp"
        );
        assert_eq!(
            with_extension("/d/Model.skp.hew", "hew"),
            "/d/Model.skp.hew"
        );
        assert_eq!(with_extension("/d/Model.hew", "skp"), "/d/Model.hew.skp");
        // A bare dotfile is a name with no extension (as before), so it
        // gets the extension once, and that result is left alone.
        assert_eq!(with_extension("/d/.hew", "hew"), "/d/.hew.hew");
        assert_eq!(with_extension("/d/.hew.hew", "hew"), "/d/.hew.hew");
    }

    #[test]
    fn the_collapse_keeps_a_cut_on_a_char_boundary() {
        // U+212A lowercases to ASCII `k` but is three bytes long: the cut
        // must not be placed by the lowercased length.
        assert_eq!(
            with_extension("/d/Model.s\u{212A}p.s\u{212A}p", "skp"),
            "/d/Model.s\u{212A}p.s\u{212A}p.skp"
        );
    }

    #[test]
    fn a_name_the_rule_changed_never_overwrites_unasked() {
        let taken = |p: &str| p == "/d/Model.hew";
        // The panel asked about `Model.hew.hew`; `Model.hew` exists: refuse.
        assert_eq!(
            unasked_overwrite("/d/Model.hew.hew", "/d/Model.hew", taken),
            Some("Model.hew already exists. Choose another name, or delete it first.".into())
        );
        // The name is as typed: the panel already asked.
        assert_eq!(
            unasked_overwrite("/d/Model.hew", "/d/Model.hew", taken),
            None
        );
        // The new name is free.
        assert_eq!(
            unasked_overwrite("/d/Other.hew.hew", "/d/Other.hew", taken),
            None
        );
        assert_eq!(unasked_overwrite("/d/Other", "/d/Other.hew", taken), None);
    }

    #[test]
    fn a_request_reads_the_webviews_formats_and_a_result_carries_every_option() {
        let formats: Vec<PanelFormat> = serde_json::from_value(serde_json::json!([
            { "id": "glb", "label": "glTF binary (.glb)", "extension": "glb" },
            { "id": "stl", "label": "STL (.stl)", "extension": "stl", "options": [
                { "kind": "select", "id": "segments", "label": "Curve resolution",
                  "choices": [{ "value": "0", "label": "As drawn" }, { "value": "96", "label": "Fine" }],
                  "value": "96" },
                { "kind": "check", "id": "solid_only", "label": "Solids only", "value": true }
            ]}
        ]))
        .unwrap();
        assert!(formats[0].options.is_empty());
        let r = result_for("/tmp/a.stl".into(), &formats[1]);
        assert_eq!(r.format, "stl");
        assert_eq!(r.options["segments"], serde_json::json!("96"));
        assert_eq!(r.options["solid_only"], serde_json::json!(true));
    }
}
