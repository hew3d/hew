//! The Preview wall, Rust half (docs/dev/DEVELOPMENT.md "Preview features").
//!
//! Unfinished features live on `main` behind a build-time flag. A build
//! compiled with `HEW_PREVIEW=1` (`cfg(hew_preview)`, set by this crate's
//! `build.rs`) runs them; any other build is a production build, and in a
//! production build no API surface reaches them: their commands are left
//! out of the registry (so out of `hew.meta.capabilities`, the MCP tool
//! surface and every listing built from it), and a call that names one
//! anyway refuses `preview_only`, naming the feature. Nothing reads the
//! flag at run time, so a production build has no switch to flip.
//!
//! What is gated comes from ONE file, `preview-features.json` at the repo
//! root, shared with the app (which gates its tools, menus, palette, dock
//! and keymap from the same entries). This module is its only reader on
//! the Rust side. Each entry's `apiCommands` are gated whole; its
//! `behaviors` gate the API parameters bound to them in [`GATED_PARAMS`],
//! the values of live fields bound to them in [`GATED_VALUES`], and the
//! handler-written refusal copy in [`GATED_API_COPY`].
//! Graduating a feature is deleting its entry.
//!
//! The checks run at the dispatcher's one execution choke point,
//! [`crate::commands::resolve`], which every first-party command passes
//! through on every path (a plain request, a transaction, a dry run, a
//! forward's nested call), and at the two places that answer a name the
//! registry does not hold (`Connection::dispatch` and the transaction's
//! static pass), so a gated name refuses typed rather than as an unknown
//! method.

use crate::refusal::Refusal;
use crate::registry::CommandDecl;
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// Whether this is a preview build (`HEW_PREVIEW=1` at build time).
pub const PREVIEW_BUILD: bool = cfg!(hew_preview);

/// The refusal name a production build answers for a preview command.
pub const PREVIEW_ONLY: &str = "preview_only";

/// The registry, embedded at compile time.
const REGISTRY_JSON: &str = include_str!("../../../preview-features.json");

/// API parameters that belong to a preview behavior rather than to a whole
/// command: `(behavior id, command, parameters)`. The registry names the
/// behavior; this table says what that behavior is on the API. A row whose
/// behavior is in no registry entry gates nothing, and the wall test fails
/// on it, so a typo or a graduated behavior cannot linger here.
pub const GATED_PARAMS: &[(&str, &str, &[&str])] = &[
    (
        "follow-me-twist",
        "hew.solid.follow_me",
        &["twist", "end_scale", "stations"],
    ),
    ("fillet-profiles", "hew.library.save", &["profile_chain"]),
    ("export-cages", "hew.doc.export", &["cages"]),
];

/// Values of a live command's field that belong to a preview behavior:
/// `(behavior id, command, field, values)`. The field is a parameter, a
/// field of the command's result, or both (`category` on the Library
/// commands). In a production build the values leave every enum of a
/// property named `field` in the command's params and result schemas; a
/// request that sets the parameter to one refuses `preview_only` (checked
/// with [`GATED_PARAMS`], on every dispatch path); and the handler refuses
/// or leaves out anything it meets that carries one
/// ([`value_allowed`], [`value_gated`]). Like [`GATED_PARAMS`], a row whose
/// behavior is in no registry entry gates nothing and fails the wall test,
/// and so does a row naming a field or value the command's schemas do not
/// declare.
pub const GATED_VALUES: &[(&str, &str, &str, &[&str])] = &[
    (
        "fillet-profiles",
        "hew.library.save",
        "category",
        &["profile"],
    ),
    (
        "fillet-profiles",
        "hew.library.list",
        "category",
        &["profile"],
    ),
    (
        "fillet-profiles",
        "hew.library.describe",
        "category",
        &["profile"],
    ),
    (
        "fillet-profiles",
        "hew.library.update_meta",
        "category",
        &["profile"],
    ),
];

