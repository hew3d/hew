//! Grants: what a connection may do (docs/agents/HEW_API.md §10,
//! docs/agents/PLUGINS.md "Permissions"). A grant replaces the old
//! two-variant profile as the enforcement object; the first-party
//! profiles are its two constructors, [`Grant::core`] and [`Grant::app`],
//! and behave exactly as the profiles did.
//!
//! Enforcement happens at the dispatcher's existing check sites and inside
//! transactions, statically (PLUGINS.md invariant 3); nothing here is a
//! UI-only gate.

use std::collections::BTreeSet;

use crate::registry::CommandDecl;

/// The widest set of commands a grant may reach: headless-safe (`Core`)
/// or live-application (`App`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ceiling {
    Core,
    App,
}

/// What a connection may do. Held in a [`Grant`] or declared on a
/// [`CommandDecl`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Permission {
    ModelRead,
    ModelWrite,
    AttributesWrite,
    AttributesWriteNs(String),
    View,
    DocumentsSave,
    DocumentsDiscard,
    DocumentsOther,
    Files,
    Clipboard,
    Library,
    Network(String),
    /// First-party connections (the UI, hew-cli, the MCP server).
    All,
    /// Declaration-only: granted to every connection. Never held.
    Always,
    /// Declaration-only: satisfied by `All` only. Never held.
    FirstParty,
}

impl Permission {
    /// `model.read`, `attributes.write:com.x`, `network:host`, ...; `All` is
    /// `all`. `Always` and `FirstParty` have no wire form. A suffix is
    /// validated exactly as a manifest's permission is.
    pub fn parse(s: &str) -> Result<Permission, String> {
        if s == "all" {
            return Ok(Permission::All);
        }
        let name = plugin_manifest::PermissionName::parse(s).map_err(|e| e.explanation())?;
        Ok(match name.split() {
            ("model.read", None) => Permission::ModelRead,
            ("model.write", None) => Permission::ModelWrite,
            ("attributes.write", None) => Permission::AttributesWrite,
            ("attributes.write", Some(ns)) => Permission::AttributesWriteNs(ns.to_string()),
            ("view", None) => Permission::View,
            ("documents.save", None) => Permission::DocumentsSave,
            ("documents.discard", None) => Permission::DocumentsDiscard,
            ("documents.other", None) => Permission::DocumentsOther,
            ("files", None) => Permission::Files,
            ("clipboard", None) => Permission::Clipboard,
            ("library", None) => Permission::Library,
            ("network", Some(host)) => Permission::Network(host.to_string()),
            _ => return Err(format!("\"{s}\" is not a permission")),
        })
    }

    /// The wire form [`Permission::parse`] reads back. `Always` and
    /// `FirstParty` render as `always` and `first_party`, which `parse`
    /// refuses: they describe commands, never connections.
    pub fn to_wire(&self) -> String {
        match self {
            Permission::ModelRead => "model.read".into(),
            Permission::ModelWrite => "model.write".into(),
            Permission::AttributesWrite => "attributes.write".into(),
            Permission::AttributesWriteNs(ns) => format!("attributes.write:{ns}"),
            Permission::View => "view".into(),
            Permission::DocumentsSave => "documents.save".into(),
            Permission::DocumentsDiscard => "documents.discard".into(),
            Permission::DocumentsOther => "documents.other".into(),
            Permission::Files => "files".into(),
            Permission::Clipboard => "clipboard".into(),
            Permission::Library => "library".into(),
            Permission::Network(host) => format!("network:{host}"),
            Permission::All => "all".into(),
            Permission::Always => "always".into(),
            Permission::FirstParty => "first_party".into(),
        }
    }
}

impl serde::Serialize for Permission {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_wire())
    }
}

