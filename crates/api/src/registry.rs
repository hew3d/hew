//! The command registry — the single source of truth for the API's
//! per-command contracts (docs/HEW_API.md §9). Every command is declared
//! once, here: name, version, class, tier, where it is served, summary,
//! parameter/result schemas, and refusal inventory. MCP tool definitions,
//! the TypeScript SDK, and the published reference are generated from
//! these declarations — hand-maintained copies are forbidden.
//!
//! Scaffold posture (§14): the full protocol-1 inventory is declared so
//! the unimplemented set is a visible burn-down list. A command's schemas
//! are refined in the change that lands its conformance tests; until it
//! is implemented, dispatch answers it with the `unimplemented` refusal.

use std::collections::BTreeMap;

/// What a command does to the document — governs envelope placement
/// (docs/HEW_API.md §6.4).
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

/// Who executes the command's effect (docs/HEW_API.md §3): the pure
/// dispatcher, or a host through the typed host trait (file I/O,
/// importers, rendering). Either way this crate owns the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Served {
    Kernel,
    Host,
}

/// Command tier at protocol 1 (docs/HEW_API.md §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Exists at 1.0; the minimum for the agent-modeling vision.
    Required,
    /// Exists at 1.0 for tool parity.
    Standard,
}

/// One command's declaration.
#[derive(Debug, Clone)]
pub struct CommandDecl {
    /// Dot-separated, lower snake_case: `hew.<area>.<command>` (§4.1).
    pub name: &'static str,
    /// Per-command version, starting at 1; evolution is additive (§9).
    pub version: u32,
    pub class: CommandClass,
    pub served: Served,
    pub tier: Tier,
    /// One-line summary for `hew.meta.capabilities` and generated docs.
    pub summary: &'static str,
    /// JSON Schema for `params`. Scaffold declarations start at the
    /// permissive object schema and are tightened with each command's
    /// conformance tests.
    pub params_schema: serde_json::Value,
    /// JSON Schema for the success result.
    pub result_schema: serde_json::Value,
    /// The machine names of the refusals this command can answer (§4.4).
    /// Empty is honest for a command whose failures are all protocol
    /// errors; every unimplemented command carries `["unimplemented"]`.
    pub refusals: Vec<&'static str>,
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

/// A named subset of the registry a host grants a connection (§10) — a
/// maximum a host may narrow, never a protocol change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Everything headless-safe: the full surface except `hew.view.*`,
    /// with one deliberate exception — `hew.view.snapshot` has a headless
    /// render path (docs/design/headless-snapshot.md's software
    /// rasterizer) and is granted here too.
    Core,
    /// `core` plus live-application-only commands (future `hew.view.*`
    /// additions: selection, camera control).
    App,
}

impl Profile {
    /// Whether this profile grants `command`.
    pub fn grants(&self, command: &CommandDecl) -> bool {
        match self {
            Profile::App => true,
            Profile::Core => {
                command.name == "hew.view.snapshot" || !command.name.starts_with("hew.view.")
            }
        }
    }
}

/// The protocol-1 registry.
#[derive(Debug)]
pub struct Registry {
    commands: BTreeMap<&'static str, CommandDecl>,
}

/// Shorthand for a permissive object schema — the scaffold placeholder
/// each command tightens when its conformance tests land.
fn obj() -> serde_json::Value {
    serde_json::json!({ "type": "object" })
}