/// Refusal copy an API handler writes itself, for a refusal that a
/// production build can reach but whose wording points at a Preview
/// feature: `(behavior id, refusal name, preview copy, production copy)`.
/// Handlers take their copy from [`api_copy`] rather than writing it
/// inline, so the wall test can hold every handler string to the rule
/// (`preview_wall.rs` scans the handler sources for a feature's name). A
/// row whose behavior is in no entry falls back to the preview copy in
/// every build, and fails the wall test.
pub const GATED_API_COPY: &[(&str, &str, &str, &str)] = &[(
    "fillet-profiles",
    "profile_not_insertable",
    "Profiles are used by the Fillet tool, not inserted. Pick it in the Fillet tool's profile chip.",
    "This Library item is a profile, so it can't be placed in the model.",
)];

/// The copy for API refusal `name` from [`GATED_API_COPY`]: the production
/// copy in a production build while the row's behavior is gated, the
/// preview copy otherwise. Empty for a name with no row (a unit test
/// holds every name a handler asks for to a row).
pub fn api_copy(name: &str) -> &'static str {
    GATED_API_COPY
        .iter()
        .find(|(_, n, _, _)| *n == name)
        .map_or("", |&(behavior, _, preview, production)| {
            if !PREVIEW_BUILD && feature_of_behavior(behavior).is_some() {
                production
            } else {
                preview
            }
        })
}

/// Refusal copy that points at a Preview feature, with what a production
/// build says instead: `(behavior id, kernel error code, production copy)`.
/// The production copy is the app's own flag-off wording
/// (`app/src/kernelErrors.ts`), so the API and the app say the same thing.
/// Like [`GATED_PARAMS`], a row whose behavior is in no entry is inert and
/// fails the wall test.
pub const GATED_COPY: &[(&str, &str, &str)] = &[(
    "health-remove-degenerate",
    "DegenerateInput",
    "One of these objects has a face with no area, so these objects can't be combined.",
)];

/// The production build's copy for kernel error `code`, when the generated
/// copy would point at a Preview feature; `None` otherwise, and always in a
/// preview build.
pub fn production_copy(code: &str) -> Option<&'static str> {
    if PREVIEW_BUILD {
        return None;
    }
    GATED_COPY
        .iter()
        .find(|(behavior, c, _)| *c == code && feature_of_behavior(behavior).is_some())
        .map(|(_, _, copy)| *copy)
}

/// One registry entry. Every field the file carries is declared, so a key
/// this reader does not know (a misspelled `apiCommand`, say) fails to
/// parse rather than silently gating nothing.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Feature {
    pub id: String,
    pub name: String,
    pub state: String,
    pub owner: String,
    pub tools: Vec<String>,
    pub actions: Vec<String>,
    pub dock_chips: Vec<String>,
    pub api_commands: Vec<String>,
    pub behaviors: Vec<String>,
    /// The switch's state before the user touches it (Settings > Preview).
    /// Absent means on. A feature that runs other people's code is off
    /// until asked for.
    #[serde(default = "default_on")]
    pub default_on: bool,
    /// The entry points of a feature that loads and runs code. Absent for
    /// every feature that does not.
    #[serde(default)]
    pub loader: Option<Loader>,
}

fn default_on() -> bool {
    true
}

/// What a code-loading feature gates beyond the five lookup kinds: entry
/// points that sit in no table the wall walks, so each list is held to its
/// own rule (docs/dev/DEVELOPMENT.md "Preview features", "Code loading").
/// Every list may be empty. In a production build none of these exists:
/// the code behind each is compiled out, not refused.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Loader {
    /// Window roots the app selects by location hash (`app/src/main.tsx`).
    pub roots: Vec<String>,
    /// Settings panes.
    pub panes: Vec<String>,
    /// Desktop shell commands, compiled only under `cfg(hew_preview)`.
    pub shell_commands: Vec<String>,
    /// Exports of the WASM module, compiled only under `cfg(hew_preview)`.
    pub kernel_exports: Vec<String>,
    /// `hew-cli` subcommands, compiled only under `cfg(hew_preview)`.
    pub cli_commands: Vec<String>,
    /// Folders, databases and settings-key prefixes a production build
    /// never reads, writes or deletes.
    pub storage: Vec<String>,
    /// Strings that must not occur in any production artifact
    /// (`scripts/check-production-artifacts.mjs`).
    pub markers: Vec<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryFile {
    features: Vec<Feature>,
}

