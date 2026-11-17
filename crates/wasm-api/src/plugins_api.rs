//! The plugin path across the wasm boundary: reading a `.hewplugin`,
//! validating a manifest, building the consent payload, checking the Hew
//! version range, and checking an install against what is already
//! installed (the web build's store lives in the page, in OPFS; these are
//! the pure checks it runs, shared with the desktop through
//! `plugin-host`); building a running plugin's grant from its stored
//! record; and reporting a plugin's state to the bus.
//!
//! Compiled only in a preview build (`lib.rs` declares this module under
//! `cfg(hew_preview)`); each export is listed in the "plugins" entry's
//! `loader.kernelExports`, and a production kernel has none of them.
//!
//! Errors are `"CODE: explanation"`: `MANIFEST_<NAME>` for a manifest
//! refusal (`NAME` is the manifest error's refusal name, uppercased),
//! `INSTALL_<NAME>` for any other install refusal, and `PLUGIN_BAD_INPUT`
//! when an argument is not the JSON or version text it should be.

use crate::{ApiError, Scene};
use plugin_host::{InstallError, InstallSource, Installed, semver};
use wasm_bindgen::prelude::*;

fn install_error(e: &InstallError) -> ApiError {
    match e {
        InstallError::Manifest(m) => ApiError::new(
            &format!("MANIFEST_{}", m.name().to_uppercase()),
            &m.explanation(),
        ),
        other => ApiError::new(
            &format!("INSTALL_{}", other.name().to_uppercase()),
            &other.explanation(),
        ),
    }
}

fn bad_input(what: &str, why: impl std::fmt::Display) -> ApiError {
    ApiError::new("PLUGIN_BAD_INPUT", &format!("{what}: {why}"))
}

/// A manifest from JSON, through the same validating parser a package's
/// `hew-plugin.json` goes through, so a manifest the page hands back in is
/// checked again rather than trusted.
fn manifest(text: &str) -> Result<plugin_manifest::Manifest, ApiError> {
    plugin_manifest::parse(text).map_err(|e| install_error(&InstallError::Manifest(e)))
}

fn to_json(value: &impl serde::Serialize) -> String {
    serde_json::to_string(value).expect("plugin install values serialize")
}

/// Reads a `.hewplugin` (`plugin_host::read_package_zip_spans`): every
/// entry stored, every path safe, the manifest valid and every file it
/// names present. Answers `{ "manifest": <normalized manifest>, "files":
/// [{ "path", "offset", "size" }, …] }`, sorted by path. The bytes stay
/// with the caller: `offset` counts from the start of the bytes passed in
/// and `bytes.slice(offset, offset + size)` is exactly the file Hew
/// checked, so the caller stores those slices and never reads the zip a
/// second time.
#[wasm_bindgen]
pub fn plugin_package_from_zip(bytes: &[u8]) -> Result<String, ApiError> {
    let (package, spans) =
        plugin_host::read_package_zip_spans(bytes).map_err(|e| install_error(&e))?;
    let files: Vec<serde_json::Value> = spans
        .iter()
        .map(|(path, span)| {
            serde_json::json!({ "path": path, "offset": span.offset, "size": span.size })
        })
        .collect();
    Ok(to_json(&serde_json::json!({
        "manifest": package.manifest,
        "files": files,
    })))
}

/// The consent payload (`plugin_host::consent`) as JSON. `manifest_json`
/// and `previous_manifest_json` are manifests (normalized or as written);
/// `source_json` an install source (`{ "kind": "file", "name" }`, ...);
/// `previous_installed_json` the installed record of the same id, when
/// this is an update. A file or folder source claiming a reserved id is
/// refused `INSTALL_RESERVED_ID` here, before anything is shown.
#[wasm_bindgen]
pub fn plugin_consent(
    manifest_json: &str,
    source_json: &str,
    previous_installed_json: Option<String>,
    previous_manifest_json: Option<String>,
    signer: Option<String>,
) -> Result<String, ApiError> {
    let manifest = manifest(manifest_json)?;
    let source: InstallSource =
        serde_json::from_str(source_json).map_err(|e| bad_input("source", e))?;
    plugin_host::check_source_id(&manifest, &source).map_err(|e| install_error(&e))?;
    let previous = previous_installed_json
        .map(|text| Installed::from_json(&manifest.id, text.as_bytes()))
        .transpose()
        .map_err(|e| install_error(&e))?;
    let previous_manifest = previous_manifest_json
        .map(|text| self::manifest(&text))
        .transpose()?;
    let payload = plugin_host::consent(
        &manifest,
        &source,
        previous.as_ref(),
        previous_manifest.as_ref(),
        signer.as_deref(),
    );
    Ok(to_json(&payload))
}