impl<'de> serde::Deserialize<'de> for Permission {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Permission::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// What a connection may call, write, and see.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub ceiling: Ceiling,
    pub permissions: BTreeSet<Permission>,
    /// The plugin's own id, when this connection is a plugin.
    #[serde(default)]
    pub plugin: Option<String>,
    /// The connected app's pairing id, when this connection is one.
    #[serde(default)]
    pub app: Option<String>,
    /// The document this grant is bound to; `None` for first-party.
    #[serde(default)]
    pub document: Option<String>,
}

impl Grant {
    /// The headless-safe first-party grant (the old `core` profile).
    pub fn core() -> Grant {
        Grant {
            ceiling: Ceiling::Core,
            permissions: BTreeSet::from([Permission::All]),
            plugin: None,
            app: None,
            document: None,
        }
    }

    /// The live-application first-party grant (the old `app` profile).
    pub fn app() -> Grant {
        Grant {
            ceiling: Ceiling::App,
            ..Grant::core()
        }
    }

    /// A plugin's grant, bound to `document`, at the ceiling its
    /// permissions derive.
    pub fn plugin(id: &str, document: Option<&str>, perms: BTreeSet<Permission>) -> Grant {
        Grant {
            ceiling: Grant::derive_ceiling(&perms),
            permissions: perms,
            plugin: Some(id.to_string()),
            app: None,
            document: document.map(str::to_string),
        }
    }

    /// A connected app's grant, at the ceiling its permissions derive.
    pub fn connected_app(pairing: &str, perms: BTreeSet<Permission>) -> Grant {
        Grant {
            ceiling: Grant::derive_ceiling(&perms),
            permissions: perms,
            plugin: None,
            app: Some(pairing.to_string()),
            document: None,
        }
    }

    /// Permissions from a manifest's names: `attributes.write` stays the
    /// bare variant, `attributes.write:<ns>` becomes `AttributesWriteNs`.
    /// A name that does not validate grants nothing.
    pub fn permissions_from_names(
        names: &[plugin_manifest::PermissionName],
    ) -> BTreeSet<Permission> {
        names
            .iter()
            .filter_map(|n| Permission::parse(n.as_str()).ok())
            .filter(|p| *p != Permission::All)
            .collect()
    }

    /// `App` if any live-host permission is held (`View`, any documents
    /// permission, `Files`, `Clipboard`), else `Core`.
    pub fn derive_ceiling(perms: &BTreeSet<Permission>) -> Ceiling {
        let live = [
            Permission::View,
            Permission::DocumentsSave,
            Permission::DocumentsDiscard,
            Permission::DocumentsOther,
            Permission::Files,
            Permission::Clipboard,
        ];
        if live.iter().any(|p| perms.contains(p)) {
            Ceiling::App
        } else {
            Ceiling::Core
        }
    }

    /// A grant that is not first-party: a plugin or a connected app.
    pub fn is_restricted(&self) -> bool {
        self.plugin.is_some() || self.app.is_some()
    }

    pub fn holds(&self, p: &Permission) -> bool {
        self.permissions.contains(p)
    }

    /// Whether this is a first-party grant holding `all`. A restricted grant
    /// never gets the `all` short-circuits, even if its permissions name
    /// `all`: the plugin and connected-app rules (namespaces, document
    /// binding, path parameters) always apply to it.
    fn holds_all(&self) -> bool {
        !self.is_restricted() && self.holds(&Permission::All)
    }

    /// The identity the connection carries: `plugin:<id>`, `app:<pairing>`,
    /// or `fallback`.
    pub fn identity_or(&self, fallback: &str) -> String {
        match (&self.plugin, &self.app) {
            (Some(id), _) => format!("plugin:{id}"),
            (None, Some(pairing)) => format!("app:{pairing}"),
            (None, None) => fallback.to_string(),
        }
    }