impl Registry {
    /// Builds the protocol-1 inventory (docs/HEW_API.md §7).
    pub fn protocol_1() -> Registry {
        let mut commands = BTreeMap::new();
        let mut add = |name: &'static str,
                       class: CommandClass,
                       served: Served,
                       tier: Tier,
                       summary: &'static str| {
            commands.insert(
                name,
                CommandDecl {
                    name,
                    version: 1,
                    class,
                    served,
                    tier,
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
        add(
            "hew.meta.documents",
            R,
            Host,
            Req,
            "The host's open documents.",
        );
        // hew.doc — lifecycle and the transaction container.
        add(
            "hew.doc.attach",
            S,
            Kernel,
            Req,
            "Bind this connection to one open document.",
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
            "Create a fresh document (headless hosts; live hosts advertise via capabilities).",
        );
        add(
            "hew.doc.open",
            S,
            Host,
            Req,
            "Open a .hew document (headless hosts; live hosts advertise via capabilities).",
        );
        add("hew.doc.save", S, Host, Req, "Save the attached document.");
        add(
            "hew.doc.export",
            S,
            Host,
            Req,
            "Export the attached document (STL/3MF/glTF/USDZ); bytes base64, or a path on hosts with filesystem access.",
        );
        add(
            "hew.doc.import",
            S,
            Host,
            Std,
            "Merge a foreign-format file into the attached document through the shared healing pipeline.",
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
            "First hit along a ray — the programmatic form of clicking.",
        );
        add(
            "hew.query.measure",
            R,
            Kernel,
            Req,
            "Distances and angles between points, edges, and faces.",
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
            "The open editing-context frame stack.",
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
            "Offset a region boundary within its sketch.",
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
            "Boolean subtraction of two solids.",
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
            "Slice a solid by a plane into two solids.",
        );
        add(
            "hew.solid.follow_me",
            M,
            Kernel,
            Std,
            "Sweep a profile along an edge-chain path, as the tool does.",
        );
        // hew.entity — transforms and lifecycle.
        add("hew.entity.rename", M, Kernel, Req, "Rename an entity.");
        add("hew.entity.delete", M, Kernel, Req, "Delete an entity.");
        add(
            "hew.entity.move",
            M,
            Kernel,
            Req,
            "Translate (with copy/array forms) by vector or from→to points.",
        );
        add(
            "hew.entity.rotate",
            M,
            Kernel,
            Req,
            "Rotate about a pivot and axis by an angle.",
        );
        add(
            "hew.entity.scale",
            M,
            Kernel,
            Req,
            "Scale about an anchor with per-axis factors.",
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
            "hew.component.make_unique",
            M,
            Kernel,
            Std,
            "Deep-copy an instance's definition into a private one.",
        );
        add(
            "hew.component.explode",
            M,
            Kernel,
            Std,
            "Bake an instance into world geometry.",
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
        // hew.scenes — named, saved views (docs/HEW_API.md's Scenes
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
            "Re-capture a Scene's properties from the document's current state. Records no undo entry (§6.4).",
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
            "History depth and the top entry's label and origin.",
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

        // The connection-lifecycle commands the dispatcher already
        // implements: the burn-down flag must tell the truth, and their
        // refusal inventories are honestly empty (their failures are
        // protocol errors, not refusals).
        for name in ["hew.meta.hello", "hew.meta.capabilities", "hew.doc.attach"] {
            let cmd = commands.get_mut(name).expect("declared above");
            cmd.implemented = true;
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
                    "profile": { "type": "string", "enum": ["core", "app"] },
                    "encoding": { "type": "string" },
                    "documents": { "type": "array" }
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
                "properties": { "document": { "type": "string" } }
            });
            cmd.result_schema = serde_json::json!({ "type": "object" });
        }
        // The transaction container itself (dispatch-inline, §6): its own
        // refusals are the runtime $ref failure plus whatever its inner
        // commands refuse (reported per command in the canonical shape).
        {
            let cmd = commands
                .get_mut("hew.doc.transact")
                .expect("declared above");
            cmd.implemented = true;
            cmd.refusals = vec!["ref_resolution_failed"];
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "label": { "type": "string" },
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
                    "label": { "type": "string" }
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
                "properties": { "documents": { "type": "array" } },
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
                    "sketches": { "type": "array" },
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
                "properties": { "object": { "type": "string" } },
                "required": ["object"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "object": { "type": "string" }, "faces": { "type": "array" } },
                "required": ["object", "faces"]
            });
            cmd.refusals = vec!["unknown_entity"];
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
                    "dir": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["origin", "dir"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "object": { "type": "string", "description": "the world object's or, for an instance hit, the instance's public id" },
                    "kind": { "type": "string", "enum": ["object", "instance"] },
                    "point": { "type": "array" },
                    "distance": { "type": "number" },
                    "normal": { "type": "array" }
                },
                "required": ["object", "kind", "point", "distance", "normal"]
            });
            cmd.refusals = vec!["locator_missed", "ambiguous_locator"];
        }
        {
            let cmd = commands
                .get_mut("hew.query.measure")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "from": {}, "to": {} },
                "required": ["from", "to"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "distance": { "type": "number" }, "delta": { "type": "array" } },
                "required": ["distance", "delta"]
            });
            cmd.refusals = vec![
                "unknown_entity",
                "locator_missed",
                "ambiguous_locator",
                "no_such_point",
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
                "properties": { "stack": { "type": "array" }, "direct_members": {} },
                "required": ["stack"]
            });
            cmd.refusals = Vec::new();
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
            let cmd = commands.get_mut("hew.doc.open").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            });
            cmd.refusals = vec!["host_capability_missing", "load_failed"];
        }
        {
            let cmd = commands.get_mut("hew.doc.save").expect("declared above");
            cmd.implemented = true;
            cmd.summary = "Save the attached document — written by hosts with filesystem access, bytes base64 by those without.";
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "bytes_base64": { "type": "string" } },
                "additionalProperties": false
            });
            cmd.refusals = vec!["host_capability_missing", "path_required", "save_failed"];
        }
        {
            let cmd = commands.get_mut("hew.doc.export").expect("declared above");
            cmd.implemented = true;
            cmd.summary = "Export the attached document — STL, 3MF, glTF/GLB, or USDZ — solids only, bytes base64, or a path on hosts with filesystem access.";
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "format": {
                        "type": "string",
                        "enum": ["stl", "3mf", "glb", "gltf", "usdz"],
                        "description": "\"gltf\" is an alias for \"glb\" — every host that implements one implements both"
                    },
                    "path": { "type": "string" },
                    "segments_per_turn": { "type": "integer", "minimum": 8, "maximum": 512 }
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
            ];
        }
        {
            let cmd = commands.get_mut("hew.doc.import").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "units": { "type": "string", "enum": ["m", "mm", "cm", "in"] }
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
            let cmd = commands
                .get_mut("hew.view.snapshot")
                .expect("declared above");
            cmd.implemented = true;
            cmd.summary = "Render the attached document to PNG, headless-rendered via a software rasterizer (a live host may render through its viewport instead) — bytes base64 by default, or a path on hosts with filesystem access.";
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
                            "fov_deg": { "type": "number", "description": "perspective only; defaults to 35" }
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
                    "scene": {
                        "type": "string",
                        "description": "a Scene's id: renders through its resolved camera and hidden sets (Document::resolve_scene) instead of the document's own — falls back to the usual cameraless resolution when the Scene captures no camera. Mutually exclusive with camera and view. The Scene's section plane, if any, is NOT rendered headlessly at 1.0."
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
                "description": "two shapes depending on whether path was given: bytes base64 inline (default), or a path plus, when include_ids was true, a sidecar path for the id-buffer",
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
                    }
                },
                "required": ["width", "height"]
            });
            cmd.refusals = vec![
                "host_capability_missing",
                "nothing_to_render",
                "save_failed",
                "unknown_scene",
            ];
        }
        {
            // hew.view.camera — sets the live viewport's camera. Same
            // camera/view vocabulary as hew.view.snapshot (one spec, not
            // two, per docs/HEW_API.md §7), minus snapshot's rendering
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
                            "fov_deg": { "type": "number", "description": "perspective only; defaults to 35" }
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
                "delete_unsupported",
                "unknown_object",
                "unknown_group",
                "unknown_instance",
                "unknown_sketch",
                "unknown_guide",
                "unknown_edge",
            ];
        }
        {
            let cmd = commands.get_mut("hew.entity.move").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "ids": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                    "translation": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "from": {},
                    "to": {},
                    "copy": {
                        "type": "object",
                        "properties": { "count": { "type": "integer", "minimum": 1, "maximum": 1000 } },
                        "additionalProperties": false
                    }
                },
                "required": ["ids"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "ids": { "type": "array", "items": { "type": "string" } } }
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
                    "ids": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                    "pivot": {},
                    "axis": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "angle": { "type": "number" },
                    "copy": {
                        "type": "object",
                        "properties": { "count": { "type": "integer", "minimum": 1, "maximum": 1000 } },
                        "additionalProperties": false
                    }
                },
                "required": ["ids", "pivot", "axis", "angle"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "ids": { "type": "array", "items": { "type": "string" } } }
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
                    "ids": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
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
                .get_mut("hew.component.make_unique")
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
                "properties": { "component": { "type": "string" } },
                "required": ["component"]
            });
            cmd.refusals = vec!["unknown_entity", "unknown_instance", "unknown_component"];
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
                        "maxItems": 4
                    }
                },
                "required": ["name", "color"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "material": { "type": "string" } },
                "required": ["material"]
            });
            // Palette additions are deliberately not undoable in the
            // kernel (docs/design/api-kernel-map.md §1.9) — nothing here
            // can be typed-refused; a malformed color/name is a protocol
            // error (`-32602`), not a refusal.
            cmd.refusals = Vec::new();
        }
        {
            let cmd = commands
                .get_mut("hew.material.paint")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "face": {},
                    "id": { "type": "string" },
                    "material": { "type": ["string", "null"] }
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
                "unknown_face",
                "unknown_material",
                "locator_missed",
                "ambiguous_locator",
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
            // hew.scenes.* — named, saved views (docs/HEW_API.md's Scenes
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
                    "fov_deg": { "type": "number", "description": "perspective only; defaults to 35" }
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
                    "fov_deg": { "type": "number" }
                },
                "required": ["eye", "target", "up", "projection", "fov_deg"]
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
                    "display": display_schema.clone()
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
                    "display": display_schema.clone()
                },
                "required": ["id"],
                "additionalProperties": false,
                "description": "properties defaults to the Scene's currently captured set (re-capture, never widen) when omitted"
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
                    "target": { "type": "string" },
                    "ns": { "type": "string" }
                },
                "required": ["target"],
                "additionalProperties": false
            });
            cmd.result_schema = serde_json::json!({ "type": "object" });
            cmd.refusals = vec![
                "unknown_entity",
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
                    "target": { "type": "string" },
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
                "invalid_attr_name",
                "reserved_attr_namespace",
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
                    "target": { "type": "string" },
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
                "invalid_attr_name",
                "reserved_attr_namespace",
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
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "undo_depth": { "type": "integer" },
                    "redo_depth": { "type": "integer" },
                    "top": {}
                },
                "required": ["undo_depth", "redo_depth", "top"]
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
            let sketch_result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "sketch": { "type": "string" },
                    "region_ids": { "type": "array", "items": { "type": "string" } },
                    "region_id": { "type": "string", "description": "present when exactly one region resulted (HEW_API.md §6.1's example)" },
                    "curve_id": { "type": "string", "description": "present when this command began a curve chain" }
                },
                "required": ["sketch", "region_ids"]
            });
            // The face-imprint result shape (docs/HEW_API.md §5.2, §5.4):
            // on-face drawing never touches a sketch, so its result is just
            // the reshaped solid's public id — a face has no public id of
            // its own, so whichever faces the cut produced are addressed
            // only through the transaction-scoped face tokens `token_doc`
            // documents. Every draw command's result is a `oneOf` of this
            // and `sketch_result_schema`, distinguished by which plane
            // spec variant the call actually used.
            let face_imprint_result_schema = |token_doc: &str| {
                serde_json::json!({
                    "type": "object",
                    "properties": { "object_id": { "type": "string" } },
                    "required": ["object_id"],
                    "description": format!(
                        "on-face drawing (plane spec {{\"face\": <locator>}}) imprints the solid's face instead of creating a sketch region; mints {token_doc}"
                    )
                })
            };
            let split_face_tokens = "transaction-scoped face tokens \"a\"/\"b\" naming the two faces the cut produced (HEW_API.md §5.2/§5.4)";
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
                    "points": { "type": "array", "items": point_schema, "minItems": 2 }
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
                    "path_too_short",
                    "endpoint_not_on_boundary",
                    "path_not_simple",
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
                    "corner_b": point_schema
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
                    "loop_not_strictly_inside",
                    "loop_self_intersects",
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
                    }
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
                    "loop_not_strictly_inside",
                    "loop_self_intersects",
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
                    }
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
                    "path_too_short",
                    "endpoint_not_on_boundary",
                    "path_not_simple",
                    // Closed-loop (`SplitFaceInner`) face-mode refusals —
                    // a full-turn sweep, or `close: "pie"`/`"segment"`:
                    "loop_not_strictly_inside",
                    "loop_self_intersects",
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
                    "sides": { "type": "integer", "minimum": 3 }
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
                    "loop_not_strictly_inside",
                    "loop_self_intersects",
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
                    "region": { "type": "string" },
                    "distance": { "type": "number", "description": "positive grows the material, negative shrinks it" }
                },
                "required": ["region", "distance"]
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "sketch": { "type": "string" },
                    "region_ids": { "type": "array", "items": { "type": "string" } },
                    "region_id": { "type": "string", "description": "present when exactly one region resulted" },
                    "curve_ids": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["sketch", "region_ids", "curve_ids"]
            });
            cmd.refusals = vec![
                "unknown_region",
                "malformed_region",
                "offset_too_small",
                "offset_collapsed",
                "unknown_sketch",
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
                    "region": { "type": "string" },
                    "distance": { "type": "number" }
                },
                "required": ["region", "distance"]
            });
            cmd.result_schema = serde_json::json!({
                "type": "object",
                "properties": { "object_id": { "type": "string" } },
                "required": ["object_id"],
                "description": "mints face tokens \"base\", \"top\", \"side.<n>\" (boundary-loop order) — HEW_API.md §5.4's normative example"
            });
            cmd.refusals = vec![
                "distance_too_small",
                "degenerate_geometry",
                "unknown_region",
                "unknown_entity",
            ];

            let cmd = commands
                .get_mut("hew.solid.push_pull")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "face": face_locator_schema,
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
            ];

            let boolean_params = serde_json::json!({
                "type": "object",
                "properties": {
                    "a": { "type": "string", "description": "a node id: obj_/grp_/ins_" },
                    "b": { "type": "string", "description": "a node id: obj_/grp_/ins_" }
                },
                "required": ["a", "b"]
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
                ]
            };

            let cmd = commands.get_mut("hew.solid.union").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = boolean_params.clone();
            cmd.result_schema = boolean_result.clone();
            cmd.refusals = boolean_refusals();

            let cmd = commands
                .get_mut("hew.solid.subtract")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = boolean_params.clone();
            cmd.result_schema = boolean_result.clone();
            cmd.refusals = boolean_refusals();

            let cmd = commands
                .get_mut("hew.solid.intersect")
                .expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = boolean_params;
            cmd.result_schema = boolean_result;
            cmd.refusals = boolean_refusals();

            let cmd = commands.get_mut("hew.solid.slice").expect("declared above");
            cmd.implemented = true;
            cmd.params_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "object": { "type": "string" },
                    "plane": {
                        "type": "object",
                        "properties": { "origin": point_schema.clone(), "normal": point_schema.clone() },
                        "required": ["origin", "normal"]
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
        ] {
            commands
                .get_mut(name)
                .expect("declared above")
                .mutates_document = true;
        }

        Registry { commands }
    }

    /// Look up a command by its exact method name.
    pub fn get(&self, name: &str) -> Option<&CommandDecl> {
        self.commands.get(name)
    }

    /// Every declaration, in name order (deterministic).
    pub fn commands(&self) -> impl Iterator<Item = &CommandDecl> {
        self.commands.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Registry completeness (docs/HEW_API.md §14): every declared command
    /// carries schemas, a summary, and a non-empty refusal inventory; names
    /// are namespaced and well-formed; versions start at 1.
    #[test]
    fn every_declaration_is_complete() {
        let reg = Registry::protocol_1();
        assert!(reg.commands().count() >= 50);
        for cmd in reg.commands() {
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
                    cmd.refusals.contains(&"unimplemented"),
                    "{} is unimplemented but does not declare the unimplemented refusal",
                    cmd.name
                );
            }
        }
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

    /// Core is App minus `hew.view.*`, except `hew.view.snapshot` — the one
    /// view command with a headless render path (§10) — which core grants
    /// too. A narrowable maximum, not a different protocol.
    #[test]
    fn core_profile_grants_snapshot_but_withholds_other_view_commands() {
        let reg = Registry::protocol_1();
        for cmd in reg.commands() {
            assert!(Profile::App.grants(cmd));
            let expected = cmd.name == "hew.view.snapshot" || !cmd.name.starts_with("hew.view.");
            assert_eq!(
                Profile::Core.grants(cmd),
                expected,
                "core grant wrong for {}",
                cmd.name
            );
        }
    }

    /// Pins the specific carve-out directly: today's registry has only one
    /// `hew.view.*` command, so this is the one live example until a
    /// future app-only view command exists to demonstrate the withheld
    /// side too.
    #[test]
    fn core_grants_snapshot_specifically() {
        let reg = Registry::protocol_1();
        let snapshot = reg.get("hew.view.snapshot").expect("declared above");
        assert!(Profile::Core.grants(snapshot));
        assert!(Profile::App.grants(snapshot));
    }
}
