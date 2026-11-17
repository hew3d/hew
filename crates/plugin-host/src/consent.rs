//! The consent payload: what the consent sheet, and the desktop's native
//! confirmation, show before a plugin is installed or its grant changes.
//!
//! Hew's own lines (the permission copy, the warnings, the unsigned line)
//! are kept apart from text the plugin wrote (its name and
//! `activationReason`), and every plugin-written string passes through
//! [`clean_text`] before it reaches the payload, so a manifest cannot make
//! its text look like Hew's.

use crate::installed::{InstallSource, Installed};
use plugin_manifest::{Activation, LocalizableString, Manifest, PermissionName};

/// The longest plugin name the payload carries, in characters.
pub const MAX_NAME_CHARS: usize = 64;
/// The longest `activationReason` the payload carries, in characters.
pub const MAX_REASON_CHARS: usize = 200;
/// The longest file name or folder path the payload carries.
const MAX_SOURCE_CHARS: usize = 512;

/// The fixed line for a package no one has signed: every package, until
/// signing exists.
pub const UNSIGNED_LINE: &str = "Unsigned. Hew cannot tell who made this.";

/// One permission, with Hew's English copy for it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PermissionLine {
    /// The permission as the manifest names it (`model.read`,
    /// `network:api.example.com`). The app localizes by it.
    pub name: String,
    /// Hew's English copy, the fallback when the app has none.
    pub text: String,
}

/// The installed plugin an install would replace.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Replaces {
    pub name: String,
    pub version: String,
}

/// What changed since the grant the user accepted for the installed
/// version.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UpdateDiff {
    /// Permissions not in the installed grant.
    pub added: Vec<PermissionLine>,
    /// Optional permissions the installed version did not declare.
    pub new_optional: Vec<PermissionLine>,
    /// `on_startup` is declared now and was not before.
    pub startup_newly: bool,
    pub from_version: String,
}

/// Everything the consent sheet shows.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ConsentPayload {
    pub id: String,
    /// The plugin's display name, cleaned ([`display_name`]).
    pub name: String,
    pub version: String,
    pub permissions: Vec<PermissionLine>,
    pub optional: Vec<PermissionLine>,
    /// Hew's warnings about combinations of permissions.
    pub warnings: Vec<String>,
    /// Whether the plugin asks to start with Hew (`on_startup`).
    pub on_startup: bool,
    /// The plugin's own words about why (`activationReason`), cleaned.
    /// Plugin text: the sheet shows it under "The plugin says", apart from
    /// Hew's lines.
    pub activation_reason: Option<String>,
    pub signer: Option<String>,
    /// True whenever `signer` is none; the sheet then shows
    /// [`UNSIGNED_LINE`].
    pub unsigned: bool,
    /// Where the package came from; file names and paths cleaned like
    /// plugin text.
    pub source: InstallSource,
    /// Present when this install replaces an installed plugin with the
    /// same id; the sheet shows [`replaces_line`].
    pub replaces: Option<Replaces>,
    /// Present on an update: what changed since the accepted grant.
    pub update: Option<UpdateDiff>,
}

/// Characters [`clean_text`] removes, as inclusive code point ranges:
/// format and invisible characters, bidirectional controls, fillers that
/// render as blank, and tag characters.
pub const CLEAN_TEXT_REMOVED: &[(u32, u32)] = &[
    (0x00AD, 0x00AD),
    (0x034F, 0x034F),
    (0x0600, 0x0605),
    (0x061C, 0x061C),
    (0x06DD, 0x06DD),
    (0x070F, 0x070F),
    (0x08E2, 0x08E2),
    (0x115F, 0x1160),
    (0x17B4, 0x17B5),
    (0x180B, 0x180F),
    (0x200B, 0x200F),
    (0x202A, 0x202E),
    (0x2060, 0x206F),
    (0x2800, 0x2800),
    (0x3164, 0x3164),
    (0xFE00, 0xFE0F),
    (0xFEFF, 0xFEFF),
    (0xFFA0, 0xFFA0),
    (0xFFF0, 0xFFFB),
    (0x1D173, 0x1D17A),
    (0xE0000, 0xE0FFF),
];

