//! The command registry — the single source of truth for the API's
//! per-command contracts (docs/agents/HEW_API.md §9). Every command is declared
//! once, here: name, version, class, tier, where it is served, summary,
//! parameter/result schemas, and refusal inventory. MCP tool definitions,
//! the TypeScript SDK, and the published reference are generated from
//! these declarations — hand-maintained copies are forbidden.
//!
//! Scaffold posture (§14): the full protocol-1 inventory is declared so
//! the unimplemented set is a visible burn-down list. A command's schemas
//! are refined in the change that lands its conformance tests; until it
//! is implemented, dispatch answers it with the `unimplemented` refusal.

use crate::host::DocFormat;
use std::collections::BTreeMap;

use crate::grant::Permission;

/// What a command does to the document — governs envelope placement
/// (docs/agents/HEW_API.md §6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandClass {
    /// Transaction payload; the one-envelope-one-undo accounting applies.
    ModelMutating,
    /// Legal anywhere: standalone (no undo entry) or interleaved inside a
    /// transaction as a `$ref` source.
    ReadOnly,
    /// Legal only as the sole command of its envelope: host effects and
    /// history operations a compound entry could not roll back.
    Solitary,
}

/// Who executes the command's effect (docs/agents/HEW_API.md §3): the pure
/// dispatcher, or a host through the typed host trait (file I/O,
/// importers, rendering). Either way this crate owns the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Served {
    Kernel,
    Host,
}

/// Command tier at protocol 1 (docs/agents/HEW_API.md §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Exists at 1.0; the minimum for the agent-modeling vision.
    Required,
    /// Exists at 1.0 for tool parity.
    Standard,
}

/// Who provides a command: the kernel or a host for every protocol-1
/// command (by where it is served), or a plugin, by id, for a command a
/// plugin's manifest contributes (docs/agents/PLUGINS.md "Commands").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provider {
    Kernel,
    Host,
    Plugin(String),
}

/// One command's declaration.
#[derive(Debug, Clone)]
pub struct CommandDecl {
    /// Dot-separated, lower snake_case: `hew.<area>.<command>` (§4.1) for a
    /// first-party command, `<plugin-id>.<local>` for a plugin's.
    pub name: String,
    /// Per-command version, starting at 1; evolution is additive (§9).
    pub version: u32,
    pub class: CommandClass,
    pub served: Served,
    pub tier: Tier,
    /// What a connection must hold to call it (docs/agents/HEW_API.md §10;
    /// [`permission_table`]).
    pub permission: Permission,
    /// `Kernel` or `Host` (by `served`) for every protocol-1 command;
    /// `Plugin(id)` for a plugin-provided one, which dispatch forwards
    /// instead of executing.
    pub provider: Provider,
    /// One-line summary for `hew.meta.capabilities` and generated docs.
    pub summary: String,
    /// The undo label of a plugin command, in English; `None` for every
    /// first-party command.
    pub label: Option<String>,
    /// How long the forwarding host waits for a plugin command's reply,
    /// at most 50 000 ms; 0 for first-party commands.
    pub timeout_ms: u32,
    /// Whether the forwarding host may run several of this plugin
    /// command at once; `Parallel` for first-party commands.
    pub concurrency: plugin_manifest::Concurrency,
    /// A deprecation notice (docs/agents/HEW_API.md §9); always `None` for
    /// now.
    pub deprecated: Option<String>,
    /// JSON Schema for `params`. Scaffold declarations start at the
    /// permissive object schema and are tightened with each command's
    /// conformance tests.
    pub params_schema: serde_json::Value,
    /// JSON Schema for the success result.
    pub result_schema: serde_json::Value,
    /// The machine names of the refusals this command can answer (§4.4).
    /// Empty is honest for a command whose failures are all protocol
    /// errors; every unimplemented command carries `["unimplemented"]`.
    pub refusals: Vec<String>,
    /// Whether dispatch executes it yet. `false` answers the
    /// `unimplemented` refusal — the burn-down list (§14).
    pub implemented: bool,
    /// Whether a successful dispatch can change the document.
    ///
    /// NOT derivable from [`CommandClass`]: the class says whether a
    /// command may ride inside a transaction (§6.4), which is a different
    /// question. `hew.history.undo`/`redo` are `Solitary` — they cannot
    /// be bracketed by the very history they move — yet they plainly
    /// change the document, and `hew.doc.new`/`open` replace it outright.
    /// Hosts read this to decide what to re-render, re-sync, and record;
    /// deriving it from the class instead silently skipped exactly those
    /// commands, leaving a live viewport showing a document that no
    /// longer existed.
    pub mutates_document: bool,
}

impl CommandDecl {
    /// Whether a plugin provides this command.
    pub fn is_plugin(&self) -> bool {
        matches!(self.provider, Provider::Plugin(_))
    }

    /// The providing plugin's id, for a plugin command.
    pub fn plugin_id(&self) -> Option<&str> {
        match &self.provider {
            Provider::Plugin(id) => Some(id),
            _ => None,
        }
    }
}

/// A protocol-1 declaration while [`Registry::protocol_1`] builds it: the
/// same fields as [`CommandDecl`] with static text, so the inventory below
/// reads as literals.
struct Draft {
    name: &'static str,
    version: u32,
    class: CommandClass,
    served: Served,
    tier: Tier,
    permission: Permission,
    summary: &'static str,
    params_schema: serde_json::Value,
    result_schema: serde_json::Value,
    refusals: Vec<&'static str>,
    implemented: bool,
    mutates_document: bool,
}

impl Draft {
    fn finish(self) -> CommandDecl {
        CommandDecl {
            name: self.name.to_string(),
            version: self.version,
            class: self.class,
            served: self.served,
            tier: self.tier,
            permission: self.permission,
            provider: match self.served {
                Served::Kernel => Provider::Kernel,
                Served::Host => Provider::Host,
            },
            summary: self.summary.to_string(),
            label: None,
            timeout_ms: 0,
            concurrency: plugin_manifest::Concurrency::Parallel,
            deprecated: None,
            params_schema: self.params_schema,
            result_schema: self.result_schema,
            refusals: self.refusals.into_iter().map(str::to_string).collect(),
            implemented: self.implemented,
            mutates_document: self.mutates_document,
        }
    }
}

/// Why [`Registry::register_plugin`] refused a plugin's commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterError {
    /// The plugin id fails the plugin id grammar or is reserved, or a
    /// command id fails the command id grammar (the full name is given).
    NameOutsidePluginNamespace(String),
    /// The full command name is already registered.
    DuplicateCommand(String),
    /// A params or result schema the dispatcher's checker cannot enforce.
    InvalidSchema { command: String, why: String },
    /// More than [`plugin_manifest::MAX_COMMANDS`] commands.
    TooManyCommands(usize),
}

impl RegisterError {
    /// The snake_case refusal name.
    pub fn name(&self) -> &'static str {
        match self {
            RegisterError::NameOutsidePluginNamespace(_) => "name_outside_plugin_namespace",
            RegisterError::DuplicateCommand(_) => "duplicate_command",
            RegisterError::InvalidSchema { .. } => "invalid_schema",
            RegisterError::TooManyCommands(_) => "too_many_commands",
        }
    }
}

impl std::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegisterError::NameOutsidePluginNamespace(name) => write!(
                f,
                "\"{name}\" is not a plugin id, or not a command name under one"
            ),
            RegisterError::DuplicateCommand(name) => {
                write!(f, "the command \"{name}\" is already registered")
            }
            RegisterError::InvalidSchema { command, why } => {
                write!(f, "the schema of \"{command}\" cannot be enforced: {why}")
            }
            RegisterError::TooManyCommands(n) => write!(
                f,
                "{n} commands is more than the {} a plugin may contribute",
                plugin_manifest::MAX_COMMANDS
            ),
        }
    }
}

/// The longest a plugin command may run, in milliseconds: below the live
/// transports' 60 s reply timeout, so a socket caller never times out
/// first. A larger manifest value is clamped.
pub const MAX_PLUGIN_TIMEOUT_MS: u32 = 50_000;

/// The command registry: protocol 1 plus whatever plugins registered.
#[derive(Debug, Clone)]
pub struct Registry {
    commands: BTreeMap<String, CommandDecl>,
}

/// Shorthand for a permissive object schema — the scaffold placeholder
/// each command tightens when its conformance tests land.
fn obj() -> serde_json::Value {
    serde_json::json!({ "type": "object" })
}

/// An occurrence locator: a world node's public id, or
/// `{ "path": [instance, …, leaf] }` (HEW_API.md §5.2).
fn occurrence_locator_schema() -> serde_json::Value {
    serde_json::json!({
        "oneOf": [
            { "type": "string" },
            {
                "type": "object",
                "properties": {
                    "path": { "type": "array", "items": { "type": "string" }, "minItems": 1 }
                },
                "required": ["path"],
                "additionalProperties": false
            }
        ]
    })
}

/// An attribute target (HEW_API.md §8): an entity's public id or
/// `"document"`, or `{ "face": <face locator> }` / `{ "edge": <edge
/// locator> }` for a solid's face or edge (§5.2 locator forms).
fn attr_target_schema() -> serde_json::Value {
    serde_json::json!({
        "oneOf": [
            { "type": "string", "description": "an entity's public id, or \"document\"" },
            {
                "type": "object",
                "properties": {
                    "face": { "description": "a face locator: {object | path, at}, {object | path, ray}, or {\"$face\": \"label#key\"}; path is an occurrence path ending at the object, at and ray in the object's own frame" }
                },
                "required": ["face"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "edge": { "description": "a solid edge locator: {object | path, at}, at in the object's own frame" }
                },
                "required": ["edge"],
                "additionalProperties": false
            }
        ]
    })
}

/// A list of `{ outer, edges }` loops in a plane's xy (the
/// `hew.compute.outline` shape), for the reserved `hew.compute.project`.
fn loops_result_items(xy: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                "outer": { "type": "boolean" },
                "edges": { "type": "array", "items": { "type": "object", "properties": { "kind": { "type": "string" }, "a": xy, "b": xy } } }
            },
            "required": ["outer", "edges"]
        }
    })
}

/// `{ id?, path? }` (exactly one), with `space` for the buffer reads.
fn occurrence_target_schema(with_space: bool) -> serde_json::Value {
    let mut properties = serde_json::json!({
        "id": { "type": "string", "description": "a world node's public id" },
        "path": { "type": "array", "items": { "type": "string" }, "minItems": 1, "description": "an occurrence path, instances first and the node last" }
    });
    if with_space {
        properties["space"] = serde_json::json!({
            "type": "string",
            "enum": ["world", "local"],
            "description": "defaults to world; local is the node's own frame (an instance's definition coordinates)"
        });
    }
    serde_json::json!({
        "type": "object",
        "properties": properties,
        "additionalProperties": false
    })
}

/// `exclude` on the part reads (HEW_API.md "exclude").
fn exclude_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "array",
        "items": { "type": "string" },
        "description": "Object ids, each one of the target's own members, left out of the frame and the measures (needs scope \"own\"); an id that is not an own member is -32602"
    })
}

/// `scope` on the part reads and the render commands (docs/agents/PLUGINS.md
/// "Reading a container's own Objects").
fn scope_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "string",
        "enum": ["own", "all"],
        "default": "all",
        "description": "all (the default) takes every leaf under the target; own takes a container's direct objects and its groups' objects, never a nested instance's members. On an object both are the same"
    })
}

/// One `hew.query.occurrences` row.
fn occurrence_json_schema() -> serde_json::Value {
    let nullable_string = serde_json::json!({ "type": ["string", "null"] });
    serde_json::json!({
        "type": "object",
        "properties": {
            "path": { "type": "array", "items": { "type": "string" }, "description": "the instances above this node, outermost first; {path: [...path, id]} locates it" },
            "id": { "type": "string" },
            "kind": { "type": "string", "enum": ["object", "group", "instance"] },
            "definition": nullable_string,
            "parent": nullable_string,
            "world_pose": { "type": "array", "items": { "type": "number" }, "minItems": 12, "maxItems": 12, "description": "row-major 3×4" },
            "mirrored": { "type": "boolean" },
            "hidden": { "type": "boolean", "description": "effective: the node's own flag and tags, its groups', and every instance on the path" },
            "user_hidden": { "type": "boolean" },
            "material": { "type": ["string", "null"], "description": "an object's base material, an instance's own; null for a group" },
            "material_effective": { "type": ["string", "null"], "description": "what the node's unpainted faces show: an object's base, else the nearest painted instance on the path; an instance's own, else the nearest painted instance above it; null for a group" },
            "tags": { "type": "array", "items": { "type": "array", "items": { "type": "string" } } },
            "name": nullable_string,
            "definition_name": nullable_string,
            "depth": { "type": "integer", "minimum": 0 },
            "members": {
                "type": "object",
                "description": "the node's direct members by kind, in member order: a group's members, an instance's definition's members, three empty lists for an object. Hidden members are left out unless include_hidden",
                "properties": {
                    "objects": { "type": "array", "items": { "type": "string" } },
                    "groups": { "type": "array", "items": { "type": "string" } },
                    "instances": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["objects", "groups", "instances"],
                "additionalProperties": false
            },
            "solid": { "type": "boolean", "description": "object rows only: the health verdict is solid (closed and manifold)" },
            "family": { "type": ["integer", "null"], "description": "object rows only: the seam family index in hew.doc.health's families, valid for the same document state; null when the object is in no family" },
            "family_closed": { "type": ["boolean", "null"], "description": "object rows only: every shared rim of the family can take a cap; null exactly when family is null" },
            "linked": { "type": "boolean", "description": "instance rows only: the definition is a set of linked copies (hew / linked)" }
        },
        "required": ["path", "id", "kind", "definition", "parent", "world_pose", "mirrored", "hidden", "user_hidden", "material", "material_effective", "tags", "name", "definition_name", "depth", "members"],
        "additionalProperties": false
    })
}

/// `selection` on export and line drawings.
fn render_selection_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "array",
        "minItems": 1,
        "items": occurrence_locator_schema(),
        "description": "draw only these: node ids, occurrence locators {path}, or a definition's cmp_ id (its leaves at the identity pose)"
    })
}

/// One entry of `hew.meta.documents` / the `hello` response
/// (docs/agents/HEW_API.md §4.2).
fn document_info_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "id": { "type": "string", "description": "doc_N, unique for the host's lifetime and never reused" },
            "title": { "type": "string" },
            "path": { "type": ["string", "null"], "description": "the file last opened from or saved to; null while unsaved" },
            "dirty": { "type": "boolean" },
            "document_id": { "type": ["string", "null"], "description": "the persistent UUID the document's file carries (lowercase, hyphenated); null until the document is first saved" },
            "label": { "type": "string", "description": "the client-chosen label given at hew.doc.new/open, if any" },
            "front": { "type": "boolean", "description": "this is the user's front-most document window" },
            "attached": { "type": "boolean", "description": "this is the connection's current document" }
        },
        "required": ["id", "title", "path", "dirty", "document_id"]
    })
}

/// `focus` on `hew.doc.new` / `hew.doc.open`.
fn window_focus_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "string",
        "enum": ["background", "front"],
        "description": "where the new window goes; background (the default) never takes the user's focus"
    })
}

/// `label` on `hew.doc.new` / `hew.doc.open`.
fn window_label_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "string",
        "description": "a name to find the document by later, resolvable by hew.doc.attach and listed by hew.meta.documents"
    })
}

/// The `hew.library` metadata fields every `hew.library.*` result carries
/// (docs/agents/HEW_API.md §8.1) — shared by `hew.library.update_meta`'s
/// result and, embedded, by [`library_item_schema`]'s `summary`-adjacent
/// meta fields, so the two never drift into two different shapes for the
/// same dictionary.
fn library_item_meta_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "id": { "type": "string" },
            "name": { "type": "string" },
            "category": { "type": "string", "enum": ["component", "material", "model"] },
            "keywords": { "type": "array", "items": { "type": "string" } },
            "collection": { "type": "string" },
            "saved_at": { "type": "string" }
        },
        "required": ["name", "category", "keywords"]
    })
}

/// One `hew.library.list`/`describe` item row: the resolved metadata
/// fields plus the file's own stats and manifest summary.
fn library_item_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "path": { "type": "string" },
            "id": { "type": "string" },
            "name": { "type": "string" },
            "category": { "type": "string", "enum": ["component", "material", "model"] },
            "keywords": { "type": "array", "items": { "type": "string" } },
            "collection": { "type": "string" },
            "saved_at": { "type": "string" },
            "size": { "type": "integer" },
            "mtime_ms": { "type": "integer" },
            "summary": { "type": "object", "description": "manifest counts (objects, materials, components, ...); absent when error is present" },
            "error": { "type": "string", "description": "present only when the file failed to parse" }
        },
        "required": ["path", "name", "category", "keywords", "size", "mtime_ms"]
    })
}

impl Registry {
    /// Builds the protocol-1 inventory (docs/agents/HEW_API.md §7).
    pub fn protocol_1() -> Registry {
        let mut commands = BTreeMap::new();
        let mut add = |name: &'static str,
                       class: CommandClass,
                       served: Served,
                       tier: Tier,
                       summary: &'static str| {
            commands.insert(
                name,
                Draft {
                    name,
                    version: 1,
                    class,
                    served,
                    tier,
                    // Fail closed until `permission_table` assigns it;
                    // `every_command_has_a_permission` pins that it does.
                    permission: Permission::FirstParty,
                    summary,
                    params_schema: obj(),
                    result_schema: obj(),
                    refusals: vec!["unimplemented"],
                    implemented: false,
                    // The common case; the solitary commands that DO
                    // change the document are corrected just below.
                    mutates_document: matches!(class, CommandClass::ModelMutating),
                },
            );
        };
        use CommandClass::{ModelMutating as M, ReadOnly as R, Solitary as S};
        use Served::{Host, Kernel};
        use Tier::{Required as Req, Standard as Std};