/// Validates a `hew-plugin.json` (`plugin_manifest::parse`) and answers it
/// normalized: every default filled in, as `serde_json` writes the
/// `Manifest`. A refusal is `MANIFEST_<NAME>` with the manifest's own
/// explanation.
#[wasm_bindgen]
pub fn plugin_manifest_parse(text: &str) -> Result<String, ApiError> {
    Ok(to_json(&manifest(text)?))
}

/// Whether the manifest's `hew` range admits `running`
/// (`plugin_host::hew_version_ok`).
#[wasm_bindgen]
pub fn plugin_hew_version_ok(manifest_json: &str, running: &str) -> Result<bool, ApiError> {
    let manifest = manifest(manifest_json)?;
    let running = semver::Version::parse(running).map_err(|e| bad_input("running", e))?;
    Ok(plugin_host::hew_version_ok(&manifest, &running))
}

/// Whether a manifest may be installed from `source` next to what is
/// installed, with the same checks the desktop's `stage` makes, in this
/// order: a file or folder source may not claim a reserved id
/// (`INSTALL_RESERVED_ID`); the manifest's `hew` range must admit
/// `running` (`INSTALL_HEW_VERSION_UNSUPPORTED`); every `requires` entry
/// must be installed in a matching version (`INSTALL_DEPENDENCY_MISSING`)
/// and the `requires` graph must stay acyclic
/// (`INSTALL_DEPENDENCY_CYCLE`). `installed_json` is an array of
/// `{ "installed": <installed.json>, "manifest": <hew-plugin.json> }`, one
/// per installed plugin; each is checked again as it is read.
#[wasm_bindgen]
pub fn plugin_install_check(
    manifest_json: &str,
    source_json: &str,
    running: &str,
    installed_json: &str,
) -> Result<(), ApiError> {
    let manifest = manifest(manifest_json)?;
    let source: InstallSource =
        serde_json::from_str(source_json).map_err(|e| bad_input("source", e))?;
    let running = semver::Version::parse(running).map_err(|e| bad_input("running", e))?;
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(installed_json).map_err(|e| bad_input("installed", e))?;
    let mut installed = Vec::with_capacity(rows.len());
    for row in rows {
        let record = row
            .get("installed")
            .ok_or_else(|| bad_input("installed", "a row has no \"installed\""))?;
        let id = record
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| bad_input("installed", "a record has no id"))?;
        let record = Installed::from_json(id, record.to_string().as_bytes())
            .map_err(|e| install_error(&e))?;
        let row_manifest = row
            .get("manifest")
            .ok_or_else(|| bad_input("installed", "a row has no \"manifest\""))?;
        installed.push((record, self::manifest(&row_manifest.to_string())?));
    }
    plugin_host::check_source_id(&manifest, &source).map_err(|e| install_error(&e))?;
    if !plugin_host::hew_version_ok(&manifest, &running) {
        return Err(install_error(&InstallError::HewVersionUnsupported {
            required: manifest
                .hew
                .as_ref()
                .map_or_else(String::new, ToString::to_string),
            running: running.to_string(),
        }));
    }
    plugin_host::check_requires(&manifest, &installed).map_err(|e| install_error(&e))
}

/// The most permission names [`plugin_grant_json`] takes: a record's
/// grant and its optional grants, each capped by the manifest's limit.
const MAX_GRANT_NAMES: usize = 2 * plugin_host::MAX_PERMISSIONS;
/// The longest document id [`plugin_grant_json`] binds a grant to.
const MAX_DOCUMENT_ID_BYTES: usize = 64;