/// Combining marks, as inclusive ranges; [`clean_text`] keeps at most
/// [`MAX_COMBINING_RUN`] in a row.
pub const CLEAN_TEXT_COMBINING: &[(u32, u32)] = &[
    (0x0300, 0x036F),
    (0x1AB0, 0x1AFF),
    (0x1DC0, 0x1DFF),
    (0x20D0, 0x20FF),
    (0xFE20, 0xFE2F),
];

/// Most combining marks [`clean_text`] keeps in a row.
pub const MAX_COMBINING_RUN: usize = 2;

fn in_ranges(c: char, ranges: &[(u32, u32)]) -> bool {
    let c = c as u32;
    ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&c))
}

/// Cleans text a plugin wrote before Hew shows it, so it cannot pass for
/// Hew's own text or hide part of itself. The rules, in order, per
/// character (a TypeScript mirror follows them exactly; the cases in
/// `tests/fixtures/clean_text.json` are shared):
///
/// 1. A character for which Rust's `char::is_whitespace` is true (the
///    Unicode White_Space property: space, tab, the line breaks, U+0085,
///    U+00A0, U+1680, U+2000 to U+200A, U+2028, U+2029, U+202F, U+205F,
///    U+3000) starts or continues a run of whitespace. A run becomes one
///    U+0020 space, and only between two kept characters: leading and
///    trailing runs vanish.
/// 2. Any other character for which `char::is_control` is true (general
///    category Cc) is removed.
/// 3. A character in [`CLEAN_TEXT_REMOVED`] is removed.
/// 4. A character in [`CLEAN_TEXT_COMBINING`] is kept only while fewer
///    than [`MAX_COMBINING_RUN`] combining marks directly precede it among
///    the kept characters; the rest of the run is dropped. Whitespace ends
///    a run; a removed character does not.
/// 5. Everything else is kept.
///
/// Then, if more than `max_chars` characters (Unicode scalar values)
/// remain, the first `max_chars - 1` are kept, trailing U+0020 spaces are
/// trimmed, and `…` (U+2026) is appended. A `max_chars` of 0 gives "".
pub fn clean_text(text: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(text.len().min(max_chars.saturating_mul(4)));
    let mut space = false;
    let mut combining = 0usize;
    for c in text.chars() {
        if c.is_whitespace() {
            space = true;
            combining = 0;
            continue;
        }
        if c.is_control() || in_ranges(c, CLEAN_TEXT_REMOVED) {
            continue;
        }
        let is_mark = in_ranges(c, CLEAN_TEXT_COMBINING);
        if is_mark && combining >= MAX_COMBINING_RUN {
            continue;
        }
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        combining = if is_mark { combining + 1 } else { 0 };
        out.push(c);
    }
    if max_chars == 0 {
        return String::new();
    }
    if out.chars().count() > max_chars {
        let mut cut: String = out.chars().take(max_chars - 1).collect();
        while cut.ends_with(' ') {
            cut.pop();
        }
        cut.push('…');
        out = cut;
    }
    out
}

/// The English form of a localizable manifest string: the plain text, the
/// `en` text, else the first locale's. A catalog key has no text without
/// the catalog, so it gives `None`.
fn english(text: &LocalizableString) -> Option<&str> {
    match text {
        LocalizableString::Plain(s) => Some(s),
        LocalizableString::ByLocale(map) => map
            .get("en")
            .or_else(|| map.values().next())
            .map(String::as_str),
        LocalizableString::Key { .. } => None,
    }
}

/// The name the consent sheet shows: the manifest's English name, cleaned
/// and capped at [`MAX_NAME_CHARS`]; the plugin id when the name is a
/// catalog key or cleans to nothing.
pub fn display_name(manifest: &Manifest) -> String {
    english(&manifest.name)
        .map(|n| clean_text(n, MAX_NAME_CHARS))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| manifest.id.clone())
}