    /// Whether this grant may call `decl`.
    pub fn grants(&self, decl: &CommandDecl) -> bool {
        if decl.permission == Permission::Always {
            return true;
        }
        if self.holds_all() {
            return within_ceiling(self.ceiling, decl);
        }
        if decl.permission == Permission::FirstParty {
            return false;
        }
        let held = self.holds(&decl.permission)
            || (decl.permission == Permission::AttributesWrite
                && self
                    .permissions
                    .iter()
                    .any(|p| matches!(p, Permission::AttributesWriteNs(_))));
        within_ceiling(self.ceiling, decl) && held
    }

    /// Whether this grant may write attributes under `ns`. The `hew`
    /// reservation is checked separately, first, by the caller.
    pub fn may_write_attr_namespace(&self, ns: &str) -> bool {
        if self.holds_all() {
            return true;
        }
        let under =
            |root: &str| ns == root || ns.strip_prefix(root).is_some_and(|r| r.starts_with('.'));
        if let Some(own) = &self.plugin
            && under(own)
        {
            return true;
        }
        self.permissions.iter().any(|p| match p {
            Permission::AttributesWriteNs(q) => under(q),
            _ => false,
        })
    }

    /// Whether this grant receives `event`.
    pub fn may_receive(&self, event: &str) -> bool {
        if self.holds_all() {
            return true;
        }
        match event {
            "document_changed" | "context_changed" | "units_changed" | "scenes_changed"
            | "document_will_save" | "document_saved" | "document_opened" | "document_closed" => {
                self.holds(&Permission::ModelRead)
            }
            "selection_changed" | "tool_changed" => self.holds(&Permission::View),
            "plugin_state" | "slot_changed" | "settings_changed" | "ui_cancel"
            | "window_closed" | "surface_closed" => true,
            _ => false,
        }
    }

    /// `All` or `DocumentsOther`.
    pub fn may_address_other_documents(&self) -> bool {
        self.holds_all() || self.holds(&Permission::DocumentsOther)
    }

    /// What `hello` and `hew.meta.documents` show this connection: for a
    /// restricted grant, only the bound document (every document with
    /// `DocumentsOther`), without `path` unless `DocumentsSave` is held.
    /// Order is preserved.
    pub fn filter_documents(&self, documents: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
        if !self.is_restricted() {
            return documents;
        }
        let keep_all = self.may_address_other_documents();
        let keep_paths = self.holds(&Permission::DocumentsSave) || self.holds_all();
        documents
            .into_iter()
            .filter(|d| {
                keep_all
                    || (self.document.is_some()
                        && d.get("id").and_then(serde_json::Value::as_str)
                            == self.document.as_deref())
            })
            .map(|mut d| {
                if !keep_paths && let Some(object) = d.as_object_mut() {
                    object.remove("path");
                }
                d
            })
            .collect()
    }
}