/// A running plugin's grant, as the JSON [`Scene::api_plugin_connection_open`]
/// takes: `api::Grant::plugin` for plugin `id`, bound to `document` (the
/// window's document id, `doc_<n>`), holding the permissions `names_json`
/// names (a JSON array of strings: the stored record's `grant` and
/// `optional_granted`), at the ceiling those permissions derive. The
/// ceiling rule is `api`'s and nowhere else.
///
/// Nothing is dropped quietly: an id outside the plugin id grammar, or any
/// name that is not a permission a plugin may hold (`all` included), is
/// refused `MANIFEST_<NAME>`; a document id that is not 1 to 64 ASCII
/// letters, digits, `_` or `-`, names that are not an array of strings, or
/// more than 128 of them, are `PLUGIN_BAD_INPUT`. Repeated names count
/// once.
#[wasm_bindgen]
pub fn plugin_grant_json(id: &str, document: &str, names_json: &str) -> Result<String, ApiError> {
    plugin_manifest::validate_plugin_id(id)
        .map_err(|e| install_error(&InstallError::Manifest(e)))?;
    let document_ok = !document.is_empty()
        && document.len() <= MAX_DOCUMENT_ID_BYTES
        && document
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if !document_ok {
        return Err(bad_input("document", "not a document id"));
    }
    let raw: Vec<String> = serde_json::from_str(names_json).map_err(|e| bad_input("names", e))?;
    if raw.len() > MAX_GRANT_NAMES {
        return Err(bad_input("names", format!("more than {MAX_GRANT_NAMES}")));
    }
    let mut names: Vec<plugin_manifest::PermissionName> = Vec::with_capacity(raw.len());
    for name in &raw {
        let parsed = plugin_manifest::PermissionName::parse(name)
            .map_err(|e| install_error(&InstallError::Manifest(e)))?;
        if !names.contains(&parsed) {
            names.push(parsed);
        }
    }
    let permissions = api::Grant::permissions_from_names(&names);
    // Every name that passed the plugin grammar must arrive as a
    // permission; one that `api` would drop is refused by name instead.
    if let Some(dropped) = names
        .iter()
        .find(|n| !permissions.iter().any(|p| p.to_wire() == n.as_str()))
    {
        return Err(install_error(&InstallError::Manifest(
            plugin_manifest::ManifestError::UnknownPermission(dropped.as_str().to_string()),
        )));
    }
    let grant = api::Grant::plugin(id, Some(document), permissions);
    Ok(to_json(&grant))
}

/// The states a plugin reports through `plugin_state`.
const PLUGIN_STATES: &[&str] = &["loaded", "unloaded", "crashed", "grant_changed"];
/// The longest `detail` a `plugin_state` event carries, in characters.
const MAX_STATE_DETAIL_CHARS: usize = 200;