/// Hew's English copy for one permission.
pub fn permission_line(p: &PermissionName) -> PermissionLine {
    let text = match p.split() {
        ("model.read", None) => {
            "Read the model: every object, component, material, tag, and attribute".to_string()
        }
        ("model.write", None) => {
            "Change the model: draw, move, delete, and edit geometry".to_string()
        }
        ("attributes.write", None) => {
            "Store its own data on parts, materials, and the document".to_string()
        }
        ("attributes.write", Some(ns)) => format!("Store data under the \"{ns}\" namespace"),
        ("view", None) => {
            "Change the selection, hide and show parts, and move the camera".to_string()
        }
        ("documents.save", None) => "Save, export, and import files".to_string(),
        ("documents.discard", None) => "Close or revert documents".to_string(),
        ("documents.other", None) => {
            "Act on every open document, not only the current one".to_string()
        }
        ("files", None) => "Ask you to pick files, and save files you choose".to_string(),
        ("clipboard", None) => "Read and write the clipboard".to_string(),
        ("library", None) => "Add to and change your Library".to_string(),
        ("network", Some(host)) => format!("Connect to {host} (checked by Hew)"),
        // A manifest obtained through `plugin_manifest::parse` holds only
        // the names above; anything else is shown as itself.
        (other, _) => clean_text(other, MAX_REASON_CHARS),
    };
    PermissionLine {
        name: p.as_str().to_string(),
        text,
    }
}

/// The hosts of every `network:` permission, required first, in manifest
/// order.
fn network_hosts<'a>(all: impl Iterator<Item = &'a PermissionName>) -> Vec<&'a str> {
    let mut hosts = Vec::new();
    for p in all {
        if let ("network", Some(host)) = p.split()
            && !hosts.contains(&host)
        {
            hosts.push(host);
        }
    }
    hosts
}

/// Most hosts a warning names before "and N more".
pub const MAX_WARNING_HOSTS: usize = 5;
/// Most permission lines a summary lists before "…and N more".
pub const MAX_SUMMARY_LINES: usize = 12;

/// Hew's warnings about a set of permissions: when `model.read` and any
/// `network:` permission are both in it, "Can send what it reads from
/// your model to <hosts>."; when `files` and any `network:`, "Can send
/// files you choose to <hosts>.". Hosts are listed in order, the first
/// [`MAX_WARNING_HOSTS`] by name and the rest as ", and N more".
pub fn permission_warnings(permissions: &[PermissionName]) -> Vec<String> {
    let hosts = network_hosts(permissions.iter());
    if hosts.is_empty() {
        return Vec::new();
    }
    let mut named = hosts
        .iter()
        .take(MAX_WARNING_HOSTS)
        .copied()
        .collect::<Vec<_>>()
        .join(", ");
    if hosts.len() > MAX_WARNING_HOSTS {
        named.push_str(&format!(", and {} more", hosts.len() - MAX_WARNING_HOSTS));
    }
    let has = |name: &str| permissions.iter().any(|p| p.as_str() == name);
    let mut warnings = Vec::new();
    if has("model.read") {
        warnings.push(format!(
            "Can send what it reads from your model to {named}."
        ));
    }
    if has("files") {
        warnings.push(format!("Can send files you choose to {named}."));
    }
    warnings
}

/// Whether `manifest.hew` admits `running`. No range admits every version.
/// A pre-release build (`1.5.0-pre.2`) is compared as its release
/// (`1.5.0`) too, so a preview build can run a plugin written for the
/// version it previews.
pub fn hew_version_ok(manifest: &Manifest, running: &semver::Version) -> bool {
    let Some(req) = &manifest.hew else {
        return true;
    };
    if req.matches(running) {
        return true;
    }
    if running.pre.is_empty() {
        return false;
    }
    let mut release = running.clone();
    release.pre = semver::Prerelease::EMPTY;
    req.matches(&release)
}