/// Every registry entry, in file order. The file is embedded at compile
/// time and parsed once; a file that does not parse stops the program at
/// the first check rather than letting anything through (the wall tests
/// parse it in both configurations, so such a file never passes the gate).
pub fn features() -> &'static [Feature] {
    static PARSED: OnceLock<Vec<Feature>> = OnceLock::new();
    PARSED.get_or_init(|| {
        serde_json::from_str::<RegistryFile>(REGISTRY_JSON)
            .expect("preview-features.json does not parse")
            .features
    })
}

/// The entry whose `apiCommands` names `command`.
pub fn feature_of_command(command: &str) -> Option<&'static Feature> {
    features()
        .iter()
        .find(|f| f.api_commands.iter().any(|c| c == command))
}

/// The entry whose loader lists `name` under the kind `pick` selects.
pub fn feature_of_loader_name(
    pick: fn(&Loader) -> &Vec<String>,
    name: &str,
) -> Option<&'static Feature> {
    features().iter().find(|f| {
        f.loader
            .as_ref()
            .is_some_and(|l| pick(l).iter().any(|n| n == name))
    })
}

/// The entry whose `behaviors` names `behavior`.
pub fn feature_of_behavior(behavior: &str) -> Option<&'static Feature> {
    features()
        .iter()
        .find(|f| f.behaviors.iter().any(|b| b == behavior))
}

/// The preview parameters of `command`, each group with the entry it
/// belongs to. Empty for a command with none, and for rows whose behavior
/// has graduated.
pub fn gated_params(command: &str) -> Vec<(&'static Feature, &'static [&'static str])> {
    GATED_PARAMS
        .iter()
        .filter(|(_, c, _)| *c == command)
        .filter_map(|(behavior, _, params)| feature_of_behavior(behavior).map(|f| (f, *params)))
        .collect()
}

/// The `preview_only` refusal for a command of `feature`.
pub fn preview_only(feature: &Feature) -> Refusal {
    Refusal::api(
        PREVIEW_ONLY,
        &format!(
            "This command belongs to a preview feature ({}) and is not part of this build.",
            feature.name
        ),
    )
    .with_detail(serde_json::json!({ "feature": feature.id, "feature_name": feature.name }))
}

/// `Ok` unless this is a production build and `command` belongs to a
/// preview feature, which refuses `preview_only`.
pub fn command_allowed(command: &str) -> Result<(), Refusal> {
    if PREVIEW_BUILD {
        return Ok(());
    }
    match feature_of_command(command) {
        Some(feature) => Err(preview_only(feature)),
        None => Ok(()),
    }
}

/// The preview values of `command`'s fields: `(feature, field, values)`.
/// Empty for a command with none, and for rows whose behavior has
/// graduated.
pub fn gated_values(
    command: &str,
) -> Vec<(&'static Feature, &'static str, &'static [&'static str])> {
    GATED_VALUES
        .iter()
        .filter(|(_, c, _, _)| *c == command)
        .filter_map(|(behavior, _, field, values)| {
            feature_of_behavior(behavior).map(|f| (f, *field, *values))
        })
        .collect()
}

/// The feature `value` of `command`'s `field` belongs to, in a production
/// build; `None` in a preview build and for any value outside
/// [`GATED_VALUES`].
pub fn value_gated(command: &str, field: &str, value: &str) -> Option<&'static Feature> {
    if PREVIEW_BUILD {
        return None;
    }
    gated_values(command)
        .into_iter()
        .find(|(_, f, values)| *f == field && values.contains(&value))
        .map(|(feature, _, _)| feature)
}

/// `Ok` unless [`value_gated`]: the `preview_only` refusal a handler gives
/// for something it meets (an existing Library item, say) whose `field`
/// holds a preview value.
pub fn value_allowed(command: &str, field: &str, value: &str) -> Result<(), Refusal> {
    match value_gated(command, field, value) {
        None => Ok(()),
        Some(feature) => Err(value_refusal(feature, field, value)),
    }
}