        // hew.meta (§7) — the handshake and introspection surface.
        add(
            "hew.meta.hello",
            R,
            Kernel,
            Req,
            "Open the connection: negotiate protocol and encoding, learn the granted profile and open documents.",
        );
        add(
            "hew.meta.capabilities",
            R,
            Kernel,
            Req,
            "The registry as data: every granted command's schemas, summary, and refusal inventory.",
        );
        // hew.event (§4.5) — server-originated notifications.
        add(
            "hew.event.subscribe",
            R,
            Kernel,
            Std,
            "Receive the named events on this connection as notification frames. Names the grant may not receive come back in `denied`.",
        );
        add(
            "hew.event.unsubscribe",
            R,
            Kernel,
            Std,
            "Stop receiving the named events on this connection.",
        );
        add(
            "hew.meta.documents",
            R,
            Host,
            Req,
            "The host's open documents: id, title, path, dirty, and — in the desktop app — label, front, and attached.",
        );
        // hew.doc — lifecycle and the transaction container.
        add(
            "hew.doc.attach",
            S,
            Kernel,
            Req,
            "Bind this connection to one open document, named by `document` (id, path, label, or title; see hew.meta.documents). With no `document`, the connection's current document.",
        );
        add(
            "hew.doc.transact",
            M,
            Kernel,
            Req,
            "Execute commands in order, atomically, as one labeled undo entry.",
        );
        add(
            "hew.doc.new",
            S,
            Host,
            Req,
            "Create a fresh document. The desktop app opens it in a new window — behind the user's current one unless `focus` says otherwise — and attaches the connection to it; headless hosts replace the working document.",
        );
        add(
            "hew.doc.open",
            S,
            Host,
            Req,
            "Open a .hew or SketchUp .skp document. The desktop app opens it in a new window (or attaches to the window that already has that file open), behind the user's current window unless `focus` says otherwise; headless hosts replace the working document.",
        );
        add(
            "hew.doc.close",
            S,
            Host,
            Std,
            "Close an open document window (the attached one, or the named `document`). Refuses unsaved changes unless `discard` is set.",
        );
        add(
            "hew.doc.focus",
            S,
            Host,
            Std,
            "Raise a document's window to the front. Every other command leaves the user's focus alone; this is the explicit opt-in.",
        );
        add("hew.doc.save", S, Host, Req, "Save the attached document.");
        add(
            "hew.doc.revert",
            S,
            Host,
            Std,
            "Throw away every change since the last save: re-read the saved file into the attached document.",
        );
        add(
            "hew.doc.export",
            S,
            Host,
            Req,
            "Export the attached document (STL/3MF/glTF/USDZ, or SketchUp skp, skp3 to skp8, skp2013 to skp2020); bytes base64, or a path on hosts with filesystem access.",
        );
        add(
            "hew.doc.import",
            S,
            Host,
            Std,
            "Merge a foreign-format file (STL, glTF/GLB, COLLADA, SketchUp) into the attached document through the shared healing pipeline, or place a PNG/JPEG picture (a component holding one face painted with it, at origin/u/v, width meters wide). Additive: existing geometry is untouched.",
        );
        add(
            "hew.doc.purge_unused",
            M,
            Kernel,
            Std,
            "Delete every unused material and component definition, as one undo entry.",
        );
        add(
            "hew.doc.health",
            R,
            Kernel,
            Std,
            "Model health: every open object (world or component member) with its verdict, open rims and seam family, and counts by kind.",
        );
        // hew.query — the read surface.
        add(
            "hew.query.scene",
            R,
            Kernel,
            Req,
            "The document tree with per-entity summaries.",
        );
        add("hew.query.entity", R, Kernel, Req, "One entity's details.");
        add(
            "hew.query.faces",
            R,
            Kernel,
            Req,
            "A solid's faces: planes, areas, centroids, boundary loops.",
        );
        add(
            "hew.query.raycast",
            R,
            Kernel,
            Req,
            "First hit along a ray, or every crossing, through any depth of nesting — the programmatic form of clicking.",
        );
        add(
            "hew.query.measure",
            R,
            Kernel,
            Req,
            "Distances between points, edges, and faces; or the volume, area, and bounding box of a part, group, or instance.",
        );
        add(
            "hew.query.resolve",
            R,
            Kernel,
            Req,
            "Resolve any locator (point, face, edge) to its concrete value without mutating.",
        );
        add(
            "hew.query.context",
            R,
            Kernel,
            Req,
            "The open editing-context frame stack, with the app's fallback entries and whether a gesture is armed.",
        );
        add(
            "hew.query.occurrences",
            R,
            Kernel,
            Std,
            "Every placement in the model with its path and world pose: objects, groups, and instances, nested definitions expanded through each instance.",
        );
        add(
            "hew.query.part_frame",
            R,
            Kernel,
            Std,
            "A part measured along its own axes: origin, axes, and extents of one placement.",
        );
        add(
            "hew.query.signature",
            R,
            Kernel,
            Std,
            "Whether two objects are the same part or mirrored twins: one object's shape and mirror hashes, and its part frame.",
        );
        add(
            "hew.query.mesh",
            R,
            Kernel,
            Std,
            "One placement's triangle mesh: positions, normals, indices, and per-triangle face indices, base64.",
        );
        add(
            "hew.query.topology",
            R,
            Kernel,
            Std,
            "One object's vertices, faces, and edges with their adjacency, planes, and analytic curves.",
        );
        add(
            "hew.query.changes_since",
            R,
            Kernel,
            Std,
            "What changed since a history generation: the union of the change sets since then, or `full` when the history no longer reaches back that far.",
        );
        // hew.sketch — drawing at tool altitude.
        add(
            "hew.sketch.draw_line",
            M,
            Kernel,
            Req,
            "Draw a line (chain) on a plane spec.",
        );
        add(
            "hew.sketch.draw_rect",
            M,
            Kernel,
            Req,
            "Draw an axis-aligned rectangle on a plane spec.",
        );
        add(
            "hew.sketch.draw_circle",
            M,
            Kernel,
            Req,
            "Draw a circle on a plane spec.",
        );
        add(
            "hew.sketch.draw_arc",
            M,
            Kernel,
            Req,
            "Draw an arc on a plane spec.",
        );
        add(
            "hew.sketch.draw_polygon",
            M,
            Kernel,
            Req,
            "Draw a regular N-gon on a plane spec.",
        );
        add(
            "hew.sketch.offset",
            M,
            Kernel,
            Req,
            "Offset a region boundary within its sketch, or a face or plane region by its unit key.",
        );
        add(
            "hew.sketch.units",
            R,
            Kernel,
            Std,
            "List a plane's drawing as units: islands of edges (with where each is stored) and regions.",
        );
        add(
            "hew.sketch.move",
            M,
            Kernel,
            Std,
            "Move drawing units (islands, edges, circles, regions) on a plane by an offset.",
        );
        add(
            "hew.sketch.rotate",
            M,
            Kernel,
            Std,
            "Rotate drawing units on a plane about the plane normal through a point.",
        );
        add(
            "hew.sketch.scale",
            M,
            Kernel,
            Std,
            "Scale drawing units on a plane about a center, uniformly or per axis.",
        );
        add(
            "hew.sketch.copy",
            M,
            Kernel,
            Std,
            "Copy drawing units on a plane by an offset.",
        );
        add(
            "hew.sketch.delete",
            M,
            Kernel,
            Std,
            "Delete drawing units from a plane.",
        );
        // hew.solid — watertight-by-construction operations.
        add(
            "hew.solid.extrude",
            M,
            Kernel,
            Req,
            "Extrude a region into a new Object, consuming the profile.",
        );
        add(
            "hew.solid.push_pull",
            M,
            Kernel,
            Req,
            "Push/pull a face of a solid with the tool's full semantics.",
        );
        add(
            "hew.solid.cap",
            M,
            Kernel,
            Std,
            "Close an open object's flat openings with faces; an object closed all round becomes a solid.",
        );
        add(
            "hew.solid.delete_faces",
            M,
            Kernel,
            Std,
            "Remove faces from an open object; islands cut loose become objects of their own. Solids refuse.",
        );
        add(
            "hew.solid.mark_surface",
            M,
            Kernel,
            Std,
            "Mark an open object as a surface on purpose (or clear the mark); hew.doc.health keeps listing it, flagged accepted.",
        );
        add(
            "hew.solid.thicken",
            M,
            Kernel,
            Std,
            "Pull a flat surface into a solid of the given thickness.",
        );
        add(
            "hew.solid.rejoin",
            M,
            Kernel,
            Std,
            "Rejoin one seam family of split pieces as a single solid (Model Health's Rejoin as one solid); one undo entry.",
        );
        add(
            "hew.solid.from_faces",
            M,
            Kernel,
            Std,
            "Make a solid from drawn faces that enclose a space (Make Solid on sketches), first filling the faces the named sketches' lines can close; a shape closed against a solid grows it.",
        );
        add(
            "hew.solid.union",
            M,
            Kernel,
            Req,
            "Boolean union of two solids.",
        );
        add(
            "hew.solid.subtract",
            M,
            Kernel,
            Req,
            "Boolean subtraction of two solids; a may be a path to a definition member, cut through that placement so every placement shows the cut, by a world object or a transient prism or box.",
        );
        add(
            "hew.solid.intersect",
            M,
            Kernel,
            Req,
            "Boolean intersection of two solids.",
        );
        add(
            "hew.solid.slice",
            M,
            Kernel,
            Req,
            "Slice a solid by a plane into two solids; the object may be a path to a definition member, sliced in the definition.",
        );
        // hew.compute — pure computations on transient solids (HEW_API.md
        // §7 "Compute"): the document is read, never changed.
        add(
            "hew.compute.boolean",
            R,
            Kernel,
            Std,
            "Union, subtract, or intersect two solids without touching the document; answers the result's measure, mesh, topology, or shell count.",
        );
        add(
            "hew.compute.slice",
            R,
            Kernel,
            Std,
            "Cut a solid by a plane without touching the document; answers both pieces.",
        );
        add(
            "hew.compute.shells",
            R,
            Kernel,
            Std,
            "Split a solid into its connected pieces without touching the document.",
        );
        add(
            "hew.compute.measure",
            R,
            Kernel,
            Std,
            "Volume, surface area, bounding box, and centroid of a solid.",
        );
        add(
            "hew.compute.outline",
            R,
            Kernel,
            Std,
            "A part's outline in a frame's xy plane: its largest face, or its silhouette, with arcs and circles kept analytic.",
        );
        add(
            "hew.compute.offset",
            R,
            Kernel,
            Std,
            "Offset closed 2D loops outward or inward by a distance.",
        );
        add(
            "hew.compute.project",
            R,
            Kernel,
            Std,
            "A single part's flat drawing on a plane, its faces grouped by depth into layers (not available yet).",
        );
        add(
            "hew.compute.stretch",
            R,
            Kernel,
            Std,
            "Preview a section stretch of parts along one axis of a frame (not available yet).",
        );
        add(
            "hew.solid.follow_me",
            M,
            Kernel,
            Std,
            "Sweep a profile along an edge-chain path, as the tool does.",
        );
        // hew.solid — editable imprints, kept for compatibility: drawn units
        // on an object's drawing planes, implemented over the drafting
        // engine's plane units (hew.sketch.units/move/... are the general form).
        add(
            "hew.solid.imprints",
            R,
            Kernel,
            Std,
            "Kept for compatibility: list an object's drawn shapes (sub-faces and chords) per drawing plane; hew.sketch.units is the general form.",
        );
        add(
            "hew.solid.move_imprint",
            M,
            Kernel,
            Std,
            "Kept for compatibility: slide a drawn shape (sub-face or chord island) in its plane; hew.sketch.move is the general form.",
        );
        add(
            "hew.solid.rotate_imprint",
            M,
            Kernel,
            Std,
            "Kept for compatibility: turn a drawn shape about its plane's normal through a pivot; hew.sketch.rotate is the general form.",
        );
        add(
            "hew.solid.scale_imprint",
            M,
            Kernel,
            Std,
            "Kept for compatibility: uniformly scale a drawn shape about an anchor in its plane (a negative factor turns it 180 degrees about the anchor and scales it by |factor|); hew.sketch.scale is the general form, and a mirror about a world axis of an axis-aligned drawing plane is hew.sketch.scale with a negative per-axis factor.",
        );
        add(
            "hew.solid.delete_imprint",
            M,
            Kernel,
            Std,
            "Kept for compatibility: delete a drawn shape (sub-face or chord island) back into its face; hew.sketch.delete is the general form.",
        );
        // hew.entity — transforms and lifecycle.
        add("hew.entity.rename", M, Kernel, Req, "Rename an entity.");
        add("hew.entity.delete", M, Kernel, Req, "Delete an entity.");
        add(
            "hew.entity.move",
            M,
            Kernel,
            Req,
            "Translate (with copy/array forms) by vector or from→to points; ids may be paths to members of one definition, moved in world terms with every placement following.",
        );
        add(
            "hew.entity.rotate",
            M,
            Kernel,
            Req,
            "Rotate about a pivot and axis by an angle; ids may be paths to members of one definition, turned in world terms with every placement following.",
        );
        add(
            "hew.entity.flip",
            M,
            Kernel,
            Std,
            "Mirror a selection across a plane (Flip Along an axis through its center, the origin, or a point; or a given plane), in place or as one mirrored copy that links by default.",
        );
        add(
            "hew.entity.scale",
            M,
            Kernel,
            Req,
            "Scale about an anchor with per-axis factors; a negative factor mirrors. ids may be paths to members of one definition, scaled in world terms with every placement following.",
        );
        add(
            "hew.entity.set_frame",
            M,
            Kernel,
            Std,
            "Set or clear the part frame override (hew.part / frame) of an object or group.",
        );
        // hew.context — the shared editing-context stack (§6.3).
        add(
            "hew.context.enter",
            M,
            Kernel,
            Req,
            "Open a group/component editing frame (transaction-balanced only).",
        );
        add(
            "hew.context.exit",
            M,
            Kernel,
            Req,
            "Close the innermost frame this envelope opened.",
        );
        // hew.group / hew.component — composition.
        add(
            "hew.group.create",
            M,
            Kernel,
            Req,
            "Group sibling nodes non-destructively.",
        );
        add(
            "hew.group.explode",
            M,
            Kernel,
            Req,
            "Dissolve a group, re-homing its members.",
        );
        add(
            "hew.group.reparent",
            M,
            Kernel,
            Std,
            "Move live nodes into a group, or out to the top level, without touching their geometry.",
        );
        add(
            "hew.component.create",
            M,
            Kernel,
            Std,
            "Fold a selection into a definition plus one instance.",
        );
        add(
            "hew.component.place",
            M,
            Kernel,
            Std,
            "Place an instance of a definition at a pose.",
        );
        add(
            "hew.component.import_definition",
            S,
            Host,
            Std,
            "Turn a file's bytes (hew, skp, glb, gltf, stl, dae) into one unplaced component definition, reusing the definition an earlier import of the same bytes made. Place it with hew.component.place.",
        );
        add(
            "hew.component.make_unique",
            M,
            Kernel,
            Std,
            "Give one placement its own private definition (through an occurrence path at any depth), with the old-to-new id map.",
        );
        add(
            "hew.component.promote",
            M,
            Kernel,
            Std,
            "Make a set of linked copies a named component: removes its linked mark and names its definition.",
        );
        add(
            "hew.component.explode",
            M,
            Kernel,
            Std,
            "Bake an instance into world geometry.",
        );
        add(
            "hew.component.rebase",
            M,
            Kernel,
            Std,
            "Re-express a definition in a new frame; every placement is compensated, so nothing moves on screen.",
        );
        add(
            "hew.component.flip_instance",
            M,
            Kernel,
            Std,
            "Mirror one world instance across one of its definition's local axes.",
        );
        // hew.material / hew.tag / hew.guide — appearance and organization.
        add(
            "hew.material.create",
            M,
            Kernel,
            Std,
            "Add a color or texture material to the palette. Registry-state: records no undo entry (§6.4).",
        );
        add(
            "hew.material.paint",
            M,
            Kernel,
            Std,
            "Paint a face or entity.",
        );
        add(
            "hew.material.set_default",
            M,
            Kernel,
            Std,
            "Set an object's default material.",
        );
        add(
            "hew.material.set_opacity",
            M,
            Kernel,
            Std,
            "Set a material's opacity.",
        );
        add(
            "hew.material.set_pbr",
            M,
            Kernel,
            Std,
            "Set a material's metallic and roughness factors (0 to 1); its maps and strengths are kept.",
        );
        add(
            "hew.material.position_texture",
            M,
            Kernel,
            Std,
            "Position a face's texture by three tile-corner pins, or reset it to the default projection.",
        );
        add(
            "hew.material.replace",
            M,
            Kernel,
            Std,
            "Replace one material with another everywhere it is painted, or within one object.",
        );
        add(
            "hew.tag.create",
            M,
            Kernel,
            Std,
            "Register a tag path. Registry-state: records no undo entry (§6.4).",
        );
        add("hew.tag.assign", M, Kernel, Std, "Assign a tag to nodes.");
        add(
            "hew.tag.set_visible",
            M,
            Kernel,
            Std,
            "Toggle a tag's visibility. Registry-state: records no undo entry (§6.4).",
        );
        add(
            "hew.tag.delete",
            M,
            Kernel,
            Std,
            "Delete a tag path, unassigning it everywhere.",
        );
        add(
            "hew.tag.rename",
            M,
            Kernel,
            Std,
            "Rename a tag path (and every tag nested under it), keeping its identity.",
        );
        add(
            "hew.guide.line",
            M,
            Kernel,
            Std,
            "Add an infinite construction guide line.",
        );
        add(
            "hew.guide.point",
            M,
            Kernel,
            Std,
            "Add a construction guide point.",
        );
        add(
            "hew.guide.angular",
            M,
            Kernel,
            Std,
            "Add an angular construction guide.",
        );
        add("hew.guide.clear", M, Kernel, Std, "Delete all guides.");
        // hew.scenes — named, saved views (docs/agents/HEW_API.md's Scenes
        // section; docs/design/scenes.md §3, §7). Every command but
        // `list` is `ModelMutating` (may ride a transaction,
        // `mutates_document = true`) but NONE of it is undoable — same
        // registry-state posture as `hew.tag.create`/`set_visible` above:
        // the kernel calls behind these never record an op, so the
        // compound entry a transaction commits ends up empty.
        add(
            "hew.scenes.list",
            R,
            Kernel,
            Std,
            "Every Scene, in tab order.",
        );
        add(
            "hew.scenes.add",
            M,
            Kernel,
            Std,
            "Add a Scene capturing the document's current view state. Records no undo entry (§6.4).",
        );
        add(
            "hew.scenes.update",
            M,
            Kernel,
            Std,
            "Re-capture a Scene's properties from the document's current state, and/or set its own transition and delay times. Records no undo entry (§6.4).",
        );
        add(
            "hew.scenes.rename",
            M,
            Kernel,
            Std,
            "Rename a Scene. Records no undo entry (§6.4).",
        );
        add(
            "hew.scenes.describe",
            M,
            Kernel,
            Std,
            "Set a Scene's free-text description. Records no undo entry (§6.4).",
        );
        add(
            "hew.scenes.remove",
            M,
            Kernel,
            Std,
            "Delete a Scene. Records no undo entry (§6.4).",
        );
        add(
            "hew.scenes.reorder",
            M,
            Kernel,
            Std,
            "Move a Scene to a new position in tab order. Records no undo entry (§6.4).",
        );
        add(
            "hew.scenes.apply",
            M,
            Kernel,
            Std,
            "Apply a Scene: write its captured camera/hidden-set/section state into the document. Records no undo entry (§6.4).",
        );
        // hew.attr — attribute dictionaries (§8).
        add(
            "hew.attr.get",
            R,
            Kernel,
            Req,
            "Read a target's attribute dictionaries.",
        );
        add("hew.attr.set", M, Kernel, Req, "Write one attribute key.");
        add(
            "hew.attr.delete",
            M,
            Kernel,
            Req,
            "Delete one attribute key or a whole namespace.",
        );
        // hew.geo — the document's coordinate reference system.
        add(
            "hew.geo.get",
            R,
            Kernel,
            Std,
            "The document's geo-reference (coordinate reference system), or null when it has none.",
        );
        add(
            "hew.geo.set",
            M,
            Kernel,
            Std,
            "Set the document's geo-reference to the given record, or clear it with null. One undo step.",
        );
        // hew.history — the shared undo history (§7 semantics notes).
        add(
            "hew.history.undo",
            S,
            Kernel,
            Req,
            "Undo the top history entry (optionally guarded by expected_label).",
        );
        add(
            "hew.history.redo",
            S,
            Kernel,
            Req,
            "Redo the most recently undone entry.",
        );
        add(
            "hew.history.status",
            S,
            Kernel,
            Req,
            "History depth, the top entry's label and origin, the saved depth, and every undo/redo entry's label and origin.",
        );
        // hew.view — live-application surface (app profile only, except
        // snapshot's headless carve-out below).
        add(
            "hew.view.snapshot",
            S,
            Host,
            Std,
            "Render the attached document to PNG through the host's viewport.",
        );
        add(
            "hew.view.camera",
            S,
            Host,
            Std,
            "Set the live viewport's camera — the same camera/view vocabulary hew.view.snapshot accepts.",
        );
        add(
            "hew.view.zoom_extents",
            S,
            Host,
            Std,
            "Frame all visible geometry in the live viewport.",
        );
        add(
            "hew.view.units",
            S,
            Host,
            Std,
            "Set the app's displayed length-unit format — a display preference, not document state.",
        );
        // Selection and view state (docs/agents/HEW_API.md §7): ReadOnly, so
        // they ride a transaction and roll back with it, and record no undo
        // entry.
        add(
            "hew.selection.get",
            R,
            Host,
            Std,
            "The app's selection: public ids in pick order, the primary first.",
        );
        add(
            "hew.selection.set",
            R,
            Host,
            Std,
            "Change the app's selection: replace it, or add, subtract, or toggle the named entities.",
        );
        add(
            "hew.selection.clear",
            R,
            Host,
            Std,
            "Clear the app's selection.",
        );
        add(
            "hew.view.hide",
            R,
            Kernel,
            Std,
            "Hide objects, groups, and component instances (view state, never undoable).",
        );
        add(
            "hew.view.show",
            R,
            Kernel,
            Std,
            "Show hidden objects, groups, and component instances again.",
        );
        add(
            "hew.view.show_all",
            R,
            Kernel,
            Std,
            "Show every hidden object, group, and component instance.",
        );
        add(
            "hew.view.hidden",
            R,
            Kernel,
            Std,
            "What is hidden: the hidden nodes, the hidden tags, and the live viewport's isolation.",
        );
        add(
            "hew.view.isolate",
            R,
            Host,
            Std,
            "Show only the named nodes or definition members in the live viewport, and frame them.",
        );
        add(
            "hew.view.clear_isolation",
            R,
            Host,
            Std,
            "End the live viewport's isolation.",
        );
        add(
            "hew.view.zoom_to",
            R,
            Host,
            Std,
            "Frame the named entities in the live viewport, with a margin.",
        );
        // The user's editing context (docs/agents/HEW_API.md §6.3): host
        // effects on the user's window, outside transactions. Solitary, so
        // a multi-command transaction refuses them -32602; `mutates_document`
        // is corrected below because opening or closing a session is a
        // history step the live app resyncs after.
        add(
            "hew.view.enter",
            S,
            Host,
            Std,
            "Put the user's window inside a node for editing, the way a double click in the Outliner does.",
        );
        add(
            "hew.view.exit",
            S,
            Host,
            Std,
            "Step the user's window out of its innermost editing context.",
        );
        add(
            "hew.view.exit_all",
            S,
            Host,
            Std,
            "Step the user's window all the way out to the top level.",
        );
        // Printing (docs/design/printing.md §9b): headless-renderable like
        // hew.view.snapshot (the same core-profile carve-out below).
        add(
            "hew.view.line_drawing",
            S,
            Host,
            Std,
            "Hidden-line drawing of the visible document from a camera — SVG at a drawing scale, or raw segments.",
        );
        add(
            "hew.print.pdf",
            S,
            Host,
            Std,
            "Print the document to a PDF: standard (one page) or to an exact drawing scale across tiled pages, vector line art or shaded.",
        );
        // hew.library — the Hew Library (docs/agents/HEW_API.md §8.1;
        // docs/design/v1.1-cycle.md's Lane B): a folder of `.hew` items a
        // host with a filesystem offers, addressed by id or relative path.
        // `list`/`describe`/`remove`/`update_meta` never touch the
        // document (pure host-side file operations); `insert` mutates it
        // through the same `Document::insert_document` the UI's own
        // library-insert flow uses; `save` extracts a selection (or the
        // whole document) read-only and writes it out, stamping
        // provenance on the ORIGINAL document's source nodes — a real
        // document mutation, but not an undoable one (see its
        // `mutates_document` correction below), so it stays `ReadOnly`
        // class like `hew.scenes.*` above.
        add(
            "hew.library.list",
            R,
            Host,
            Std,
            "List items in the Hew Library, optionally filtered by category, collection, or a text query.",
        );
        add(
            "hew.library.describe",
            R,
            Host,
            Std,
            "One library item's full metadata, manifest summary, and document attributes.",
        );
        add(
            "hew.library.insert",
            M,
            Kernel,
            Std,
            "Insert a library item into the attached document, wired for idempotent re-insert exactly like the UI's own placement; a material item goes into the palette instead, recording no undo entry (§6.4).",
        );
        add(
            "hew.library.save",
            R,
            Kernel,
            Std,
            "Save a selection, one material, or the whole document to the library. Records no undo entry (§6.4).",
        );
        add("hew.library.remove", R, Host, Std, "Delete a library item.");
        add(
            "hew.library.update_meta",
            R,
            Host,
            Std,
            "Edit a library item's name, keywords, or collection in place.",
        );