/// The ceiling rule (the old `core` profile's, verbatim): `App` reaches
/// everything; `Core` withholds the window commands and every
/// `hew.view.*` command but the two with a headless render path.
pub fn within_ceiling(ceiling: Ceiling, decl: &CommandDecl) -> bool {
    match ceiling {
        Ceiling::App => true,
        Ceiling::Core => {
            !matches!(decl.name.as_str(), "hew.doc.close" | "hew.doc.focus")
                && (decl.name == "hew.view.snapshot"
                    || decl.name == "hew.view.line_drawing"
                    || !decl.name.starts_with("hew.view."))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Registry;
    use serde_json::json;

    fn set(perms: &[Permission]) -> BTreeSet<Permission> {
        perms.iter().cloned().collect()
    }

    fn every_holdable() -> Vec<Permission> {
        vec![
            Permission::ModelRead,
            Permission::ModelWrite,
            Permission::AttributesWrite,
            Permission::AttributesWriteNs("com.other.app".into()),
            Permission::View,
            Permission::DocumentsSave,
            Permission::DocumentsDiscard,
            Permission::DocumentsOther,
            Permission::Files,
            Permission::Clipboard,
            Permission::Library,
            Permission::Network("api.example.com".into()),
            Permission::All,
        ]
    }

    #[test]
    fn core_and_app_match_the_old_profile_rule() {
        // The deleted `Profile::Core` rule, inlined.
        fn old_core(name: &str) -> bool {
            !matches!(name, "hew.doc.close" | "hew.doc.focus")
                && (name == "hew.view.snapshot"
                    || name == "hew.view.line_drawing"
                    || !name.starts_with("hew.view."))
        }
        let reg = Registry::protocol_1();
        for c in reg.commands() {
            assert_eq!(Grant::core().grants(c), old_core(&c.name), "{}", c.name);
            assert!(Grant::app().grants(c), "{}", c.name);
        }
    }

    #[test]
    fn plugin_grant_requires_held_permission() {
        let reg = Registry::protocol_1();
        let g = Grant::plugin(
            "com.example.p",
            Some("doc_1"),
            set(&[Permission::ModelRead]),
        );
        assert!(g.grants(reg.get("hew.query.scene").unwrap()));
        assert!(g.grants(reg.get("hew.meta.hello").unwrap()), "always");
        assert!(g.grants(reg.get("hew.doc.transact").unwrap()), "always");
        assert!(!g.grants(reg.get("hew.sketch.draw_rect").unwrap()));
        assert!(!g.grants(reg.get("hew.attr.set").unwrap()));
        assert!(!g.grants(reg.get("hew.doc.save").unwrap()));
        // Within the ceiling AND held: a Core-ceiling grant holding View
        // cannot exist through `plugin`, but a hand-built one is refused.
        let mut hand = g.clone();
        hand.permissions.insert(Permission::View);
        assert!(
            !hand.grants(reg.get("hew.view.camera").unwrap()),
            "core ceiling"
        );
        hand.ceiling = Ceiling::App;
        assert!(hand.grants(reg.get("hew.view.camera").unwrap()));
        // Every command a plugin grant reaches declares a held permission.
        let wide = Grant::plugin("com.example.p", None, set(&every_holdable()[..12]));
        for c in reg.commands() {
            if wide.grants(c) {
                assert!(
                    c.permission == Permission::Always || wide.holds(&c.permission),
                    "{}",
                    c.name
                );
            }
        }
    }

    #[test]
    fn attributes_write_ns_satisfies_attributes_write() {
        let reg = Registry::protocol_1();
        let g = Grant::plugin(
            "com.example.p",
            None,
            set(&[Permission::AttributesWriteNs("com.other.app".into())]),
        );
        assert!(g.grants(reg.get("hew.attr.set").unwrap()));
        assert!(g.grants(reg.get("hew.attr.delete").unwrap()));
        assert!(
            !g.grants(reg.get("hew.attr.get").unwrap()),
            "reads need model.read"
        );
    }

    #[test]
    fn derive_ceiling_rule() {
        assert_eq!(Grant::derive_ceiling(&set(&[])), Ceiling::Core);
        for p in [
            Permission::ModelRead,
            Permission::ModelWrite,
            Permission::AttributesWrite,
            Permission::AttributesWriteNs("a.b".into()),
            Permission::Library,
            Permission::Network("a.b".into()),
            Permission::All,
        ] {
            assert_eq!(
                Grant::derive_ceiling(&set(std::slice::from_ref(&p))),
                Ceiling::Core,
                "{p:?}"
            );
        }
        for p in [
            Permission::View,
            Permission::DocumentsSave,
            Permission::DocumentsDiscard,
            Permission::DocumentsOther,
            Permission::Files,
            Permission::Clipboard,
        ] {
            assert_eq!(
                Grant::derive_ceiling(&set(&[Permission::ModelRead, p.clone()])),
                Ceiling::App,
                "{p:?}"
            );
        }
    }

    #[test]
    fn may_write_attr_namespace_cases() {
        let own = Grant::plugin("com.example.p", None, set(&[Permission::AttributesWrite]));
        assert!(own.may_write_attr_namespace("com.example.p"), "own");
        assert!(
            own.may_write_attr_namespace("com.example.p.parts"),
            "nested"
        );
        assert!(
            !own.may_write_attr_namespace("com.example.pp"),
            "prefix is not nesting"
        );
        assert!(!own.may_write_attr_namespace("com.example"), "parent");
        assert!(
            !own.may_write_attr_namespace("com.example.other"),
            "sibling"
        );
        let declared = Grant::plugin(
            "com.example.p",
            None,
            set(&[Permission::AttributesWriteNs("org.shared.data".into())]),
        );
        assert!(declared.may_write_attr_namespace("org.shared.data"));
        assert!(declared.may_write_attr_namespace("org.shared.data.x"));
        assert!(!declared.may_write_attr_namespace("org.shared.database"));
        assert!(
            declared.may_write_attr_namespace("com.example.p"),
            "own still"
        );
        let app = Grant::connected_app("pair_1", set(&[Permission::AttributesWrite]));
        assert!(!app.may_write_attr_namespace("pair_1"), "no own namespace");
        assert!(!app.may_write_attr_namespace("com.example.p"));
        let app_ns = Grant::connected_app(
            "pair_1",
            set(&[Permission::AttributesWriteNs("com.example.app".into())]),
        );
        assert!(app_ns.may_write_attr_namespace("com.example.app.x"));
        assert!(
            Grant::core().may_write_attr_namespace("anything.at.all"),
            "first-party"
        );
        assert!(Grant::app().may_write_attr_namespace("x"));
    }

    #[test]
    fn filter_documents_hides_other_documents_and_paths() {
        let docs = vec![
            json!({"id": "doc_1", "title": "A", "path": "/a.hew", "dirty": false}),
            json!({"id": "doc_2", "title": "B", "path": null, "dirty": true}),
            json!({"id": "doc_3", "title": "C", "path": "/c.hew", "dirty": false}),
        ];
        assert_eq!(Grant::core().filter_documents(docs.clone()), docs);
        assert_eq!(Grant::app().filter_documents(docs.clone()), docs);

        let bound = Grant::plugin(
            "com.example.p",
            Some("doc_3"),
            set(&[Permission::ModelRead]),
        );
        assert_eq!(
            bound.filter_documents(docs.clone()),
            vec![json!({"id": "doc_3", "title": "C", "dirty": false})]
        );
        let saver = Grant::plugin(
            "com.example.p",
            Some("doc_3"),
            set(&[Permission::ModelRead, Permission::DocumentsSave]),
        );
        assert_eq!(saver.filter_documents(docs.clone()), vec![docs[2].clone()]);
        let other = Grant::plugin(
            "com.example.p",
            Some("doc_3"),
            set(&[Permission::DocumentsOther]),
        );
        let seen = other.filter_documents(docs.clone());
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0]["id"], "doc_1", "order preserved");
        assert!(seen.iter().all(|d| d.get("path").is_none()));
        let unbound = Grant::connected_app("pair_1", set(&[Permission::ModelRead]));
        assert!(unbound.filter_documents(docs).is_empty());
    }

    #[test]
    fn permission_wire_round_trip() {
        for p in every_holdable() {
            let wire = p.to_wire();
            assert_eq!(Permission::parse(&wire), Ok(p.clone()), "{wire}");
            let json = serde_json::to_value(&p).unwrap();
            assert_eq!(json, serde_json::Value::String(wire));
            assert_eq!(serde_json::from_value::<Permission>(json).unwrap(), p);
        }
        assert!(Permission::parse(&Permission::Always.to_wire()).is_err());
        assert!(Permission::parse(&Permission::FirstParty.to_wire()).is_err());
        assert!(Permission::parse("network:*").is_err());
        assert!(Permission::parse("attributes.write:hew.x").is_err());
    }

    #[test]
    fn permission_names_match_manifest_vocabulary() {
        for name in plugin_manifest::PERMISSION_NAMES {
            let p = Permission::parse(name).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(p.to_wire(), *name);
        }
        let names: Vec<plugin_manifest::PermissionName> = [
            "model.read",
            "attributes.write",
            "attributes.write:com.other.app",
            "network:api.example.com",
        ]
        .iter()
        .map(|s| plugin_manifest::PermissionName::parse(s).unwrap())
        .collect();
        assert_eq!(
            Grant::permissions_from_names(&names),
            set(&[
                Permission::ModelRead,
                Permission::AttributesWrite,
                Permission::AttributesWriteNs("com.other.app".into()),
                Permission::Network("api.example.com".into()),
            ])
        );
    }

    #[test]
    fn grant_json_shape() {
        let g = Grant::plugin(
            "com.example.p",
            Some("doc_1"),
            set(&[Permission::ModelRead, Permission::AttributesWrite]),
        );
        let v = serde_json::to_value(&g).unwrap();
        assert_eq!(
            v,
            json!({
                "ceiling": "core",
                "permissions": ["model.read", "attributes.write"],
                "plugin": "com.example.p",
                "app": null,
                "document": "doc_1"
            })
        );
        assert_eq!(serde_json::from_value::<Grant>(v).unwrap(), g);
        assert!(
            serde_json::from_value::<Grant>(
                json!({"ceiling": "core", "permissions": [], "extra": 1})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<Grant>(json!({"ceiling": "core", "permissions": ["always"]}))
                .is_err()
        );
        assert_eq!(g.identity_or("x"), "plugin:com.example.p");
        assert_eq!(
            Grant::connected_app("p1", set(&[])).identity_or("x"),
            "app:p1"
        );
        assert_eq!(Grant::core().identity_or("x"), "x");
    }

    /// `all` on a restricted grant reaches nothing extra: it is ignored.
    #[test]
    fn all_on_a_restricted_grant_is_ignored() {
        let reg = Registry::protocol_1();
        let g = Grant::plugin("com.example.p", Some("doc_1"), set(&[Permission::All]));
        assert!(!g.grants(reg.get("hew.query.scene").unwrap()));
        assert!(!g.grants(reg.get("hew.sketch.draw_rect").unwrap()));
        assert!(g.grants(reg.get("hew.meta.hello").unwrap()), "always still");
        assert!(!g.may_write_attr_namespace("com.other"));
        assert!(!g.may_address_other_documents());
        assert!(!g.may_receive("document_changed"));
        let docs = vec![
            json!({"id": "doc_1", "title": "A", "path": "/a", "dirty": false}),
            json!({"id": "doc_2", "title": "B", "path": "/b", "dirty": false}),
        ];
        assert_eq!(
            g.filter_documents(docs),
            vec![json!({"id": "doc_1", "title": "A", "dirty": false})]
        );
        let app = Grant::connected_app("pair_1", set(&[Permission::All]));
        assert!(!app.grants(reg.get("hew.query.scene").unwrap()));
    }

    #[test]
    fn may_receive_table() {
        let none = Grant::plugin("com.example.p", None, set(&[]));
        let read = Grant::plugin("com.example.p", None, set(&[Permission::ModelRead]));
        let view = Grant::plugin("com.example.p", None, set(&[Permission::View]));
        for e in ["document_changed", "document_closed", "scenes_changed"] {
            assert!(
                !none.may_receive(e) && read.may_receive(e) && !view.may_receive(e),
                "{e}"
            );
        }
        for e in ["selection_changed", "tool_changed"] {
            assert!(!read.may_receive(e) && view.may_receive(e), "{e}");
        }
        for e in ["plugin_state", "surface_closed", "ui_cancel"] {
            assert!(none.may_receive(e), "{e}");
        }
        assert!(!read.may_receive("something_new"));
        assert!(Grant::core().may_receive("selection_changed"));
    }
}