fn value_refusal(feature: &Feature, field: &str, value: &str) -> Refusal {
    Refusal::api(
        PREVIEW_ONLY,
        &format!(
            "A {field} of {value:?} belongs to a preview feature ({}) and is not part of this build.",
            feature.name
        ),
    )
    .with_detail(serde_json::json!({
        "feature": feature.id,
        "feature_name": feature.name,
        "params": [field],
        "value": value,
    }))
}

/// `Ok` unless this is a production build and `params` sets a parameter of
/// `command` that belongs to a preview behavior, which refuses
/// `preview_only`. A parameter counts as set when its key is present at
/// all, `null` included.
pub fn params_allowed(command: &str, params: &serde_json::Value) -> Result<(), Refusal> {
    if PREVIEW_BUILD {
        return Ok(());
    }
    let Some(object) = params.as_object() else {
        return Ok(());
    };
    for (feature, names) in gated_params(command) {
        let set: Vec<&str> = names
            .iter()
            .copied()
            .filter(|p| object.contains_key(*p))
            .collect();
        if !set.is_empty() {
            return Err(Refusal::api(
                PREVIEW_ONLY,
                &format!(
                    "The {} parameter of this command belongs to a preview feature ({}) and is not part of this build.",
                    set.join(", "),
                    feature.name
                ),
            )
            .with_detail(serde_json::json!({
                "feature": feature.id,
                "feature_name": feature.name,
                "params": set,
            })));
        }
    }
    for (feature, field, values) in gated_values(command) {
        if let Some(value) = object
            .get(field)
            .and_then(serde_json::Value::as_str)
            .filter(|v| values.contains(v))
        {
            return Err(value_refusal(feature, field, value));
        }
    }
    Ok(())
}

/// [`command_allowed`], then [`params_allowed`]: what the dispatcher asks
/// before it runs any first-party command.
pub fn check(command: &str, params: &serde_json::Value) -> Result<(), Refusal> {
    command_allowed(command)?;
    params_allowed(command, params)
}

/// The production-build view of a full inventory: every preview command
/// removed, every preview parameter removed from its command's params
/// schema, and every preview value removed from its field's enums in its
/// command's params and result schemas, with `preview_only` added to each
/// such command's refusals. Called
/// only for a production build ([`crate::registry::Registry::protocol_1`]).
pub(crate) fn wall_off(commands: &mut BTreeMap<String, CommandDecl>) {
    commands.retain(|name, _| feature_of_command(name).is_none());
    for &(_, command, _) in GATED_PARAMS {
        let groups = gated_params(command);
        if groups.is_empty() {
            continue;
        }
        let Some(decl) = commands.get_mut(command) else {
            continue;
        };
        for (_, names) in groups {
            if let Some(properties) = decl
                .params_schema
                .get_mut("properties")
                .and_then(serde_json::Value::as_object_mut)
            {
                for name in names {
                    properties.remove(*name);
                }
            }
            if let Some(required) = decl
                .params_schema
                .get_mut("required")
                .and_then(serde_json::Value::as_array_mut)
            {
                required.retain(|r| r.as_str().is_none_or(|r| !names.contains(&r)));
            }
        }
        if !decl.refusals.iter().any(|r| r == PREVIEW_ONLY) {
            decl.refusals.push(PREVIEW_ONLY.to_string());
        }
    }
    for &(_, command, _, _) in GATED_VALUES {
        let rows = gated_values(command);
        if rows.is_empty() {
            continue;
        }
        let Some(decl) = commands.get_mut(command) else {
            continue;
        };
        for (_, field, values) in rows {
            strip_enum_values(&mut decl.params_schema, field, values);
            strip_enum_values(&mut decl.result_schema, field, values);
        }
        if !decl.refusals.iter().any(|r| r == PREVIEW_ONLY) {
            decl.refusals.push(PREVIEW_ONLY.to_string());
        }
    }
}