        // The connection-lifecycle commands the dispatcher already
        // implements: the burn-down flag must tell the truth, and their
        // refusal inventories are honestly empty (their failures are
        // protocol errors, not refusals).
        for name in ["hew.meta.hello", "hew.meta.capabilities", "hew.doc.attach"] {
            let cmd = commands.get_mut(name).expect("declared above");
            cmd.implemented = true;
            cmd.refusals = Vec::new();
        }
        for name in ["hew.event.subscribe", "hew.event.unsubscribe"] {
            let cmd = commands.get_mut(name).expect("declared above");
            cmd.implemented = true;
            cmd.refusals = Vec::new();
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "events": {
                        "type": "array",
                        "items": { "type": "string", "enum": crate::events::EVENT_NAMES },
                        "minItems": 1,
                        "maxItems": 32
                    }
                },
                "required": ["events"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "subscribed": { "type": "array", "items": { "type": "string" } },
                    "denied": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["subscribed", "denied"],
                "additionalProperties": false
            });
        }
        {
            let cmd = commands
                .get_mut("hew.query.changes_since")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "generation": { "type": "integer", "minimum": 0 } },
                "required": ["generation"],
                "additionalProperties": false
            });
            let ids = serde_json::json!({ "type": "array", "items": { "type": "string" } });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "generation": { "type": "integer" },
                    "epoch": { "type": "integer" },
                    "from": { "type": "integer" },
                    "changes": {
                        "type": "object",
                        "properties": {
                            "added": ids.clone(),
                            "removed": ids.clone(),
                            "modified": ids,
                            "attrs": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "id": { "type": "string" },
                                        "ns": { "type": "string" },
                                        "scope": { "enum": ["entity", "face", "edge"] }
                                    },
                                    "required": ["id", "ns", "scope"],
                                    "additionalProperties": false
                                }
                            },
                            "document": { "type": "boolean" },
                            "full": { "type": "boolean" },
                            "linked": {
                                "type": "array",
                                "description": "linked-copy wraps made in the range: the source, the placement that now stands for it, and the shared definition",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "source": { "type": "string" },
                                        "instance": { "type": "string" },
                                        "definition": { "type": "string" }
                                    },
                                    "required": ["source", "instance", "definition"],
                                    "additionalProperties": false
                                }
                            },
                            "copied_from": {
                                "type": "array",
                                "description": "[new, source] pairs of public ids for entities copied in the range",
                                "items": {
                                    "type": "array",
                                    "items": { "type": "string" },
                                    "minItems": 2,
                                    "maxItems": 2
                                }
                            }
                        },
                        "required": ["added", "removed", "modified", "attrs", "document", "full", "linked", "copied_from"],
                        "additionalProperties": false
                    }
                },
                "required": ["generation", "epoch", "from", "changes"],
                "additionalProperties": false
            });
            cmd.refusals = Vec::new();
        }
        {
            let cmd = commands.get_mut("hew.meta.hello").expect("declared above");
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "protocol": { "type": "integer" },
                    "token": { "type": "string" },
                    "client": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "version": { "type": "string" }
                        }
                    },
                    "encodings": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["protocol"]
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "protocol": { "type": "integer" },
                    "app": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "version": { "type": "string" }
                        },
                        "required": ["name", "version"]
                    },
                    "profile": { "type": "string", "enum": ["core", "app", "plugin"] },
                    "grant": { "type": "object" },
                    "encoding": { "type": "string" },
                    "documents": { "type": "array", "items": document_info_schema() }
                },
                "required": ["protocol", "app", "profile", "encoding", "documents"]
            });
        }
        {
            let cmd = commands
                .get_mut("hew.meta.capabilities")
                .expect("declared above");
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "commands": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string" },
                                "version": { "type": "integer" },
                                "summary": { "type": "string" },
                                "class": {
                                    "type": "string",
                                    "enum": ["model_mutating", "read_only", "solitary"]
                                },
                                "params": { "type": "object" },
                                "result": { "type": "object" },
                                "refusals": { "type": "array", "items": { "type": "string" } },
                                "implemented": { "type": "boolean" }
                            },
                            "required": [
                                "name", "version", "summary", "class",
                                "params", "result", "refusals", "implemented"
                            ]
                        }
                    }
                },
                "required": ["commands"]
            });
        }
        {
            // Single-document hosts attach with no params; multi-document
            // hosts name the target (§4.2). The result is deliberately an
            // empty object — attachment is connection state, not data.
            let cmd = commands.get_mut("hew.doc.attach").expect("declared above");
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "document": {
                        "type": "string",
                        "description": "a document id (doc_N), or its exact path, label, or title when unambiguous"
                    }
                },
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "document": document_info_schema() }
            });
            cmd.refusals = vec![
                "unknown_document",
                "ambiguous_document",
                "document_not_granted",
            ];
        }
        // The transaction container itself (dispatch-inline, §6): its own
        // refusals are the runtime $ref failure plus whatever its inner
        // commands refuse (reported per command in the canonical shape).
        // `plugin_command_in_transaction` is not declared here: like an
        // inner command's refusal it is attributed to a command, the plugin
        // command's index, and declaring it would change first-party
        // capabilities and the published reference (HEW_API.md §6.4).
        {
            let cmd = commands
                .get_mut("hew.doc.transact")
                .expect("declared above");
            cmd.implemented = true;
            cmd.refusals = vec![
                "ref_resolution_failed",
                "generation_moved",
                "dry_run_host_command",
                "compute_busy",
            ];
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "label": { "type": "string" },
                    "dry_run": {
                        "type": "boolean",
                        "description": "rehearse: run the envelope on a copy of the document and answer what a commit would, changing nothing (docs/agents/HEW_API.md §6.6); needs only read permissions; no coalesce"
                    },
                    "expect_generation": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "refuse with generation_moved, before any command runs, unless the document's history generation is this value (docs/agents/HEW_API.md §6.5)"
                    },
                    "coalesce": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": 64,
                        "description": "merge into the top undo entry when it was committed under the same key by a connection with this connection's identity (docs/agents/HEW_API.md §6.5)"
                    },
                    "commands": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "method": { "type": "string" },
                                "as": { "type": "string" },
                                "params": { "type": "object" }
                            },
                            "required": ["method"]
                        }
                    }
                },
                "required": ["commands"]
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "results": { "type": "array" },
                    "label": { "type": "string" },
                    "dry_run": { "type": "boolean", "description": "true on a rehearsal's answer" }
                },
                "required": ["results", "label"]
            });
        }

        // hew.query.* and hew.meta.documents — Wave A (docs/design/
        // api-implementation-conventions.md): honest schemas and refusal
        // inventories for the read surface `crates/api/src/commands/
        // query.rs` implements.
        {
            let cmd = commands
                .get_mut("hew.meta.documents")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "documents": { "type": "array", "items": document_info_schema() }
                },
                "required": ["documents"]
            });
            cmd.refusals = Vec::new();
        }
        {
            let cmd = commands.get_mut("hew.query.scene").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "document": { "type": "object" },
                    "tree": { "type": "array" },
                    "sketches": { "type": "array", "description": "each sketch's id, plane, regions, curves and edges; a detached sketch (a shape drawn with `detached`, in no plane's arrangement) also carries \"detached\": true" },
                    "guides": { "type": "array" },
                    "materials": { "type": "array" },
                    "tags": { "type": "array" },
                    "components": { "type": "array" }
                },
                "required": ["document", "tree", "sketches", "guides", "materials", "tags", "components"]
            });
            cmd.refusals = Vec::new();
        }
        {
            let cmd = commands
                .get_mut("hew.query.entity")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "any public id, including a sketch's own edge id (\"edg_…\", HEW_API.md §5.2) — a sketch's `hew.query.scene`/`hew.query.entity` listing hands these out, and this command answers them directly with `{kind:\"edge\", sketch, from, to, length, curve}`"
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "kind": { "type": "string" }, "id": { "type": "string" } },
                "required": ["kind", "id"]
            });
            cmd.refusals = vec!["unknown_entity"];
        }
        {
            let cmd = commands.get_mut("hew.query.faces").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "object": { "type": "string", "description": "an object id; give this or path" },
                    "path": { "type": "array", "items": { "type": "string" }, "minItems": 1, "description": "an occurrence path ending at an object ([...instances, object]); each face then also reports material_effective through it" }
                },
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "object": { "type": "string" },
                    "faces": { "type": "array", "description": "per face: object, plane, area, centroid, outer, holes, material, surface, texture_pins, attrs (the face's attribute dictionary, namespace -> key -> value, {} for none), and material_effective when asked through a path" }
                },
                "required": ["object", "faces"]
            });
            cmd.refusals = vec!["unknown_entity", "unknown_occurrence"];
        }
        {
            let cmd = commands
                .get_mut("hew.query.raycast")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "origin": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "dir": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "all": { "type": "boolean", "description": "list every face crossing along the ray, nearest first (front and back faces both), instead of only the nearest; an empty list is a success" },
                    "max_distance": { "type": "number", "exclusiveMinimum": 0, "description": "meters along the ray; farther crossings are dropped" },
                    "exclude": {
                        "type": "array",
                        "maxItems": 64,
                        "items": {
                            "oneOf": [
                                { "type": "string" },
                                {
                                    "type": "object",
                                    "properties": { "path": { "type": "array", "items": { "type": "string" }, "minItems": 1 } },
                                    "required": ["path"],
                                    "additionalProperties": false
                                }
                            ]
                        },
                        "description": "world node ids or occurrence locators {path}; each entry and everything under it is ignored by the ray"
                    },
                    "include_hidden": { "type": "boolean", "description": "let the ray hit parts that are effectively hidden (user-hidden or on a hidden tag); each hit reports hidden. Default false: hidden parts fall through, as rendering does" }
                },
                "required": ["origin", "dir"],
                "additionalProperties": false
            });
            let hit = serde_json::json!({
                "type": "object",
                "properties": {
                    "object": { "type": "string", "description": "the world object's or, for a hit inside an instance, the outermost instance's public id" },
                    "kind": { "type": "string", "enum": ["object", "instance"] },
                    "point": { "type": "array" },
                    "distance": { "type": "number" },
                    "normal": { "type": "array" },
                    "path": { "type": "array", "items": { "type": "string" }, "description": "the hit part's occurrence path without the part itself (the form hew.query.occurrences uses); [] for a world object" },
                    "leaf": { "type": "string", "description": "the hit Object's public id" },
                    "face": { "type": "object", "description": "a face locator {object, at} with at in the leaf Object's own frame; feed it to hew.query.resolve, hew.material.paint, or hew.solid.push_pull" },
                    "hidden": { "type": "boolean", "description": "effective visibility including tags; false unless include_hidden" }
                },
                "required": ["object", "kind", "point", "distance", "normal", "path", "leaf", "face", "hidden"]
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "oneOf": [
                    hit.clone(),
                    {
                        "type": "object",
                        "properties": {
                            "hits": { "type": "array", "items": hit, "maxItems": 1000 },
                            "raycast_hits_capped": { "type": "boolean", "description": "present and true when more than 1,000 crossings were found and the farthest were dropped" }
                        },
                        "required": ["hits"]
                    }
                ]
            });
            cmd.refusals = vec![
                "locator_missed",
                "ambiguous_locator",
                "unknown_entity",
                "unknown_occurrence",
                "too_many_occurrences",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.query.measure")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "oneOf": [
                    {
                        "type": "object",
                        "properties": { "from": {}, "to": {} },
                        "required": ["from", "to"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": { "id": { "type": "string", "description": "a world object, group, or instance; a definition member needs path" }, "scope": scope_schema(), "exclude": exclude_schema() },
                        "required": ["id"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": { "path": { "type": "array", "items": { "type": "string" }, "minItems": 1, "description": "an occurrence path [...instances, node]" }, "scope": scope_schema(), "exclude": exclude_schema() },
                        "required": ["path"],
                        "additionalProperties": false
                    }
                ]
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "oneOf": [
                    {
                        "type": "object",
                        "properties": { "distance": { "type": "number" }, "delta": { "type": "array" } },
                        "required": ["distance", "delta"]
                    },
                    {
                        "type": "object",
                        "description": "meters, square meters, cubic meters; groups and instances sum their parts. extents are the sizes of the world-axis-aligned bbox (part-frame extents are hew.query.part_frame's). volume is null when no part is watertight; open parts add nothing to it and are counted in open_leaves",
                        "properties": {
                            "volume": { "type": ["number", "null"] },
                            "area": { "type": "number" },
                            "bbox": {
                                "type": "object",
                                "properties": { "min": { "type": "array" }, "max": { "type": "array" } },
                                "required": ["min", "max"]
                            },
                            "extents": { "type": "array" },
                            "leaf_count": { "type": "integer" },
                            "open_leaves": { "type": "integer" }
                        },
                        "required": ["volume", "area", "bbox", "extents", "leaf_count", "open_leaves"]
                    }
                ]
            });
            cmd.refusals = vec![
                "unknown_entity",
                "unknown_occurrence",
                "locator_missed",
                "ambiguous_locator",
                "no_such_point",
                "too_many_occurrences",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.query.resolve")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "point": {},
                    "face": {},
                    "edge": {
                        "description": "HEW_API.md §5.2's edge locator: a solid edge by {object,at}, a sketch edge's own public id (\"edg_…\") as a bare string, or a sketch edge by {sketch,at} / {sketch,from,to}"
                    },
                    "occurrence": {
                        "type": "object",
                        "description": "an occurrence locator {path: [instance, …, leaf]}; answers {occurrence: OccurrenceJson without members, solid, family, or linked}"
                    }
                },
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "description": "a resolved \"edge\" carries \"kind\": \"solid\" ({object,from,to}) or \"sketch\" ({id,sketch,from,to,curve})"
            });
            cmd.refusals = vec![
                "unknown_entity",
                "locator_missed",
                "ambiguous_locator",
                "no_such_point",
                "face_token_unknown",
                "face_token_stale",
                "unknown_occurrence",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.query.context")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "stack": { "type": "array" },
                    "direct_members": {},
                    "fallback": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "kind": { "type": "string", "enum": ["object", "instance"] },
                                "id": { "type": "string" }
                            },
                            "required": ["kind", "id"],
                            "additionalProperties": false
                        },
                        "description": "the app's in-context entries inside the innermost open frame, outermost first; empty headless"
                    },
                    "armed": { "type": "boolean", "description": "a tool gesture is armed in the user's window; false headless" }
                },
                "required": ["stack", "fallback", "armed"]
            });
            cmd.refusals = Vec::new();
        }

        {
            let cmd = commands
                .get_mut("hew.query.occurrences")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "roots": {
                        "type": "array",
                        "minItems": 1,
                        "items": occurrence_locator_schema(),
                        "description": "where to start: node ids (world nodes) or occurrence locators {path}; absent is the whole world. Rows come in root order, each root's subtree depth-first, a placement listed once even when two roots reach it"
                    },
                    "kinds": {
                        "type": "array",
                        "items": { "type": "string", "enum": ["object", "group", "instance"] },
                        "description": "which rows to return (never what is traversed); defaults to all three"
                    },
                    "include_hidden": { "type": "boolean", "description": "defaults to false: hidden placements are skipped and hidden instances are not descended" }
                },
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "occurrences": { "type": "array", "items": occurrence_json_schema() }
                },
                "required": ["occurrences"],
                "additionalProperties": false
            });
            cmd.refusals = vec![
                "unknown_entity",
                "unknown_occurrence",
                "too_many_occurrences",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.query.part_frame")
                .expect("declared above");
            cmd.implemented = true;
            let mut params = occurrence_target_schema(false);
            params["properties"]["scope"] = scope_schema();
            params["properties"]["exclude"] = exclude_schema();
            cmd.params_schema = params;
            let vec3 = serde_json::json!({ "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "origin": vec3,
                    "x": vec3,
                    "y": vec3,
                    "z": vec3,
                    "extents": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "along x, y, z: [length, width, thickness] for a face frame; the definition's own axis order for an instance" },
                    "source": { "type": "string", "enum": ["attribute", "definition", "face"] },
                    "mirrored": { "type": "boolean" }
                },
                "required": ["origin", "x", "y", "z", "extents", "source", "mirrored"],
                "additionalProperties": false
            });
            cmd.refusals = vec![
                "unknown_entity",
                "unknown_occurrence",
                "no_part_frame",
                "too_many_occurrences",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.query.signature")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = occurrence_target_schema(false);
            let vec3 = serde_json::json!({ "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 });
            let hash = serde_json::json!({ "type": "string", "pattern": "^[0-9a-f]{64}$", "description": "SHA-256, lowercase hex" });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "shape": hash.clone(),
                    "mirror": { "type": "string", "pattern": "^[0-9a-f]{64}$", "description": "the shape hash of the object's mirror image: a.shape == b.mirror means a and b are mirrored twins" },
                    "symmetric": { "type": "boolean", "description": "shape == mirror: the part is its own mirror image" },
                    "frame": {
                        "type": "object",
                        "description": "the part's face frame (l ≥ w ≥ t, right-handed), as hew.query.part_frame reports it without a hew.part / frame override; origin is the part's minimum corner in that frame and extents its size along x, y, z, so the frame's center is origin + extents / 2 along the axes",
                        "properties": { "origin": vec3.clone(), "x": vec3.clone(), "y": vec3.clone(), "z": vec3.clone(), "extents": vec3 },
                        "required": ["origin", "x", "y", "z", "extents"],
                        "additionalProperties": false
                    }
                },
                "required": ["shape", "mirror", "symmetric", "frame"],
                "additionalProperties": false
            });
            cmd.refusals = vec![
                "unknown_entity",
                "unknown_occurrence",
                "not_an_object",
                "no_part_frame",
                "too_many_occurrences",
            ];
        }
        {
            let cmd = commands.get_mut("hew.query.mesh").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = occurrence_target_schema(true);
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "vertex_count": { "type": "integer", "minimum": 0 },
                    "triangle_count": { "type": "integer", "minimum": 0 },
                    "positions_base64": { "type": "string", "description": "f32 little-endian xyz per vertex" },
                    "normals_base64": { "type": "string", "description": "f32 little-endian xyz per vertex, unit length" },
                    "indices_base64": { "type": "string", "description": "u32 little-endian, three per triangle, counter-clockwise from outside" },
                    "face_indices_base64": { "type": "string", "description": "u32 little-endian per triangle: the face's index in hew.query.topology's faces, plus its part's face_index_base" },
                    "parts": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "face_index_base": { "type": "integer", "minimum": 0 },
                                "index_base": { "type": "integer", "minimum": 0 },
                                "index_count": { "type": "integer", "minimum": 0 }
                            },
                            "required": ["id", "face_index_base", "index_base", "index_count"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["vertex_count", "triangle_count", "positions_base64", "normals_base64", "indices_base64", "face_indices_base64", "parts"],
                "additionalProperties": false
            });
            cmd.refusals = vec![
                "unknown_entity",
                "unknown_occurrence",
                "mesh_too_large",
                "too_many_occurrences",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.query.topology")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = occurrence_target_schema(true);
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "object": { "type": "string" },
                    "vertices": { "type": "array", "items": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 } },
                    "faces": { "type": "array", "items": { "type": "object", "description": "{index, plane: {point, normal}, outer: [vertex indices], holes: [[...]], material, back_material, surface: {kind: \"cylinder\", axis_point, axis, radius} | null, area, attrs: the face's attribute dictionary, {} for none}" } },
                    "edges": { "type": "array", "items": { "type": "object", "description": "{index, a, b (vertex indices), faces: [face indices, 1 or 2], curve: {kind: \"circle\", center, axis, radius} | null, attrs: the edge's attribute dictionary, {} for none}" } }
                },
                "required": ["object", "vertices", "faces", "edges"],
                "additionalProperties": false
            });
            cmd.refusals = vec!["unknown_entity", "unknown_occurrence", "not_an_object"];
        }

        // hew.doc.* (host-implemented lifecycle) and hew.view.snapshot —
        // Wave D (docs/design/api-implementation-conventions.md): the
        // contract is fully specified here even though the effect belongs
        // to the host (§3, `crates/api/src/host.rs`). A host lacking the
        // capability always answers `host_capability_missing`, so every
        // one of these inventories carries it.
        {
            let cmd = commands.get_mut("hew.doc.new").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "focus": window_focus_schema(),
                    "label": window_label_schema()
                },
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "document": document_info_schema() }
            });
            cmd.refusals = vec!["host_capability_missing", "window_not_ready"];
        }
        {
            let cmd = commands.get_mut("hew.doc.open").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "a .hew file, or a SketchUp .skp file (either container) when the extension is .skp; a later hew.doc.save with no format writes the same container back"
                    },
                    "focus": window_focus_schema(),
                    "label": window_label_schema()
                },
                "required": ["path"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "document": document_info_schema(),
                    "already_open": {
                        "type": "boolean",
                        "description": "the file was already open in a window; the connection was attached to it instead of opening a second copy"
                    }
                }
            });
            cmd.refusals = vec!["host_capability_missing", "load_failed", "window_not_ready"];
        }
        {
            let cmd = commands.get_mut("hew.doc.close").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "document": { "type": "string" },
                    "discard": {
                        "type": "boolean",
                        "description": "close even with unsaved changes (they are lost); default false"
                    }
                },
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "closed": document_info_schema() }
            });
            cmd.refusals = vec![
                "host_capability_missing",
                "unknown_document",
                "ambiguous_document",
                "unsaved_changes",
                "last_document",
            ];
        }
        {
            let cmd = commands.get_mut("hew.doc.focus").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "document": { "type": "string" } },
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "document": document_info_schema() }
            });
            cmd.refusals = vec![
                "host_capability_missing",
                "unknown_document",
                "ambiguous_document",
            ];
        }
        {
            let cmd = commands.get_mut("hew.doc.save").expect("declared above");
            cmd.implemented = true;
            cmd.summary = "Save the attached document (Save / Save As) as .hew or SketchUp .skp: the file is written, the unsaved mark cleared, and the document bound to it. `copy` writes a copy without any of that. With no `path` it re-saves in place; a document with no file yet, in a host that cannot ask where, hands the bytes back instead.";
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "copy": {
                        "type": "boolean",
                        "description": "write a copy to `path` — the document is not rebound to it and keeps its unsaved mark; requires `path`"
                    },
                    "return_bytes": {
                        "type": "boolean",
                        "description": "hand the document's bytes back and write nothing: no file, no binding, no saved mark; takes no `path`"
                    },
                    "format": {
                        "type": "string",
                        "enum": DocFormat::wire_names(),
                        "description": "\"skp\" is the SketchUp 2021-and-later container (what current SketchUp saves); \"skp3\" to \"skp8\" and \"skp2013\" to \"skp2020\" are those releases' own formats (SketchUp opens a file of its own release or later). Absent: a .skp path keeps the working file's own .skp container (the 2026 one for a document that did not come from a .skp), any other path writes .hew, and no path uses the working file's own format. A format that contradicts the path's extension is refused (format_mismatch)."
                    },
                    "compat": {
                        "type": "boolean",
                        "description": "For the 2021-and-later container (\"skp\") only: write its revision 17, which every SketchUp since 2021 opens, leaving out what only newer versions hold (layer folders, hatch patterns, PBR materials, environments, snaps, procedures, page extras, CRS), each kind itemised in `writer_notes`. Absent or false: the lowest revision that holds the content. A save back over a revision-17 file keeps revision 17 either way. Refused for any other format."
                    },
                    "units": {
                        "type": "string",
                        "enum": ["m", "cm", "mm", "arch", "frac_in", "dec_in"],
                        "description": "The display units a SketchUp file opens in (hew.view.units's formats), a save back over a .skp included. Absent: a save back keeps the file's own units and a new file takes the host's display units (the app's preference live; meters headless). A .hew ignores it."
                    }
                },
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "bytes_base64": { "type": "string" },
                    "format": { "type": "string", "enum": DocFormat::wire_names(), "description": "the format the document was saved in" },
                    "simplified": { "type": "array", "items": { "type": "string" }, "description": "present for a .skp: what the file could not hold exactly" },
                    "writer_notes": { "type": "array", "items": { "type": "string" }, "description": "present for a .skp: what a stream release or revision-17 (`compat`) save dropped, one entry per kind" }
                },
                "additionalProperties": false
            });
            cmd.refusals = vec![
                "file_open_elsewhere",
                "format_mismatch",
                "host_capability_missing",
                "path_required",
                "save_failed",
            ];
        }
        {
            let cmd = commands.get_mut("hew.doc.revert").expect("declared above");
            cmd.implemented = true;
            cmd.mutates_document = true;
            cmd.summary = "Revert to Saved: re-read the document's saved file in place, discarding every change since the last save. The undo and redo history is cleared and the document reads clean. Refuses `nothing_to_revert` when the document has no saved file or has no changes since the last save. There is no confirmation step: the caller is deliberate.";
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "document": { "type": "string" } },
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec![
                "host_capability_missing",
                "load_failed",
                "nothing_to_revert",
                "unknown_document",
                "ambiguous_document",
            ];
        }
        {
            let cmd = commands.get_mut("hew.doc.export").expect("declared above");
            cmd.implemented = true;
            let export_formats: Vec<String> = ["stl", "3mf", "glb", "gltf", "usdz"]
                .into_iter()
                .map(String::from)
                .chain(DocFormat::skp_wire_names())
                .collect();
            cmd.summary = "Export the attached document — STL, 3MF, glTF/GLB, USDZ (solids only), or a SketchUp .skp (\"skp\", the 2021-and-later container; \"skp3\" to \"skp8\", \"skp2013\" to \"skp2020\") — bytes base64, or a path on hosts with filesystem access.";
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "format": {
                        "type": "string",
                        "enum": export_formats,
                        "description": "\"gltf\" is an alias for \"glb\" — every host that implements one implements both. \"skp\" is the SketchUp 2021-and-later container, \"skp3\" to \"skp8\" and \"skp2013\" to \"skp2020\" those releases' formats; exporting never changes the document's own file"
                    },
                    "path": { "type": "string" },
                    "segments_per_turn": { "type": "integer", "minimum": 8, "maximum": 512 },
                    "units": {
                        "type": "string",
                        "enum": ["m", "cm", "mm", "arch", "frac_in", "dec_in"],
                        "description": "SketchUp formats only: the display units the file opens in (hew.view.units's formats). Absent: the host's display units (the app's preference live; meters headless)."
                    },
                    "selection": render_selection_schema(),
                    "compat": {
                        "type": "boolean",
                        "description": "Format \"skp\" only: write revision 17 of the container, leaving out what only newer SketchUp versions hold (see hew.doc.save). Absent or false: the lowest revision that holds the content."
                    }
                },
                "required": ["format"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "format": { "type": "string" },
                    "bytes_base64": { "type": "string" }
                },
                "required": ["format"]
            });
            cmd.refusals = vec![
                "export_failed",
                "host_capability_missing",
                "nothing_to_export",
                "save_failed",
                "unknown_entity",
                "unknown_occurrence",
                "too_many_occurrences",
            ];
        }
        {
            let cmd = commands.get_mut("hew.doc.import").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "units": { "type": "string", "enum": ["m", "mm", "cm", "in"] },
                    "bytes_base64": {
                        "type": "string",
                        "description": "the file's bytes, for a host with no filesystem (the desktop app); `path` then only names the file. hew-cli supplies this for you"
                    },
                    "images": {
                        "type": "object",
                        "description": "COLLADA texture images by the uri the file uses: {bytes_base64, format: png|jpeg}",
                        "additionalProperties": { "type": "object" }
                    },
                    "origin": {
                        "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3,
                        "description": "a picture's (.png/.jpg/.jpeg) lower-left corner, meters; default [0,0,0]"
                    },
                    "u": {
                        "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3,
                        "description": "a picture's width direction; default [1,0,0]"
                    },
                    "v": {
                        "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3,
                        "description": "toward a picture's top (not parallel to u); default [0,1,0]"
                    },
                    "width": {
                        "type": "number", "exclusiveMinimum": 0,
                        "description": "a picture's width in meters (its height follows from its proportions); default 1"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "report": { "type": "object" } },
                "required": ["report"]
            });
            cmd.refusals = vec![
                "host_capability_missing",
                "units_required",
                "load_failed",
                "unsupported_format",
            ];
        }
        {
            let verdict = serde_json::json!({
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["flat_sheet", "bent_sheet", "cappable", "holes", "mixed", "non_manifold"] },
                    "holes": { "type": "integer", "description": "flat_sheet: holes in the sheet" },
                    "planar": { "type": "integer", "description": "holes: how many are flat (a face can close each)" },
                    "warped": { "type": "integer", "description": "holes: how many are not flat" }
                },
                "required": ["kind"]
            });
            let cmd = commands.get_mut("hew.doc.health").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "objects": { "type": "array", "items": { "type": "string" }, "description": "obj_ ids to diagnose; omitted = every live object, component members included" }
                },
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "rows": {
                        "type": "array",
                        "description": "one per open (non-solid) object; a component member is one row however often it is placed",
                        "items": {
                            "type": "object",
                            "properties": {
                                "object": { "type": "string" },
                                "name": { "type": ["string", "null"] },
                                "definition": { "type": ["string", "null"], "description": "cmp_ id when the object is a component member" },
                                "definition_name": { "type": ["string", "null"] },
                                "placements": { "type": "integer" },
                                "instances": { "type": "array", "items": { "type": "string" }, "description": "outermost ins_ ids that place the member" },
                                "accepted": { "type": "boolean", "description": "marked as a surface with hew.solid.mark_surface" },
                                "family": { "type": ["integer", "null"], "description": "index into families" },
                                "verdict": verdict,
                                "rims": {
                                    "type": "array",
                                    "description": "open rims; their order is what hew.solid.cap's rims index",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "role": { "type": "string", "enum": ["outer", "hole"] },
                                            "planar": { "type": "boolean" },
                                            "cappable": { "type": "boolean", "description": "a face can close it: closed, planar, not the edge of a flat part of the surface, no branching vertex" },
                                            "closed": { "type": "boolean" },
                                            "sheet_edge": { "type": "boolean", "description": "planar, but runs along the edge of a flat part of the surface on its plane (a basin's lip), so a face there would lie on that part's back" },
                                            "shared": { "type": "boolean", "description": "another row's rim coincides with it: a seam between pieces of one shape the importer split (see family)" },
                                            "perimeter": { "type": "number" },
                                            "extent": { "type": "number" },
                                            "edges": { "type": "integer" }
                                        }
                                    }
                                },
                                "branching_vertices": { "type": "integer" },
                                "islands": { "type": "integer" },
                                "faces": { "type": "integer", "description": "how many faces the object has" }
                            },
                            "required": ["object", "verdict", "rims", "accepted", "placements"]
                        }
                    },
                    "families": {
                        "type": "array",
                        "description": "objects an importer split from one shape, joined by shared rims",
                        "items": {
                            "type": "object",
                            "properties": {
                                "members": { "type": "array", "items": { "type": "string" } },
                                "shared_rims": { "type": "integer" }
                            }
                        }
                    },
                    "counts": { "type": "object", "description": "objects, solids, flat_sheets, bent_sheets, cappable, holes, mixed, non_manifold, seam_families" }
                },
                "required": ["rows", "families", "counts"]
            });
            cmd.refusals = vec!["unknown_entity"];

            let repair_refusals = |extra: &[&'static str]| {
                let mut v = vec![
                    "object_is_solid",
                    "unknown_component",
                    "unknown_entity",
                    "explode_session_scope",
                    "invalid",
                ];
                v.extend_from_slice(extra);
                v
            };
            let cmd = commands.get_mut("hew.solid.cap").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "object": { "type": "string", "description": "obj_ id (a world object or a component member)" },
                    "rims": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "description": "indices into the object's hew.doc.health rims; omitted = every cappable rim (when none is, all of them, so the refusal says why)" }
                },
                "required": ["object"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "object_id": { "type": "string" },
                    "faces": { "type": "integer", "description": "faces added (face tokens cap.1, cap.2, ... in a transaction)" },
                    "closed": { "type": "boolean", "description": "the object is now a solid" },
                    "reversed": { "type": "boolean", "description": "an island the caps closed was inside out, so that island's faces were turned over (a cavity inside another island is left as is)" }
                },
                "required": ["object_id", "faces", "closed", "reversed"]
            });
            cmd.refusals = repair_refusals(&[
                "unknown_rim",
                "rim_not_planar",
                "rim_self_intersecting",
                "cap_would_be_coplanar",
                "non_manifold_rim",
                "caps_overlap",
            ]);

            let cmd = commands
                .get_mut("hew.solid.rejoin")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "family": { "type": "integer", "minimum": 0, "description": "index into the families of hew.doc.health over the whole model (the family a hew.query.occurrences row reports), resolved when the call runs; an index read from a hew.doc.health call with an objects scope may name a different family" },
                    "as": { "type": "string", "enum": ["one_solid"] }
                },
                "required": ["family", "as"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "object": { "type": "string", "description": "obj_ id of the joined solid; it keeps the largest piece's name, tags, and attributes" }
                },
                "required": ["object"]
            });
            cmd.refusals = vec![
                "unknown_family",
                "not_cappable",
                "rejoin_not_possible",
                "rim_not_planar",
                "rim_self_intersecting",
                "cap_would_be_coplanar",
                "non_manifold_rim",
                "caps_overlap",
                "invalid",
                "degenerate_contact",
                "boolean_operand_not_solid",
                "boolean_operand_empty",
                "unknown_object",
                "unknown_component",
                "explode_session_scope",
            ];

            let cmd = commands
                .get_mut("hew.solid.delete_faces")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "object": { "type": "string" },
                    "faces": {
                        "type": "array",
                        "items": { "type": "object", "description": "HEW_API.md §5.2 face locator: {object,at} | {object,ray} | {\"$face\":\"label#key\"}" },
                        "minItems": 1,
                        "description": "faces of that object; omitted = every face, which deletes the object (a component member too, unless it is its component's last object: last_definition_member)"
                    }
                },
                "required": ["object"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "object_id": { "type": "string" },
                    "deleted": { "type": "boolean", "description": "every face went, so the object was deleted" },
                    "object_ids": { "type": "array", "items": { "type": "string" }, "description": "islands cut loose, now objects of their own" }
                },
                "required": ["object_id", "deleted", "object_ids"]
            });
            cmd.refusals = repair_refusals(&[
                "unknown_face",
                "last_definition_member",
                "locator_missed",
                "ambiguous_locator",
                "face_token_unknown",
                "face_token_stale",
            ]);

            let cmd = commands
                .get_mut("hew.solid.mark_surface")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "object": { "type": "string" },
                    "accepted": { "type": "boolean", "default": true, "description": "false clears the mark" }
                },
                "required": ["object"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "object_id": { "type": "string" },
                    "accepted": { "type": "boolean" }
                },
                "required": ["object_id", "accepted"]
            });
            cmd.refusals = vec!["object_is_solid", "unknown_entity"];

            let cmd = commands
                .get_mut("hew.solid.thicken")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "object": { "type": "string" },
                    "distance": { "type": "number", "description": "meters along each face's front normal (negative: behind it)" },
                    "centered": { "type": "boolean", "default": false, "description": "split the thickness evenly on both sides of the surface" }
                },
                "required": ["object", "distance"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "object_id": { "type": "string", "description": "the surface, now a solid" },
                    "object_ids": { "type": "array", "items": { "type": "string" }, "description": "further separate pieces of the surface, each now a solid of its own" }
                },
                "required": ["object_id", "object_ids"]
            });
            cmd.refusals = repair_refusals(&["not_a_flat_sheet", "thickness_too_small"]);
        }
        {
            let geo_schema = serde_json::json!({
                "type": ["object", "null"],
                "description": "the coordinate reference system, or null when the document has none; positions are in the system's own map unit",
                "properties": {
                    "name": { "type": "string" },
                    "description": { "type": "string" },
                    "easting": { "type": "number", "description": "model origin easting, in the map unit" },
                    "northing": { "type": "number", "description": "model origin northing, in the map unit" },
                    "height": { "type": "number", "description": "model origin height, in the map unit" },
                    "x_axis": {
                        "type": "array",
                        "items": { "type": "number" },
                        "minItems": 2,
                        "maxItems": 2,
                        "description": "the model's x axis in the map plane, a 2D vector (not an angle)"
                    },
                    "scale": { "type": "number", "description": "map scale factor" },
                    "geodetic_datum": { "type": "string" },
                    "vertical_datum": { "type": "string" },
                    "map_projection": { "type": "string" },
                    "map_zone": { "type": "string" },
                    "map_unit": { "type": "string", "description": "the unit's name as the file holds it, e.g. METRE" },
                    "map_unit_metres": { "type": "number", "exclusiveMinimum": 0, "description": "meters per map unit, when known" },
                    "tgl_id": { "type": "string" },
                    "calibration_file_name": { "type": "string" }
                },
                "additionalProperties": false
            });
            let cmd = commands.get_mut("hew.geo.get").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "geo": geo_schema.clone() },
                "required": ["geo"],
                "additionalProperties": false
            });
            cmd.refusals = vec![];

            let cmd = commands.get_mut("hew.geo.set").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "geo": geo_schema },
                "required": ["geo"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec!["invalid_geo_reference"];
        }
        {
            let cmd = commands
                .get_mut("hew.doc.purge_unused")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "materials": { "type": "integer" },
                    "definitions": { "type": "integer" }
                },
                "required": ["materials", "definitions"],
                "additionalProperties": false
            });
            cmd.refusals = vec![];
        }
        {
            let cmd = commands
                .get_mut("hew.view.snapshot")
                .expect("declared above");
            cmd.implemented = true;
            cmd.summary = "Render the attached document to PNG, rendered by the host's software rasterizer, identically in the app and in hew-cli, optionally of a selection on a transparent background — bytes base64 by default, or a path on hosts with filesystem access.";
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "width": {
                        "type": "integer",
                        "minimum": 16,
                        "maximum": 2048,
                        "description": "defaults to 512; out-of-range values are clamped, not refused"
                    },
                    "height": {
                        "type": "integer",
                        "minimum": 16,
                        "maximum": 2048,
                        "description": "defaults to 512; out-of-range values are clamped, not refused"
                    },
                    "camera": {
                        "type": "object",
                        "properties": {
                            "eye": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                            "target": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                            "up": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                            "projection": { "type": "string", "enum": ["perspective", "parallel"] },
                            "fov_deg": { "type": "number", "description": "perspective only; defaults to 35" },
                            "parallel_height": { "type": "number", "description": "the height the view spans at the target, meters (what a parallel view frames); defaults to what the perspective view spans there" }
                        },
                        "required": ["eye", "target"],
                        "additionalProperties": false,
                        "description": "mutually exclusive with view"
                    },
                    "view": {
                        "type": "string",
                        "enum": ["iso", "front", "back", "left", "right", "top", "bottom"],
                        "description": "a named standard view fitted to the scene bounding box; mutually exclusive with camera and scene"
                    },
                    "selection": {
                        "type": "array",
                        "minItems": 1,
                        "items": occurrence_locator_schema(),
                        "description": "render only these: node ids, occurrence locators {path}, or a definition's cmp_ id (its members at the identity pose, one picture however many times it is placed). With no camera or view the camera is a fitted isometric over the selection, never the document's saved camera; view fits that standard view to the selection. Mutually exclusive with scene."
                    },
                    "background": {
                        "type": "string",
                        "description": "\"transparent\", #rrggbb, or #rrggbbaa; the fill of pixels no surface touches (defaults to the rasterizer's light gray). The id, depth, and normals passes are unaffected."
                    },
                    "scope": scope_schema(),
                    "orient": {
                        "type": "string",
                        "enum": ["world", "part"],
                        "default": "world",
                        "description": "part renders the one selection root in its part frame: the named view (iso when none) means the frame's axes, front looking along the frame's +y, top along -z, right along -x. Needs a selection of exactly one entity and no camera"
                    },
                    "scene": {
                        "type": "string",
                        "description": "a Scene's id: renders through its resolved camera and hidden sets (Document::resolve_scene) instead of the document's own — falls back to the usual cameraless resolution when the Scene captures no camera. Mutually exclusive with camera and view. The Scene's section plane, if any, is NOT rendered headlessly at 1.0."
                    },
                    "include_depth": {
                        "type": "boolean",
                        "description": "defaults to false; when true, also returns a depth pass as a grayscale PNG — NEAR is WHITE, background 0 — with depth_near/depth_far (meters from the eye along the view axis) so absolute depth is recoverable: depth = depth_far - (v - 1) / (max - 1) * (depth_far - depth_near), max = 255 or 65535. The conditioning input depth-aware image models expect"
                    },
                    "depth_bits": {
                        "type": "integer",
                        "enum": [8, 16],
                        "description": "bits per pixel of the depth PNG (default 8); only with include_depth"
                    },
                    "include_normals": {
                        "type": "boolean",
                        "description": "defaults to false; when true, also returns a camera-space normals pass as an RGB PNG: (n * 0.5 + 0.5) * 255 per axis, x right, y up, z toward the viewer, background (128, 128, 255)"
                    },
                    "include_ids": {
                        "type": "boolean",
                        "description": "defaults to false; when true, also returns a per-pixel id-buffer and its palette"
                    },
                    "path": {
                        "type": "string",
                        "description": "when given, the PNG is written here instead of returned inline, honored by hosts with filesystem access and refused typed elsewhere (mirrors hew.doc.export)"
                    }
                },
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "description": "two shapes depending on whether path was given: bytes base64 inline (default), or a path plus a sidecar path for each extra pass asked for (id-buffer, depth, normals)",
                "properties": {
                    "png_base64": {
                        "type": "string",
                        "description": "present only when path was not given"
                    },
                    "path": {
                        "type": "string",
                        "description": "present only when path was given: echoes it back"
                    },
                    "width": { "type": "integer" },
                    "height": { "type": "integer" },
                    "id_buffer_base64": {
                        "type": "string",
                        "description": "present only when include_ids was true and path was not given: u16 little-endian per pixel, index into id_palette (0 = background)"
                    },
                    "id_buffer_path": {
                        "type": "string",
                        "description": "present only when include_ids and path were both given: \"<path>.ids.bin\", the same u16 little-endian per-pixel encoding written to disk"
                    },
                    "id_palette": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "public ids; id_palette[i] is what the id-buffer (inline or on disk) reports as index i+1"
                    },
                    "depth_png_base64": {
                        "type": "string",
                        "description": "present only when include_depth was true and path was not given"
                    },
                    "depth_path": {
                        "type": "string",
                        "description": "present only when include_depth and path were both given: \"<path>.depth.png\""
                    },
                    "depth_near": { "type": "number", "description": "with a depth pass: meters from the eye, along the view axis, of the nearest drawn surface" },
                    "depth_far": { "type": "number", "description": "with a depth pass: the farthest drawn surface" },
                    "depth_bits": { "type": "integer", "description": "with a depth pass: 8 or 16" },
                    "normals_png_base64": {
                        "type": "string",
                        "description": "present only when include_normals was true and path was not given"
                    },
                    "normals_path": {
                        "type": "string",
                        "description": "present only when include_normals and path were both given: \"<path>.normals.png\""
                    }
                },
                "required": ["width", "height"]
            });
            cmd.refusals = vec![
                "host_capability_missing",
                "nothing_to_render",
                "save_failed",
                "unknown_entity",
                "unknown_occurrence",
                "unknown_scene",
                "too_many_occurrences",
                "no_part_frame",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.view.line_drawing")
                .expect("declared above");
            cmd.implemented = true;
            cmd.summary = "Hidden-line drawing of the visible document, or of a selection, from a camera (crates/hlr), served identically in the app and in hew-cli: hard edges, curved-wall silhouettes, section-cut outlines, optionally dashed hidden lines — as a true-size SVG at a drawing scale (inline or written to path), or as raw segments in view-plane meters.";
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "camera": { "type": "object", "description": "identical vocabulary to hew.view.snapshot's camera; mutually exclusive with view and scene" },
                    "view": { "type": "string", "enum": ["iso", "front", "back", "left", "right", "top", "bottom"], "description": "a named standard view (parallel projection); mutually exclusive with camera and scene" },
                    "scene": { "type": "string", "description": "a Scene's id: its camera and hidden sets, as hew.view.snapshot" },
                    "include_hidden": { "type": "boolean", "description": "defaults to false; when true, hidden pieces are returned too (kind \"hidden\", dashed in SVG)" },
                    "include_soft": { "type": "boolean", "description": "defaults to false; when true, non-silhouette curved-wall facet seams are returned (kind \"soft\")" },
                    "format": { "type": "string", "enum": ["svg", "segments"], "description": "defaults to svg" },
                    "scale": { "type": "number", "description": "svg only: paper/model drawing scale, defaults to 1 (full size); the SVG's width/height are millimeters at that scale" },
                    "path": { "type": "string", "description": "svg only: write the file here instead of returning it inline (hosts with filesystem access)" },
                    "selection": render_selection_schema(),
                    "scope": scope_schema()
                },
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "svg": { "type": "string", "description": "format svg without path" },
                    "path": { "type": "string", "description": "format svg with path" },
                    "segments": { "type": "array", "items": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4 }, "description": "format segments: [ax, ay, bx, by] in view-plane meters, y up, origin at the camera target" },
                    "kinds": { "type": "array", "items": { "type": "string", "enum": ["hard", "silhouette", "soft", "section", "hidden"] } },
                    "ids": { "type": "array", "items": { "type": "string" }, "description": "public id of the entity each segment belongs to" },
                    "count": { "type": "integer" },
                    "bounds": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4, "description": "[min_x, min_y, max_x, max_y] in view-plane meters; null when empty" }
                },
                "required": ["count"]
            });
            cmd.refusals = vec![
                "host_capability_missing",
                "nothing_to_render",
                "too_complex",
                "save_failed",
                "unknown_scene",
                "unknown_entity",
                "unknown_occurrence",
                "too_many_occurrences",
            ];
        }
        {
            let cmd = commands.get_mut("hew.print.pdf").expect("declared above");
            cmd.implemented = true;
            cmd.summary = "Print the document to a PDF the way File ▸ Print… does: standard (one page, the view as-is) or scaled (parallel projection at an exact drawing scale, tiled across pages with overlap bands, crop/trim marks, a scale bar, and a title block). Line art is vector (hidden lines removed); shaded is a software-rasterized bitmap per page. Bytes base64 inline, or written to path.";
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "paper": { "description": "\"letter\" | \"legal\" | \"tabloid\" | \"a5\" | \"a4\" | \"a3\" (default a4) or {w_mm, h_mm}", "oneOf": [ { "type": "string" }, { "type": "object", "properties": { "w_mm": { "type": "number" }, "h_mm": { "type": "number" } }, "required": ["w_mm", "h_mm"] } ] },
                    "orientation": { "type": "string", "enum": ["auto", "portrait", "landscape"], "description": "defaults to auto (fewer tiles; standard follows the view's aspect)" },
                    "margin_mm": { "type": "number", "description": "defaults to 12.7 (½ in)" },
                    "mode": { "type": "string", "enum": ["scaled", "standard"], "description": "defaults to scaled" },
                    "scale": { "type": "number", "description": "scaled: paper/model, defaults to 0.1 (1:10)" },
                    "scale_label": { "type": "string", "description": "title-block scale text; defaults to the ratio (\"1:10\")" },
                    "camera": { "type": "object", "description": "as hew.view.snapshot; scaled prints use its direction with parallel projection" },
                    "view": { "type": "string", "enum": ["iso", "front", "back", "left", "right", "top", "bottom"] },
                    "scene": { "type": "string" },
                    "style": { "type": "string", "enum": ["line_art", "shaded"], "description": "defaults to line_art (vector)" },
                    "include_hidden": { "type": "boolean", "description": "line art: dashed hidden lines" },
                    "title_block": { "type": "boolean", "description": "defaults to true" },
                    "scale_bar": { "type": "boolean", "description": "defaults to true" },
                    "marks": { "type": "boolean", "description": "crop/trim marks and neighbour labels; defaults to true" },
                    "overlap_mm": { "type": "number", "description": "tile overlap band, defaults to 10; 0 for none" },
                    "units": { "type": "string", "enum": ["metric", "imperial"], "description": "scale-bar family, defaults to metric" },
                    "title": { "type": "string", "description": "title-block document name; defaults to the document's" },
                    "path": { "type": "string", "description": "write the PDF here instead of returning it inline" }
                },
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "pdf_base64": { "type": "string", "description": "present only when path was not given" },
                    "path": { "type": "string" },
                    "pages": { "type": "integer" },
                    "cols": { "type": "integer" },
                    "rows": { "type": "integer" }
                },
                "required": ["pages", "cols", "rows"]
            });
            cmd.refusals = vec![
                "host_capability_missing",
                "nothing_to_render",
                "too_complex",
                "save_failed",
                "unknown_scene",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.library.list")
                .expect("declared above");
            cmd.implemented = true;
            cmd.summary = "List items in the Hew Library, optionally filtered by category, collection, or a text query.";
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "category": { "type": "string", "enum": ["component", "material", "model"] },
                    "query": { "type": "string", "description": "case-insensitive substring match against name and keywords" },
                    "collection": { "type": "string", "description": "matches this collection path and its subtree" }
                },
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "folder": { "type": "string" },
                    "items": { "type": "array", "items": library_item_schema() }
                },
                "required": ["folder", "items"]
            });
            cmd.refusals = vec!["host_capability_missing"];
        }
        {
            let cmd = commands
                .get_mut("hew.library.describe")
                .expect("declared above");
            cmd.implemented = true;
            cmd.summary =
                "One library item's full metadata, manifest summary, and document attributes.";
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "item": { "type": "string", "description": "the item's id or relative path" }
                },
                "required": ["item"],
                "additionalProperties": false
            });
            let mut result = library_item_schema();
            if let Some(props) = result.get_mut("properties") {
                props["attrs"] = serde_json::json!({ "type": "object", "description": "the item's full document attribute dictionaries, namespace -> key -> value" });
            }
            if let Some(req) = result.get_mut("required").and_then(|r| r.as_array_mut()) {
                req.push(serde_json::json!("attrs"));
            }
            cmd.result_schema = result;
            cmd.refusals = vec!["host_capability_missing", "unknown_library_item"];
        }
        {
            let cmd = commands
                .get_mut("hew.library.insert")
                .expect("declared above");
            cmd.implemented = true;
            cmd.summary = "Insert a library item into the attached document, wired for idempotent re-insert exactly like the UI's own placement; a material item goes into the palette instead, recording no undo entry (§6.4).";
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "item": { "type": "string", "description": "the item's id or relative path — exactly one of item/bytes_base64" },
                    "bytes_base64": { "type": "string", "description": "the item's raw .hew bytes — exactly one of item/bytes_base64" },
                    "content_hash": { "type": "string", "description": "sha256 hex of bytes_base64, for idempotent re-insert matching; ignored (and provenance is skipped) without it" },
                    "at": { "description": "a translation applied to the item's world roots; identity when omitted", "oneOf": [ { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }, { "type": "object" } ] },
                    "name": { "type": "string" }
                },
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "roots": { "type": "array", "items": { "type": "string" } },
                    "materials": { "type": "array", "items": { "type": "string" }, "description": "a material item only: its palette materials, added or reused, in item order" },
                    "definitions_added": { "type": "integer" },
                    "definitions_reused": { "type": "integer" },
                    "materials_added": { "type": "integer" },
                    "materials_reused": { "type": "integer" },
                    "objects_added": { "type": "integer" },
                    "guides_added": { "type": "integer" },
                    "world_sketches_skipped": { "type": "integer" },
                    "annotations_skipped": { "type": "integer" }
                },
                "required": ["roots", "definitions_added", "definitions_reused", "materials_added", "materials_reused", "objects_added", "guides_added", "world_sketches_skipped", "annotations_skipped"]
            });
            cmd.refusals = vec![
                "host_capability_missing",
                "unknown_library_item",
                "load_failed",
                "component_expansion_exceeded",
                "singular",
                "degenerate_axis",
                "explode_session_scope",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.library.save")
                .expect("declared above");
            cmd.implemented = true;
            cmd.summary = "Save a selection, one material, or the whole document to the library. Records no undo entry (§6.4).";
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "selection": { "type": "array", "items": { "type": "string" }, "description": "node ids to save as a component item; omitted saves the whole document as a model item" },
                    "name": { "type": "string" },
                    "material": { "type": "string", "description": "one palette material to save alone as a material item, as the palette's Save to Library does — not with selection" },
                    "category": { "type": "string", "enum": ["component", "material", "model"], "description": "defaults to component (a selection), material (a material), or model (the whole document)" },
                    "keywords": { "type": "array", "items": { "type": "string" } },
                    "collection": { "type": "string" },
                    "return_bytes": { "type": "boolean", "description": "also return the saved item's bytes base64" },
                    "source_doc": { "type": "string", "description": "display-only \"saved from\" bookkeeping; defaults to the host's current document path when known (e.g. hew-cli --file), else omitted" }
                },
                "required": ["name"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "id": { "type": "string" },
                    "bytes_base64": { "type": "string", "description": "present only when return_bytes was true" }
                },
                "required": ["path", "id"]
            });
            cmd.refusals = vec![
                "host_capability_missing",
                "unknown_entity",
                "empty_component",
                "duplicate_member",
                "unknown_object",
                "unknown_group",
                "unknown_instance",
                "explode_session_scope",
                "save_failed",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.library.remove")
                .expect("declared above");
            cmd.implemented = true;
            cmd.summary = "Delete a library item.";
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "item": { "type": "string", "description": "the item's id or relative path" }
                },
                "required": ["item"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "removed": { "type": "string" } },
                "required": ["removed"]
            });
            cmd.refusals = vec!["host_capability_missing", "unknown_library_item"];
        }
        {
            let cmd = commands
                .get_mut("hew.library.update_meta")
                .expect("declared above");
            cmd.implemented = true;
            cmd.summary = "Edit a library item's name, keywords, or collection in place.";
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "item": { "type": "string", "description": "the item's id or relative path" },
                    "name": { "type": "string" },
                    "keywords": { "type": "array", "items": { "type": "string" } },
                    "collection": { "type": "string" }
                },
                "required": ["item"],
                "additionalProperties": false
            });
            cmd.result_schema = library_item_meta_schema();
            cmd.refusals = vec![
                "host_capability_missing",
                "unknown_library_item",
                "save_failed",
            ];
        }
        {
            // hew.view.camera — sets the live viewport's camera. Same
            // camera/view vocabulary as hew.view.snapshot (one spec, not
            // two, per docs/agents/HEW_API.md §7), minus snapshot's rendering
            // parameters (width/height/include_ids/path) which have no
            // meaning here. Unlike snapshot, EXACTLY one of camera/view
            // is required — a live camera-set command given neither has
            // no honest default to fall back to.
            let cmd = commands.get_mut("hew.view.camera").expect("declared above");
            cmd.implemented = true;
            cmd.summary = "Set the live desktop viewport's camera. A host effect on the view, not a document edit (mutates_document = false: never recorded, never resyncs the document). Headless clients pass a camera per hew.view.snapshot call instead.";
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "camera": {
                        "type": "object",
                        "properties": {
                            "eye": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                            "target": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                            "up": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                            "projection": { "type": "string", "enum": ["perspective", "parallel"] },
                            "fov_deg": { "type": "number", "description": "perspective only; defaults to 35" },
                            "parallel_height": { "type": "number", "description": "the height the view spans at the target, meters (what a parallel view frames); defaults to what the perspective view spans there" }
                        },
                        "required": ["eye", "target"],
                        "additionalProperties": false,
                        "description": "mutually exclusive with view; identical vocabulary to hew.view.snapshot's camera"
                    },
                    "view": {
                        "type": "string",
                        "enum": ["iso", "front", "back", "left", "right", "top", "bottom"],
                        "description": "a named standard view; mutually exclusive with camera"
                    }
                },
                "additionalProperties": false,
                "description": "exactly one of camera or view is required"
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec!["host_capability_missing"];
        }
        {
            // hew.view.zoom_extents — frames all visible geometry, the
            // API counterpart of View > Zoom Extents. No params, no
            // camera choice: it always targets everything currently
            // visible. Mirrors the live app's own zoomExtents(), which is
            // a documented no-op on an empty scene rather than a typed
            // refusal, so this inventory carries none beyond the shared
            // host-capability one.
            let cmd = commands
                .get_mut("hew.view.zoom_extents")
                .expect("declared above");
            cmd.implemented = true;
            cmd.summary = "Frame all visible geometry in the live viewport (View > Zoom Extents). A view effect, not a document edit (mutates_document = false).";
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec!["host_capability_missing"];
        }
        {
            // hew.view.units — sets app/src/settings/units.ts's
            // LengthFormat, an app-level DISPLAY PREFERENCE, never
            // document state and never touched by the file format (see
            // that module's own doc comment: "It never touches kernel
            // state"). Named under hew.view rather than a new namespace:
            // it governs how the live app currently PRESENTS the model,
            // exactly as hew.view.camera governs how it's framed — both
            // are host/session view state, not model data, and both are
            // meaningless off a live host.
            let cmd = commands.get_mut("hew.view.units").expect("declared above");
            cmd.implemented = true;
            cmd.summary = "Set the app's displayed length-unit format (app/src/settings/units.ts's LengthFormat) — an app-level display PREFERENCE, never document state or file-format data.";
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "format": {
                        "type": "string",
                        "enum": ["m", "cm", "mm", "arch", "frac_in", "dec_in"],
                        "description": "metric: m, cm, mm; imperial: arch (feet+inches, e.g. 5' 3-1/8\"), frac_in (fractional inches), dec_in (decimal inches)"
                    }
                },
                "required": ["format"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec!["host_capability_missing"];
        }
        // Selection and view state (docs/agents/HEW_API.md §7, "Selection
        // and view state"): `crates/api/src/commands/{selection,view}.rs`.
        {
            let ids = || serde_json::json!({ "type": "array", "items": { "type": "string" } });
            let selection_result = || {
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "ids": { "type": "array", "items": { "type": "string" }, "description": "public ids in pick order; index 0 is the primary" },
                        "primary": { "type": ["string", "null"] },
                        "generation": { "type": "integer", "minimum": 0 },
                        "epoch": { "type": "integer", "minimum": 0 }
                    },
                    "required": ["ids", "primary", "generation", "epoch"],
                    "additionalProperties": false
                })
            };
            let empty = || {
                serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                })
            };
            let isolation_entry = || {
                serde_json::json!({
                    "oneOf": [
                        { "type": "string", "description": "an object, group, or instance id" },
                        {
                            "type": "object",
                            "properties": {
                                "object": { "type": "string", "description": "a definition member object" },
                                "instance": { "type": "string", "description": "an instance that places it" }
                            },
                            "required": ["object", "instance"],
                            "additionalProperties": false
                        }
                    ]
                })
            };
            let hidden_result = || {
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "hidden": { "type": "array", "items": { "type": "string" }, "description": "every hidden object, group, and instance" }
                    },
                    "required": ["hidden"],
                    "additionalProperties": false
                })
            };
            for (name, summary, params, result, refusals) in [
                (
                    "hew.selection.get",
                    "The app's selection: public ids in pick order, the primary first. A drafting selection reads as empty.",
                    empty(),
                    selection_result(),
                    vec!["host_capability_missing"],
                ),
                (
                    "hew.selection.set",
                    "Change the app's selection: replace it (the default), or add, subtract, or toggle the named entities. Nodes and sketches mix; a guide or an annotation is selected alone. Hidden entities refuse.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "ids": { "type": "array", "items": { "type": "string" }, "maxItems": 10000, "description": "objects, groups, instances, and sketches; or exactly one guide; or exactly one annotation" },
                            "mode": { "type": "string", "enum": ["replace", "add", "subtract", "toggle"], "description": "defaults to replace" }
                        },
                        "required": ["ids"],
                        "additionalProperties": false
                    }),
                    selection_result(),
                    vec![
                        "unknown_entity",
                        "selection_target_not_visible",
                        "selection_mixed_kinds",
                        "host_capability_missing",
                    ],
                ),
                (
                    "hew.selection.clear",
                    "Clear the app's selection.",
                    empty(),
                    selection_result(),
                    vec!["host_capability_missing"],
                ),
                (
                    "hew.view.hide",
                    "Hide objects, groups, and component instances. View state: saved with the document, never undoable. Tags hide through hew.tag.set_visible.",
                    serde_json::json!({
                        "type": "object",
                        "properties": { "ids": ids() },
                        "required": ["ids"],
                        "additionalProperties": false
                    }),
                    hidden_result(),
                    vec!["unknown_entity", "not_a_node"],
                ),
                (
                    "hew.view.show",
                    "Show hidden objects, groups, and component instances again.",
                    serde_json::json!({
                        "type": "object",
                        "properties": { "ids": ids() },
                        "required": ["ids"],
                        "additionalProperties": false
                    }),
                    hidden_result(),
                    vec!["unknown_entity", "not_a_node"],
                ),
                (
                    "hew.view.show_all",
                    "Show every hidden object, group, and component instance. Hidden tags stay hidden.",
                    empty(),
                    serde_json::json!({
                        "type": "object",
                        "properties": { "hidden": { "type": "array", "items": { "type": "string" }, "maxItems": 0 } },
                        "required": ["hidden"],
                        "additionalProperties": false
                    }),
                    vec![],
                ),
                (
                    "hew.view.hidden",
                    "What is hidden: the hidden objects, groups, and instances, the hidden tags, and the live viewport's isolation (null when nothing is isolated or the host has no viewport).",
                    empty(),
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "hidden": { "type": "array", "items": { "type": "string" } },
                            "tags_hidden": { "type": "array", "items": { "type": "string" } },
                            "isolated": {
                                "oneOf": [
                                    { "type": "array", "items": isolation_entry() },
                                    { "type": "null" }
                                ]
                            }
                        },
                        "required": ["hidden", "tags_hidden", "isolated"],
                        "additionalProperties": false
                    }),
                    vec![],
                ),
                (
                    "hew.view.isolate",
                    "Show only the named objects, groups, instances, or definition members (each with an instance that places it) in the live viewport, and frame them. View-only: nothing in the document changes.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "ids": { "type": "array", "items": isolation_entry(), "minItems": 1, "maxItems": 10000 }
                        },
                        "required": ["ids"],
                        "additionalProperties": false
                    }),
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "isolated": { "type": "array", "items": isolation_entry() }
                        },
                        "required": ["isolated"],
                        "additionalProperties": false
                    }),
                    vec!["unknown_entity", "host_capability_missing"],
                ),
                (
                    "hew.view.clear_isolation",
                    "End the live viewport's isolation; a no-op when nothing is isolated.",
                    empty(),
                    empty(),
                    vec!["host_capability_missing"],
                ),
                (
                    "hew.view.zoom_to",
                    "Frame the named entities in the live viewport. padding widens the box on every side by that fraction of its largest extent (default 0.1). Answers the box before padding.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "ids": ids(),
                            "padding": { "type": "number", "minimum": 0, "maximum": 2, "description": "defaults to 0.1" }
                        },
                        "required": ["ids"],
                        "additionalProperties": false
                    }),
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "min": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                            "max": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                        },
                        "required": ["min", "max"],
                        "additionalProperties": false
                    }),
                    vec![
                        "unknown_entity",
                        "nothing_to_frame",
                        "host_capability_missing",
                    ],
                ),
                (
                    "hew.view.enter",
                    "Converge the user's window onto a node, opening group and component sessions level by level and falling back to the in-context entry for an instance a session cannot take (unevenly scaled, or placed in a group elsewhere; a mirrored instance opens a session like any other), then select it. Give exactly one of id (an object, group, or instance in the world) or path (instances outermost first, ending at the leaf). The result is empty; the outcome arrives as context_changed, and hew.query.context reports the stack and fallback afterward. Acts on the user's window, so it is refused inside a transaction; distinct from hew.context.enter, which scopes a plugin's own edits.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "id": { "type": "string", "description": "a world object, group, or instance" },
                            "path": { "type": "array", "items": { "type": "string" }, "minItems": 1, "description": "an occurrence path: instances outermost first, the last element the node to enter" }
                        },
                        "additionalProperties": false
                    }),
                    serde_json::json!({
                        "type": "object",
                        "properties": {},
                        "additionalProperties": false
                    }),
                    vec![
                        "unknown_entity",
                        "unknown_occurrence",
                        "gesture_in_progress",
                        "host_capability_missing",
                    ],
                ),
                (
                    "hew.view.exit",
                    "Pop the user's innermost editing context, a fallback entry or a session frame. The outcome arrives as context_changed.",
                    empty(),
                    empty(),
                    vec![
                        "gesture_in_progress",
                        "nothing_to_exit",
                        "host_capability_missing",
                    ],
                ),
                (
                    "hew.view.exit_all",
                    "Close every editing context in the user's window, back to the top level. The outcome arrives as context_changed.",
                    empty(),
                    empty(),
                    vec!["gesture_in_progress", "host_capability_missing"],
                ),
            ] {
                let cmd = commands.get_mut(name).expect("declared above");
                cmd.implemented = true;
                cmd.summary = summary;
                cmd.params_schema = params;
                cmd.result_schema = result;
                cmd.refusals = refusals;
            }
        }

        // hew.entity.*, hew.context.*, hew.group.*, hew.component.*,
        // hew.material.*, hew.tag.*, hew.guide.*, hew.attr.*,
        // hew.history.* — Wave C (docs/design/
        // api-implementation-conventions.md): honest schemas and refusal
        // inventories for `crates/api/src/commands/{entity,structure,
        // style,attrs,history}.rs`. `hew.guide.angular` has no direct
        // kernel backing (the kernel's `Guide` is Line|Point only) but
        // needs none — it composes `Document::add_guide_line` with a
        // client-computed direction (protractor semantics), exactly as
        // `hew.guide.line` does.
        {
            let cmd = commands
                .get_mut("hew.entity.rename")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "name": { "type": ["string", "null"] }
                },
                "required": ["id"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec![
                "unknown_entity",
                "rename_unsupported",
                "unknown_object",
                "unknown_group",
                "unknown_instance",
                "unknown_component",
                "unknown_material",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.entity.delete")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "any public id; a sketch edge id (\"edg_…\") erases just that one edge — the eraser's own kernel path (Sketch::remove_edge) — as one undo entry, rather than the whole sketch"
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec![
                "unknown_entity",
                "unknown_object",
                "unknown_group",
                "unknown_instance",
                "unknown_sketch",
                "unknown_guide",
                "unknown_edge",
                "unknown_material",
                "unknown_component",
                "definition_nested_in_definition",
                "explode_session_scope",
            ];
        }
        // `copy.linked` and the copy result's `linked` (docs/agents/PLUGINS.md
        // "Linked copies").
        let copy_linked_schema = serde_json::json!({
            "type": "boolean",
            "default": true,
            "description": "true (the default on the bus, whatever the app's preference): each object or group copied becomes a set of linked copies, its copies placements of one shared solid; false: independent geometry"
        });
        let copy_wraps_schema = serde_json::json!({
            "type": "array",
            "description": "present whenever copy was given: each object or group the copy wrapped as a linked set (it keeps its id, now inside definition, placed by instance in its old spot); empty for linked: false or when every root already was a placement",
            "items": {
                "type": "object",
                "properties": {
                    "source": { "type": "string" },
                    "instance": { "type": "string" },
                    "definition": { "type": "string" }
                },
                "required": ["source", "instance", "definition"],
                "additionalProperties": false
            }
        });
        // `ids` of move/rotate/scale: world ids, or occurrence paths to
        // members of ONE definition by ONE path (HEW_API.md §7).
        let transform_ids_schema = serde_json::json!({
            "type": "array",
            "minItems": 1,
            "items": occurrence_locator_schema(),
            "description": "world node and sketch ids, or occurrence locators {path: [instance, …, member]} that all reach members of one definition by the same path (the gesture is world space; every placement of the definition follows). One call is one scope: a mix refuses -32602"
        });
        {
            let cmd = commands.get_mut("hew.entity.move").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "ids": transform_ids_schema.clone(),
                    "translation": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "from": {},
                    "to": {},
                    "copy": {
                        "type": "object",
                        "properties": {
                            "count": { "type": "integer", "minimum": 1, "maximum": 1000, "description": "copies at step, 2·step, …, count·step along the translation" },
                            "grid": {
                                "type": "array",
                                "minItems": 1,
                                "maxItems": 3,
                                "description": "a grid copy: one entry per axis; count is the number of slots along it, the original's included, so [{count: 4, step}] makes three copies. Takes the place of translation/from/to and of copy.count; the product of counts is at most 1000",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "count": { "type": "integer", "minimum": 1 },
                                        "step": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                                    },
                                    "required": ["count", "step"],
                                    "additionalProperties": false
                                }
                            },
                            "linked": copy_linked_schema.clone()
                        },
                        "additionalProperties": false
                    }
                },
                "required": ["ids"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "ids": { "type": "array", "items": { "type": "string" }, "description": "the copies, placement-major, each placement's block in ids order; a linked copy's are placements (ins_)" },
                    "linked": copy_wraps_schema.clone()
                }
            });
            cmd.refusals = vec![
                "unknown_entity",
                "mixed_selection_unsupported",
                "sketch_copy_unsupported",
                "array_count_too_large",
                "empty_selection",
                "duplicate_member",
                "unknown_object",
                "unknown_group",
                "unknown_instance",
                "unknown_sketch",
                "unknown_occurrence",
                "unknown_component",
                "explode_session_scope",
                "component_expansion_exceeded",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.entity.rotate")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "ids": transform_ids_schema.clone(),
                    "pivot": {},
                    "axis": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "angle": { "type": "number", "description": "radians, right-handed about axis" },
                    "copy": {
                        "type": "object",
                        "properties": {
                            "count": { "type": "integer", "minimum": 1, "maximum": 1000 },
                            "linked": copy_linked_schema.clone()
                        },
                        "additionalProperties": false
                    }
                },
                "required": ["ids", "pivot", "axis", "angle"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "ids": { "type": "array", "items": { "type": "string" } },
                    "linked": copy_wraps_schema.clone()
                }
            });
            cmd.refusals = vec![
                "unknown_entity",
                "mixed_selection_unsupported",
                "sketch_copy_unsupported",
                "array_count_too_large",
                "empty_selection",
                "duplicate_member",
                "unknown_object",
                "unknown_group",
                "unknown_instance",
                "unknown_sketch",
                "unknown_occurrence",
                "unknown_component",
                "explode_session_scope",
                "component_expansion_exceeded",
            ];
        }
        {
            let cmd = commands.get_mut("hew.entity.flip").expect("declared above");
            cmd.implemented = true;
            let xyz = serde_json::json!({ "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 });
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "ids": transform_ids_schema.clone(),
                    "axis": { "type": "string", "enum": ["x", "y", "z"], "description": "the mirror plane's world normal (SketchUp's Flip Along); exactly one of axis and plane" },
                    "plane": {
                        "type": "object",
                        "properties": {
                            "point": { "description": "a point on the plane: [x, y, z] or a derived-point locator (§5.3)" },
                            "normal": xyz.clone()
                        },
                        "required": ["point", "normal"],
                        "additionalProperties": false,
                        "description": "the mirror plane in world coordinates; takes no about"
                    },
                    "about": {
                        "oneOf": [
                            { "type": "string", "enum": ["center", "origin"] },
                            xyz
                        ],
                        "description": "where an axis plane passes: center (the default, the selection's world bounding-box center), origin, or a point"
                    },
                    "copy": {
                        "type": "object",
                        "properties": { "linked": copy_linked_schema.clone() },
                        "additionalProperties": false,
                        "description": "one mirrored copy instead of flipping in place; a linked copy is another placement at a mirrored pose"
                    }
                },
                "required": ["ids"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "ids": { "type": "array", "items": { "type": "string" }, "description": "with copy: the mirrored copies (placements for a linked copy)" },
                    "linked": copy_wraps_schema.clone()
                },
                "description": "{} for a flip in place"
            });
            cmd.refusals = vec![
                "unknown_entity",
                "sketch_copy_unsupported",
                "empty_selection",
                "duplicate_member",
                "unknown_object",
                "unknown_group",
                "unknown_instance",
                "unknown_sketch",
                "unknown_occurrence",
                "unknown_component",
                "explode_session_scope",
                "component_expansion_exceeded",
                "too_many_occurrences",
                "singular",
                "degenerate_axis",
                "degenerate_texture_frame",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.entity.scale")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "ids": transform_ids_schema.clone(),
                    "anchor": {},
                    "factors": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["ids", "anchor", "factors"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec![
                "unknown_entity",
                "mixed_selection_unsupported",
                "empty_selection",
                "unknown_object",
                "unknown_group",
                "unknown_instance",
                "unknown_sketch",
                "unknown_occurrence",
                "unknown_component",
                "explode_session_scope",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.context.enter")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec![
                "unknown_entity",
                "explode_session_open",
                "explode_session_nested_group",
                "explode_session_pose_unsupported",
                "explode_session_grouped_instance",
                "unknown_group",
                "unknown_instance",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.context.exit")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec!["explode_session_not_open"];
        }
        {
            let cmd = commands
                .get_mut("hew.group.create")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "members": { "type": "array", "items": { "type": "string" } } },
                "required": ["members"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "group": { "type": "string" } },
                "required": ["group"]
            });
            cmd.refusals = vec![
                "unknown_entity",
                "empty_group",
                "duplicate_member",
                "mixed_parents",
                "unknown_object",
                "unknown_group",
                "unknown_instance",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.group.explode")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec!["unknown_entity", "unknown_group"];
        }
        {
            let cmd = commands
                .get_mut("hew.group.reparent")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "ids": { "type": "array", "items": { "type": "string" } },
                    "parent": { "type": ["string", "null"] }
                },
                "required": ["ids"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec![
                "unknown_entity",
                "empty_ids",
                "group_cycle",
                "explode_session_scope",
                "unknown_object",
                "unknown_group",
                "unknown_instance",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.component.create")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "members": { "type": "array", "items": { "type": "string" } } },
                "required": ["members"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "component": { "type": "string" }, "instance": { "type": "string" } },
                "required": ["component", "instance"]
            });
            cmd.refusals = vec![
                "unknown_entity",
                "empty_component",
                "duplicate_member",
                "nested_component_unsupported",
                "unknown_object",
                "unknown_group",
                "unknown_instance",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.component.place")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "component": { "type": "string" },
                    "pose": {}
                },
                "required": ["component"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "instance": { "type": "string" } },
                "required": ["instance"]
            });
            cmd.refusals = vec!["unknown_entity", "unknown_component", "singular"];
        }
        {
            let cmd = commands
                .get_mut("hew.component.import_definition")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "bytes_base64": {
                        "type": "string",
                        "description": "the file's bytes; at most 64 MiB together with all images"
                    },
                    "format": {
                        "type": "string",
                        "enum": ["hew", "skp", "glb", "gltf", "stl", "dae"],
                        "description": "the file's format; a picture format is refused"
                    },
                    "name": {
                        "type": "string",
                        "description": "the definition's name when this import creates it (a later import of the same bytes keeps the first name); at most 128 bytes, no control characters; default \"Imported\""
                    },
                    "units": {
                        "type": "string", "enum": ["m", "mm", "cm", "in"],
                        "description": "meters-per-source-unit hint; STL requires it, other formats ignore it"
                    },
                    "images": {
                        "type": "object",
                        "description": "COLLADA texture images by the uri the file uses: {bytes_base64, format: png|jpeg}",
                        "additionalProperties": { "type": "object" }
                    }
                },
                "required": ["bytes_base64", "format"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "definition": { "type": "string", "description": "the definition's cmp_ id, ready for hew.component.place" },
                    "reused": { "type": "boolean", "description": "true when an earlier import of the same bytes already made it; nothing changed" },
                    "report": {
                        "type": "object",
                        "properties": {
                            "objects": { "type": "integer", "description": "the definition's direct member objects" },
                            "members": { "type": "integer", "description": "all direct members: objects, groups, and nested instances" }
                        },
                        "required": ["objects", "members"]
                    }
                },
                "required": ["definition", "reused", "report"]
            });
            cmd.refusals = vec![
                "host_capability_missing",
                "units_required",
                "load_failed",
                "unsupported_format",
                "unsupported_for_definition",
                "import_too_large",
                "empty_group",
                "explode_session_scope",
                "component_expansion_exceeded",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.component.make_unique")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "instance": { "type": "string", "description": "a world placement: the same as path [instance]" },
                    "path": { "type": "array", "items": { "type": "string" }, "minItems": 1, "description": "instance ids, outermost first; the last is the placement to fork" },
                    "cascade": { "type": "boolean", "default": true, "description": "true: every instance on the path is made unique in turn, so only the placement at the end diverges; false: only the last definition is forked, in place inside its owner, so every placement of the owners changes" },
                    "deep": { "type": "boolean", "default": false, "description": "also give every nested definition of the fork a private copy" }
                },
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "component": { "type": "string", "description": "the forked definition" },
                    "instance": { "type": "string", "description": "the placement at the end of the path after the call: the leaf itself for a one-element path, its divergent twin (a value in remap) for a deeper one" },
                    "remap": {
                        "type": "object",
                        "additionalProperties": { "type": "string" },
                        "description": "every id the call minted, keyed by the id it copies, over every kind (objects, groups, instances, definitions); only new ids are values"
                    }
                },
                "required": ["component", "instance", "remap"]
            });
            cmd.refusals = vec![
                "unknown_entity",
                "unknown_occurrence",
                "unknown_instance",
                "unknown_component",
                "explode_session_scope",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.component.promote")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "component": { "type": "string", "description": "the linked set's definition (cmp_)" },
                    "name": { "type": "string", "description": "the component's name: not blank, at most 128 bytes, no control characters" }
                },
                "required": ["component", "name"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "component": { "type": "string" } },
                "required": ["component"],
                "additionalProperties": false
            });
            cmd.refusals = vec!["unknown_entity", "not_linked"];
        }
        {
            let cmd = commands
                .get_mut("hew.component.explode")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "instance": { "type": "string" } },
                "required": ["instance"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "objects": { "type": "array", "items": { "type": "string" } } },
                "required": ["objects"]
            });
            cmd.refusals = vec![
                "unknown_entity",
                "unknown_instance",
                "cannot_explode_reflected",
            ];
        }
        {
            let vec3 = serde_json::json!({ "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 });
            let cmd = commands
                .get_mut("hew.component.rebase")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "a definition (\"cmp_…\"), or an instance (\"ins_…\") meaning its definition" },
                    "origin": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "the new origin, in the definition's current local coordinates" },
                    "x": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "the new x axis, unit length, in the definition's current local coordinates" },
                    "y": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "the new y axis, unit length and perpendicular to x; z is x × y" }
                },
                "required": ["id", "origin", "x", "y"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "component": { "type": "string" },
                    "instances_updated": { "type": "integer", "minimum": 0, "description": "how many instances place the definition, world or nested in another definition; each pose was compensated" }
                },
                "required": ["component", "instances_updated"],
                "additionalProperties": false
            });
            cmd.refusals = vec!["unknown_entity", "invalid_frame", "rebase_during_session"];

            let cmd = commands
                .get_mut("hew.component.flip_instance")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "instance": { "type": "string", "description": "a world instance (\"ins_…\")" },
                    "axis": { "type": "string", "enum": ["x", "y", "z"], "description": "the definition-local axis to mirror across (the plane normal to it, through the local origin)" }
                },
                "required": ["instance", "axis"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "instance": { "type": "string" },
                    "pose": { "type": "array", "items": { "type": "number" }, "minItems": 12, "maxItems": 12, "description": "the new pose, row-major 3×4" },
                    "mirrored": { "type": "boolean", "description": "whether the new pose's determinant is negative" }
                },
                "required": ["instance", "pose", "mirrored"],
                "additionalProperties": false
            });
            cmd.refusals = vec!["unknown_entity"];

            let cmd = commands
                .get_mut("hew.entity.set_frame")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "an object (\"obj_…\") or group (\"grp_…\")" },
                    "origin": vec3.clone(),
                    "x": vec3.clone(),
                    "y": vec3,
                    "clear": { "type": "boolean", "description": "true removes the override; give it alone" }
                },
                "required": ["id"],
                "additionalProperties": false,
                "description": "origin, x, and y are coordinates in the face frame hew.query.part_frame reports for the part without an override (after its [length, width, thickness] swap): origin an offset along that frame's axes, x and y unit, perpendicular directions in it. Give all three, or clear: true."
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "source": { "type": "string", "enum": ["attribute", "face"], "description": "where hew.query.part_frame now takes the frame from" }
                },
                "required": ["id", "source"],
                "additionalProperties": false
            });
            cmd.refusals = vec!["unknown_entity", "not_a_part", "invalid_frame"];
        }
        {
            let cmd = commands
                .get_mut("hew.material.create")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "color": {
                        "type": "array",
                        "items": { "type": "integer", "minimum": 0, "maximum": 255 },
                        "minItems": 3,
                        "maxItems": 4,
                        "description": "required without a texture; with one it tints the image (default white: the image as-is)"
                    },
                    "texture": {
                        "type": "object",
                        "description": "an image texture: a PNG or JPEG, inline or a file path (hew-cli reads it), and the meters one tile covers",
                        "properties": {
                            "bytes_base64": { "type": "string", "description": "the encoded PNG/JPEG — exactly one of bytes_base64/path" },
                            "path": { "type": "string", "description": "an image file, read by hew-cli — exactly one of bytes_base64/path" },
                            "world_size": {
                                "type": "array",
                                "items": { "type": "number", "exclusiveMinimum": 0 },
                                "minItems": 2,
                                "maxItems": 2,
                                "description": "[width, height] in meters of one tile"
                            }
                        },
                        "required": ["world_size"],
                        "additionalProperties": false
                    }
                },
                "required": ["name"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "material": { "type": "string" } },
                "required": ["material"]
            });
            // Palette additions are deliberately not undoable in the
            // kernel (docs/design/api-kernel-map.md §1.9). A malformed
            // color/name/texture (not PNG/JPEG, bad base64, world_size not
            // positive) is a protocol error (`-32602`); only reading a
            // texture `path` can be refused.
            cmd.refusals = vec!["load_failed", "host_capability_missing"];
        }
        {
            let cmd = commands
                .get_mut("hew.material.paint")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "face": { "description": "a face locator {object | path, at} | {object | path, ray} | {\"$face\"}; through a path the definition's face is painted, so every copy shows it" },
                    "id": { "type": "string", "description": "an object (its base material), an instance (the placement's own material), or a definition (every object it owns)" },
                    "material": { "type": ["string", "null"] },
                    "scope": { "type": "string", "enum": ["instance", "definition"], "description": "for an instance id: the placement (default) or its definition; a definition id takes only \"definition\"" }
                },
                "required": ["material"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec![
                "unknown_entity",
                "unknown_object",
                "unknown_instance",
                "unknown_component",
                "unknown_face",
                "unknown_material",
                "locator_missed",
                "ambiguous_locator",
                "unknown_occurrence",
                "not_an_object",
                "explode_session_scope",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.material.set_default")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "material": { "type": ["string", "null"] }
                },
                "required": ["id", "material"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec!["unknown_entity", "unknown_object", "unknown_material"];
        }
        {
            let cmd = commands
                .get_mut("hew.material.set_opacity")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "material": { "type": "string" },
                    "alpha": { "type": "integer", "minimum": 0, "maximum": 255 }
                },
                "required": ["material", "alpha"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec!["unknown_entity", "unknown_material"];
        }
        {
            let cmd = commands
                .get_mut("hew.material.set_pbr")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "material": { "type": "string" },
                    "metallic": { "type": "number", "minimum": 0, "maximum": 1, "description": "0 dielectric to 1 metal" },
                    "roughness": { "type": "number", "minimum": 0, "maximum": 1, "description": "0 mirror to 1 fully diffuse" }
                },
                "required": ["material", "metallic", "roughness"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec!["unknown_entity", "unknown_material"];
        }
        {
            let cmd = commands
                .get_mut("hew.material.position_texture")
                .expect("declared above");
            cmd.implemented = true;
            let point = serde_json::json!({
                "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3
            });
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "face": { "description": "a face locator (§5.2)" },
                    "pins": {
                        "type": "object",
                        "description": "three corners of one texture tile, in the face's object frame (as a locator's `at`), projected onto the face: origin, one tile along the width (u), one tile along the height (v) — exactly one of pins/reset",
                        "properties": { "origin": point, "u": point, "v": point },
                        "required": ["origin", "u", "v"],
                        "additionalProperties": false
                    },
                    "reset": {
                        "type": "boolean",
                        "description": "true: back to the default projection at the material's tile size — exactly one of pins/reset"
                    }
                },
                "required": ["face"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec![
                "unknown_entity",
                "unknown_object",
                "unknown_face",
                "locator_missed",
                "ambiguous_locator",
                "face_token_unknown",
                "face_token_stale",
                "degenerate_pins",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.material.replace")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "from": { "type": ["string", "null"], "description": "the material to replace; null: unpainted object bases" },
                    "to": { "type": ["string", "null"], "description": "the material it becomes; null: unpaint" },
                    "object": { "type": "string", "description": "only within this object; omitted: the whole document, instance materials included" }
                },
                "required": ["from", "to"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec!["unknown_entity", "unknown_object"];
        }
        {
            let cmd = commands.get_mut("hew.tag.create").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                    "hidden": { "type": "boolean" }
                },
                "required": ["path"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "tag": { "type": "string" } },
                "required": ["tag"]
            });
            // set_tag_hidden (the kernel call behind tag creation) is
            // deliberately not undoable, and never refuses (§1.10) — a
            // malformed path is a protocol error, not a refusal.
            cmd.refusals = Vec::new();
        }
        {
            let cmd = commands.get_mut("hew.tag.assign").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "path": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                    "remove": { "type": "boolean" }
                },
                "required": ["id", "path"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec![
                "unknown_entity",
                "unknown_object",
                "unknown_group",
                "unknown_instance",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.tag.set_visible")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                    "visible": { "type": "boolean" }
                },
                "required": ["path", "visible"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            // Same not-undoable posture as hew.tag.create (§1.10).
            cmd.refusals = Vec::new();
        }
        {
            let cmd = commands.get_mut("hew.tag.delete").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "array", "items": { "type": "string" }, "minItems": 1 } },
                "required": ["path"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec!["unknown_tag"];
        }
        {
            let cmd = commands.get_mut("hew.tag.rename").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                    "new_path": { "type": "array", "items": { "type": "string" }, "minItems": 1 }
                },
                "required": ["path", "new_path"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec!["unknown_tag", "duplicate_tag", "invalid_tag_path"];
        }
        {
            let cmd = commands.get_mut("hew.guide.line").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "origin": {}, "direction": {} },
                "required": ["origin", "direction"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "guide": { "type": "string" } },
                "required": ["guide"]
            });
            cmd.refusals = vec![
                "degenerate_guide",
                "unknown_entity",
                "locator_missed",
                "ambiguous_locator",
                "no_such_point",
            ];
        }
        {
            let cmd = commands.get_mut("hew.guide.point").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "position": {} },
                "required": ["position"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "guide": { "type": "string" } },
                "required": ["guide"]
            });
            cmd.refusals = vec![
                "degenerate_guide",
                "unknown_entity",
                "locator_missed",
                "ambiguous_locator",
                "no_such_point",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.guide.angular")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "origin": {},
                    "plane_normal": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "base_dir": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "angle": { "type": "number", "description": "radians, right-handed about plane_normal" }
                },
                "required": ["origin", "plane_normal", "base_dir", "angle"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "guide": { "type": "string" } },
                "required": ["guide"]
            });
            cmd.refusals = vec![
                "degenerate_guide",
                "unknown_entity",
                "locator_missed",
                "ambiguous_locator",
                "no_such_point",
            ];
        }
        {
            let cmd = commands.get_mut("hew.guide.clear").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = Vec::new();
        }
        {
            // hew.scenes.* — named, saved views (docs/agents/HEW_API.md's Scenes
            // section). `camera_schema`/`display_schema`/`properties_schema`
            // are the wire shapes `crates/api/src/commands/scenes.rs`
            // parses, shared across `add`/`update`/`list`'s result.
            let camera_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "eye": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "target": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "up": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "projection": { "type": "string", "enum": ["perspective", "parallel"] },
                    "fov_deg": { "type": "number", "description": "perspective only; defaults to 35" },
                    "parallel_height": { "type": "number", "description": "the height the view spans at the target, meters (what a parallel view frames); defaults to what the perspective view spans there" }
                },
                "required": ["eye", "target"],
                "additionalProperties": false,
                "description": "an explicit camera to capture — no named-view shorthand, a Scene captures a concrete eye/target, not a fitted view; when omitted and the camera property is captured, falls back to the document's own saved working camera"
            });
            let display_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "grid": { "type": "boolean" },
                    "axes": { "type": "boolean" },
                    "guides": { "type": "boolean" }
                },
                "required": ["grid", "axes", "guides"],
                "additionalProperties": false,
                "description": "opaque editor display toggles: stored and returned, never interpreted by the kernel"
            });
            let properties_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "camera": { "type": "boolean" },
                    "hidden_nodes": { "type": "boolean" },
                    "hidden_tags": { "type": "boolean" },
                    "section": { "type": "boolean" },
                    "display": { "type": "boolean" }
                },
                "additionalProperties": false,
                "description": "which of the five capturable properties to (re-)capture; each defaults to true"
            });
            let camera_result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "eye": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "target": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "up": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "projection": { "type": "string", "enum": ["perspective", "parallel"] },
                    "fov_deg": { "type": "number" },
                    "parallel_height": { "type": "number", "description": "the height the view spans at the target, meters" }
                },
                "required": ["eye", "target", "up", "projection", "fov_deg", "parallel_height"]
            });
            let section_result_schema = serde_json::json!({
                "oneOf": [
                    { "type": "null" },
                    {
                        "type": "object",
                        "properties": {
                            "origin": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                            "normal": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                            "active": { "type": "boolean" }
                        },
                        "required": ["origin", "normal", "active"]
                    }
                ],
                "description": "null means captured-but-no-plane-placed"
            });
            let scene_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "sid": { "type": "integer" },
                    "name": { "type": "string" },
                    "description": { "type": "string" },
                    "transition_s": { "type": ["number", "null"], "description": "the Scene's own transition time in seconds; null is the document's own" },
                    "delay_s": { "type": ["number", "null"], "description": "the Scene's own delay in seconds; null is the document's own" },
                    "props": {
                        "type": "object",
                        "properties": {
                            "camera": { "type": "boolean" },
                            "hidden_nodes": { "type": "boolean" },
                            "hidden_tags": { "type": "boolean" },
                            "section": { "type": "boolean" },
                            "display": { "type": "boolean" }
                        },
                        "required": ["camera", "hidden_nodes", "hidden_tags", "section", "display"]
                    },
                    "camera": camera_result_schema.clone(),
                    "section": section_result_schema.clone(),
                    "display": display_schema.clone(),
                    "view_style": {
                        "type": "object",
                        "properties": {
                            "background": { "type": "array", "items": { "type": "integer" }, "minItems": 3, "maxItems": 3 },
                            "sky": { "oneOf": [{ "type": "array", "items": { "type": "integer" }, "minItems": 3, "maxItems": 3 }, { "type": "null" }] },
                            "ground": { "oneOf": [{ "type": "array", "items": { "type": "integer" }, "minItems": 3, "maxItems": 3 }, { "type": "null" }] },
                            "axes": { "type": "boolean" }
                        },
                        "required": ["background", "sky", "ground", "axes"],
                        "description": "the look the Scene restores with its display property: backdrop (sRGB background, sky and ground or null) and whether the model axes are drawn; captured from the document's own view style, so absent for a document shown Hew's way"
                    }
                },
                "required": ["id", "sid", "name", "description", "props"],
                "description": "camera/display are present only when that property is captured AND has something to report; section is present (possibly null) whenever the section property is captured"
            });

            let cmd = commands.get_mut("hew.scenes.list").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "scenes": { "type": "array", "items": scene_schema } },
                "required": ["scenes"]
            });
            cmd.refusals = Vec::new();

            let cmd = commands.get_mut("hew.scenes.add").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "must be non-empty and unused; auto-named \"Scene N\" when omitted" },
                    "description": { "type": "string" },
                    "camera": camera_schema.clone(),
                    "display": display_schema.clone(),
                    "properties": properties_schema.clone(),
                    "after": { "type": "string", "description": "insert after this Scene's id; appended at the end when omitted" }
                },
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "sid": { "type": "integer" },
                    "name": { "type": "string" }
                },
                "required": ["id", "sid", "name"]
            });
            cmd.refusals = vec!["duplicate_scene_name", "empty_scene_name", "unknown_scene"];

            let cmd = commands
                .get_mut("hew.scenes.update")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "properties": properties_schema.clone(),
                    "camera": camera_schema,
                    "display": display_schema.clone(),
                    "transition_s": { "type": ["number", "null"], "minimum": 0, "description": "the Scene's own transition time in seconds; null returns it to the document's own" },
                    "delay_s": { "type": ["number", "null"], "minimum": 0, "description": "the Scene's own delay in seconds; null returns it to the document's own" }
                },
                "required": ["id"],
                "additionalProperties": false,
                "description": "properties defaults to the Scene's currently captured set (re-capture, never widen) when omitted; with only transition_s/delay_s given, nothing is re-captured"
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec!["unknown_scene"];

            let cmd = commands
                .get_mut("hew.scenes.rename")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "id": { "type": "string" }, "name": { "type": "string" } },
                "required": ["id", "name"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec!["unknown_scene", "duplicate_scene_name", "empty_scene_name"];

            let cmd = commands
                .get_mut("hew.scenes.describe")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "id": { "type": "string" }, "description": { "type": "string" } },
                "required": ["id", "description"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec!["unknown_scene"];

            let cmd = commands
                .get_mut("hew.scenes.remove")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec!["unknown_scene"];

            let cmd = commands
                .get_mut("hew.scenes.reorder")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "index": { "type": "integer", "minimum": 0, "description": "tab-order position; clamped to the end for an out-of-range index, never refused" }
                },
                "required": ["id", "index"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec!["unknown_scene"];

            let cmd = commands
                .get_mut("hew.scenes.apply")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "camera": camera_result_schema,
                    "section": section_result_schema,
                    "hidden_object_ids": { "type": "array", "items": { "type": "string" } },
                    "hidden_instance_ids": { "type": "array", "items": { "type": "string" } }
                },
                "description": "each key present only when the Scene captured that property; hidden_object_ids/hidden_instance_ids appear as a pair (possibly empty arrays) whenever hidden_nodes or hidden_tags is captured, and are OMITTED entirely — not empty-arrayed — when neither is, so a partial-capture Scene can never read as \"show everything\""
            });
            cmd.refusals = vec!["unknown_scene"];
        }
        {
            let cmd = commands.get_mut("hew.attr.get").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "target": attr_target_schema(),
                    "ns": { "type": "string" }
                },
                "required": ["target"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({ "type": "object" });
            cmd.refusals = vec![
                "unknown_entity",
                "unknown_face",
                "unknown_edge",
                "not_a_solid_edge",
                "locator_missed",
                "ambiguous_locator",
                "face_token_unknown",
                "face_token_stale",
                "unknown_occurrence",
                "not_an_object",
                "explode_session_scope",
                "unknown_object",
                "unknown_group",
                "unknown_instance",
                "unknown_sketch",
                "unknown_guide",
                "unknown_material",
                "unknown_component",
                "unknown_tag",
            ];
        }
        {
            let cmd = commands.get_mut("hew.attr.set").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "target": attr_target_schema(),
                    "ns": { "type": "string" },
                    "key": { "type": "string" },
                    "value": {}
                },
                "required": ["target", "ns", "key", "value"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec![
                "unknown_entity",
                "unknown_face",
                "unknown_edge",
                "not_a_solid_edge",
                "locator_missed",
                "ambiguous_locator",
                "face_token_unknown",
                "face_token_stale",
                "unknown_occurrence",
                "not_an_object",
                "explode_session_scope",
                "invalid_attr_name",
                "reserved_attr_namespace",
                "attr_namespace_not_granted",
                "non_finite_attr_value",
                "attr_value_too_deep",
                "unrepresentable_attr_value",
                "unknown_object",
                "unknown_group",
                "unknown_instance",
                "unknown_sketch",
                "unknown_guide",
                "unknown_material",
                "unknown_component",
                "unknown_tag",
            ];
        }
        {
            let cmd = commands.get_mut("hew.attr.delete").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "target": attr_target_schema(),
                    "ns": { "type": "string" },
                    "key": { "type": "string" }
                },
                "required": ["target", "ns"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec![
                "unknown_entity",
                "unknown_face",
                "unknown_edge",
                "not_a_solid_edge",
                "locator_missed",
                "ambiguous_locator",
                "face_token_unknown",
                "face_token_stale",
                "unknown_occurrence",
                "not_an_object",
                "explode_session_scope",
                "invalid_attr_name",
                "reserved_attr_namespace",
                "attr_namespace_not_granted",
                "unknown_attr",
                "unknown_object",
                "unknown_group",
                "unknown_instance",
                "unknown_sketch",
                "unknown_guide",
                "unknown_material",
                "unknown_component",
                "unknown_tag",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.history.undo")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "expected_label": { "type": "string" } },
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec![
                "expected_label_mismatch",
                "nothing_to_undo",
                "inverse_failed",
                "inverse_diverged",
            ];
        }
        {
            let cmd = commands
                .get_mut("hew.history.redo")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec!["nothing_to_redo", "inverse_failed", "inverse_diverged"];
        }
        {
            let cmd = commands
                .get_mut("hew.history.status")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            let entry_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "label": { "type": "string" },
                    "origin": {},
                    "bookkeeping": { "type": "boolean" }
                },
                "required": ["label", "origin", "bookkeeping"]
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "undo_depth": { "type": "integer" },
                    "redo_depth": { "type": "integer" },
                    "top": {},
                    "saved_depth": {},
                    "entries": {
                        "type": "object",
                        "properties": {
                            "undo": { "type": "array", "items": entry_schema.clone() },
                            "redo": { "type": "array", "items": entry_schema }
                        },
                        "required": ["undo", "redo"]
                    }
                },
                "required": ["undo_depth", "redo_depth", "top", "saved_depth", "entries"]
            });
            cmd.refusals = Vec::new();
        }

        // hew.sketch.* / hew.solid.* — Wave B (docs/design/
        // api-implementation-conventions.md, api-kernel-map.md
        // §1.1-1.4, §7): honest schemas and refusal inventories for
        // `crates/api/src/commands/{sketch,solid}.rs`.
        {
            let point_schema = serde_json::json!({
                "oneOf": [
                    { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    { "type": "object", "description": "a derived-point locator (HEW_API.md §5.3)" }
                ]
            });
            let plane_schema = serde_json::json!({
                "type": "object",
                "description": "HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>}"
            });
            let detached_schema = serde_json::json!({
                "type": "boolean",
                "description": "keep the shape detached: it joins no face or drawing on its plane, and extruding it always makes a new Object; default false. Refused (invalid params) with a {sketch} plane spec, whose own flag decides"
            });
            // What a drawing edit closed (wireframe faces): a loop on one
            // plane is a face whichever sketches hold its lines, and faces
            // that enclose a space become a solid in the same undo step.
            let faces_closed_schema = serde_json::json!({
                "type": "array",
                "items": { "type": "string" },
                "description": "present when the edit closed a face on another plane (a loop on one plane is a face, whichever sketches hold its lines): the sketches that now hold such a face; a face a solid consumed in the same step is not listed"
            });
            let solid_schema = serde_json::json!({
                "type": "object",
                "description": "present when the edit's faces enclosed a space and became a solid in the same undo step: the first such solid; `grew` when the shape closed against an existing solid and that solid grew (its new id is `object_id`, carrying the old one's name and tags)",
                "properties": {
                    "object_id": { "type": "string" },
                    "grew": { "type": "boolean" }
                },
                "required": ["object_id", "grew"]
            });
            let sketch_result_schema = serde_json::json!({
                "type": "object",
                "description": "drawing on the ground, a new plane, or into a sketch; also every `detached: true` draw, which makes one new detached sketch whatever the plane spec",
                "properties": {
                    "sketch": { "type": "string" },
                    "region_ids": { "type": "array", "items": { "type": "string" } },
                    "region_id": { "type": "string", "description": "present when exactly one region resulted (HEW_API.md §6.1's example)" },
                    "curve_id": { "type": "string", "description": "present when this command began a curve chain" },
                    "faces_closed": faces_closed_schema.clone(),
                    "solid": solid_schema.clone()
                },
                "required": ["sketch", "region_ids"]
            });
            // The face-imprint result shape (docs/agents/HEW_API.md §5.2, §5.4):
            // on-face drawing never touches a sketch, so its result is just
            // the reshaped solid's public id — a face has no public id of
            // its own, so whichever faces the cut produced are addressed
            // only through the transaction-scoped face tokens `token_doc`
            // documents. Every draw command's result is a `oneOf` of this
            // and `sketch_result_schema`, distinguished by which plane
            // spec variant the call actually used.
            let touched_units_schema = serde_json::json!({
                "type": "array",
                "description": "the plane units the call touched: every drawing edge lying along what was drawn, then every region whose boundary carries it",
                "items": {
                    "type": "object",
                    "properties": {
                        "kind": { "type": "string", "enum": ["edge", "region"] },
                        "key": { "type": "string", "description": "edg_<hex16> or rgn_<hex16>, as hew.sketch.units lists them" },
                        "store": { "type": "string", "enum": ["topology", "owned", "free"], "description": "topology: the solid's own face structure; owned: drawing kept on the solid's face; free: drawing over no face" },
                        "object_id": { "type": "string", "description": "the owning solid, for topology and owned" },
                        "sketch_id": { "type": "string", "description": "the sketch holding the drawing, for owned and free when it has one" },
                        "detached": { "type": "boolean", "description": "true on a free store that is a detached sketch; absent otherwise" }
                    },
                    "required": ["kind", "key", "store"]
                }
            });
            let face_imprint_result_schema = |token_doc: &str| {
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "object_id": { "type": "string" },
                        "units": touched_units_schema.clone(),
                        "faces_closed": faces_closed_schema.clone(),
                        "solid": solid_schema.clone()
                    },
                    "required": ["object_id", "units"],
                    "description": format!(
                        "on-face drawing (plane spec {{\"face\": <locator>}}) draws through the drafting engine instead of creating a sketch region; `units` lists the plane units the draw touched; mints {token_doc} when the draw produced a face"
                    )
                })
            };
            let split_face_tokens = "transaction-scoped face tokens \"a\"/\"b\" naming the faces either side of the path's last cut — for a single cut, the two faces it produced (HEW_API.md §5.2/§5.4)";
            let split_face_inner_tokens = "transaction-scoped face tokens \"face\" (the new sub-face) and \"parent\" (the reshaped parent, now carrying the loop as a hole) (HEW_API.md §5.2/§5.4)";
            // Refusals common to every face-imprint path (both
            // `SplitFace` and `SplitFaceInner` route through
            // `apply_object_op` — or, for a component-definition
            // member's face, `apply_def_op` — and share this
            // backstop/liveness inventory; `unknown_component` is the
            // latter's own liveness gate).
            let face_imprint_common_refusals = || {
                vec![
                    "unknown_object",
                    "unknown_component",
                    "unknown_face",
                    "point_not_on_face",
                    "would_corrupt",
                ]
            };

            let cmd = commands
                .get_mut("hew.sketch.draw_line")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "plane": plane_schema,
                    "points": { "type": "array", "items": point_schema, "minItems": 2 },
                    "detached": detached_schema.clone()
                },
                "required": ["plane", "points"]
            });
            cmd.result_schema = serde_json::json!({
                "oneOf": [sketch_result_schema.clone(), face_imprint_result_schema(split_face_tokens)]
            });
            cmd.refusals = [
                vec![
                    "point_off_plane",
                    "degenerate_segment",
                    "unknown_sketch",
                    "unknown_entity",
                    "locator_missed",
                    "ambiguous_locator",
                ],
                face_imprint_common_refusals(),
            ]
            .concat();

            let cmd = commands
                .get_mut("hew.sketch.draw_rect")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "plane": plane_schema,
                    "corner_a": point_schema,
                    "corner_b": point_schema,
                    "detached": detached_schema.clone()
                },
                "required": ["plane", "corner_a", "corner_b"]
            });
            cmd.result_schema = serde_json::json!({
                "oneOf": [sketch_result_schema.clone(), face_imprint_result_schema(split_face_inner_tokens)]
            });
            cmd.refusals = [
                vec![
                    "point_off_plane",
                    "degenerate_segment",
                    "unknown_sketch",
                    "unknown_entity",
                    "locator_missed",
                    "ambiguous_locator",
                ],
                face_imprint_common_refusals(),
            ]
            .concat();

            let cmd = commands
                .get_mut("hew.sketch.draw_circle")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "plane": plane_schema,
                    "center": point_schema,
                    "radius": { "type": "number", "exclusiveMinimum": 0 },
                    "segments": {
                        "type": "integer",
                        "description": "facet count; defaults to 48, must fall in [MIN_CIRCLE_SEGMENTS, MAX_CIRCLE_SEGMENTS] = [24, 1024]"
                    },
                    "detached": detached_schema.clone()
                },
                "required": ["plane", "center", "radius"]
            });
            cmd.result_schema = serde_json::json!({
                "oneOf": [sketch_result_schema.clone(), face_imprint_result_schema(split_face_inner_tokens)]
            });
            cmd.refusals = [
                vec![
                    "point_off_plane",
                    "degenerate_curve",
                    "degenerate_segment",
                    "unknown_sketch",
                    "segments_below_floor",
                    "segments_above_cap",
                    "unknown_entity",
                    "locator_missed",
                    "ambiguous_locator",
                    "curve_claim_off_loop",
                ],
                face_imprint_common_refusals(),
            ]
            .concat();

            let cmd = commands
                .get_mut("hew.sketch.draw_arc")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "plane": plane_schema,
                    "center": point_schema,
                    "radius": { "type": "number", "exclusiveMinimum": 0 },
                    "start_angle": { "type": "number", "description": "radians" },
                    "end_angle": { "type": "number", "description": "radians" },
                    "segments": {
                        "type": "integer",
                        "description": "facet count; defaults proportionally to the sweep, capped at MAX_CIRCLE_SEGMENTS = 1024. A single chord (1) is fine for close: \"open\", but close: \"pie\"/\"segment\" needs at least 2 — a single chord can't form a non-degenerate closed loop — and the proportional default is floored at 2 for those modes too."
                    },
                    "close": {
                        "type": "string",
                        "enum": ["open", "pie", "segment"],
                        "description": "how the arc's ends are closed: \"open\" (default, a bare arc), \"pie\" (closed wedge — two radii to the center), or \"segment\" (closed circular segment — the chord). \"pie\"/\"segment\" commit a closed profile (a region in plane/sketch mode, a SplitFaceInner loop in face mode) like draw_rect/draw_circle, and need at least 2 segments (see \"segments\"). Must be \"open\" when the sweep is a full turn (already closed)."
                    },
                    "detached": detached_schema.clone()
                },
                "required": ["plane", "center", "radius", "start_angle", "end_angle"]
            });
            cmd.result_schema = serde_json::json!({
                "oneOf": [
                    sketch_result_schema.clone(),
                    face_imprint_result_schema(
                        "transaction-scoped face tokens \"a\"/\"b\" (an open sweep — a plain boundary-to-boundary cut) or \"face\"/\"parent\" (a full-turn sweep, or a pie/segment close — closed, imprinted like a circle) (HEW_API.md §5.2/§5.4)"
                    )
                ]
            });
            cmd.refusals = [
                vec![
                    "point_off_plane",
                    "degenerate_curve",
                    "degenerate_segment",
                    "unknown_sketch",
                    "segments_above_cap",
                    "unknown_entity",
                    "locator_missed",
                    "ambiguous_locator",
                    // Open-sweep, `close: "open"` (`SplitFace`) face-mode
                    // refusals:
                    // Closed-loop (`SplitFaceInner`) face-mode refusals —
                    // a full-turn sweep, or `close: "pie"`/`"segment"`:
                    "curve_claim_off_loop",
                ],
                face_imprint_common_refusals(),
            ]
            .concat();

            let cmd = commands
                .get_mut("hew.sketch.draw_polygon")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "plane": plane_schema,
                    "center": point_schema,
                    "radius": { "type": "number", "exclusiveMinimum": 0 },
                    "sides": { "type": "integer", "minimum": 3, "maximum": 120 },
                    "detached": detached_schema.clone()
                },
                "required": ["plane", "center", "radius", "sides"]
            });
            cmd.result_schema = serde_json::json!({
                "oneOf": [sketch_result_schema.clone(), face_imprint_result_schema(split_face_inner_tokens)]
            });
            cmd.refusals = [
                vec![
                    "point_off_plane",
                    "degenerate_curve",
                    "degenerate_segment",
                    "unknown_sketch",
                    "unknown_entity",
                    "locator_missed",
                    "ambiguous_locator",
                    "sides_above_cap",
                ],
                face_imprint_common_refusals(),
            ]
            .concat();

            let cmd = commands
                .get_mut("hew.sketch.offset")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "region": { "type": "string", "description": "a sketch region id; with `plane`, a region unit key rgn_<hex16> from hew.sketch.units (a face or plane region)" },
                    "distance": { "type": "number", "description": "positive grows the material, negative shrinks it" },
                    "plane": plane_schema.clone()
                },
                "required": ["region", "distance"]
            });
            cmd.result_schema = serde_json::json!({
                "oneOf": [
                    {
                        "type": "object",
                        "properties": {
                            "sketch": { "type": "string" },
                            "region_ids": { "type": "array", "items": { "type": "string" } },
                            "region_id": { "type": "string", "description": "present when exactly one region resulted" },
                            "curve_ids": { "type": "array", "items": { "type": "string" } }
                        },
                        "required": ["sketch", "region_ids", "curve_ids"]
                    },
                    {
                        "type": "object",
                        "description": "the unit-key form (`plane` given): the plane units the offset made",
                        "properties": { "units": touched_units_schema.clone() },
                        "required": ["units"]
                    }
                ]
            });
            cmd.refusals = vec![
                "unknown_region",
                "malformed_region",
                "offset_too_small",
                "offset_collapsed",
                "unknown_sketch",
                "unknown_entity",
                "unknown_unit",
                "draft_boundary_edge_touched",
            ];

            // hew.sketch.units / move / rotate / scale / copy / delete —
            // the drafting-engine unit verbs (commands/draft.rs).
            let units_param = serde_json::json!({
                "type": "array",
                "minItems": 1,
                "items": { "type": "string" },
                "description": "unit keys from hew.sketch.units: isl_<hex16> (an island: all its edges), edg_<hex16> (one edge), crv_<hex16> (every facet of one circle), rgn_<hex16> (a region's ring edges that are drawing edges)"
            });
            let edit_result = serde_json::json!({
                "type": "object",
                "properties": {
                    "units": touched_units_schema.clone(),
                    "faces_closed": faces_closed_schema.clone(),
                    "solid": solid_schema.clone()
                },
                "required": ["units"],
                "description": "the plane units the edit made (new keys after the edit); empty for delete"
            });
            // Delete removes drawing and so closes nothing.
            let delete_result = serde_json::json!({
                "type": "object",
                "properties": { "units": touched_units_schema.clone() },
                "required": ["units"],
                "description": "the plane units the edit made (new keys after the edit); empty for delete"
            });
            let edit_refusals = || {
                vec![
                    "unknown_unit",
                    "draft_boundary_edge_touched",
                    "unknown_sketch",
                    "unknown_entity",
                    "locator_missed",
                    "ambiguous_locator",
                    "point_off_plane",
                    "degenerate_segment",
                ]
            };

            let cmd = commands
                .get_mut("hew.sketch.units")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "plane": {
                        "type": "object",
                        "description": "HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>}. A plane or face spec lists the plane's arrangement, which no detached sketch joins; {sketch:<id>} on a detached sketch lists exactly that sketch"
                    }
                },
                "required": ["plane"]
            });
            let point_array = serde_json::json!({ "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "units": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "key": { "type": "string", "description": "isl_<hex16>" },
                                "edges": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "key": { "type": "string", "description": "edg_<hex16>" },
                                            "a": point_array.clone(),
                                            "b": point_array.clone(),
                                            "store": { "type": "string", "enum": ["topology", "owned", "free"] },
                                            "object_id": { "type": "string" },
                                            "sketch_id": { "type": "string" },
                                            "detached": { "type": "boolean", "description": "true on a free store that is a detached sketch (listed only by a {sketch} plane spec naming it); absent otherwise" },
                                            "curve": {
                                                "type": "object",
                                                "description": "present when the edge is a facet of an analytic circle",
                                                "properties": {
                                                    "key": { "type": "string", "description": "crv_<hex16>" },
                                                    "center": point_array.clone(),
                                                    "radius": { "type": "number" }
                                                }
                                            }
                                        },
                                        "required": ["key", "a", "b", "store"]
                                    }
                                }
                            },
                            "required": ["key", "edges"]
                        }
                    },
                    "regions": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "key": { "type": "string", "description": "rgn_<hex16>" },
                                "outer": { "type": "array", "items": point_array.clone() },
                                "holes": { "type": "array", "items": { "type": "array", "items": point_array.clone() } },
                                "object_id": { "type": "string", "description": "the solid under the region, when one is" },
                                "face": { "type": "object", "description": "a face locator ({object, at}) for the region when it IS a face of that solid — pass it to hew.solid.push_pull" },
                                "extrudable": { "type": "boolean", "description": "true when the region lies over no face and hew.solid.extrude takes it as a new Object" }
                            },
                            "required": ["key", "outer", "holes", "extrudable"]
                        }
                    }
                },
                "required": ["units", "regions"]
            });
            cmd.refusals = vec![
                "unknown_sketch",
                "unknown_entity",
                "locator_missed",
                "ambiguous_locator",
            ];

            let cmd = commands.get_mut("hew.sketch.move").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "plane": plane_schema.clone(),
                    "units": units_param.clone(),
                    "offset": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "translation [x, y, z] in meters; must stay in the plane" }
                },
                "required": ["plane", "units", "offset"]
            });
            cmd.result_schema = edit_result.clone();
            cmd.refusals = edit_refusals();

            let cmd = commands
                .get_mut("hew.sketch.rotate")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "plane": plane_schema.clone(),
                    "units": units_param.clone(),
                    "axis_point": point_schema.clone(),
                    "angle": { "type": "number", "description": "radians, counter-clockwise seen from the plane normal side" }
                },
                "required": ["plane", "units", "axis_point", "angle"]
            });
            cmd.result_schema = edit_result.clone();
            cmd.refusals = edit_refusals();

            let cmd = commands
                .get_mut("hew.sketch.scale")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "plane": plane_schema.clone(),
                    "units": units_param.clone(),
                    "center": point_schema.clone(),
                    "factor": { "type": "number", "description": "uniform scale; give this or `factors`" },
                    "factors": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "per-axis [sx, sy, sz] in world axes; must keep the drawing in its plane" }
                },
                "required": ["plane", "units", "center"]
            });
            cmd.result_schema = edit_result.clone();
            cmd.refusals = edit_refusals();

            let cmd = commands.get_mut("hew.sketch.copy").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "plane": plane_schema.clone(),
                    "units": units_param.clone(),
                    "offset": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["plane", "units", "offset"]
            });
            cmd.result_schema = edit_result.clone();
            cmd.refusals = edit_refusals();

            let cmd = commands
                .get_mut("hew.sketch.delete")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "plane": plane_schema.clone(),
                    "units": units_param.clone()
                },
                "required": ["plane", "units"]
            });
            cmd.result_schema = delete_result;
            cmd.refusals = edit_refusals();

            let cmd = commands
                .get_mut("hew.solid.from_faces")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "sketches": {
                        "type": "array",
                        "minItems": 1,
                        "items": { "type": "string" },
                        "description": "skt_ ids of free sketches; their lines first fill every face they can close (as a draw would), then every face (faced region) they hold, and every face so filled, seeds the search, which gathers every face connected to one through shared edges (two faces with the same loop on one plane are one face: the ordinary one, else the earlier drawn)"
                    }
                },
                "required": ["sketches"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "description": "the faces enclosed a space and became a solid, one undo step; the faces' sketches lose the consumed edges (an emptied sketch is deleted)",
                "properties": {
                    "object_id": { "type": "string", "description": "the first solid made" },
                    "grew": { "type": "boolean", "description": "the shape closed against an existing solid, which grew into object_id (carrying its name and tags)" }
                },
                "required": ["object_id", "grew"]
            });
            cmd.refusals = vec![
                "faces_not_closed",
                "loop_not_planar",
                "faces_not_solid",
                "unknown_entity",
            ];
        }
        {
            let face_locator_schema = serde_json::json!({
                "type": "object",
                "description": "HEW_API.md §5.2 face locator: {object,at} | {object,ray} | {\"$face\":\"label#key\"}"
            });
            let point_schema = serde_json::json!({
                "oneOf": [
                    { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    { "type": "object", "description": "a derived-point locator (HEW_API.md §5.3)" }
                ]
            });

            let cmd = commands
                .get_mut("hew.solid.extrude")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "region": { "type": "string", "description": "a sketch region id; with `plane`, a region unit key rgn_<hex16> from hew.sketch.units" },
                    "distance": { "type": "number" },
                    "plane": { "type": "object", "description": "HEW_API.md §7 plane spec naming the drafting context a unit-key `region` belongs to. A region over no face extrudes as a new Object; a region that is a face of a solid push/pulls that face (result as hew.solid.push_pull); a region over a face that cannot hold it refuses region_not_pushable" }
                },
                "required": ["region", "distance"]
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "object_id": { "type": "string" },
                    "object_ids": { "type": "array", "items": { "type": "string" }, "description": "a through-cut's resulting pieces (unit-key form over a face, as hew.solid.push_pull)" }
                },
                "description": "mints face tokens \"base\", \"top\", \"side.<n>\" (boundary-loop order) — HEW_API.md §5.4's normative example — when a new Object results"
            });
            cmd.refusals = vec![
                "distance_too_small",
                "degenerate_geometry",
                "unknown_region",
                "region_has_no_face",
                "unknown_entity",
                "unknown_unit",
                "region_not_pushable",
                "region_touches_hole",
            ];

            let cmd = commands
                .get_mut("hew.solid.push_pull")
                .expect("declared above");
            cmd.implemented = true;
            let mut push_face = face_locator_schema.clone();
            push_face["description"] = serde_json::json!(
                "HEW_API.md §5.2 face locator: {object | path, at} | {object | path, ray} | {\"$face\":\"label#key\"}; path (or object as {path: [...]}) is an occurrence path [instance, …, object] that reaches a definition member through a placement, every placement following (at and ray stay in the object's own frame)"
            );
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "face": push_face,
                    "distance": { "type": "number" }
                },
                "required": ["face", "distance"]
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "object_id": { "type": "string", "description": "the pushed/pulled object, still standing (in-place or sub-face case)" },
                    "object_ids": { "type": "array", "items": { "type": "string" }, "description": "a through-cut's resulting pieces, replacing the source object" }
                },
                "description": "exactly one of object_id / object_ids is present, depending on the tool's three-way branch (HEW_API.md §7 semantics notes)"
            });
            cmd.refusals = vec![
                "object_not_solid",
                "distance_too_small",
                "would_vanish",
                "non_manifold_result",
                "not_a_sub_face",
                "radius_vanishes",
                "wall_neighbor_non_planar",
                "unknown_face",
                "unknown_object",
                "unknown_component",
                "grouped_operand",
                "unknown_entity",
                "locator_missed",
                "ambiguous_locator",
                "face_token_unknown",
                "face_token_stale",
                "unknown_occurrence",
                "explode_session_scope",
                "not_an_object",
            ];

            let mut operand_a = occurrence_locator_schema();
            operand_a["description"] = serde_json::json!(
                "a world node id (obj_/grp_/ins_), or {path: [instance, …, object]}: a definition member through one placement, edited in the definition so every placement follows"
            );
            let mut operand_b = occurrence_locator_schema();
            operand_b["description"] = serde_json::json!(
                "a world node id; with a by path, a world object (subtract only, consumed) or {path} to a member of the same definition by the same path"
            );
            // Subtract alone takes a transient cutter beside a path `a`
            // (Addendum B6): a SolidSpec prism or box, world coordinates.
            let mut subtract_b = operand_b.clone();
            subtract_b["oneOf"]
                .as_array_mut()
                .expect("a oneOf")
                .extend([
                    serde_json::json!({
                        "type": "object",
                        "properties": { "prism": { "type": "object", "description": "{plane: {origin, normal}, loops: [[[x,y,z], ...], ...], distance}: the first loop outer, the rest holes" } },
                        "required": ["prism"],
                        "additionalProperties": false
                    }),
                    serde_json::json!({
                        "type": "object",
                        "properties": { "box": { "type": "object", "description": "{min, max, pose?}" } },
                        "required": ["box"],
                        "additionalProperties": false
                    }),
                ]);
            subtract_b["description"] = serde_json::json!(
                "a world node id; with a by path, a world object (consumed), {path} to a member of the same definition by the same path, or a transient cutter {prism} | {box} in world coordinates (no object is created)"
            );
            let boolean_params = serde_json::json!({
                "type": "object",
                "properties": {
                    "a": operand_a,
                    "b": operand_b
                },
                "required": ["a", "b"]
            });
            // Union and intersect with a placement operand split it from
            // its copies, so they ask first (Addendum D §5).
            let mut join_params = boolean_params.clone();
            join_params["properties"]["unlink"] = serde_json::json!({
                "type": "boolean",
                "description": "true: a placement operand (an instance, a member by path, or a group holding one) is unlinked and exploded into plain solids, then joined; its other copies keep the shared solid. Without it such an operand refuses would_unlink. Ignored when no operand is a placement"
            });
            let join_result = serde_json::json!({
                "type": "object",
                "properties": {
                    "result": { "type": "string" },
                    "unlinked": { "type": "array", "items": { "type": "string" }, "description": "present when unlink: true split placements off: the placements exploded" }
                },
                "required": ["result"]
            });
            let boolean_result = serde_json::json!({
                "type": "object",
                "properties": { "result": { "type": "string" } },
                "required": ["result"]
            });
            let boolean_refusals = || {
                vec![
                    "boolean_operand_has_instance",
                    "boolean_operand_not_solid",
                    "boolean_operand_empty",
                    "grouped_operand",
                    "degenerate_contact",
                    "unknown_object",
                    "unknown_group",
                    "unknown_instance",
                    "unknown_entity",
                    "unknown_occurrence",
                    "unknown_component",
                    "explode_session_scope",
                ]
            };

            let cmd = commands.get_mut("hew.solid.union").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = join_params.clone();
            cmd.result_schema = join_result.clone();
            cmd.refusals = boolean_refusals();
            cmd.refusals.push("would_unlink");

            let cmd = commands
                .get_mut("hew.solid.subtract")
                .expect("declared above");
            cmd.implemented = true;
            let mut subtract_params = boolean_params.clone();
            subtract_params["properties"]["b"] = subtract_b;
            cmd.params_schema = subtract_params;
            cmd.result_schema = boolean_result.clone();
            cmd.refusals = boolean_refusals();
            // Subtract alone refuses when its cutter is disjoint from, or
            // only touches, the target: union and intersect have an honest
            // answer there (two shells; an empty result).
            cmd.refusals.push("nothing_removed");
            // A transient cutter's own refusals.
            cmd.refusals.extend(["mirrored_pose", "compute_too_large"]);

            let cmd = commands
                .get_mut("hew.solid.intersect")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = join_params;
            cmd.result_schema = join_result;
            cmd.refusals = boolean_refusals();
            cmd.refusals.push("would_unlink");

            let cmd = commands.get_mut("hew.solid.slice").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "object": {
                        "oneOf": [
                            { "type": "string" },
                            {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "array", "items": { "type": "string" }, "minItems": 1 }
                                },
                                "required": ["path"],
                                "additionalProperties": false
                            }
                        ],
                        "description": "a world object id, or {path: [instance, …, object]}: a definition member through one placement; the pieces stay in the definition and every placement follows"
                    },
                    "plane": {
                        "type": "object",
                        "properties": { "origin": point_schema.clone(), "normal": point_schema.clone() },
                        "required": ["origin", "normal"],
                        "description": "world coordinates, also through a path"
                    }
                },
                "required": ["object", "plane"]
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "positive": { "type": "string" }, "negative": { "type": "string" } },
                "required": ["positive", "negative"]
            });
            cmd.refusals = vec![
                "not_solid",
                "plane_misses_solid",
                "degenerate",
                "unknown_object",
                "unknown_entity",
                "unknown_occurrence",
                "unknown_instance",
                "unknown_component",
                "grouped_operand",
                "explode_session_scope",
            ];

            let cmd = commands
                .get_mut("hew.solid.follow_me")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "profile": {
                        "oneOf": [
                            { "type": "string", "description": "a sketch region id" },
                            { "type": "object", "properties": { "face": face_locator_schema.clone() }, "required": ["face"] }
                        ]
                    },
                    "path": {
                        "oneOf": [
                            { "type": "object", "properties": { "edges": { "type": "array" } }, "required": ["edges"], "description": "Reserved: no kernel mapping yet — refuses unimplemented" },
                            { "type": "object", "properties": { "face": face_locator_schema }, "required": ["face"] },
                            { "type": "object", "properties": { "curve": { "type": "string" } }, "required": ["curve"] }
                        ]
                    }
                },
                "required": ["profile", "path"]
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "object_id": { "type": "string" } },
                "required": ["object_id"]
            });
            cmd.refusals = vec![
                "empty_path",
                "unknown_path_edge",
                "path_branches",
                "path_disconnected",
                "path_segment_too_short",
                "profile_not_perpendicular",
                "follow_me_in_component_unsupported",
                "path_detached_from_profile",
                "path_reverses",
                "path_too_tight",
                "profile_crosses_axis",
                "partial_sweep_on_pole",
                "sweep_self_intersects",
                "sweep_degenerate",
                "unknown_region",
                "unknown_object",
                "unknown_face",
                "unknown_entity",
                "locator_missed",
                "ambiguous_locator",
                "unimplemented",
            ];

            // hew.solid.imprints / move_imprint / rotate_imprint /
            // scale_imprint / delete_imprint — editable imprints (§7's
            // `hew.solid` semantics notes; `commands/solid.rs`'s module
            // doc comment for the imprint-locator resolution these all
            // share).
            let imprint_locator_schema = serde_json::json!({
                "type": "object",
                "description": "HEW_API.md §5.2 face locator ({object,at} | {object,ray} | {\"$face\":\"label#key\"}) naming a sub-face; {object,at} also picks a CHORD when the point lies on one of its lines (a chord is tried first, then the sub-face containing the point)"
            });
            let point_or_derived_schema = serde_json::json!({
                "oneOf": [
                    { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    { "type": "object", "description": "a derived-point locator (HEW_API.md §5.3)" }
                ]
            });
            let imprint_refusals = || {
                vec![
                    "not_in_plane",
                    "not_an_imprint",
                    "draft_boundary_edge_touched",
                    "unknown_object",
                    "unknown_entity",
                    "locator_missed",
                    "ambiguous_locator",
                    "face_token_unknown",
                    "face_token_stale",
                ]
            };

            let cmd = commands
                .get_mut("hew.solid.imprints")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "object": { "type": "string" } },
                "required": ["object"]
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "imprints": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "kind": { "type": "string", "enum": ["sub_face", "chord"] },
                                "at": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "sub_face: an interior point clear of any nested sub-face; chord: the midpoint of the run's first segment — the locator to pass to move_imprint/rotate_imprint/scale_imprint/delete_imprint" },
                                "loop": { "type": "array", "items": { "type": "array" }, "description": "sub_face only: the outer loop's positions, cycle order" },
                                "curve": { "description": "sub_face only: {center, radius} when the whole loop is one drawn circle, else null" },
                                "curves": { "type": "array", "description": "per edge of the loop (sub_face) or run (chord), in order: {center, radius} when that edge is a facet of a drawn circle or arc, else null — a pie, segment, or edge-to-edge arc reports its arc here" },
                                "nested": { "type": "integer", "description": "sub_face only: count of sub-faces lying inside this one" },
                                "holes": { "type": "array", "items": { "type": "array" }, "description": "sub_face only: the hole loops' positions" },
                                "normal": { "type": "array", "items": { "type": "number" }, "description": "the drawing plane's normal" },
                                "region": { "type": "string", "description": "sub_face only: the region's unit key (rgn_…) on that plane, as hew.sketch.units reports it" },
                                "unit": { "type": "string", "description": "the island's unit key (isl_…) on that plane, as hew.sketch.units reports it" },
                                "path": { "type": "array", "items": { "type": "array" }, "description": "chord only: the run's vertex positions, first to last" }
                            },
                            "required": ["kind", "at"]
                        }
                    },
                    "curve_units": {
                        "type": "array",
                        "description": "drawn circles and arcs (whole or cut by other drawing into several runs) on the object's drawing planes: each {closed, curve: {center, radius}, runs: [{at, path}]}, a run that is a chord above carrying that chord's at locator — the whole drawn curve, a circle when closed, else an arc",
                        "items": {
                            "type": "object",
                            "properties": {
                                "closed": { "type": "boolean" },
                                "curve": { "type": "object" },
                                "runs": { "type": "array", "items": { "type": "object" } }
                            },
                            "required": ["closed", "curve", "runs"]
                        }
                    }
                },
                "required": ["imprints", "curve_units"]
            });
            cmd.refusals = vec!["unknown_object", "unknown_entity"];

            let cmd = commands
                .get_mut("hew.solid.move_imprint")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "imprint": imprint_locator_schema.clone(),
                    "offset": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["imprint", "offset"]
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "object_id": { "type": "string" } },
                "required": ["object_id"]
            });
            cmd.refusals = imprint_refusals();

            let cmd = commands
                .get_mut("hew.solid.rotate_imprint")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "imprint": imprint_locator_schema.clone(),
                    "angle": { "type": "number", "description": "radians" },
                    "about": point_or_derived_schema.clone()
                },
                "required": ["imprint", "angle", "about"]
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "object_id": { "type": "string" } },
                "required": ["object_id"]
            });
            cmd.refusals = [imprint_refusals(), vec!["no_such_point"]].concat();

            let cmd = commands
                .get_mut("hew.solid.scale_imprint")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "imprint": imprint_locator_schema.clone(),
                    "factor": { "type": "number", "description": "uniform scale factor, finite and nonzero; a negative factor turns the shape 180 degrees about `about` and scales it by |factor| (a mirror about a world axis of an axis-aligned drawing plane is hew.sketch.scale with a negative per-axis factor)" },
                    "about": point_or_derived_schema.clone()
                },
                "required": ["imprint", "factor", "about"]
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "object_id": { "type": "string" } },
                "required": ["object_id"]
            });
            cmd.refusals = [imprint_refusals(), vec!["no_such_point"]].concat();

            let cmd = commands
                .get_mut("hew.solid.delete_imprint")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "imprint": imprint_locator_schema },
                "required": ["imprint"]
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "object_id": { "type": "string" } },
                "required": ["object_id"]
            });
            cmd.refusals = imprint_refusals();
        }

        // The solitary commands that nevertheless change the document
        // (see `CommandDecl::mutates_document`). Everything else solitary
        // — attach, save, export, snapshot — leaves the document exactly
        // as it found it.
        for name in [
            "hew.history.undo",
            "hew.history.redo",
            "hew.doc.new",
            "hew.doc.open",
            // Additive, but a real mutation: it merges a file's objects in
            // (an `Imported` undo entry) — a live host must resync after it.
            "hew.doc.import",
            // Makes (or finds) an unplaced definition: one undo entry when
            // it is new, none when an earlier import's definition is reused.
            "hew.component.import_definition",
            // ReadOnly class (no undo bracket, no `$ref`-blocking transact
            // requirement) but a real mutation: it stamps `hew.library`
            // source provenance on the saved selection's nodes
            // (`Document::stamp_library_source`), which — like the tag
            // registry and `hew.scenes.*` above — is bookkeeping the
            // kernel never records as an undo step.
            "hew.library.save",
            // Host effects on the user's window; opening or closing a
            // session is a history step, so a live host resyncs after.
            "hew.view.enter",
            "hew.view.exit",
            "hew.view.exit_all",
        ] {
            commands
                .get_mut(name)
                .expect("declared above")
                .mutates_document = true;
        }

        // hew.compute.* (HEW_API.md §7 "Compute"): pure computations on
        // `SolidSpec` operands (`crate::solid_spec`), served on every host.
        {
            let xyz = serde_json::json!({ "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 });
            let xy = serde_json::json!({ "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 });
            let pose = serde_json::json!({
                "type": "array", "items": { "type": "number" }, "minItems": 12, "maxItems": 12,
                "description": "row-major 3×4; replaces the world pose; a mirroring pose refuses mirrored_pose"
            });
            let plane = serde_json::json!({
                "type": "object",
                "properties": { "origin": xyz.clone(), "normal": xyz.clone() },
                "required": ["origin", "normal"],
                "additionalProperties": false,
                "description": "world coordinates"
            });
            let solid_spec = serde_json::json!({
                "description": "SolidSpec: {id|path, pose?} (exactly one object), {prism: {plane, loops, distance}} (first loop outer, the rest holes), or {box: {min, max, pose?}}",
                "oneOf": [
                    {
                        "type": "object",
                        "properties": { "id": { "type": "string" }, "pose": pose.clone() },
                        "required": ["id"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": {
                            "path": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                            "pose": pose.clone()
                        },
                        "required": ["path"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": {
                            "prism": {
                                "type": "object",
                                "properties": {
                                    "plane": plane.clone(),
                                    "loops": { "type": "array", "minItems": 1, "items": { "type": "array", "minItems": 3, "items": xyz.clone() } },
                                    "distance": { "type": "number" }
                                },
                                "required": ["plane", "loops", "distance"],
                                "additionalProperties": false
                            }
                        },
                        "required": ["prism"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": {
                            "box": {
                                "type": "object",
                                "properties": { "min": xyz.clone(), "max": xyz.clone(), "pose": pose.clone() },
                                "required": ["min", "max"],
                                "additionalProperties": false
                            }
                        },
                        "required": ["box"],
                        "additionalProperties": false
                    }
                ]
            });
            let want = serde_json::json!({
                "type": "array",
                "items": { "type": "string", "enum": ["mesh", "topology", "measure", "shells"] },
                "description": "the result blocks to answer; defaults to [\"measure\"]"
            });
            let bbox = serde_json::json!({
                "type": ["object", "null"],
                "properties": { "min": xyz.clone(), "max": xyz.clone() },
                "required": ["min", "max"]
            });
            let measure = serde_json::json!({
                "type": "object",
                "properties": {
                    "volume": { "type": ["number", "null"], "description": "cubic meters; null for an object that is not watertight" },
                    "area": { "type": "number", "description": "square meters, holes subtracted" },
                    "bbox": bbox.clone()
                },
                "required": ["volume", "area", "bbox"]
            });
            // The query shapes, with the ids a transient result does not
            // have made nullable.
            let mut mesh = commands
                .get("hew.query.mesh")
                .expect("declared above")
                .result_schema
                .clone();
            mesh["properties"]["parts"]["items"]["properties"]["id"] = serde_json::json!({ "type": ["string", "null"], "description": "null on a computed mesh" });
            let mut topology = commands
                .get("hew.query.topology")
                .expect("declared above")
                .result_schema
                .clone();
            topology["properties"]["object"] = serde_json::json!({ "type": ["string", "null"], "description": "null on a computed solid" });
            let blocks = |solid: bool| {
                let mut properties = serde_json::json!({
                    "mesh": mesh.clone(),
                    "topology": topology.clone(),
                    "measure": measure.clone(),
                    "shells": { "type": "integer", "minimum": 0, "description": "connected pieces in the result" }
                });
                let mut required = serde_json::json!([]);
                if solid {
                    properties["solid"] = serde_json::json!({ "type": "boolean", "description": "false when nothing is left (no other block then)" });
                    required = serde_json::json!(["solid"]);
                }
                serde_json::json!({ "type": "object", "properties": properties, "required": required })
            };
            let solid_refusals = || {
                vec![
                    "unknown_entity",
                    "unknown_occurrence",
                    "not_an_object",
                    "mirrored_pose",
                    "compute_busy",
                    "compute_too_large",
                    "mesh_too_large",
                ]
            };
            let edge = serde_json::json!({
                "oneOf": [
                    { "type": "object", "properties": { "kind": { "const": "line" }, "a": xy.clone(), "b": xy.clone() }, "required": ["kind", "a", "b"] },
                    { "type": "object", "properties": { "kind": { "const": "arc" }, "center": xy.clone(), "radius": { "type": "number" }, "start": xy.clone(), "end": xy.clone(), "ccw": { "type": "boolean" } }, "required": ["kind", "center", "radius", "start", "end", "ccw"] },
                    { "type": "object", "properties": { "kind": { "const": "circle" }, "center": xy.clone(), "radius": { "type": "number" } }, "required": ["kind", "center", "radius"] }
                ]
            });
            let loops_result = serde_json::json!({
                "type": "object",
                "properties": {
                    "loops": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "outer": { "type": "boolean", "description": "an outer boundary (counter-clockwise), else a hole (clockwise)" },
                                "edges": { "type": "array", "items": edge }
                            },
                            "required": ["outer", "edges"]
                        }
                    }
                },
                "required": ["loops"]
            });

            let cmd = commands
                .get_mut("hew.compute.boolean")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "op": { "type": "string", "enum": ["union", "subtract", "intersect"] },
                    "a": solid_spec.clone(),
                    "b": solid_spec.clone(),
                    "want": want.clone()
                },
                "required": ["op", "a", "b"],
                "additionalProperties": false
            });
            cmd.result_schema = blocks(true);
            cmd.refusals = solid_refusals();
            cmd.refusals.extend([
                "operand_not_solid",
                "degenerate_contact",
                "nothing_removed",
                "singular_transform",
            ]);

            let cmd = commands
                .get_mut("hew.compute.slice")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "solid": solid_spec.clone(), "plane": plane.clone(), "want": want.clone() },
                "required": ["solid", "plane"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "positive": blocks(false),
                    "negative": blocks(false)
                },
                "required": ["positive", "negative"],
                "description": "positive is the piece on the side the plane normal points to"
            });
            cmd.refusals = solid_refusals();
            cmd.refusals
                .extend(["not_solid", "plane_misses_solid", "degenerate"]);

            let cmd = commands
                .get_mut("hew.compute.shells")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "solid": solid_spec.clone(),
                    "want_mesh": { "type": "boolean", "description": "answer each piece's mesh too; defaults to false" }
                },
                "required": ["solid"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "count": { "type": "integer", "minimum": 0 },
                    "shells": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": { "measure": measure.clone(), "mesh": mesh.clone() },
                            "required": ["measure"]
                        }
                    }
                },
                "required": ["count", "shells"]
            });
            cmd.refusals = solid_refusals();

            let cmd = commands
                .get_mut("hew.compute.measure")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "solid": solid_spec.clone() },
                "required": ["solid"],
                "additionalProperties": false
            });
            let mut with_centroid = measure.clone();
            with_centroid["properties"]["centroid"] = serde_json::json!({
                "type": ["array", "null"], "items": { "type": "number" }, "minItems": 3, "maxItems": 3,
                "description": "the volume centroid; null for an object that is not watertight"
            });
            with_centroid["required"] = serde_json::json!(["volume", "area", "bbox", "centroid"]);
            cmd.result_schema = with_centroid;
            cmd.refusals = solid_refusals();
            cmd.refusals.retain(|r| *r != "mesh_too_large");

            let cmd = commands
                .get_mut("hew.compute.outline")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "a world node's public id" },
                    "path": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                    "frame": {
                        "description": "\"part\" (the default: hew.query.part_frame's frame) or {origin, x, y, z}, orthonormal and right-handed",
                        "oneOf": [
                            { "type": "string", "enum": ["part"] },
                            {
                                "type": "object",
                                "properties": { "origin": xyz.clone(), "x": xyz.clone(), "y": xyz.clone(), "z": xyz.clone() },
                                "required": ["origin", "x", "y", "z"],
                                "additionalProperties": false
                            }
                        ]
                    },
                    "which": {
                        "type": "string",
                        "enum": ["largest_face", "projection"],
                        "description": "largest_face: the loops of the part's largest face; projection: the silhouette seen along the frame's z"
                    }
                },
                "required": ["which"],
                "additionalProperties": false
            });
            cmd.result_schema = loops_result.clone();
            cmd.refusals = vec![
                "unknown_entity",
                "unknown_occurrence",
                "invalid_frame",
                "no_part_frame",
                "compute_busy",
                "compute_too_large",
                "too_many_occurrences",
            ];

            let cmd = commands
                .get_mut("hew.compute.offset")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "loops": {
                        "type": "array",
                        "minItems": 1,
                        "items": { "type": "array", "minItems": 3, "items": xy.clone() },
                        "description": "closed 2D loops, the first the outer boundary, the rest holes, in either winding"
                    },
                    "distance": { "type": "number", "description": "positive grows the material, negative shrinks it" },
                    "join": { "type": "string", "enum": ["miter", "round"], "description": "miter (the default); round is reserved and not available yet" }
                },
                "required": ["loops", "distance"],
                "additionalProperties": false
            });
            cmd.result_schema = loops_result;
            cmd.refusals = vec!["offset_too_small", "offset_collapsed", "compute_busy"];

            // Reserved rows (core C6 and C7): the shapes are pinned now and
            // may change with their bodies.
            let cmd = commands
                .get_mut("hew.compute.project")
                .expect("declared above");
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "solid": solid_spec.clone(),
                    "plane": plane.clone(),
                    "want": { "type": "array", "items": { "type": "string", "enum": ["outline", "layers"] } }
                },
                "required": ["solid", "plane"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "outline": loops_result_items(&xy),
                    "layers": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": { "depth": { "type": "number" }, "loops": loops_result_items(&xy) },
                            "required": ["depth", "loops"]
                        }
                    }
                }
            });
            let cmd = commands
                .get_mut("hew.compute.stretch")
                .expect("declared above");
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "paths": { "type": "array", "items": occurrence_locator_schema() },
                    "frame": {
                        "oneOf": [
                            { "type": "string", "enum": ["active", "context", "entity"] },
                            {
                                "type": "object",
                                "properties": { "origin": xyz.clone(), "x": xyz.clone(), "y": xyz.clone(), "z": xyz.clone() },
                                "required": ["origin", "x", "y", "z"],
                                "additionalProperties": false
                            }
                        ]
                    },
                    "axis": { "type": "integer", "enum": [0, 1, 2] },
                    "cutters": {
                        "oneOf": [
                            { "type": "array", "items": { "type": "number" } },
                            {
                                "type": "object",
                                "properties": { "positions": { "type": "array", "items": { "type": "number" } } },
                                "required": ["positions"],
                                "additionalProperties": false
                            }
                        ]
                    },
                    "mode": {
                        "oneOf": [
                            { "type": "string", "enum": ["end", "centered"] },
                            {
                                "type": "object",
                                "properties": { "interior": { "type": "integer", "minimum": 0 } },
                                "required": ["interior"],
                                "additionalProperties": false
                            }
                        ]
                    },
                    "distance": { "type": "number" },
                    "dimension": { "type": "number" }
                },
                "required": ["paths", "axis"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "applied": { "type": "number" },
                    "max_compression": { "type": "number" },
                    "cutters": { "type": "array", "items": { "type": "number" } }
                }
            });
        }

        // The document's clock on every read of the model and of its
        // history (docs/agents/HEW_API.md §7): what `expect_generation`
        // compares, and the epoch that never runs backwards.
        for name in [
            "hew.query.scene",
            "hew.query.entity",
            "hew.query.faces",
            "hew.query.measure",
            "hew.query.raycast",
            "hew.query.resolve",
            "hew.query.context",
            "hew.query.occurrences",
            "hew.query.part_frame",
            "hew.query.signature",
            "hew.query.mesh",
            "hew.query.topology",
            "hew.history.status",
        ] {
            let root = commands
                .get_mut(name)
                .expect("declared above")
                .result_schema
                .as_object_mut()
                .expect("an object schema");
            // A `oneOf` result carries the clock in every branch (the
            // generated client types render only the branches).
            let mut targets: Vec<&mut serde_json::Map<String, serde_json::Value>> =
                match root.get_mut("oneOf").and_then(|v| v.as_array_mut()) {
                    Some(branches) => branches
                        .iter_mut()
                        .map(|b| b.as_object_mut().expect("a branch is an object schema"))
                        .collect(),
                    None => vec![root],
                };
            for schema in targets.iter_mut() {
                let properties = schema
                    .entry("properties")
                    .or_insert_with(|| serde_json::json!({}))
                    .as_object_mut()
                    .expect("properties is an object");
                properties.insert(
                    "generation".into(),
                    serde_json::json!({ "type": "integer", "minimum": 0 }),
                );
                properties.insert(
                    "epoch".into(),
                    serde_json::json!({ "type": "integer", "minimum": 0 }),
                );
                let required = schema
                    .entry("required")
                    .or_insert_with(|| serde_json::json!([]));
                let required = required.as_array_mut().expect("required is a list");
                required.push("generation".into());
                required.push("epoch".into());
            }
        }

        // A document replacement cannot be undone by a compound abort, so
        // it is refused while one is open (docs/agents/PLUGINS.md
        // "Compounds").
        for name in [
            "hew.doc.new",
            "hew.doc.open",
            "hew.doc.revert",
            "hew.doc.close",
        ] {
            commands
                .get_mut(name)
                .expect("declared above")
                .refusals
                .push("document_busy");
        }

        for (name, permission) in permission_table() {
            commands
                .get_mut(name)
                .expect("permission_table names a declared command")
                .permission = permission;
        }

        Registry {
            commands: commands
                .into_iter()
                .map(|(name, draft)| (name.to_string(), draft.finish()))
                .collect(),
        }
    }

    /// Look up a command by its exact method name.
    pub fn get(&self, name: &str) -> Option<&CommandDecl> {
        self.commands.get(name)
    }

    /// Every declaration, in name order (deterministic; PLUGINS.md
    /// invariant 9).
    pub fn commands(&self) -> impl Iterator<Item = &CommandDecl> {
        self.commands.values()
    }

    /// Adds a plugin's manifest commands as `<id>.<local>` entries that
    /// dispatch forwards rather than executes (docs/agents/PLUGINS.md
    /// "Commands"). Atomic: everything is validated first, so on any error
    /// nothing of this plugin is inserted.
    pub fn register_plugin(
        &mut self,
        id: &str,
        decls: &[plugin_manifest::CommandContribution],
    ) -> Result<(), RegisterError> {
        if plugin_manifest::validate_plugin_id(id).is_err() {
            return Err(RegisterError::NameOutsidePluginNamespace(id.to_string()));
        }
        if decls.len() > plugin_manifest::MAX_COMMANDS {
            return Err(RegisterError::TooManyCommands(decls.len()));
        }
        let mut staged: BTreeMap<String, CommandDecl> = BTreeMap::new();
        for decl in decls {
            let name = format!("{id}.{}", decl.id);
            if !plugin_manifest::is_command_id(&decl.id) {
                return Err(RegisterError::NameOutsidePluginNamespace(name));
            }
            if self.commands.contains_key(&name) || staged.contains_key(&name) {
                return Err(RegisterError::DuplicateCommand(name));
            }
            // A refusal reaches the wire as `<plugin-id>.<name>`; a name
            // outside the refusal grammar would be a name outside the
            // plugin's namespace.
            if let Some(bad) = decl
                .refusals
                .iter()
                .find(|r| !plugin_manifest::is_refusal_name(r))
            {
                return Err(RegisterError::NameOutsidePluginNamespace(format!(
                    "{id}.{bad}"
                )));
            }
            for schema in [&decl.params, &decl.result] {
                crate::schema_check::validate_schema(schema).map_err(|why| {
                    RegisterError::InvalidSchema {
                        command: name.clone(),
                        why,
                    }
                })?;
            }
            let mutating = decl.class == plugin_manifest::CommandContributionClass::Mutating;
            let permission = match &decl.permission {
                None if mutating => Permission::ModelWrite,
                None => Permission::ModelRead,
                // `plugin_manifest::parse` refuses a permission a command may
                // not demand, but declarations handed to the registry
                // directly (the wasm boundary) skip it: anything outside
                // that set fails closed, reachable by first-party
                // connections only.
                Some(p) if plugin_manifest::is_command_permission(p.as_str()) => {
                    Permission::parse(p.as_str()).unwrap_or(Permission::FirstParty)
                }
                Some(_) => Permission::FirstParty,
            };
            staged.insert(
                name.clone(),
                CommandDecl {
                    name,
                    version: 1,
                    class: if mutating {
                        CommandClass::Solitary
                    } else {
                        CommandClass::ReadOnly
                    },
                    served: Served::Host,
                    tier: Tier::Standard,
                    permission,
                    provider: Provider::Plugin(id.to_string()),
                    summary: decl.summary.clone(),
                    label: decl.label.as_ref().map(english),
                    timeout_ms: decl.timeout_ms.min(MAX_PLUGIN_TIMEOUT_MS),
                    concurrency: decl.concurrency,
                    deprecated: None,
                    params_schema: decl.params.clone(),
                    result_schema: decl.result.clone(),
                    refusals: decl.refusals.clone(),
                    implemented: true,
                    mutates_document: mutating,
                },
            );
        }
        self.commands.extend(staged);
        Ok(())
    }

    /// Removes every command `id` provides.
    pub fn unregister_plugin(&mut self, id: &str) {
        self.commands
            .retain(|_, c| c.plugin_id().is_none_or(|p| p != id));
    }

    /// The commands `id` provides, in name order.
    pub fn plugin_commands<'a>(
        &'a self,
        id: &'a str,
    ) -> impl Iterator<Item = &'a CommandDecl> + 'a {
        self.commands
            .values()
            .filter(move |c| c.plugin_id() == Some(id))
    }

    /// Protocol 1 plus every plugin-provided entry a
    /// `hew.meta.capabilities` result lists (entries carrying
    /// `provider: { plugin }`); used by MCP in `--live` mode. Unknown or
    /// malformed entries are skipped, never fatal, and never displace a
    /// protocol-1 entry.
    pub fn from_capabilities(result: &serde_json::Value) -> Registry {
        let mut registry = Registry::protocol_1();
        let entries = result
            .get("commands")
            .and_then(serde_json::Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for entry in entries {
            if let Some(decl) = plugin_decl_from_capability(entry)
                && !registry.commands.contains_key(&decl.name)
            {
                registry.commands.insert(decl.name.clone(), decl);
            }
        }
        registry
    }
}