/// The fixed line for a replacement: Hew cannot tell whether the new
/// package comes from whoever made the installed one.
pub fn replaces_line(replaces: &Replaces) -> String {
    format!(
        "Replaces {} v{}. Hew cannot check it is the same author.",
        replaces.name, replaces.version
    )
}

/// Whether the install replaces the same version that is installed: a
/// reinstall, not an update.
pub fn is_reinstall(payload: &ConsentPayload) -> bool {
    payload
        .replaces
        .as_ref()
        .is_some_and(|r| r.version == payload.version)
}

/// The fixed line for a reinstall of the installed version, in place of
/// [`replaces_line`].
pub fn reinstall_line(version: &str) -> String {
    format!("Replaces the installed copy of v{version}.")
}

/// The title and accept label of the confirmation before an install: a
/// fresh install, an update, or a reinstall of the installed version
/// ([`is_reinstall`]). Hew's own words; nothing from the package.
pub fn install_confirmation(payload: &ConsentPayload) -> (&'static str, &'static str) {
    if is_reinstall(payload) {
        ("Install this plugin again?", "Install")
    } else if payload.replaces.is_some() {
        ("Update this plugin?", "Update")
    } else {
        ("Install this plugin?", "Install")
    }
}

fn cleaned_source(source: &InstallSource) -> InstallSource {
    match source {
        InstallSource::File { name } => InstallSource::File {
            name: clean_text(name, MAX_SOURCE_CHARS),
        },
        InstallSource::Dir { path } => InstallSource::Dir {
            path: clean_text(path, MAX_SOURCE_CHARS),
        },
        other => other.clone(),
    }
}

/// The consent payload for installing `manifest` from `source`.
///
/// Warnings are [`permission_warnings`] of `permissions` and
/// `optionalPermissions` together.
///
/// `previous` is the installed record of the same id and
/// `previous_manifest` its stored `hew-plugin.json`. With `previous`,
/// `replaces` and `update` are set: `added` is every permission not in the
/// accepted grant, `new_optional` every optional permission the previous
/// manifest did not declare (all of them when it is not given), and
/// `startup_newly` is `on_startup` declared now and not before.
pub fn consent(
    manifest: &Manifest,
    source: &InstallSource,
    previous: Option<&Installed>,
    previous_manifest: Option<&Manifest>,
    signer: Option<&str>,
) -> ConsentPayload {
    let permissions: Vec<PermissionLine> =
        manifest.permissions.iter().map(permission_line).collect();
    let optional: Vec<PermissionLine> = manifest
        .optional_permissions
        .iter()
        .map(permission_line)
        .collect();
    let declared: Vec<PermissionName> = manifest
        .permissions
        .iter()
        .chain(&manifest.optional_permissions)
        .cloned()
        .collect();
    let warnings = permission_warnings(&declared);
    let on_startup = manifest.activation.contains(&Activation::OnStartup);
    let activation_reason = manifest
        .activation_reason
        .as_ref()
        .map(|r| match english(r) {
            Some(text) => clean_text(text, MAX_REASON_CHARS),
            None => String::new(),
        })
        .filter(|r| !r.is_empty());

    let replaces = previous.map(|p| Replaces {
        name: previous_manifest
            .map(display_name)
            .unwrap_or_else(|| p.id.clone()),
        version: p.version.to_string(),
    });
    let update = previous.map(|p| {
        let was_startup =
            previous_manifest.is_some_and(|m| m.activation.contains(&Activation::OnStartup));
        UpdateDiff {
            added: manifest
                .permissions
                .iter()
                .filter(|n| !p.grant.contains(n))
                .map(permission_line)
                .collect(),
            new_optional: manifest
                .optional_permissions
                .iter()
                .filter(|n| previous_manifest.is_none_or(|m| !m.optional_permissions.contains(n)))
                .map(permission_line)
                .collect(),
            startup_newly: on_startup && !was_startup,
            from_version: p.version.to_string(),
        }
    });

    let signer = signer
        .map(|s| clean_text(s, MAX_NAME_CHARS))
        .filter(|s| !s.is_empty());
    ConsentPayload {
        id: manifest.id.clone(),
        name: display_name(manifest),
        version: manifest.version.to_string(),
        permissions,
        optional,
        warnings,
        on_startup,
        activation_reason,
        unsigned: signer.is_none(),
        signer,
        source: cleaned_source(source),
        replaces,
        update,
    }
}

