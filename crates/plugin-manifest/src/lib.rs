//! The plugin manifest, `hew-plugin.json` (docs/agents/PLUGINS.md
//! "Manifest"): its typed shape and the one validating parser every host
//! shares. Pure: no I/O, no clock, no network.
//!
//! [`parse`] is the only supported way to obtain a [`Manifest`]. It runs
//! the validation steps in a fixed order, so a file with several defects
//! always reports the same one; the order is part of the contract and is
//! documented beside each step below and in PLUGINS.md.

use std::collections::{BTreeMap, BTreeSet};

/// The manifest format version this parser reads.
pub const MANIFEST_VERSION: u32 = 1;
/// Longest plugin id, in bytes (the id grammar is ASCII).
pub const MAX_ID_LEN: usize = 128;
/// Longest display name, in characters.
pub const MAX_NAME_LEN: usize = 64;
/// Most commands one manifest may contribute.
pub const MAX_COMMANDS: usize = 256;
/// Largest `agent.notes` file, in bytes (checked by the installer, which
/// has the file; the parser sees only its path).
pub const MAX_AGENT_NOTES_BYTES: usize = 4096;
/// The bus protocol major plugins may declare in `api`.
pub const SUPPORTED_API: u32 = 1;

/// A parsed, validated `hew-plugin.json`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub manifest: u32,
    pub id: String,
    pub name: LocalizableString,
    pub version: semver::Version,
    #[serde(default)]
    pub description: Option<LocalizableString>,
    #[serde(default)]
    pub author: Option<Author>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub hew: Option<semver::VersionReq>,
    pub api: u32,
    /// Required. A missing key reads as empty, so it is refused as
    /// `manifest_no_entry` at step 6 rather than as a malformed file.
    #[serde(default)]
    pub main: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub requires: Vec<Requirement>,
    #[serde(default)]
    pub permissions: Vec<PermissionName>,
    #[serde(default, rename = "optionalPermissions")]
    pub optional_permissions: Vec<PermissionName>,
    #[serde(default = "Activation::default_set")]
    pub activation: Vec<Activation>,
    #[serde(default, rename = "activationReason")]
    pub activation_reason: Option<LocalizableString>,
    #[serde(default)]
    pub data: DataSpec,
    #[serde(default)]
    pub locales: Option<Locales>,
    #[serde(default)]
    pub agent: Option<AgentSpec>,
    #[serde(default)]
    pub contributes: Contributes,
}

/// A user-visible string: plain, per-locale, or a catalog key.
///
/// Deserialized by hand rather than `untagged`: an untagged enum would read
/// `{ "$t": "key" }` as a one-locale map named `$t`, and would accept an
/// empty locale map as a string with no text in any language.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(untagged)]
pub enum LocalizableString {
    Plain(String),
    /// `{ "en": "...", "ja": "..." }`
    ByLocale(BTreeMap<String, String>),
    /// `{ "$t": "key" }`
    Key {
        #[serde(rename = "$t")]
        key: String,
    },
}