#[wasm_bindgen]
impl Scene {
    /// The host reports a plugin's state; the next [`Scene::drain_events`]
    /// sends `plugin_state` `{plugin, state, detail?}` to every connection
    /// subscribed to it. `id` must follow the plugin id grammar
    /// (`MANIFEST_<NAME>` otherwise) and `state` be one of `loaded`,
    /// `unloaded`, `crashed` or `grant_changed` (`PLUGIN_BAD_INPUT`
    /// otherwise). `detail` may come from the plugin (a crash message), so
    /// it is cleaned like any plugin text and cut to 200 characters; one
    /// that cleans to nothing is left out. Nothing is queued on a refusal.
    pub fn note_plugin_state(
        &mut self,
        id: &str,
        state: &str,
        detail: Option<String>,
    ) -> Result<(), ApiError> {
        plugin_manifest::validate_plugin_id(id)
            .map_err(|e| install_error(&InstallError::Manifest(e)))?;
        if !PLUGIN_STATES.contains(&state) {
            return Err(bad_input("state", "not a plugin state"));
        }
        let mut params = serde_json::json!({ "plugin": id, "state": state });
        // Cut before cleaning, so a huge message costs no more than a long
        // one; cleaning only ever shortens it further.
        if let Some(detail) = detail
            .map(|d| {
                d.chars()
                    .take(16 * MAX_STATE_DETAIL_CHARS)
                    .collect::<String>()
            })
            .map(|d| plugin_host::clean_text(&d, MAX_STATE_DETAIL_CHARS))
            .filter(|d| !d.is_empty())
        {
            params["detail"] = serde_json::Value::String(detail);
        }
        self.pending_host_events
            .push(("plugin_state".to_string(), params));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::io::Write;

    const MANIFEST: &str = r#"{"manifest":1,"id":"com.example.web","name":"Web","version":"1.2.0","api":1,"main":"main.js","hew":">=1.0","permissions":["model.read","network:api.example.com"]}"#;

    fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            w.start_file(*name, options).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn code(e: ApiError) -> String {
        e.to_string().split(':').next().unwrap().to_string()
    }

    #[test]
    fn a_package_answers_its_manifest_and_file_list() {
        let bytes = zip(&[
            ("hew-plugin.json", MANIFEST.as_bytes()),
            ("main.js", b"// main"),
            ("assets/icon.svg", b"<svg/>"),
        ]);
        let v: Value = serde_json::from_str(&plugin_package_from_zip(&bytes).unwrap()).unwrap();
        assert_eq!(v["manifest"]["id"], "com.example.web");
        let files = v["files"].as_array().unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f["path"].as_str().unwrap()).collect();
        assert_eq!(paths, ["assets/icon.svg", "hew-plugin.json", "main.js"]);
        // Each span is the file's bytes in the archive given.
        for (f, expected) in files.iter().zip([
            b"<svg/>".as_slice(),
            MANIFEST.as_bytes(),
            b"// main".as_slice(),
        ]) {
            let offset = f["offset"].as_u64().unwrap() as usize;
            let size = f["size"].as_u64().unwrap() as usize;
            assert_eq!(&bytes[offset..offset + size], expected, "{f}");
        }
        // The normalized manifest parses back to the same manifest.
        let again = plugin_manifest_parse(&v["manifest"].to_string()).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&again).unwrap(),
            v["manifest"]
        );
    }

    #[test]
    fn package_refusals_carry_their_names() {
        assert_eq!(
            code(plugin_package_from_zip(&zip(&[("main.js", b"x")])).unwrap_err()),
            "INSTALL_MANIFEST_MISSING"
        );
        assert_eq!(
            code(
                plugin_package_from_zip(&zip(&[("hew-plugin.json", MANIFEST.as_bytes())]))
                    .unwrap_err()
            ),
            "INSTALL_MANIFEST_FILE_MISSING"
        );
        assert_eq!(
            code(plugin_package_from_zip(&zip(&[("hew-plugin.json", b"{}")])).unwrap_err()),
            "MANIFEST_MANIFEST_MALFORMED"
        );
        assert_eq!(
            code(plugin_package_from_zip(b"nope").unwrap_err()),
            "INSTALL_PACKAGE_UNREADABLE"
        );
        let bad = zip(&[
            ("hew-plugin.json", MANIFEST.as_bytes()),
            ("main.js", b"x"),
            ("CON.js", b"x"),
        ]);
        assert_eq!(
            code(plugin_package_from_zip(&bad).unwrap_err()),
            "INSTALL_PACKAGE_PATH_INVALID"
        );
    }

    #[test]
    fn manifest_parse_normalizes_or_refuses_by_name() {
        let v: Value = serde_json::from_str(&plugin_manifest_parse(MANIFEST).unwrap()).unwrap();
        assert_eq!(v["data"]["version"], 1, "defaults are filled in");
        assert!(v["activation"].is_array());
        let e = plugin_manifest_parse(r#"{"manifest":1}"#).unwrap_err();
        assert_eq!(code(e), "MANIFEST_MANIFEST_MALFORMED");
        let unknown = MANIFEST.replace("model.read", "everything");
        let e = plugin_manifest_parse(&unknown).unwrap_err();
        assert_eq!(code(e), "MANIFEST_UNKNOWN_PERMISSION");
        assert!(e_text(plugin_manifest_parse(&unknown)).contains("everything"));
    }

    fn e_text(r: Result<String, ApiError>) -> String {
        r.unwrap_err().to_string()
    }

    #[test]
    fn consent_answers_the_payload() {
        let source = r#"{"kind":"file","name":"web.hewplugin"}"#;
        let v: Value =
            serde_json::from_str(&plugin_consent(MANIFEST, source, None, None, None).unwrap())
                .unwrap();
        assert_eq!(v["name"], "Web");
        assert_eq!(v["unsigned"], true);
        assert_eq!(
            v["warnings"],
            json!(["Can send what it reads from your model to api.example.com."])
        );
        assert_eq!(
            v["source"],
            json!({ "kind": "file", "name": "web.hewplugin" })
        );
        assert_eq!(v["update"], Value::Null);

        let previous = json!({
            "schema": 1, "id": "com.example.web", "version": "1.0.0",
            "source": { "kind": "file", "name": "web.hewplugin" }, "installed_at_ms": 1,
            "grant": ["model.read"], "enabled": true, "data_version": 1
        })
        .to_string();
        let v: Value = serde_json::from_str(
            &plugin_consent(
                MANIFEST,
                source,
                Some(previous),
                Some(MANIFEST.replace("1.2.0", "1.0.0")),
                None,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(v["update"]["from_version"], "1.0.0");
        assert_eq!(v["update"]["added"][0]["name"], "network:api.example.com");
        assert_eq!(v["replaces"], json!({ "name": "Web", "version": "1.0.0" }));
    }

    #[test]
    fn consent_refuses_bad_input_and_reserved_ids() {
        let source = r#"{"kind":"file","name":"x"}"#;
        assert_eq!(
            code(plugin_consent(MANIFEST, "{}", None, None, None).unwrap_err()),
            "PLUGIN_BAD_INPUT"
        );
        let reserved = MANIFEST.replace("com.example.web", "org.hew3d.web");
        assert_eq!(
            code(plugin_consent(&reserved, source, None, None, None).unwrap_err()),
            "INSTALL_RESERVED_ID"
        );
        let newer = json!({ "schema": 2, "id": "com.example.web" }).to_string();
        assert_eq!(
            code(plugin_consent(MANIFEST, source, Some(newer), None, None).unwrap_err()),
            "INSTALL_INSTALLED_UNREADABLE"
        );
        assert_eq!(
            code(plugin_consent("{", source, None, None, None).unwrap_err()),
            "MANIFEST_MANIFEST_MALFORMED"
        );
    }

    fn row(id: &str, version: &str, requires: &[&str]) -> Value {
        let manifest = json!({
            "manifest": 1, "id": id, "name": "Dep", "version": version, "api": 1,
            "main": "main.js",
            "requires": requires.iter().map(|r| json!({ "id": r, "range": "*" })).collect::<Vec<_>>()
        });
        json!({
            "installed": {
                "schema": 1, "id": id, "version": version,
                "source": { "kind": "file", "name": "x" }, "installed_at_ms": 1,
                "grant": [], "enabled": true, "data_version": 1
            },
            "manifest": manifest
        })
    }

    #[test]
    fn install_check_runs_each_check_in_order() {
        let file = r#"{"kind":"file","name":"x.hewplugin"}"#;
        assert_eq!(
            plugin_install_check(MANIFEST, file, "1.4.0", "[]").map_err(code),
            Ok(())
        );
        // Reserved id first, even when the version would also fail.
        let reserved = MANIFEST
            .replace("com.example.web", "org.hew3d.web")
            .replace(">=1.0", ">=9.0");
        assert_eq!(
            code(plugin_install_check(&reserved, file, "1.4.0", "[]").unwrap_err()),
            "INSTALL_RESERVED_ID"
        );
        // Then the version, even when a dependency is also missing.
        let later = MANIFEST.replace(
            r#""hew":">=1.0""#,
            r#""hew":">=9.0","requires":[{"id":"com.example.base","range":"*"}]"#,
        );
        assert_eq!(
            code(plugin_install_check(&later, file, "1.4.0", "[]").unwrap_err()),
            "INSTALL_HEW_VERSION_UNSUPPORTED"
        );
        // Then requires: missing, then a cycle through what is installed.
        let needs = MANIFEST.replace(
            r#""hew":">=1.0""#,
            r#""hew":">=1.0","requires":[{"id":"com.example.base","range":"^1"}]"#,
        );
        assert_eq!(
            code(plugin_install_check(&needs, file, "1.4.0", "[]").unwrap_err()),
            "INSTALL_DEPENDENCY_MISSING"
        );
        let old_base = json!([row("com.example.base", "0.5.0", &[])]).to_string();
        assert_eq!(
            code(plugin_install_check(&needs, file, "1.4.0", &old_base).unwrap_err()),
            "INSTALL_DEPENDENCY_MISSING"
        );
        let base = json!([row("com.example.base", "1.0.0", &[])]).to_string();
        assert_eq!(
            plugin_install_check(&needs, file, "1.4.0", &base).map_err(code),
            Ok(())
        );
        let looped = json!([row("com.example.base", "1.0.0", &["com.example.web"])]).to_string();
        let e = plugin_install_check(&needs, file, "1.4.0", &looped).unwrap_err();
        assert!(
            e.to_string()
                .contains("com.example.web → com.example.base → com.example.web"),
            "{e}"
        );
        assert_eq!(code(e), "INSTALL_DEPENDENCY_CYCLE");
    }

    #[test]
    fn install_check_refuses_bad_input() {
        let file = r#"{"kind":"file","name":"x"}"#;
        for (source, running, installed) in [
            ("{}", "1.4.0", "[]"),
            (file, "one", "[]"),
            (file, "1.4.0", "{}"),
            (file, "1.4.0", r#"[{"manifest":{}}]"#),
        ] {
            assert_eq!(
                code(plugin_install_check(MANIFEST, source, running, installed).unwrap_err()),
                "PLUGIN_BAD_INPUT",
                "{source} {running} {installed}"
            );
        }
        let newer =
            json!([{ "installed": { "schema": 2, "id": "com.example.base" }, "manifest": {} }])
                .to_string();
        assert_eq!(
            code(plugin_install_check(MANIFEST, file, "1.4.0", &newer).unwrap_err()),
            "INSTALL_INSTALLED_UNREADABLE"
        );
    }

    #[test]
    fn hew_version_checks_the_range() {
        assert!(plugin_hew_version_ok(MANIFEST, "1.4.0").unwrap());
        let later = MANIFEST.replace(">=1.0", ">=9.0");
        assert!(!plugin_hew_version_ok(&later, "1.4.0").unwrap());
        assert_eq!(
            code(plugin_hew_version_ok(MANIFEST, "one").unwrap_err()),
            "PLUGIN_BAD_INPUT"
        );
    }

    fn names(list: &[&str]) -> String {
        serde_json::to_string(list).unwrap()
    }

    #[test]
    fn a_grant_from_names_opens_a_connection_holding_exactly_it() {
        let list = [
            "model.read",
            "view",
            "network:api.example.com",
            "attributes.write:com.example.web",
            "model.read",
        ];
        let json = plugin_grant_json("com.example.web", "doc_3", &names(&list)).unwrap();
        let parsed: Vec<plugin_manifest::PermissionName> = list
            .iter()
            .map(|n| plugin_manifest::PermissionName::parse(n).unwrap())
            .collect();
        let expected = api::Grant::plugin(
            "com.example.web",
            Some("doc_3"),
            api::Grant::permissions_from_names(&parsed),
        );
        assert_eq!(serde_json::from_str::<api::Grant>(&json).unwrap(), expected);
        assert_eq!(expected.permissions.len(), 4, "a repeat counts once");
        assert_eq!(
            expected.ceiling,
            api::Ceiling::App,
            "view is a live permission"
        );

        let mut scene = Scene::new();
        let conn = scene.api_plugin_connection_open(&json).unwrap();
        assert_eq!(scene.api_connections[&conn].grant(), &expected);

        // Without a live permission the ceiling is Core, decided by `api`.
        let json = plugin_grant_json("com.example.web", "doc_1", &names(&["model.read"])).unwrap();
        let grant: api::Grant = serde_json::from_str(&json).unwrap();
        assert_eq!(grant.ceiling, api::Ceiling::Core);
        assert_eq!(grant.plugin.as_deref(), Some("com.example.web"));
        assert_eq!(grant.document.as_deref(), Some("doc_1"));
        let conn = scene.api_plugin_connection_open(&json).unwrap();
        assert_eq!(scene.api_connections[&conn].grant(), &grant);

        // No names: a grant that holds nothing, still a plugin's.
        let json = plugin_grant_json("com.example.web", "doc_1", "[]").unwrap();
        let grant: api::Grant = serde_json::from_str(&json).unwrap();
        assert!(grant.permissions.is_empty());
        scene.api_plugin_connection_open(&json).unwrap();
    }

    #[test]
    fn a_grant_refuses_what_a_plugin_may_not_hold_by_name() {
        let ok = names(&["model.read"]);
        for (list, expected) in [
            (vec!["model.read", "all"], "MANIFEST_UNKNOWN_PERMISSION"),
            (vec!["always"], "MANIFEST_UNKNOWN_PERMISSION"),
            (vec!["first_party"], "MANIFEST_UNKNOWN_PERMISSION"),
            (
                vec!["network:*.example.com"],
                "MANIFEST_PERMISSION_TOO_BROAD",
            ),
            (
                vec!["attributes.write:hew.core"],
                "MANIFEST_UNKNOWN_PERMISSION",
            ),
            (vec!["Model.Read"], "MANIFEST_UNKNOWN_PERMISSION"),
            (vec![""], "MANIFEST_UNKNOWN_PERMISSION"),
        ] {
            let e = plugin_grant_json("com.example.web", "doc_1", &names(&list)).unwrap_err();
            assert_eq!(code(e), expected, "{list:?}");
        }
        let e = plugin_grant_json("com.example.web", "doc_1", &names(&["all"])).unwrap_err();
        assert!(e.to_string().contains("\"all\""), "{e}");
        for (id, expected) in [
            ("../x", "MANIFEST_INVALID_PLUGIN_ID"),
            ("A.b", "MANIFEST_INVALID_PLUGIN_ID"),
            ("", "MANIFEST_INVALID_PLUGIN_ID"),
            ("hew.core", "MANIFEST_RESERVED_PLUGIN_ID"),
        ] {
            assert_eq!(
                code(plugin_grant_json(id, "doc_1", &ok).unwrap_err()),
                expected,
                "{id:?}"
            );
        }
        let long = "d".repeat(MAX_DOCUMENT_ID_BYTES + 1);
        for document in ["", "doc 1", "doc_1\n", "doc/1", long.as_str()] {
            assert_eq!(
                code(plugin_grant_json("com.example.web", document, &ok).unwrap_err()),
                "PLUGIN_BAD_INPUT",
                "{document:?}"
            );
        }
        let many: Vec<String> = (0..=MAX_GRANT_NAMES)
            .map(|i| format!("network:h{i}.example.com"))
            .collect();
        let many = serde_json::to_string(&many).unwrap();
        for bad in ["{}", "[1]", "\"model.read\"", "", many.as_str()] {
            assert_eq!(
                code(plugin_grant_json("com.example.web", "doc_1", bad).unwrap_err()),
                "PLUGIN_BAD_INPUT",
                "{bad}"
            );
        }
    }

    fn reply(scene: &mut Scene, conn: u32, frame: Value) -> Value {
        let text = scene.api_dispatch_reply(conn, &frame.to_string()).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    /// The `plugin_state` frames each connection was sent by one drain.
    fn plugin_states(scene: &mut Scene) -> Vec<(u32, Value)> {
        let out: Value = serde_json::from_str(&scene.drain_events()).unwrap();
        let mut got = Vec::new();
        for batch in out.as_array().unwrap() {
            let Some(conn) = batch["conn"].as_u64() else {
                continue;
            };
            for frame in batch["frames"].as_array().unwrap() {
                let frame: Value = serde_json::from_str(frame.as_str().unwrap()).unwrap();
                if frame["method"] == "hew.event.plugin_state" {
                    got.push((conn as u32, frame["params"].clone()));
                }
            }
        }
        got
    }

    #[test]
    fn a_plugin_state_reaches_every_subscriber_once() {
        let mut scene = Scene::new();
        let first = scene.api_connection_open();
        let grant = plugin_grant_json("com.example.other", "doc_1", "[]").unwrap();
        let plugin = scene.api_plugin_connection_open(&grant).unwrap();
        let quiet = scene.api_connection_open();
        for conn in [first, plugin] {
            let r = reply(
                &mut scene,
                conn,
                json!({"jsonrpc":"2.0","id":1,"method":"hew.meta.hello","params":{"protocol":1}}),
            );
            assert!(r.get("result").is_some(), "{r}");
            let r = reply(
                &mut scene,
                conn,
                json!({"jsonrpc":"2.0","id":2,"method":"hew.event.subscribe","params":{"events":["plugin_state"]}}),
            );
            assert_eq!(r["result"]["subscribed"], json!(["plugin_state"]), "{r}");
        }
        scene.drain_events();

        scene
            .note_plugin_state("com.example.web", "crashed", Some("boom".into()))
            .unwrap();
        let got = plugin_states(&mut scene);
        let expected = json!({ "plugin": "com.example.web", "state": "crashed", "detail": "boom" });
        assert_eq!(got, vec![(first, expected.clone()), (plugin, expected)]);
        assert!(got.iter().all(|(c, _)| *c != quiet));
        assert!(plugin_states(&mut scene).is_empty(), "sent once");

        // No detail, or one that cleans to nothing, is left out.
        scene
            .note_plugin_state("com.example.web", "loaded", None)
            .unwrap();
        scene
            .note_plugin_state(
                "com.example.web",
                "unloaded",
                Some("\u{202e}\u{200b}".into()),
            )
            .unwrap();
        let got = plugin_states(&mut scene);
        assert_eq!(got.len(), 4);
        assert_eq!(
            got[0].1,
            json!({ "plugin": "com.example.web", "state": "loaded" })
        );
        // Each connection's frames in the order they were noted.
        assert_eq!(
            got[1].1,
            json!({ "plugin": "com.example.web", "state": "unloaded" })
        );
        assert_eq!(got[2].1, got[0].1);

        // A plugin's message is cleaned and cut.
        let hostile = format!("\u{202e}evil\n{}", "x".repeat(10_000));
        scene
            .note_plugin_state("com.example.web", "grant_changed", Some(hostile))
            .unwrap();
        let got = plugin_states(&mut scene);
        let detail = got[0].1["detail"].as_str().unwrap();
        assert!(detail.starts_with("evil"), "{detail:?}");
        assert!(!detail.contains('\u{202e}'));
        assert!(detail.chars().count() <= MAX_STATE_DETAIL_CHARS);
        assert!(detail.ends_with('…'));
    }

    #[test]
    fn a_plugin_state_refuses_a_bad_id_or_state_and_queues_nothing() {
        let mut scene = Scene::new();
        let conn = scene.api_connection_open();
        reply(
            &mut scene,
            conn,
            json!({"jsonrpc":"2.0","id":1,"method":"hew.meta.hello","params":{"protocol":1}}),
        );
        reply(
            &mut scene,
            conn,
            json!({"jsonrpc":"2.0","id":2,"method":"hew.event.subscribe","params":{"events":["plugin_state"]}}),
        );
        scene.drain_events();
        for (id, state, expected) in [
            ("../x", "loaded", "MANIFEST_INVALID_PLUGIN_ID"),
            ("hew.x", "loaded", "MANIFEST_RESERVED_PLUGIN_ID"),
            ("com.example.web", "", "PLUGIN_BAD_INPUT"),
            ("com.example.web", "Loaded", "PLUGIN_BAD_INPUT"),
            ("com.example.web", "deleted_everything", "PLUGIN_BAD_INPUT"),
        ] {
            let e = scene.note_plugin_state(id, state, None).unwrap_err();
            assert_eq!(code(e), expected, "{id} {state}");
        }
        assert!(plugin_states(&mut scene).is_empty());
        for state in PLUGIN_STATES {
            scene
                .note_plugin_state("com.example.web", state, None)
                .unwrap();
        }
        assert_eq!(plugin_states(&mut scene).len(), PLUGIN_STATES.len());
    }
}