/// The fixed line for a folder install, whose files are read in place.
pub const FOLDER_LINE: &str = "The files in this folder can change after you install.";

/// Appends `heading` and one bullet per line, at most
/// [`MAX_SUMMARY_LINES`] then "…and N more"; nothing for no lines.
fn push_list(out: &mut String, heading: &str, lines: &[&str]) {
    if lines.is_empty() {
        return;
    }
    out.push_str(&format!("\n{heading}\n"));
    for line in lines.iter().take(MAX_SUMMARY_LINES) {
        out.push_str(&format!("• {line}\n"));
    }
    if lines.len() > MAX_SUMMARY_LINES {
        out.push_str(&format!("…and {} more\n", lines.len() - MAX_SUMMARY_LINES));
    }
}

fn push_warnings(out: &mut String, warnings: &[String]) {
    if warnings.is_empty() {
        return;
    }
    out.push('\n');
    for w in warnings {
        out.push_str(&format!("⚠ {w}\n"));
    }
}

fn texts(lines: &[PermissionLine]) -> Vec<&str> {
    lines.iter().map(|l| l.text.as_str()).collect()
}

/// The payload as plain text, for a confirmation a host builds itself (the
/// desktop's native dialog) from the payload it holds rather than from
/// anything the page sends. It carries what the consent sheet shows, in
/// this order: name and version; the source line; for a folder,
/// [`FOLDER_LINE`]; [`UNSIGNED_LINE`]; the replacement line
/// ([`reinstall_line`] for the installed version again); the warnings;
/// on an update, "New since vX:" with the added and newly optional
/// permissions; "Starts with Hew." (or "Now starts with Hew." when the
/// update adds it); "It can:" with the permissions; "May ask for later:"
/// with the optional ones; and last the plugin's own words under "The
/// plugin says:". Each list shows at most [`MAX_SUMMARY_LINES`] lines.
pub fn consent_summary(payload: &ConsentPayload) -> String {
    let mut out = format!("{} {}\n", payload.name, payload.version);
    match &payload.source {
        InstallSource::File { name } => out.push_str(&format!("From file: {name}\n")),
        InstallSource::Dir { path } => {
            out.push_str(&format!("Unpacked: {path}\n"));
            out.push_str(FOLDER_LINE);
            out.push('\n');
        }
        _ => {}
    }
    if payload.unsigned {
        out.push_str(UNSIGNED_LINE);
        out.push('\n');
    }
    if let Some(r) = &payload.replaces {
        if is_reinstall(payload) {
            out.push_str(&reinstall_line(&payload.version));
        } else {
            out.push_str(&replaces_line(r));
        }
        out.push('\n');
    }
    push_warnings(&mut out, &payload.warnings);
    if let Some(update) = &payload.update {
        let mut new: Vec<&str> = texts(&update.added);
        new.extend(texts(&update.new_optional));
        push_list(
            &mut out,
            &format!("New since v{}:", update.from_version),
            &new,
        );
    }
    if payload.on_startup {
        let newly = payload.update.as_ref().is_some_and(|u| u.startup_newly);
        out.push_str(if newly {
            "\nNow starts with Hew.\n"
        } else {
            "\nStarts with Hew.\n"
        });
    }
    push_list(&mut out, "It can:", &texts(&payload.permissions));
    push_list(&mut out, "May ask for later:", &texts(&payload.optional));
    if let Some(reason) = &payload.activation_reason {
        out.push_str(&format!("\nThe plugin says:\n{reason}\n"));
    }
    out
}