/// The English form of a manifest string: the plain text, the `en` text
/// (else the first locale's), or the catalog key.
fn english(text: &plugin_manifest::LocalizableString) -> String {
    match text {
        plugin_manifest::LocalizableString::Plain(s) => s.clone(),
        plugin_manifest::LocalizableString::ByLocale(map) => map
            .get("en")
            .or_else(|| map.values().next())
            .cloned()
            .unwrap_or_default(),
        plugin_manifest::LocalizableString::Key { key } => key.clone(),
    }
}

/// One plugin entry of a capabilities result back into a declaration, or
/// `None` when it is not a well-formed plugin entry.
fn plugin_decl_from_capability(entry: &serde_json::Value) -> Option<CommandDecl> {
    use serde_json::Value;
    let plugin = entry.pointer("/provider/plugin")?.as_str()?;
    plugin_manifest::validate_plugin_id(plugin).ok()?;
    let name = entry.get("name")?.as_str()?;
    let local = name.strip_prefix(plugin)?.strip_prefix('.')?;
    if !plugin_manifest::is_command_id(local) {
        return None;
    }
    let (class, mutating) = match entry.get("class")?.as_str()? {
        "read_only" => (CommandClass::ReadOnly, false),
        "solitary" => (CommandClass::Solitary, true),
        _ => return None,
    };
    let params_schema = entry.get("params")?.clone();
    let result_schema = entry.get("result")?.clone();
    crate::schema_check::validate_schema(&params_schema).ok()?;
    crate::schema_check::validate_schema(&result_schema).ok()?;
    let refusals = entry
        .get("refusals")?
        .as_array()?
        .iter()
        .map(|r| r.as_str().map(str::to_string))
        .collect::<Option<Vec<_>>>()?;
    let label = match entry.get("label") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return None,
    };
    let concurrency = match entry.get("concurrency")?.as_str()? {
        "parallel" => plugin_manifest::Concurrency::Parallel,
        "serial" => plugin_manifest::Concurrency::Serial,
        _ => return None,
    };
    let timeout_ms = u32::try_from(entry.get("timeout_ms")?.as_u64()?).ok()?;
    Some(CommandDecl {
        name: name.to_string(),
        version: u32::try_from(entry.get("version")?.as_u64()?).ok()?,
        class,
        served: Served::Host,
        tier: Tier::Standard,
        // `first_party` is how capabilities shows the fail-closed permission
        // of a declaration that named none a command may demand; it has no
        // parse form, so it is read back here.
        permission: match entry.get("permission")?.as_str()? {
            "first_party" => Permission::FirstParty,
            other => Permission::parse(other).ok()?,
        },
        provider: Provider::Plugin(plugin.to_string()),
        summary: entry.get("summary")?.as_str()?.to_string(),
        label,
        timeout_ms: timeout_ms.min(MAX_PLUGIN_TIMEOUT_MS),
        concurrency,
        deprecated: None,
        params_schema,
        result_schema,
        refusals,
        implemented: entry.get("implemented")?.as_bool()?,
        mutates_document: mutating,
    })
}