impl<'de> serde::Deserialize<'de> for LocalizableString {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let value = serde_json::Value::deserialize(d)?;
        match value {
            serde_json::Value::String(s) => Ok(LocalizableString::Plain(s)),
            serde_json::Value::Object(map) => {
                if map.contains_key("$t") {
                    if map.len() != 1 {
                        return Err(D::Error::custom(
                            "a `$t` localizable string holds only the `$t` key",
                        ));
                    }
                    return match &map["$t"] {
                        serde_json::Value::String(key) => {
                            Ok(LocalizableString::Key { key: key.clone() })
                        }
                        _ => Err(D::Error::custom("`$t` must be a string")),
                    };
                }
                if map.is_empty() {
                    return Err(D::Error::custom(
                        "a localizable string object names at least one locale",
                    ));
                }
                let mut out = BTreeMap::new();
                for (locale, text) in map {
                    match text {
                        serde_json::Value::String(t) => {
                            out.insert(locale, t);
                        }
                        _ => {
                            return Err(D::Error::custom(format!(
                                "the `{locale}` text of a localizable string must be a string"
                            )));
                        }
                    }
                }
                Ok(LocalizableString::ByLocale(out))
            }
            _ => Err(D::Error::custom(
                "expected a string, a locale map, or a `$t` key object",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Author {
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requirement {
    pub id: String,
    pub range: semver::VersionReq,
}

/// A permission string from the manifest, validated by
/// [`PermissionName::parse`].
///
/// Deserialization keeps the raw string and does not validate it, so that
/// [`parse`] reports a bad permission as `unknown_permission` (step 7) and
/// not as a malformed file (step 1). A manifest obtained through [`parse`]
/// holds only valid names.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(from = "String", into = "String")]
pub struct PermissionName(String);

impl From<String> for PermissionName {
    fn from(s: String) -> Self {
        PermissionName(s)
    }
}

impl From<PermissionName> for String {
    fn from(p: PermissionName) -> String {
        p.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activation {
    OnCommand,
    OnTool,
    OnSurfaceOpen,
    OnEvent,
    OnDocumentHasNamespace,
    OnStartup,
}

impl Activation {
    /// The activation set a manifest gets when it names none.
    pub fn default_set() -> Vec<Activation> {
        vec![
            Activation::OnCommand,
            Activation::OnTool,
            Activation::OnSurfaceOpen,
            Activation::OnEvent,
            Activation::OnDocumentHasNamespace,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataSpec {
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub migrate: Option<String>,
}

impl Default for DataSpec {
    fn default() -> DataSpec {
        DataSpec {
            version: one(),
            migrate: None,
        }
    }
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Locales {
    pub default: String,
    pub catalogs: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSpec {
    pub notes: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contributes {
    #[serde(default)]
    pub commands: Vec<CommandContribution>,
    #[serde(default)]
    pub presave: bool,
    // Opaque until their stages validate them.
    #[serde(default)]
    pub actions: Option<serde_json::Value>,
    #[serde(default)]
    pub toolbars: Option<serde_json::Value>,
    #[serde(default)]
    pub menus: Option<serde_json::Value>,
    #[serde(default)]
    pub tools: Option<serde_json::Value>,
    #[serde(default)]
    pub hud: Option<serde_json::Value>,
    #[serde(default)]
    pub panels: Option<serde_json::Value>,
    #[serde(default)]
    pub windows: Option<serde_json::Value>,
    #[serde(default, rename = "viewportSurfaces")]
    pub viewport_surfaces: Option<serde_json::Value>,
    #[serde(default)]
    pub sheets: Option<serde_json::Value>,
    #[serde(default)]
    pub events: Option<serde_json::Value>,
    #[serde(default, rename = "dialogSlots")]
    pub dialog_slots: Option<serde_json::Value>,
    #[serde(default)]
    pub settings: Option<serde_json::Value>,
    #[serde(default, rename = "printPages")]
    pub print_pages: Option<serde_json::Value>,
    #[serde(default, rename = "exportFormats")]
    pub export_formats: Option<serde_json::Value>,
    #[serde(default, rename = "importFormats")]
    pub import_formats: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandContribution {
    pub id: String,
    pub class: CommandContributionClass,
    pub summary: String,
    #[serde(default)]
    pub label: Option<LocalizableString>,
    #[serde(default)]
    pub permission: Option<PermissionName>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u32,
    #[serde(default)]
    pub concurrency: Concurrency,
    #[serde(default = "empty_object_schema")]
    pub params: serde_json::Value,
    #[serde(default = "empty_object_schema")]
    pub result: serde_json::Value,
    #[serde(default)]
    pub refusals: Vec<String>,
}

fn default_timeout_ms() -> u32 {
    50_000
}

fn empty_object_schema() -> serde_json::Value {
    serde_json::json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandContributionClass {
    ReadOnly,
    Mutating,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Concurrency {
    #[default]
    Parallel,
    Serial,
}

/// Why a manifest was refused. [`ManifestError::name`] is the refusal name
/// an installer reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// Not JSON, or not the manifest's shape. Unknown keys anywhere in the
    /// typed structs land here and are named `manifest_unknown_field`;
    /// every other shape defect is `manifest_malformed`.
    Json(String),
    UnsupportedManifestVersion(u32),
    InvalidId(String),
    ReservedId(String),
    NameTooLong,
    NoEntry,
    /// `main`, `icon`, `locales.catalogs`, or `agent.notes` with a `..`
    /// segment, a leading `/`, or a backslash.
    InvalidPath(String),
    UnknownPermission(String),
    PermissionTooBroad(String),
    DuplicatePermission(String),
    ActivationReasonRequired,
    InvalidCommandId(String),
    DuplicateCommand(String),
    TooManyCommands(usize),
    InvalidSchema {
        command: String,
        why: String,
    },
    DuplicateRequirement(String),
    SelfRequirement,
    UnsupportedApi(u32),
}

impl ManifestError {
    /// The snake_case refusal name.
    pub fn name(&self) -> &'static str {
        match self {
            ManifestError::Json(text) if is_unknown_field_error(text) => "manifest_unknown_field",
            ManifestError::Json(_) => "manifest_malformed",
            ManifestError::UnsupportedManifestVersion(_) => "manifest_version_unsupported",
            ManifestError::InvalidId(_) => "invalid_plugin_id",
            ManifestError::ReservedId(_) => "reserved_plugin_id",
            ManifestError::NameTooLong => "name_too_long",
            ManifestError::NoEntry => "manifest_no_entry",
            ManifestError::InvalidPath(_) => "manifest_invalid_path",
            ManifestError::UnknownPermission(_) => "unknown_permission",
            ManifestError::PermissionTooBroad(_) => "permission_too_broad",
            ManifestError::DuplicatePermission(_) => "duplicate_permission",
            ManifestError::ActivationReasonRequired => "activation_reason_required",
            ManifestError::InvalidCommandId(_) => "invalid_command_id",
            ManifestError::DuplicateCommand(_) => "duplicate_command",
            ManifestError::TooManyCommands(_) => "too_many_commands",
            ManifestError::InvalidSchema { .. } => "invalid_schema",
            ManifestError::DuplicateRequirement(_) => "duplicate_requirement",
            ManifestError::SelfRequirement => "self_requirement",
            ManifestError::UnsupportedApi(_) => "api_unsupported",
        }
    }

    /// One sentence a person can act on.
    pub fn explanation(&self) -> String {
        match self {
            ManifestError::Json(text) => format!(
                "hew-plugin.json is not a valid manifest: {text}. Fix the file and install it again."
            ),
            ManifestError::UnsupportedManifestVersion(v) => format!(
                "This plugin uses manifest format {v}, and this version of Hew reads format {MANIFEST_VERSION}. Update Hew, or ask the author for a compatible build."
            ),
            ManifestError::InvalidId(id) => format!(
                "\"{id}\" is not a valid plugin id. Use lowercase reverse-DNS form with at least two parts, like \"com.example.myplugin\"."
            ),
            ManifestError::ReservedId(id) => format!(
                "\"{id}\" starts with \"hew\", which is reserved for Hew itself. Choose an id under a domain you own."
            ),
            ManifestError::NameTooLong => format!(
                "The plugin name must be 1 to {MAX_NAME_LEN} characters in every language. Shorten it."
            ),
            ManifestError::NoEntry => {
                "The manifest has no \"main\" script. Name the background script the plugin runs.".to_string()
            }
            ManifestError::InvalidPath(path) => format!(
                "\"{path}\" is not a path inside the plugin package. Use a relative path with forward slashes and no \"..\"."
            ),
            ManifestError::UnknownPermission(p) => format!(
                "\"{p}\" is not a permission Hew knows. Check the spelling against the permission list."
            ),
            ManifestError::PermissionTooBroad(p) => format!(
                "\"{p}\" asks for more than one host. Name each host the plugin talks to."
            ),
            ManifestError::DuplicatePermission(p) => format!(
                "\"{p}\" is listed more than once across permissions and optionalPermissions. List it once."
            ),
            ManifestError::ActivationReasonRequired => {
                "A plugin that starts with Hew must say why in \"activationReason\". Add a short reason, or drop on_startup.".to_string()
            }
            ManifestError::InvalidCommandId(id) => format!(
                "\"{id}\" is not a valid command or refusal name. Use lowercase letters, digits, and underscores, with dots between parts."
            ),
            ManifestError::DuplicateCommand(id) => {
                format!("The command \"{id}\" is declared more than once. Declare it once.")
            }
            ManifestError::TooManyCommands(n) => format!(
                "The manifest declares {n} commands, and a plugin may declare at most {MAX_COMMANDS}. Combine some of them."
            ),
            ManifestError::InvalidSchema { command, why } => format!(
                "The schema of the command \"{command}\" is not accepted: {why}. Fix the schema."
            ),
            ManifestError::DuplicateRequirement(id) => format!(
                "\"{id}\" is listed more than once in requires. List it once with one range."
            ),
            ManifestError::SelfRequirement => {
                "The plugin lists itself in requires. Remove that entry.".to_string()
            }
            ManifestError::UnsupportedApi(v) => format!(
                "This plugin speaks API {v}, and this version of Hew speaks API {SUPPORTED_API}. Update Hew, or ask the author for a compatible build."
            ),
        }
    }
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.name(), self.explanation())
    }
}

impl std::error::Error for ManifestError {}

/// The permission vocabulary, in canonical order. `attributes.write:` and
/// `network:` take a suffix; nothing else does.
pub const PERMISSION_NAMES: &[&str] = &[
    "model.read",
    "model.write",
    "attributes.write",
    "view",
    "documents.save",
    "documents.discard",
    "documents.other",
    "files",
    "clipboard",
    "library",
];

/// The permissions a contributed command may demand of its callers. A
/// plugin command cannot require a documents or network permission.
const COMMAND_PERMISSIONS: &[&str] = &[
    "model.read",
    "model.write",
    "attributes.write",
    "view",
    "files",
    "clipboard",
    "library",
];

impl PermissionName {
    /// Validates one permission string: a bare name from
    /// [`PERMISSION_NAMES`], `attributes.write:<ns>` where `<ns>` follows
    /// the plugin id grammar and is not `hew` or under `hew.`, or
    /// `network:<host>` for one hostname. Any `*` in a network permission
    /// is `permission_too_broad`; everything else is `unknown_permission`.
    pub fn parse(s: &str) -> Result<PermissionName, ManifestError> {
        let unknown = || ManifestError::UnknownPermission(s.to_string());
        if PERMISSION_NAMES.contains(&s) {
            return Ok(PermissionName(s.to_string()));
        }
        if let Some(ns) = s.strip_prefix("attributes.write:") {
            if ns == "hew" || ns.starts_with("hew.") || !id_grammar_ok(ns) {
                return Err(unknown());
            }
            return Ok(PermissionName(s.to_string()));
        }
        if let Some(host) = s.strip_prefix("network:") {
            if host.contains('*') {
                return Err(ManifestError::PermissionTooBroad(s.to_string()));
            }
            if !validate_hostname(host) {
                return Err(unknown());
            }
            return Ok(PermissionName(s.to_string()));
        }
        Err(unknown())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `("attributes.write", Some("com.x"))`, `("network", Some("host"))`,
    /// or `(name, None)`.
    pub fn split(&self) -> (&str, Option<&str>) {
        if let Some(ns) = self.0.strip_prefix("attributes.write:") {
            return ("attributes.write", Some(ns));
        }
        if let Some(host) = self.0.strip_prefix("network:") {
            return ("network", Some(host));
        }
        (&self.0, None)
    }
}

/// `label ("." label)+`, label `[a-z0-9][a-z0-9-]*` of 1..=63, total
/// `<= MAX_ID_LEN`; no reservation check.
fn id_grammar_ok(id: &str) -> bool {
    if id.is_empty() || id.len() > MAX_ID_LEN {
        return false;
    }
    let labels: Vec<&str> = id.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|label| {
            let bytes = label.as_bytes();
            !bytes.is_empty()
                && bytes.len() <= 63
                && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
                && bytes
                    .iter()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        })
}

/// The plugin id grammar (docs/agents/PLUGINS.md "Manifest"):
/// `label ("." label)+`, label = `[a-z0-9][a-z0-9-]*`, 1..=63 per label,
/// total `<= MAX_ID_LEN`; an id whose first label is `hew` is reserved.
pub fn validate_plugin_id(id: &str) -> Result<(), ManifestError> {
    if !id_grammar_ok(id) {
        return Err(ManifestError::InvalidId(id.to_string()));
    }
    if id.split('.').next() == Some("hew") {
        return Err(ManifestError::ReservedId(id.to_string()));
    }
    Ok(())
}

/// A hostname for `network:<host>`: RFC 1123 labels joined by `.`,
/// lowercase, no port, no scheme, no `*`, total <= 253.
pub fn validate_hostname(host: &str) -> bool {
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    host.split('.').all(|label| {
        let bytes = label.as_bytes();
        !bytes.is_empty()
            && bytes.len() <= 63
            && bytes[0] != b'-'
            && bytes[bytes.len() - 1] != b'-'
            && bytes
                .iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
    })
}

/// Whether a serde error is `deny_unknown_fields` rejecting a key. serde
/// words that error `unknown field `<key>`, expected ...` and puts it first
/// in the message; matching the start, not any substring, keeps a value or
/// locale key that happens to contain the phrase from being misnamed.
fn is_unknown_field_error(text: &str) -> bool {
    text.starts_with("unknown field `")
}

/// A path inside the package: non-empty, relative, forward slashes only,
/// no `..` segment, and no drive or scheme prefix (no `:` in the first
/// segment, so `C:/x.js` and `C:x.js` are refused). A file path (`main`,
/// `icon`, `agent.notes`) may not end in `/`; a directory path
/// (`locales.catalogs`) may.
fn package_path_ok(path: &str, directory: bool) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path
            .split('/')
            .next()
            .is_some_and(|first| first.contains(':'))
        && !path.split('/').any(|seg| seg == "..")
        && (directory || !path.ends_with('/'))
}

/// A contributed command id: `label ("." label)*`, label `[a-z0-9_]+`.
fn command_id_ok(id: &str) -> bool {
    !id.is_empty()
        && id.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
}

/// Whether `id` is a valid contributed command id (`label ("." label)*`,
/// label `[a-z0-9_]+`): the check [`parse`] makes, for a registry handed
/// command declarations directly.
pub fn is_command_id(id: &str) -> bool {
    command_id_ok(id)
}

/// Whether `name` is a valid refusal name (`[a-z][a-z0-9_]*`), the check
/// [`parse`] makes on a contributed command's `refusals`.
pub fn is_refusal_name(name: &str) -> bool {
    refusal_name_ok(name)
}

/// Whether `permission` is one a contributed command may demand of its
/// callers (no documents or network permission), the check [`parse`]
/// makes on a contributed command's `permission`.
pub fn is_command_permission(permission: &str) -> bool {
    COMMAND_PERMISSIONS.contains(&permission)
}

/// A refusal name: `[a-z][a-z0-9_]*`.
fn refusal_name_ok(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
}

/// The root of a contributed schema must be a closed object.
fn check_schema(
    command: &str,
    which: &str,
    schema: &serde_json::Value,
) -> Result<(), ManifestError> {
    let invalid = |why: String| ManifestError::InvalidSchema {
        command: command.to_string(),
        why,
    };
    let Some(object) = schema.as_object() else {
        return Err(invalid(format!("`{which}` must be a JSON object")));
    };
    if object.get("type") != Some(&serde_json::Value::String("object".into())) {
        return Err(invalid(format!(
            "`{which}` must have \"type\": \"object\" at its root"
        )));
    }
    if object.get("additionalProperties") != Some(&serde_json::Value::Bool(false)) {
        return Err(invalid(format!(
            "`{which}` must have \"additionalProperties\": false at its root"
        )));
    }
    Ok(())
}

fn name_ok(text: &str) -> bool {
    let n = text.chars().count();
    (1..=MAX_NAME_LEN).contains(&n)
}

/// Parses and validates a `hew-plugin.json`. The steps run in this order
/// and the first failure is the answer:
///
/// 1. JSON into [`Manifest`] (`manifest_unknown_field` / `manifest_malformed`).
/// 2. `manifest == MANIFEST_VERSION`.
/// 3. `api == 1`.
/// 4. The id grammar and the `hew` reservation.
/// 5. The display name's length.
/// 6. `main` (missing or empty is `manifest_no_entry`), then `main`,
///    `icon`, `locales.catalogs`, `agent.notes` as package paths.
/// 7. Every permission, `permissions` first, then `optionalPermissions`;
///    then duplicates across both lists.
/// 8. `on_startup` needs `activationReason`.
/// 9. `contributes.commands`: every id, then duplicates, then the count,
///    then per command its `params` schema, `result` schema,
///    `permission`, and refusal names.
/// 10. `requires`: per entry, the id grammar and reservation, a repeated
///     id, then the plugin itself.
pub fn parse(text: &str) -> Result<Manifest, ManifestError> {
    // 1
    let manifest: Manifest =
        serde_json::from_str(text).map_err(|e| ManifestError::Json(e.to_string()))?;
    // 2
    if manifest.manifest != MANIFEST_VERSION {
        return Err(ManifestError::UnsupportedManifestVersion(manifest.manifest));
    }
    // 3
    if manifest.api != SUPPORTED_API {
        return Err(ManifestError::UnsupportedApi(manifest.api));
    }
    // 4
    validate_plugin_id(&manifest.id)?;
    // 5
    match &manifest.name {
        LocalizableString::Plain(s) => {
            if !name_ok(s) {
                return Err(ManifestError::NameTooLong);
            }
        }
        LocalizableString::ByLocale(map) => {
            if !map.values().all(|s| name_ok(s)) {
                return Err(ManifestError::NameTooLong);
            }
        }
        LocalizableString::Key { .. } => {}
    }
    // 6
    if manifest.main.is_empty() {
        return Err(ManifestError::NoEntry);
    }
    let paths = [
        (Some(manifest.main.as_str()), false),
        (manifest.icon.as_deref(), false),
        (manifest.locales.as_ref().map(|l| l.catalogs.as_str()), true),
        (manifest.agent.as_ref().map(|a| a.notes.as_str()), false),
    ];
    for (path, directory) in paths {
        if let Some(path) = path
            && !package_path_ok(path, directory)
        {
            return Err(ManifestError::InvalidPath(path.to_string()));
        }
    }
    // 7
    let all_permissions = manifest
        .permissions
        .iter()
        .chain(&manifest.optional_permissions);
    for p in all_permissions.clone() {
        PermissionName::parse(p.as_str())?;
    }
    let mut seen = BTreeSet::new();
    for p in all_permissions {
        if !seen.insert(p.as_str()) {
            return Err(ManifestError::DuplicatePermission(p.as_str().to_string()));
        }
    }
    // 8
    if manifest.activation.contains(&Activation::OnStartup) && manifest.activation_reason.is_none()
    {
        return Err(ManifestError::ActivationReasonRequired);
    }
    // 9
    let commands = &manifest.contributes.commands;
    for c in commands {
        if !command_id_ok(&c.id) {
            return Err(ManifestError::InvalidCommandId(c.id.clone()));
        }
    }
    let mut ids = BTreeSet::new();
    for c in commands {
        if !ids.insert(c.id.as_str()) {
            return Err(ManifestError::DuplicateCommand(c.id.clone()));
        }
    }
    if commands.len() > MAX_COMMANDS {
        return Err(ManifestError::TooManyCommands(commands.len()));
    }
    for c in commands {
        check_schema(&c.id, "params", &c.params)?;
        check_schema(&c.id, "result", &c.result)?;
        if let Some(p) = &c.permission
            && !COMMAND_PERMISSIONS.contains(&p.as_str())
        {
            return Err(ManifestError::UnknownPermission(p.as_str().to_string()));
        }
        if let Some(bad) = c.refusals.iter().find(|r| !refusal_name_ok(r)) {
            return Err(ManifestError::InvalidCommandId(bad.clone()));
        }
    }
    // 10
    let mut required = BTreeSet::new();
    for r in &manifest.requires {
        validate_plugin_id(&r.id)?;
        if !required.insert(r.id.as_str()) {
            return Err(ManifestError::DuplicateRequirement(r.id.clone()));
        }
        if r.id == manifest.id {
            return Err(ManifestError::SelfRequirement);
        }
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_id_grammar() {
        for ok in [
            "com.example.plugin",
            "org.hew3d.cutlist",
            "a.b",
            "com.example-co.x1",
            "hewlett.example",
        ] {
            assert_eq!(validate_plugin_id(ok), Ok(()), "{ok}");
        }
        for bad in [
            "",
            "single",
            "com..x",
            ".com.x",
            "com.x.",
            "com.-x",
            "com.Example",
            "com.ex_ample",
            "com.ex ample",
        ] {
            assert!(
                matches!(validate_plugin_id(bad), Err(ManifestError::InvalidId(_))),
                "{bad}"
            );
        }
        let long_label = format!("com.{}", "a".repeat(64));
        assert!(validate_plugin_id(&long_label).is_err());
        let ok_label = format!("com.{}", "a".repeat(63));
        assert!(validate_plugin_id(&ok_label).is_ok());
        let too_long = format!("com.{}.{}", "a".repeat(63), "b".repeat(61));
        assert_eq!(too_long.len(), MAX_ID_LEN + 1);
        assert!(validate_plugin_id(&too_long).is_err());
        assert!(matches!(
            validate_plugin_id("hew.x"),
            Err(ManifestError::ReservedId(_))
        ));
    }

    #[test]
    fn hostnames() {
        for ok in ["example.com", "localhost", "a-b.example.com", "1.example"] {
            assert!(validate_hostname(ok), "{ok}");
        }
        for bad in [
            "",
            "Example.com",
            "example.com:80",
            "https://example.com",
            "*.example.com",
            "-a.example.com",
            "a-.example.com",
            "example..com",
            "example.com.",
            "exa_mple.com",
        ] {
            assert!(!validate_hostname(bad), "{bad}");
        }
        assert!(!validate_hostname(&"a.".repeat(127)));
    }

    #[test]
    fn permission_parse_and_split() {
        for name in PERMISSION_NAMES {
            let p = PermissionName::parse(name).unwrap();
            assert_eq!(p.split(), (*name, None));
        }
        let ns = PermissionName::parse("attributes.write:com.other.app").unwrap();
        assert_eq!(ns.split(), ("attributes.write", Some("com.other.app")));
        let net = PermissionName::parse("network:api.example.com").unwrap();
        assert_eq!(net.split(), ("network", Some("api.example.com")));
        for unknown in [
            "all",
            "network",
            "model",
            "attributes.write:hew",
            "attributes.write:hew.scenes",
            "attributes.write:single",
            "network:example.com:443",
            "view:x",
            "",
        ] {
            assert!(
                matches!(
                    PermissionName::parse(unknown),
                    Err(ManifestError::UnknownPermission(_))
                ),
                "{unknown}"
            );
        }
        for broad in ["network:*", "network:*.example.com", "network:a*b"] {
            assert!(
                matches!(
                    PermissionName::parse(broad),
                    Err(ManifestError::PermissionTooBroad(_))
                ),
                "{broad}"
            );
        }
    }

    #[test]
    fn package_paths() {
        for ok in [
            "main.js",
            "dist/background.js",
            "a..b.js",
            "./x.js",
            "a/b:c.js",
        ] {
            assert!(package_path_ok(ok, false), "{ok}");
        }
        assert!(package_path_ok("locales/", true));
        assert!(package_path_ok("locales", true));
        for bad in [
            "",
            "/main.js",
            "../x.js",
            "a/../../x.js",
            "a\\b.js",
            "..",
            "C:/evil.js",
            "C:evil.js",
            "c:",
            "file:x.js",
            "dist/",
        ] {
            assert!(!package_path_ok(bad, false), "{bad:?}");
        }
        assert!(!package_path_ok("", true));
        assert!(!package_path_ok("C:/locales/", true));
    }

    #[test]
    fn unknown_field_errors_are_named_by_their_prefix_only() {
        assert!(is_unknown_field_error(
            "unknown field `mian`, expected one of ..."
        ));
        assert!(!is_unknown_field_error(
            "unknown variant `unknown field`, expected ..."
        ));
        assert!(!is_unknown_field_error(
            "invalid type: string \"unknown field\", expected u32"
        ));
    }

    #[test]
    fn localizable_string_forms() {
        let plain: LocalizableString = serde_json::from_str("\"Hi\"").unwrap();
        assert_eq!(plain, LocalizableString::Plain("Hi".into()));
        let key: LocalizableString = serde_json::from_str(r#"{"$t":"k"}"#).unwrap();
        assert_eq!(key, LocalizableString::Key { key: "k".into() });
        let by: LocalizableString = serde_json::from_str(r#"{"en":"Hi","ja":"やあ"}"#).unwrap();
        assert!(matches!(by, LocalizableString::ByLocale(ref m) if m.len() == 2));
        for bad in [
            r#"{}"#,
            r#"{"$t":"k","en":"x"}"#,
            r#"{"en":1}"#,
            "3",
            r#"{"$t":1}"#,
        ] {
            assert!(
                serde_json::from_str::<LocalizableString>(bad).is_err(),
                "{bad}"
            );
        }
        for v in [plain, key, by] {
            let back: LocalizableString =
                serde_json::from_value(serde_json::to_value(&v).unwrap()).unwrap();
            assert_eq!(back, v);
        }
    }

    #[test]
    fn every_error_has_a_distinct_name_and_an_explanation() {
        let all = [
            ManifestError::Json("unknown field `x`".into()),
            ManifestError::Json("expected value".into()),
            ManifestError::UnsupportedManifestVersion(2),
            ManifestError::InvalidId("x".into()),
            ManifestError::ReservedId("hew.x".into()),
            ManifestError::NameTooLong,
            ManifestError::NoEntry,
            ManifestError::InvalidPath("/x".into()),
            ManifestError::UnknownPermission("all".into()),
            ManifestError::PermissionTooBroad("network:*".into()),
            ManifestError::DuplicatePermission("view".into()),
            ManifestError::ActivationReasonRequired,
            ManifestError::InvalidCommandId("X".into()),
            ManifestError::DuplicateCommand("x".into()),
            ManifestError::TooManyCommands(300),
            ManifestError::InvalidSchema {
                command: "x".into(),
                why: "y".into(),
            },
            ManifestError::DuplicateRequirement("a.b".into()),
            ManifestError::SelfRequirement,
            ManifestError::UnsupportedApi(2),
        ];
        let names: BTreeSet<&str> = all.iter().map(ManifestError::name).collect();
        assert_eq!(names.len(), all.len());
        for e in &all {
            assert!(!e.explanation().is_empty());
        }
    }
}