/// Removes `values` from the `enum` of every property named `field`
/// anywhere in `schema`.
pub(crate) fn strip_enum_values(schema: &mut serde_json::Value, field: &str, values: &[&str]) {
    match schema {
        serde_json::Value::Object(map) => {
            if let Some(enumeration) = map
                .get_mut("properties")
                .and_then(|p| p.get_mut(field))
                .and_then(|f| f.get_mut("enum"))
                .and_then(serde_json::Value::as_array_mut)
            {
                enumeration.retain(|v| v.as_str().is_none_or(|v| !values.contains(&v)));
            }
            for child in map.values_mut() {
                strip_enum_values(child, field, values);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                strip_enum_values(child, field, values);
            }
        }
        _ => {}
    }
}

/// Every enum of a property named `field` anywhere in `schema`, for the
/// wall test's check that a [`GATED_VALUES`] row names real values.
pub fn enums_of_field(schema: &serde_json::Value, field: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    fn walk(schema: &serde_json::Value, field: &str, out: &mut Vec<Vec<String>>) {
        match schema {
            serde_json::Value::Object(map) => {
                if let Some(enumeration) = map
                    .get("properties")
                    .and_then(|p| p.get(field))
                    .and_then(|f| f.get("enum"))
                    .and_then(serde_json::Value::as_array)
                {
                    out.push(
                        enumeration
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect(),
                    );
                }
                for child in map.values() {
                    walk(child, field, out);
                }
            }
            serde_json::Value::Array(items) => {
                for child in items {
                    walk(child, field, out);
                }
            }
            _ => {}
        }
    }
    walk(schema, field, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_parses_and_ids_are_unique() {
        let all = features();
        assert!(!all.is_empty());
        let mut ids: Vec<&str> = all.iter().map(|f| f.id.as_str()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(
            ids.len(),
            before,
            "duplicate feature id in preview-features.json"
        );
    }

    #[test]
    fn no_command_or_behavior_belongs_to_two_entries() {
        let mut commands: Vec<&str> = Vec::new();
        let mut behaviors: Vec<&str> = Vec::new();
        for f in features() {
            commands.extend(f.api_commands.iter().map(String::as_str));
            behaviors.extend(f.behaviors.iter().map(String::as_str));
        }
        for list in [&mut commands, &mut behaviors] {
            list.sort_unstable();
            let before = list.len();
            list.dedup();
            assert_eq!(list.len(), before, "a name is listed by two entries");
        }
    }

    #[test]
    fn no_loader_name_is_listed_twice_in_its_kind() {
        type Pick = fn(&Loader) -> &Vec<String>;
        let picks: [(&str, Pick); 6] = [
            ("roots", |l| &l.roots),
            ("panes", |l| &l.panes),
            ("shellCommands", |l| &l.shell_commands),
            ("kernelExports", |l| &l.kernel_exports),
            ("cliCommands", |l| &l.cli_commands),
            ("storage", |l| &l.storage),
        ];
        for (kind, pick) in picks {
            let mut names: Vec<&str> = features()
                .iter()
                .filter_map(|f| f.loader.as_ref())
                .flat_map(|l| pick(l).iter().map(String::as_str))
                .collect();
            assert!(
                names.iter().all(|n| !n.trim().is_empty()),
                "an empty {kind} name"
            );
            names.sort_unstable();
            let before = names.len();
            names.dedup();
            assert_eq!(names.len(), before, "a {kind} name is listed twice");
        }
    }

    #[test]
    fn a_feature_that_loads_code_is_off_until_asked_for() {
        for f in features() {
            if f.loader.is_some() {
                assert!(
                    !f.default_on,
                    "{} loads code, so its switch must default to off (defaultOn: false)",
                    f.id
                );
            }
        }
    }

    #[test]
    fn a_marker_is_long_enough_to_mean_something() {
        for f in features() {
            for m in f.loader.iter().flat_map(|l| &l.markers) {
                assert!(
                    m.len() >= 8,
                    "marker {m:?} of {} is too short to scan for",
                    f.id
                );
            }
        }
    }

    #[test]
    fn the_flag_is_what_the_build_says() {
        assert_eq!(PREVIEW_BUILD, cfg!(hew_preview));
    }

    #[test]
    fn a_command_outside_the_registry_is_always_allowed() {
        assert!(command_allowed("hew.sketch.draw_rect").is_ok());
        assert!(params_allowed("hew.sketch.draw_rect", &serde_json::json!({ "twist": 1 })).is_ok());
    }
}