/// Every protocol-1 command's permission (docs/agents/HEW_API.md §10).
/// Declared per command, not derived from its class: the classes do not
/// line up with what a person grants (`hew.scenes.describe` is
/// model-mutating, `hew.library.save` read-only).
pub fn permission_table() -> Vec<(&'static str, Permission)> {
    use Permission::*;
    let rows: [(Permission, &[&'static str]); 9] = [
        (
            Always,
            &[
                "hew.meta.hello",
                "hew.meta.capabilities",
                "hew.doc.attach",
                "hew.doc.transact",
                "hew.event.subscribe",
                "hew.event.unsubscribe",
            ],
        ),
        (
            ModelRead,
            &[
                "hew.query.changes_since",
                "hew.query.context",
                "hew.query.entity",
                "hew.geo.get",
                "hew.query.faces",
                "hew.query.measure",
                "hew.query.mesh",
                "hew.query.occurrences",
                "hew.query.part_frame",
                "hew.query.raycast",
                "hew.query.signature",
                "hew.query.resolve",
                "hew.query.scene",
                "hew.query.topology",
                "hew.attr.get",
                "hew.meta.documents",
                "hew.history.status",
                "hew.scenes.list",
                "hew.scenes.describe",
                "hew.library.list",
                "hew.library.describe",
                "hew.doc.health",
                "hew.solid.imprints",
                "hew.sketch.units",
                "hew.view.snapshot",
                "hew.view.line_drawing",
                "hew.print.pdf",
                "hew.compute.boolean",
                "hew.compute.slice",
                "hew.compute.shells",
                "hew.compute.measure",
                "hew.compute.outline",
                "hew.compute.offset",
                "hew.compute.project",
                "hew.compute.stretch",
            ],
        ),
        (
            ModelWrite,
            &[
                "hew.sketch.copy",
                "hew.sketch.delete",
                "hew.sketch.draw_arc",
                "hew.sketch.draw_circle",
                "hew.sketch.draw_line",
                "hew.sketch.draw_polygon",
                "hew.sketch.draw_rect",
                "hew.sketch.move",
                "hew.sketch.offset",
                "hew.sketch.rotate",
                "hew.sketch.scale",
                "hew.solid.cap",
                "hew.solid.delete_faces",
                "hew.solid.delete_imprint",
                "hew.solid.extrude",
                "hew.solid.follow_me",
                "hew.solid.from_faces",
                "hew.solid.intersect",
                "hew.solid.mark_surface",
                "hew.solid.move_imprint",
                "hew.solid.push_pull",
                "hew.solid.rejoin",
                "hew.solid.rotate_imprint",
                "hew.solid.scale_imprint",
                "hew.solid.slice",
                "hew.solid.subtract",
                "hew.solid.thicken",
                "hew.solid.union",
                "hew.entity.delete",
                "hew.entity.flip",
                "hew.entity.move",
                "hew.entity.rename",
                "hew.entity.rotate",
                "hew.entity.scale",
                "hew.entity.set_frame",
                "hew.context.enter",
                "hew.context.exit",
                "hew.group.create",
                "hew.group.explode",
                "hew.group.reparent",
                "hew.component.create",
                "hew.component.explode",
                "hew.component.make_unique",
                "hew.component.place",
                "hew.component.promote",
                "hew.component.import_definition",
                "hew.component.rebase",
                "hew.component.flip_instance",
                "hew.material.create",
                "hew.material.paint",
                "hew.material.position_texture",
                "hew.material.replace",
                "hew.material.set_default",
                "hew.material.set_opacity",
                "hew.material.set_pbr",
                "hew.geo.set",
                "hew.tag.assign",
                "hew.tag.create",
                "hew.tag.delete",
                "hew.tag.rename",
                "hew.tag.set_visible",
                "hew.guide.angular",
                "hew.guide.clear",
                "hew.guide.line",
                "hew.guide.point",
                "hew.scenes.add",
                "hew.scenes.update",
                "hew.scenes.rename",
                "hew.scenes.remove",
                "hew.scenes.reorder",
                "hew.scenes.apply",
                "hew.history.undo",
                "hew.history.redo",
                "hew.doc.purge_unused",
            ],
        ),
        (AttributesWrite, &["hew.attr.set", "hew.attr.delete"]),
        (
            View,
            &[
                "hew.view.camera",
                "hew.view.zoom_extents",
                "hew.view.units",
                "hew.selection.get",
                "hew.selection.set",
                "hew.selection.clear",
                "hew.view.hide",
                "hew.view.show",
                "hew.view.show_all",
                "hew.view.hidden",
                "hew.view.isolate",
                "hew.view.clear_isolation",
                "hew.view.zoom_to",
                "hew.view.enter",
                "hew.view.exit",
                "hew.view.exit_all",
            ],
        ),
        (
            DocumentsSave,
            &["hew.doc.save", "hew.doc.export", "hew.doc.import"],
        ),
        (DocumentsDiscard, &["hew.doc.close", "hew.doc.revert"]),
        (
            DocumentsOther,
            &["hew.doc.new", "hew.doc.open", "hew.doc.focus"],
        ),
        (
            Library,
            &[
                "hew.library.insert",
                "hew.library.save",
                "hew.library.remove",
                "hew.library.update_meta",
            ],
        ),
    ];
    rows.into_iter()
        .flat_map(|(p, names)| names.iter().map(move |n| (*n, p.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_angle_says_radians() {
        let reg = Registry::protocol_1();
        let rotate = reg.get("hew.entity.rotate").expect("declared");
        let description = rotate.params_schema["properties"]["angle"]["description"]
            .as_str()
            .expect("angle is described");
        assert!(description.contains("radians"), "{description}");
    }

    /// Two plugin commands under `com.example.lib`: a read-only `sum` and a
    /// mutating `shelf.add`.
    pub(crate) fn sample_plugin() -> Vec<plugin_manifest::CommandContribution> {
        serde_json::from_value(serde_json::json!([
            {
                "id": "sum",
                "class": "read_only",
                "summary": "Adds.",
                "params": {
                    "type": "object",
                    "properties": { "a": { "type": "number" }, "b": { "type": "number" } },
                    "required": ["a", "b"],
                    "additionalProperties": false
                },
                "refusals": ["too_big"]
            },
            {
                "id": "shelf.add",
                "class": "mutating",
                "summary": "Adds a shelf.",
                "label": { "ja": "棚", "en": "Add shelf" },
                "timeout_ms": 900000,
                "concurrency": "serial"
            }
        ]))
        .expect("sample declarations parse")
    }

    /// Registry completeness (docs/agents/HEW_API.md §14): every declared command
    /// carries schemas, a summary, and a non-empty refusal inventory; names
    /// are namespaced and well-formed; versions start at 1. A plugin is
    /// registered first so both naming arms run.
    #[test]
    fn every_declaration_is_complete() {
        let mut reg = Registry::protocol_1();
        reg.register_plugin("com.example.lib", &sample_plugin())
            .expect("the sample registers");
        assert!(reg.commands().count() >= 50);
        let mut plugin_entries = 0;
        for cmd in reg.commands() {
            if let Some(plugin) = cmd.plugin_id() {
                plugin_entries += 1;
                let local = cmd
                    .name
                    .strip_prefix(plugin)
                    .and_then(|rest| rest.strip_prefix('.'))
                    .unwrap_or_else(|| panic!("{} is not under {plugin}", cmd.name));
                assert_eq!(cmd.name, format!("{plugin}.{local}"));
                assert!(
                    plugin_manifest::is_command_id(local),
                    "local name {local} breaks the command id grammar"
                );
            } else {
                assert!(
                    cmd.name.starts_with("hew.") && cmd.name.split('.').count() == 3,
                    "malformed name {}",
                    cmd.name
                );
                assert!(
                    cmd.name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_'),
                    "name not lower snake_case: {}",
                    cmd.name
                );
            }
            assert!(!cmd.summary.is_empty(), "{} lacks a summary", cmd.name);
            assert!(
                cmd.params_schema.is_object(),
                "{} lacks a params schema",
                cmd.name
            );
            assert!(
                cmd.result_schema.is_object(),
                "{} lacks a result schema",
                cmd.name
            );
            assert!(cmd.version >= 1);
            if !cmd.implemented {
                assert!(
                    cmd.refusals.iter().any(|r| r == "unimplemented"),
                    "{} is unimplemented but does not declare the unimplemented refusal",
                    cmd.name
                );
            }
        }
        assert_eq!(plugin_entries, 2);
    }

    #[test]
    fn first_party_entries_carry_their_provider_and_defaults() {
        for cmd in Registry::protocol_1().commands() {
            let expected = match cmd.served {
                Served::Kernel => Provider::Kernel,
                Served::Host => Provider::Host,
            };
            assert_eq!(cmd.provider, expected, "{}", cmd.name);
            assert_eq!(cmd.label, None);
            assert_eq!(cmd.timeout_ms, 0);
            assert_eq!(cmd.concurrency, plugin_manifest::Concurrency::Parallel);
            assert_eq!(cmd.deprecated, None);
        }
    }

    #[test]
    fn register_plugin_is_atomic_and_refuses_duplicates() {
        let mut reg = Registry::protocol_1();
        let before = reg.commands().count();
        reg.register_plugin("com.example.lib", &sample_plugin())
            .expect("registers");
        let sum = reg.get("com.example.lib.sum").expect("sum registered");
        assert_eq!(sum.class, CommandClass::ReadOnly);
        assert!(!sum.mutates_document);
        assert_eq!(sum.permission, Permission::ModelRead);
        assert_eq!(sum.served, Served::Host);
        assert_eq!(sum.tier, Tier::Standard);
        assert_eq!(sum.provider, Provider::Plugin("com.example.lib".into()));
        assert_eq!(sum.refusals, vec!["too_big".to_string()]);
        assert!(sum.implemented);
        let add = reg
            .get("com.example.lib.shelf.add")
            .expect("shelf.add registered");
        assert_eq!(add.class, CommandClass::Solitary);
        assert!(add.mutates_document);
        assert_eq!(add.permission, Permission::ModelWrite);
        assert_eq!(add.label.as_deref(), Some("Add shelf"));
        assert_eq!(add.concurrency, plugin_manifest::Concurrency::Serial);
        assert_eq!(reg.plugin_commands("com.example.lib").count(), 2);

        // Registering the same plugin again collides on its first name and
        // leaves the registry as it was.
        let snapshot: Vec<String> = reg.commands().map(|c| c.name.clone()).collect();
        assert_eq!(
            reg.register_plugin("com.example.lib", &sample_plugin()),
            Err(RegisterError::DuplicateCommand(
                "com.example.lib.sum".into()
            ))
        );
        // A batch whose LAST command is bad inserts none of the good ones.
        let mut partly_bad = sample_plugin();
        partly_bad[1].result = serde_json::json!({ "type": "object", "properties": {} });
        let err = reg
            .register_plugin("com.other.lib", &partly_bad)
            .unwrap_err();
        assert_eq!(err.name(), "invalid_schema");
        // Two decls naming the same command inside one batch.
        let twice = vec![sample_plugin()[0].clone(), sample_plugin()[0].clone()];
        assert_eq!(
            reg.register_plugin("com.twice.lib", &twice),
            Err(RegisterError::DuplicateCommand("com.twice.lib.sum".into()))
        );
        for bad_id in ["hew.evil", "nodot", "Com.Example"] {
            assert_eq!(
                reg.register_plugin(bad_id, &sample_plugin()),
                Err(RegisterError::NameOutsidePluginNamespace(bad_id.into()))
            );
        }
        let mut bad_local = sample_plugin();
        bad_local[0].id = "Bad Name".into();
        assert_eq!(
            reg.register_plugin("com.local.lib", &bad_local)
                .unwrap_err()
                .name(),
            "name_outside_plugin_namespace"
        );
        let mut bad_refusal = sample_plugin();
        bad_refusal[0].refusals = vec!["Too Big".into()];
        assert_eq!(
            reg.register_plugin("com.refusal.lib", &bad_refusal),
            Err(RegisterError::NameOutsidePluginNamespace(
                "com.refusal.lib.Too Big".into()
            ))
        );
        let many = vec![sample_plugin()[0].clone(); plugin_manifest::MAX_COMMANDS + 1];
        assert_eq!(
            reg.register_plugin("com.many.lib", &many),
            Err(RegisterError::TooManyCommands(
                plugin_manifest::MAX_COMMANDS + 1
            ))
        );
        let after: Vec<String> = reg.commands().map(|c| c.name.clone()).collect();
        assert_eq!(after, snapshot, "a refused registration inserts nothing");

        reg.unregister_plugin("com.example.lib");
        assert_eq!(reg.commands().count(), before);
        assert_eq!(reg.plugin_commands("com.example.lib").count(), 0);
    }

    #[test]
    fn register_plugin_clamps_timeout() {
        let mut reg = Registry::protocol_1();
        reg.register_plugin("com.example.lib", &sample_plugin())
            .expect("registers");
        assert_eq!(reg.get("com.example.lib.sum").unwrap().timeout_ms, 50_000);
        assert_eq!(
            reg.get("com.example.lib.shelf.add").unwrap().timeout_ms,
            MAX_PLUGIN_TIMEOUT_MS,
            "900 s in the manifest is clamped, not refused"
        );
        let mut short = sample_plugin();
        short[0].timeout_ms = 1200;
        reg.register_plugin("com.short.lib", &short[..1]).unwrap();
        assert_eq!(reg.get("com.short.lib.sum").unwrap().timeout_ms, 1200);
    }

    #[test]
    fn from_capabilities_round_trips() {
        let mut reg = Registry::protocol_1();
        reg.register_plugin("com.example.lib", &sample_plugin())
            .unwrap();
        let mut other = sample_plugin();
        other[0].permission =
            Some(plugin_manifest::PermissionName::parse("attributes.write").unwrap());
        other[0].label = Some(plugin_manifest::LocalizableString::Plain("Total".into()));
        reg.register_plugin("org.other.tool", &other).unwrap();
        // A permission a command may not demand, handed over directly,
        // fails closed and still round-trips.
        let mut closed = sample_plugin();
        closed[0].permission =
            Some(serde_json::from_value(serde_json::json!("documents.save")).unwrap());
        reg.register_plugin("net.closed.tool", &closed[..1])
            .unwrap();
        assert_eq!(
            reg.get("net.closed.tool.sum").unwrap().permission,
            Permission::FirstParty
        );

        let mut conn = crate::dispatch::Connection::new(crate::grant::Grant::core(), "t");
        let mut doc = kernel::Document::new();
        let request = |id: i64, method: &str| crate::envelope::Request {
            jsonrpc: "2.0".into(),
            id: Some(crate::envelope::RequestId::Number(id)),
            method: method.into(),
            params: Some(serde_json::json!({ "protocol": 1 })),
            document: None,
        };
        conn.dispatch(
            &reg,
            &mut doc,
            &mut crate::host::NoHost,
            request(0, "hew.meta.hello"),
        );
        let crate::dispatch::DispatchOutcome::Reply(caps) = conn.dispatch(
            &reg,
            &mut doc,
            &mut crate::host::NoHost,
            request(1, "hew.meta.capabilities"),
        ) else {
            panic!("capabilities replies")
        };
        let mut result = caps.result.expect("capabilities succeeds");
        // A malformed plugin entry, one under the reserved `hew` id claiming
        // a first-party name, and a second entry for a name already read
        // are skipped without dropping the valid ones; the first entry for
        // a name wins.
        let commands = result["commands"].as_array_mut().unwrap();
        commands
            .push(serde_json::json!({ "name": "com.bad.x", "provider": { "plugin": "com.bad" } }));
        let mut impostor = commands
            .iter()
            .find(|c| c["name"] == "com.example.lib.sum")
            .unwrap()
            .clone();
        impostor["name"] = "hew.query.scene".into();
        impostor["provider"] = serde_json::json!({ "plugin": "hew" });
        commands.push(impostor);
        let mut repeat = commands
            .iter()
            .find(|c| c["name"] == "com.example.lib.sum")
            .unwrap()
            .clone();
        repeat["summary"] = "A second, later entry.".into();
        commands.push(repeat);

        let rebuilt = Registry::from_capabilities(&result);
        assert!(rebuilt.get("com.bad.x").is_none());
        assert!(!rebuilt.get("hew.query.scene").unwrap().is_plugin());
        let names = |r: &Registry| -> Vec<String> {
            r.commands()
                .filter(|c| c.is_plugin())
                .map(|c| c.name.clone())
                .collect()
        };
        assert_eq!(names(&rebuilt), names(&reg));
        for original in reg.commands().filter(|c| c.is_plugin()) {
            let copy = rebuilt.get(&original.name).unwrap();
            assert_eq!(copy.name, original.name);
            assert_eq!(copy.version, original.version);
            assert_eq!(copy.class, original.class);
            assert_eq!(copy.served, original.served);
            assert_eq!(copy.tier, original.tier);
            assert_eq!(copy.permission, original.permission);
            assert_eq!(copy.provider, original.provider);
            assert_eq!(copy.summary, original.summary);
            assert_eq!(copy.label, original.label);
            assert_eq!(copy.timeout_ms, original.timeout_ms);
            assert_eq!(copy.concurrency, original.concurrency);
            assert_eq!(copy.deprecated, original.deprecated);
            assert_eq!(copy.params_schema, original.params_schema);
            assert_eq!(copy.result_schema, original.result_schema);
            assert_eq!(copy.refusals, original.refusals);
            assert_eq!(copy.implemented, original.implemented);
            assert_eq!(copy.mutates_document, original.mutates_document);
        }
        assert_eq!(
            rebuilt.commands().filter(|c| !c.is_plugin()).count(),
            Registry::protocol_1().commands().count()
        );
    }

    /// The §7 tier table's Required set is present.
    #[test]
    fn required_namespaces_are_declared() {
        let reg = Registry::protocol_1();
        for name in [
            "hew.meta.hello",
            "hew.doc.transact",
            "hew.query.scene",
            "hew.sketch.draw_rect",
            "hew.solid.extrude",
            "hew.solid.push_pull",
            "hew.entity.move",
            "hew.context.enter",
            "hew.group.create",
            "hew.attr.set",
            "hew.history.undo",
        ] {
            assert!(reg.get(name).is_some(), "missing {name}");
        }
    }

    /// Every table row names a registered command, every registered
    /// command appears exactly once, and the always-granted set is exactly
    /// the handshake, introspection, attaching to the bound document, and
    /// the transaction container.
    #[test]
    fn every_command_has_a_permission() {
        let reg = Registry::protocol_1();
        let table = permission_table();
        let mut seen = std::collections::BTreeSet::new();
        for (name, permission) in &table {
            let decl = reg
                .get(name)
                .unwrap_or_else(|| panic!("{name} is not a registered command"));
            assert!(seen.insert(name.to_string()), "{name} appears twice");
            assert_eq!(&decl.permission, permission, "{name}");
        }
        for cmd in reg.commands() {
            assert!(
                seen.contains(&cmd.name),
                "{} has no permission row",
                cmd.name
            );
        }
        assert_eq!(table.len(), reg.commands().count());
        let always: std::collections::BTreeSet<&str> = table
            .iter()
            .filter(|(_, p)| *p == Permission::Always)
            .map(|(n, _)| *n)
            .collect();
        assert_eq!(
            always,
            [
                "hew.meta.hello",
                "hew.meta.capabilities",
                "hew.doc.attach",
                "hew.doc.transact",
                "hew.event.subscribe",
                "hew.event.unsubscribe",
            ]
            .into_iter()
            .collect()
        );
    }
}