/// What a rollback brings back, as plain text for a confirmation the host
/// builds itself: "NAME vCURRENT goes back to vPREVIOUS.", the warnings of
/// the previous manifest ([`permission_warnings`] of its permissions and
/// optional permissions, as [`consent`] computes them), then "It will be
/// able to:" with one line of Hew's copy per permission the previous
/// record holds (its grant, then its optional grants), at most
/// [`MAX_SUMMARY_LINES`]. The name is the previous manifest's, cleaned.
pub fn rollback_summary(
    current: &Installed,
    previous: &Installed,
    previous_manifest: &Manifest,
) -> String {
    let mut out = format!(
        "{} v{} goes back to v{}.\n",
        display_name(previous_manifest),
        current.version,
        previous.version
    );
    let declared: Vec<PermissionName> = previous_manifest
        .permissions
        .iter()
        .chain(&previous_manifest.optional_permissions)
        .cloned()
        .collect();
    push_warnings(&mut out, &permission_warnings(&declared));
    let lines: Vec<String> = previous
        .grant
        .iter()
        .chain(&previous.optional_granted)
        .map(|p| clean_text(&permission_line(p).text, MAX_REASON_CHARS))
        .collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    push_list(&mut out, "It will be able to:", &lines);
    out
}

/// A grant being widened, as plain text for the host's own confirmation:
/// `name_version` (the host passes the plugin's cleaned name and version),
/// then "It will also be able to:" with a line per permission in `added`
/// (at most [`MAX_SUMMARY_LINES`]), then the warnings the `resulting`
/// grant triggers ([`permission_warnings`]).
pub fn grant_change_summary(
    name_version: &str,
    added: &[PermissionName],
    resulting: &[PermissionName],
) -> String {
    let mut out = format!("{}\n", clean_text(name_version, MAX_REASON_CHARS));
    let lines: Vec<String> = added.iter().map(|p| permission_line(p).text).collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    push_list(&mut out, "It will also be able to:", &lines);
    push_warnings(&mut out, &permission_warnings(resulting));
    out
}

/// The installed plugin as one line: its cleaned name and version.
fn name_version(installed: &Installed, manifest: &Manifest) -> String {
    format!("{} {}", display_name(manifest), installed.version)
}

/// What uninstalling removes, as plain text for the host's own
/// confirmation: the plugin's cleaned name and version, then Hew's line
/// about what goes with it.
pub fn uninstall_summary(installed: &Installed, manifest: &Manifest) -> String {
    format!(
        "{}\nRemoves the plugin, its earlier version, and everything it saved.\n",
        name_version(installed, manifest)
    )
}

/// What turning a plugin on allows, as plain text for the host's own
/// confirmation: the cleaned name and version; [`UNSIGNED_LINE`] when no
/// signer is recorded; the warnings of what it holds; "Starts with Hew."
/// when its manifest asks to; then "It can:" with a line per permission
/// the record holds (its grant, then its optional grants), at most
/// [`MAX_SUMMARY_LINES`].
pub fn enable_summary(installed: &Installed, manifest: &Manifest) -> String {
    let mut out = format!("{}\n", name_version(installed, manifest));
    if installed.signer.is_none() {
        out.push_str(UNSIGNED_LINE);
        out.push('\n');
    }
    let held: Vec<PermissionName> = installed
        .grant
        .iter()
        .chain(&installed.optional_granted)
        .cloned()
        .collect();
    push_warnings(&mut out, &permission_warnings(&held));
    if manifest.activation.contains(&Activation::OnStartup) {
        out.push_str("\nStarts with Hew.\n");
    }
    let lines: Vec<String> = held.iter().map(|p| permission_line(p).text).collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    push_list(&mut out, "It can:", &lines);
    out
}
