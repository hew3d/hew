//! The desktop's plugin commands (docs/agents/PLUGINS.md "Installing"):
//! the Tauri commands Settings ▸ Plugins calls to pick, stage, confirm,
//! list, change and remove installed plugins, and the four a running
//! plugin's host calls to keep the plugin's saved data
//! (`plugins/<id>/storage.json`). The store itself is `plugin_host::store`;
//! this file decides who may call it and with what.
//!
//! These commands are the only thing between a web page and installing
//! other people's code, so every argument from the page is treated as
//! hostile:
//!
//! - Every command first checks the calling window's label (`main`,
//!   `main-<n>` or `settings`), which Tauri supplies, not the page. The
//!   Library window and any future plugin window are refused.
//! - Every plugin id is checked against the plugin id grammar before it is
//!   used for anything, the store folder included.
//! - A package or folder is staged only from a path the user picked in this
//!   run's native picker, recorded here in a set of its own and consumed on
//!   use. `ApprovedPaths` is not consulted: drag-drop and neighbor-folder
//!   approval widen it, and neither is a choice to install.
//! - The page only ever sees `{token, consent}`; the package stays here.
//!   Accepting shows a native confirmation whose text is built from the
//!   consent payload this side holds, so a page cannot install anything the
//!   user was not shown by Hew itself. Widening a grant, rolling back,
//!   turning a plugin on and uninstalling ask the same way, one dialog at a
//!   time ([`super::confirm`]: only the accept button says yes), and re-read
//!   the store after the answer so what is written is what was shown.
//!   Turning a plugin off and narrowing a grant ask nothing.
//! - Saved data is only ever a value in one JSON object: a key is never a
//!   path, and the file is found from the checked id alone.
//! - A picked path is matched by exact string against what the picker
//!   returned; the page's string is never resolved on the filesystem.
//!
//! Each command is a thin wrapper over a plain function that takes the
//! window label, the state, the arguments and a [`Ui`] (the native dialog
//! and the change event), so the rules are tested without a running app.

use plugin_host::store::{self, Listed, Pending, PluginDir, PENDING_TTL_MS};
use plugin_host::{ConsentPayload, InstallError, InstallSource, Installed};
use plugin_manifest::PermissionName;
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use tauri::{Emitter, Manager};

/// The global event every command that changes the store emits.
pub const CHANGED_EVENT: &str = "plugins-changed";
/// The global event a change to a plugin's saved data emits, with
/// [`StorageChanged`], so every window's host can tell that plugin.
pub const STORAGE_CHANGED_EVENT: &str = "plugins-storage-changed";
/// The package file extension, compared without regard to case.
pub const PACKAGE_EXTENSION: &str = "hewplugin";
/// How many staged installs may wait for an answer at once. Each holds a
/// whole package in memory, so a page cannot pile them up.
pub const MAX_PENDING: usize = 4;
/// How many unstaged picks are remembered; the oldest is forgotten first.
pub const MAX_PICKS: usize = 8;

/// What a native picker returned: a package file or an unpacked folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PickKind {
    Package,
    Folder,
}

/// The commands' state, managed by Tauri in a preview build.
pub struct PluginState {
    /// The store's root, or `None` when no data folder could be found.
    dir: Option<PluginDir>,
    /// Paths the user picked in a native picker and has not yet staged,
    /// oldest first.
    picked: Mutex<VecDeque<Pick>>,
    /// Set while a native confirmation is open, so a page cannot stack
    /// dialogs behind the one the user is reading.
    asking: AtomicBool,
    /// Staged installs waiting for the user's answer, by token.
    pending: Mutex<HashMap<String, Pending>>,
    /// Held around every change to the store, so two commands never
    /// rearrange the same folders at once. Never held while a dialog shows.
    store: Mutex<()>,
    /// Why every command is refused, when the build is not one that can
    /// run plugins; `None` when it can.
    disabled: Option<&'static str>,
}

impl PluginState {
    pub fn new(dir: Option<PluginDir>) -> PluginState {
        PluginState {
            dir,
            picked: Mutex::new(VecDeque::new()),
            asking: AtomicBool::new(false),
            pending: Mutex::new(HashMap::new()),
            store: Mutex::new(()),
            disabled: None,
        }
    }

    /// The state for the running app: the store under the app's data
    /// folder, unless the content security policy this shell was started
    /// with does not allow the realm's boot script (`boot_hash`, the
    /// base64 SHA-256 the realm frame is pinned to). A shell built without
    /// the preview policy (the Tauri CLI run directly instead of through
    /// `scripts/tauri-policy.mjs`) would otherwise offer plugins whose
    /// realm can never start; it refuses every command and says why.
    pub fn resolve(csp: Option<&str>, boot_hash: &str) -> PluginState {
        if policy_allows_boot(csp, boot_hash) {
            return PluginState::new(PluginDir::resolve());
        }
        eprintln!(
            "hew: ERROR: the content security policy does not allow the plugin realm's boot script; plugins are disabled. Build a preview shell with `pnpm --dir shells/tauri build` (or dev), not the Tauri CLI directly."
        );
        let mut state = PluginState::new(None);
        state.disabled = Some(
            "This build of Hew was not given the policy plugins need, so plugins are turned off.",
        );
        state
    }

    fn dir(&self) -> Result<&PluginDir, String> {
        if let Some(why) = self.disabled {
            return Err(refusal("plugins_disabled", why));
        }
        self.dir.as_ref().ok_or_else(|| {
            refusal(
                "no_plugin_dir",
                "Hew could not find a folder to keep plugins in.",
            )
        })
    }

    fn store_lock(&self) -> MutexGuard<'_, ()> {
        lock(&self.store)
    }
}

/// Whether `csp`'s `script-src` allows the script whose base64 SHA-256 is
/// `boot_hash`.
pub fn policy_allows_boot(csp: Option<&str>, boot_hash: &str) -> bool {
    let source = format!("'sha256-{boot_hash}'");
    !boot_hash.is_empty()
        && csp.is_some_and(|csp| {
            csp.split(';')
                .map(str::trim)
                .filter(|d| d.starts_with("script-src "))
                .any(|d| d.split_whitespace().any(|s| s == source))
        })
}

/// A lock that a panic elsewhere does not turn into a refusal forever. The
/// data behind these locks stays consistent across a panic: each is
/// changed by a single insert or remove.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What a staged install shows the page: its token and the consent payload.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PendingView {
    pub token: String,
    pub consent: ConsentPayload,
}

/// [`STORAGE_CHANGED_EVENT`]'s payload: the plugin and the keys that
/// changed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StorageChanged {
    pub id: String,
    pub keys: Vec<String>,
}

/// The native side of a command: the confirmation dialog and the change
/// events. The app implements it with [`super::confirm::confirm_native`]
/// and global events; tests stub it.
pub trait Ui {
    /// Shows a native confirmation and blocks until it is answered. True
    /// only when the user activated the button labeled `accept`.
    fn confirm(&self, title: &str, body: &str, accept: &str) -> bool;
    /// Tells every window the store changed.
    fn changed(&self);
    /// Tells every window a plugin's saved data changed.
    fn storage_changed(&self, change: StorageChanged);
}

fn refusal(name: &str, text: &str) -> String {
    format!("{name}: {text}")
}

fn install_error(e: InstallError) -> String {
    format!("{}: {}", e.name(), e.explanation())
}

/// Milliseconds since the Unix epoch, the clock pending installs expire by.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// This build's version, the one a manifest's `hew` range is checked
/// against.
fn running_version() -> &'static plugin_host::semver::Version {
    static RUNNING: OnceLock<plugin_host::semver::Version> = OnceLock::new();
    RUNNING.get_or_init(|| {
        plugin_host::semver::Version::parse(env!("CARGO_PKG_VERSION"))
            .expect("the shell's package version is a semantic version")
    })
}

/// Whether a window may manage plugins: a document window (`main`,
/// `main-<n>`) or Settings. Everything else is refused, the Library window
/// and any window a plugin may one day get included.
pub fn window_allowed(label: &str) -> bool {
    label == "main"
        || label == "settings"
        || label
            .strip_prefix("main-")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

fn check_window(label: &str) -> Result<(), String> {
    if window_allowed(label) {
        Ok(())
    } else {
        Err(refusal(
            "window_not_allowed",
            "Plugins can be managed only from a model window or Settings.",
        ))
    }
}

fn check_id(id: &str) -> Result<(), String> {
    plugin_manifest::validate_plugin_id(id).map_err(|e| install_error(InstallError::Manifest(e)))
}

fn not_picked() -> String {
    refusal(
        "path_not_picked",
        "Choose the plugin again with the Install button.",
    )
}

/// One remembered pick: the canonical path as the pick command returned it
/// to the page, the path itself, and what kind of picker produced it.
#[derive(Debug, Clone)]
pub struct Pick {
    shown: String,
    path: PathBuf,
    kind: PickKind,
}

/// Records a path the user picked in a native picker, canonical, and
/// returns it as the string the page must hand back. A package must be a
/// file and a folder a folder. Past [`MAX_PICKS`] the oldest is forgotten.
pub fn record_pick(state: &PluginState, path: &Path, kind: PickKind) -> Result<String, String> {
    let canonical = std::fs::canonicalize(path).map_err(|_| not_picked())?;
    let right_kind = match kind {
        PickKind::Package => canonical.is_file(),
        PickKind::Folder => canonical.is_dir(),
    };
    if !right_kind {
        return Err(not_picked());
    }
    let shown = canonical.to_string_lossy().into_owned();
    remember_pick(
        &mut lock(&state.picked),
        Pick {
            shown: shown.clone(),
            path: canonical,
            kind,
        },
    );
    Ok(shown)
}

/// Adds a pick, replacing an equal one and forgetting the oldest past the
/// cap.
fn remember_pick(picks: &mut VecDeque<Pick>, pick: Pick) {
    picks.retain(|p| !(p.shown == pick.shown && p.kind == pick.kind));
    picks.push_back(pick);
    while picks.len() > MAX_PICKS {
        picks.pop_front();
    }
}

/// Removes and returns the pick whose string is exactly `path`, of `kind`.
/// Pure: the page's string is only compared, never resolved, so a path
/// that was not picked costs no filesystem work at all.
fn take_from(picks: &mut VecDeque<Pick>, path: &str, kind: PickKind) -> Option<PathBuf> {
    let at = picks
        .iter()
        .position(|p| p.shown == path && p.kind == kind)?;
    picks.remove(at).map(|p| p.path)
}

/// Removes and returns a picked path of the given kind; anything not
/// picked, or picked as the other kind, is `path_not_picked`.
fn take_pick(state: &PluginState, path: &str, kind: PickKind) -> Result<PathBuf, String> {
    take_from(&mut lock(&state.picked), path, kind).ok_or_else(not_picked)
}

/// Marks a native confirmation as open until dropped, a panic included.
struct Asking<'a>(&'a AtomicBool);

impl Drop for Asking<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Claims the one native confirmation, or refuses `confirmation_open`
/// while another is showing.
fn begin_asking(state: &PluginState) -> Result<Asking<'_>, String> {
    state
        .asking
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| Asking(&state.asking))
        .map_err(|_| {
            refusal(
                "confirmation_open",
                "Answer the question Hew is already asking first.",
            )
        })
}

fn not_a_package() -> String {
    refusal(
        "not_a_package",
        "Hew installs plugins from .hewplugin files.",
    )
}

/// Reads a package file of at most the package size cap, measured on the
/// bytes read.
fn read_package_file(path: &Path) -> Result<Vec<u8>, String> {
    let file =
        std::fs::File::open(path).map_err(|e| install_error(InstallError::Io(e.to_string())))?;
    let is_file = file
        .metadata()
        .map_err(|e| install_error(InstallError::Io(e.to_string())))?
        .is_file();
    if !is_file {
        return Err(not_a_package());
    }
    let cap = plugin_host::MAX_PACKAGE_BYTES;
    let mut bytes = Vec::new();
    file.take(cap as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| install_error(InstallError::Io(e.to_string())))?;
    if bytes.len() > cap {
        return Err(install_error(InstallError::PackageTooLarge));
    }
    Ok(bytes)
}

/// Drops every pending install older than the time limit.
fn prune(pending: &mut HashMap<String, Pending>, now_ms: u64) {
    pending.retain(|_, p| now_ms.saturating_sub(p.created_at_ms) <= PENDING_TTL_MS);
}

/// Checks a read package against the store and holds it for an answer. A
/// newer stage of the same plugin replaces any older one still waiting.
fn hold(
    state: &PluginState,
    package: plugin_host::Package,
    source: InstallSource,
    now_ms: u64,
) -> Result<PendingView, String> {
    let dir = state.dir()?;
    let staged = {
        let _store = state.store_lock();
        store::stage(dir, package, source, running_version(), now_ms).map_err(install_error)?
    };
    let mut pending = lock(&state.pending);
    prune(&mut pending, now_ms);
    pending.retain(|_, p| p.consent.id != staged.consent.id);
    if pending.len() >= MAX_PENDING {
        return Err(refusal(
            "too_many_pending",
            "Too many installs are waiting for an answer. Finish or cancel one first.",
        ));
    }
    let view = PendingView {
        token: staged.token.clone(),
        consent: staged.consent.clone(),
    };
    pending.insert(staged.token.clone(), staged);
    Ok(view)
}

/// `plugin_list`: every plugin in the store.
pub fn list(label: &str, state: &PluginState) -> Result<Vec<Listed>, String> {
    check_window(label)?;
    // Under the store lock, so a change in another window is seen whole or
    // not at all, never half way through its renames. No command holds the
    // lock while a dialog is open, so this waits only for a rearrangement.
    let _store = state.store_lock();
    Ok(store::list(state.dir()?))
}

/// `plugin_stage_file`: reads a picked `.hewplugin` and holds it.
pub fn stage_file(
    label: &str,
    state: &PluginState,
    path: &str,
    now_ms: u64,
) -> Result<PendingView, String> {
    check_window(label)?;
    let path = take_pick(state, path, PickKind::Package)?;
    let is_package = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(PACKAGE_EXTENSION));
    if !is_package {
        return Err(not_a_package());
    }
    state.dir()?;
    let bytes = read_package_file(&path)?;
    let package = plugin_host::read_package_zip(&bytes).map_err(install_error)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    hold(state, package, InstallSource::File { name }, now_ms)
}

/// `plugin_stage_dir`: reads a picked unpacked plugin folder and holds it.
pub fn stage_dir(
    label: &str,
    state: &PluginState,
    path: &str,
    now_ms: u64,
) -> Result<PendingView, String> {
    check_window(label)?;
    let path = take_pick(state, path, PickKind::Folder)?;
    state.dir()?;
    let package = store::read_package_dir(&path).map_err(install_error)?;
    let source = InstallSource::Dir {
        path: path.to_string_lossy().into_owned(),
    };
    hold(state, package, source, now_ms)
}

/// `plugin_commit`: `accept: false` drops the pending install. `accept:
/// true` takes it out of the waiting set, asks the user natively with the
/// consent this side holds, and installs only on yes; a no returns `None`.
/// Either way the token cannot be used again.
pub fn commit(
    label: &str,
    state: &PluginState,
    token: &str,
    accept: bool,
    ui: &dyn Ui,
    now_ms: u64,
) -> Result<Option<Installed>, String> {
    check_window(label)?;
    // Claimed before the pending install is taken, so a refusal here leaves
    // it waiting.
    let asking = if accept {
        Some(begin_asking(state)?)
    } else {
        None
    };
    let taken = {
        let mut pending = lock(&state.pending);
        let taken = pending.remove(token);
        prune(&mut pending, now_ms);
        taken
    };
    let Some(taken) = taken else {
        return Err(install_error(InstallError::PendingUnknown));
    };
    if !accept {
        return Ok(None);
    }
    if now_ms.saturating_sub(taken.created_at_ms) > PENDING_TTL_MS {
        return Err(install_error(InstallError::PendingExpired));
    }
    let dir = state.dir()?;
    let (title, accept_label) = plugin_host::install_confirmation(&taken.consent);
    let body = plugin_host::consent_summary(&taken.consent);
    let yes = ui.confirm(title, &body, accept_label);
    drop(asking);
    if !yes {
        return Ok(None);
    }
    // The store refuses `pending_stale` if the installed record changed
    // since staging; that error returns here before anything is emitted.
    let installed = {
        let _store = state.store_lock();
        store::commit(dir, taken, true, now_ms).map_err(install_error)?
    };
    ui.changed();
    Ok(installed)
}

/// Parses permission names from the page, dropping repeats.
fn permission_names(names: &[String]) -> Result<Vec<PermissionName>, String> {
    let mut out: Vec<PermissionName> = Vec::with_capacity(names.len());
    for name in names {
        let p =
            PermissionName::parse(name).map_err(|e| install_error(InstallError::Manifest(e)))?;
        if !out.contains(&p) {
            out.push(p);
        }
    }
    Ok(out)
}

/// `plugin_set_grant`: replaces a plugin's grant. Anything the manifest does
/// not declare is refused. When the new grant adds anything the stored
/// record does not hold, the user is asked natively, listing what is added
/// and the warnings the resulting grant brings; a no leaves the record as
/// it was and returns it. The record is read again after the answer and
/// the change refused `grant_changed` if it moved meanwhile. Narrowing asks
/// nothing and is decided and written under one hold of the store lock.
pub fn set_grant(
    label: &str,
    state: &PluginState,
    id: &str,
    grant: &[String],
    optional: &[String],
    ui: &dyn Ui,
) -> Result<Installed, String> {
    check_window(label)?;
    check_id(id)?;
    let dir = state.dir()?;
    let grant = permission_names(grant)?;
    let optional = permission_names(optional)?;
    let (current, manifest, added) = {
        let _store = state.store_lock();
        let (current, manifest) = store::installed(dir, id).map_err(install_error)?;
        let added = grant_added(&current, &manifest, &grant, &optional)?;
        if added.is_empty() {
            let installed = store::set_grant(dir, id, grant, optional).map_err(install_error)?;
            drop(_store);
            ui.changed();
            return Ok(installed);
        }
        (current, manifest, added)
    };
    let name_version = format!(
        "{} {}",
        plugin_host::display_name(&manifest),
        current.version
    );
    let resulting: Vec<PermissionName> = grant.iter().chain(&optional).cloned().collect();
    let body = plugin_host::grant_change_summary(&name_version, &added, &resulting);
    let yes = {
        let _asking = begin_asking(state)?;
        ui.confirm("Allow more for this plugin?", &body, "Allow")
    };
    if !yes {
        return Ok(current);
    }
    let installed = {
        let _store = state.store_lock();
        let (now_current, now_manifest) = store::installed(dir, id).map_err(install_error)?;
        if now_current != current || now_manifest != manifest {
            return Err(refusal(
                "grant_changed",
                "The plugin's permissions changed while Hew was asking. Try again.",
            ));
        }
        store::set_grant(dir, id, grant, optional).map_err(install_error)?
    };
    ui.changed();
    Ok(installed)
}

/// What `grant` and `optional` add to the stored record, after refusing
/// anything the manifest does not declare (`unknown_permission`).
fn grant_added(
    current: &Installed,
    manifest: &plugin_manifest::Manifest,
    grant: &[PermissionName],
    optional: &[PermissionName],
) -> Result<Vec<PermissionName>, String> {
    let undeclared = grant
        .iter()
        .find(|p| !manifest.permissions.contains(p))
        .or_else(|| {
            optional
                .iter()
                .find(|p| !manifest.optional_permissions.contains(p))
        });
    if let Some(p) = undeclared {
        return Err(install_error(InstallError::Manifest(
            plugin_manifest::ManifestError::UnknownPermission(p.as_str().to_string()),
        )));
    }
    Ok(grant
        .iter()
        .filter(|p| !current.grant.contains(p))
        .chain(
            optional
                .iter()
                .filter(|p| !current.optional_granted.contains(p)),
        )
        .cloned()
        .collect())
}

/// `plugin_set_enabled`: turns a plugin on or off; returns the plugins that
/// were turned off with it.
///
/// Turning one off asks nothing. Turning one on lets it run, and start
/// with Hew if it asks to, so the user is asked natively with what it
/// holds; a no is `enable_declined` and changes nothing. A plugin already
/// on and allowed to start is left as it is without asking. The record is
/// read again after the answer and the change refused `plugin_changed` if
/// it moved meanwhile.
pub fn set_enabled(
    label: &str,
    state: &PluginState,
    id: &str,
    enabled: bool,
    ui: &dyn Ui,
) -> Result<Vec<String>, String> {
    check_window(label)?;
    check_id(id)?;
    let dir = state.dir()?;
    if !enabled {
        let (_, also) = {
            let _store = state.store_lock();
            store::set_enabled(dir, id, false).map_err(install_error)?
        };
        ui.changed();
        return Ok(also);
    }
    let (current, manifest) = {
        let _store = state.store_lock();
        store::installed(dir, id).map_err(install_error)?
    };
    if current.enabled && current.startup_allowed {
        return Ok(Vec::new());
    }
    let body = plugin_host::enable_summary(&current, &manifest);
    let yes = {
        let _asking = begin_asking(state)?;
        ui.confirm("Turn on this plugin?", &body, "Turn On")
    };
    if !yes {
        return Err(refusal("enable_declined", "The plugin was left off."));
    }
    let also = {
        let _store = state.store_lock();
        let now = store::installed(dir, id).map_err(install_error)?;
        if now != (current, manifest) {
            return Err(plugin_changed());
        }
        store::set_enabled(dir, id, true).map_err(install_error)?.1
    };
    ui.changed();
    Ok(also)
}

fn plugin_changed() -> String {
    refusal(
        "plugin_changed",
        "The plugin changed while Hew was asking. Try again.",
    )
}

/// `plugin_uninstall`: removes a plugin and everything the store holds
/// for it, its saved data included, after asking natively. A no is
/// `uninstall_declined` and removes nothing. Refusals the store would make
/// (not installed, still required by enabled plugins) come before the
/// question. A plugin whose record cannot be read is named by its id.
pub fn uninstall(label: &str, state: &PluginState, id: &str, ui: &dyn Ui) -> Result<(), String> {
    check_window(label)?;
    check_id(id)?;
    let dir = state.dir()?;
    let shown = {
        let _store = state.store_lock();
        let shown = match store::installed(dir, id) {
            Ok(found) => Some(found),
            Err(InstallError::NotInstalled(_)) => {
                return Err(install_error(InstallError::NotInstalled(id.to_string())));
            }
            // Unreadable records can still be removed.
            Err(_) => None,
        };
        let dependents = store::dependents_of(dir, id);
        if !dependents.is_empty() {
            return Err(install_error(InstallError::DependencyMissing(dependents)));
        }
        shown
    };
    let body = match &shown {
        Some((installed, manifest)) => plugin_host::uninstall_summary(installed, manifest),
        // The id passed the plugin id grammar, so it is plain text.
        None => format!(
            "{id}\nHew cannot read this plugin's record. Removes the plugin and everything it saved.\n"
        ),
    };
    let yes = {
        let _asking = begin_asking(state)?;
        ui.confirm("Uninstall this plugin?", &body, "Uninstall")
    };
    if !yes {
        return Err(refusal(
            "uninstall_declined",
            "The plugin was left installed.",
        ));
    }
    {
        let _store = state.store_lock();
        store::uninstall(dir, id).map_err(install_error)?;
    }
    ui.changed();
    Ok(())
}

/// `plugin_rollback`: goes back to the version installed before. The
/// previous version's record carries the grant it had, which may be wider
/// than the one the user has since narrowed to, so the user is asked
/// natively with what it will be able to do. A no leaves everything as it
/// is. The records are read again after the answer, and the rollback is
/// refused `plugin_changed` if either changed while the dialog was open, so
/// what runs is what was shown.
pub fn rollback(
    label: &str,
    state: &PluginState,
    id: &str,
    ui: &dyn Ui,
) -> Result<Installed, String> {
    check_window(label)?;
    check_id(id)?;
    let dir = state.dir()?;
    let shown = {
        let _store = state.store_lock();
        let (current, _) = store::installed(dir, id).map_err(install_error)?;
        let previous = store::previous(dir, id).map_err(install_error)?;
        (current, previous)
    };
    let (current, (previous, previous_manifest)) = &shown;
    let body = plugin_host::rollback_summary(current, previous, previous_manifest);
    let yes = {
        let _asking = begin_asking(state)?;
        ui.confirm("Roll back this plugin?", &body, "Roll back")
    };
    if !yes {
        return Err(rollback_declined());
    }
    let installed = {
        let _store = state.store_lock();
        let (now_current, _) = store::installed(dir, id).map_err(install_error)?;
        let now_previous = store::previous(dir, id).map_err(install_error)?;
        if &now_current != current || now_previous != (previous.clone(), previous_manifest.clone())
        {
            return Err(plugin_changed());
        }
        store::rollback(dir, id).map_err(install_error)?
    };
    ui.changed();
    Ok(installed)
}

fn rollback_declined() -> String {
    refusal("rollback_declined", "The plugin was left as it is.")
}

/// `plugin_read_file`: one file of an installed plugin, through the
/// store's jail (no path outside the plugin's files, no links).
pub fn read_file(label: &str, state: &PluginState, id: &str, rel: &str) -> Result<Vec<u8>, String> {
    check_window(label)?;
    check_id(id)?;
    let dir = state.dir()?;
    let _store = state.store_lock();
    store::read_file(dir, id, rel).map_err(install_error)
}

/// `plugin_storage_get`: the value plugin `id` saved under `key`, if any.
pub fn storage_get(
    label: &str,
    state: &PluginState,
    id: &str,
    key: &str,
) -> Result<Option<String>, String> {
    check_window(label)?;
    check_id(id)?;
    let dir = state.dir()?;
    let _store = state.store_lock();
    store::storage_get(dir, id, key).map_err(install_error)
}

/// `plugin_storage_list`: every key plugin `id` has saved, sorted.
pub fn storage_list(label: &str, state: &PluginState, id: &str) -> Result<Vec<String>, String> {
    check_window(label)?;
    check_id(id)?;
    let dir = state.dir()?;
    let _store = state.store_lock();
    store::storage_list(dir, id).map_err(install_error)
}

/// `plugin_storage_set`: saves `value` under `key` for plugin `id`, then
/// tells every window. Refused `storage_quota` past the plugin's quota and
/// `storage_key_invalid` for an empty or overlong key, with nothing
/// written.
pub fn storage_set(
    label: &str,
    state: &PluginState,
    id: &str,
    key: &str,
    value: &str,
    ui: &dyn Ui,
) -> Result<(), String> {
    check_window(label)?;
    check_id(id)?;
    let dir = state.dir()?;
    {
        let _store = state.store_lock();
        store::storage_set(dir, id, key, value).map_err(install_error)?;
    }
    ui.storage_changed(StorageChanged {
        id: id.to_string(),
        keys: vec![key.to_string()],
    });
    Ok(())
}

/// `plugin_storage_delete`: removes `key` from plugin `id`'s saved data;
/// tells every window only when it was there.
pub fn storage_delete(
    label: &str,
    state: &PluginState,
    id: &str,
    key: &str,
    ui: &dyn Ui,
) -> Result<(), String> {
    check_window(label)?;
    check_id(id)?;
    let dir = state.dir()?;
    let removed = {
        let _store = state.store_lock();
        store::storage_delete(dir, id, key).map_err(install_error)?
    };
    if removed {
        ui.storage_changed(StorageChanged {
            id: id.to_string(),
            keys: vec![key.to_string()],
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The Tauri commands. Each runs its plain function off the main thread
// (packages are read whole, and the confirmations block until answered).
// ---------------------------------------------------------------------------

/// The app's [`Ui`]: Hew's native confirmation over the calling window,
/// and global events.
struct WindowUi {
    window: tauri::WebviewWindow,
}

impl Ui for WindowUi {
    fn confirm(&self, title: &str, body: &str, accept: &str) -> bool {
        super::confirm::confirm_native(&self.window, title, body, accept)
    }

    fn changed(&self) {
        let _ = self.window.app_handle().emit(CHANGED_EVENT, ());
    }

    fn storage_changed(&self, change: StorageChanged) {
        let _ = self.window.app_handle().emit(STORAGE_CHANGED_EVENT, change);
    }
}

/// Runs `f` on a blocking thread with the window's label and state.
async fn off_main<T, F>(window: tauri::WebviewWindow, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&str, &PluginState, &WindowUi) -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let state = window.state::<PluginState>();
        let label = window.label().to_string();
        let ui = WindowUi {
            window: window.clone(),
        };
        f(&label, &state, &ui)
    })
    .await
    .map_err(|e| install_error(InstallError::Io(e.to_string())))?
}

#[tauri::command]
pub async fn plugin_list(window: tauri::WebviewWindow) -> Result<Vec<Listed>, String> {
    off_main(window, |label, state, _| list(label, state)).await
}

/// Shows the native open panel filtered to `.hewplugin` and records the
/// pick for `plugin_stage_file`. `None` when canceled.
#[tauri::command]
pub async fn plugin_pick_package(window: tauri::WebviewWindow) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    check_window(window.label())?;
    let dialog = window
        .dialog()
        .file()
        .set_title("Install a Plugin")
        .add_filter("Hew plugin", &[PACKAGE_EXTENSION]);
    pick(window, PickKind::Package, move || {
        dialog.blocking_pick_file()
    })
    .await
}

/// Shows the native folder picker and records the pick for
/// `plugin_stage_dir`. `None` when canceled.
#[tauri::command]
pub async fn plugin_pick_dir(window: tauri::WebviewWindow) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    check_window(window.label())?;
    let dialog = window
        .dialog()
        .file()
        .set_title("Install an Unpacked Plugin");
    pick(window, PickKind::Folder, move || {
        dialog.blocking_pick_folder()
    })
    .await
}

async fn pick<F>(
    window: tauri::WebviewWindow,
    kind: PickKind,
    show: F,
) -> Result<Option<String>, String>
where
    F: FnOnce() -> Option<tauri_plugin_dialog::FilePath> + Send + 'static,
{
    let picked = tauri::async_runtime::spawn_blocking(show)
        .await
        .map_err(|e| install_error(InstallError::Io(e.to_string())))?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|_| not_picked())?;
    record_pick(&window.state::<PluginState>(), &path, kind).map(Some)
}

#[tauri::command]
pub async fn plugin_stage_file(
    window: tauri::WebviewWindow,
    path: String,
) -> Result<PendingView, String> {
    let now = now_ms();
    off_main(window, move |label, state, _| {
        stage_file(label, state, &path, now)
    })
    .await
}

#[tauri::command]
pub async fn plugin_stage_dir(
    window: tauri::WebviewWindow,
    path: String,
) -> Result<PendingView, String> {
    let now = now_ms();
    off_main(window, move |label, state, _| {
        stage_dir(label, state, &path, now)
    })
    .await
}

#[tauri::command]
pub async fn plugin_commit(
    window: tauri::WebviewWindow,
    token: String,
    accept: bool,
) -> Result<Option<Installed>, String> {
    let now = now_ms();
    off_main(window, move |label, state, ui| {
        commit(label, state, &token, accept, ui, now)
    })
    .await
}

#[tauri::command]
pub async fn plugin_set_enabled(
    window: tauri::WebviewWindow,
    id: String,
    enabled: bool,
) -> Result<Vec<String>, String> {
    off_main(window, move |label, state, ui| {
        set_enabled(label, state, &id, enabled, ui)
    })
    .await
}

#[tauri::command]
pub async fn plugin_set_grant(
    window: tauri::WebviewWindow,
    id: String,
    grant: Vec<String>,
    optional: Vec<String>,
) -> Result<Installed, String> {
    off_main(window, move |label, state, ui| {
        set_grant(label, state, &id, &grant, &optional, ui)
    })
    .await
}

#[tauri::command]
pub async fn plugin_uninstall(window: tauri::WebviewWindow, id: String) -> Result<(), String> {
    off_main(window, move |label, state, ui| {
        uninstall(label, state, &id, ui)
    })
    .await
}

#[tauri::command]
pub async fn plugin_rollback(
    window: tauri::WebviewWindow,
    id: String,
) -> Result<Installed, String> {
    off_main(window, move |label, state, ui| {
        rollback(label, state, &id, ui)
    })
    .await
}

#[tauri::command]
pub async fn plugin_read_file(
    window: tauri::WebviewWindow,
    id: String,
    rel: String,
) -> Result<tauri::ipc::Response, String> {
    off_main(window, move |label, state, _| {
        read_file(label, state, &id, &rel)
    })
    .await
    .map(tauri::ipc::Response::new)
}

#[tauri::command]
pub async fn plugin_storage_get(
    window: tauri::WebviewWindow,
    id: String,
    key: String,
) -> Result<Option<String>, String> {
    off_main(window, move |label, state, _| {
        storage_get(label, state, &id, &key)
    })
    .await
}

#[tauri::command]
pub async fn plugin_storage_set(
    window: tauri::WebviewWindow,
    id: String,
    key: String,
    value: String,
) -> Result<(), String> {
    off_main(window, move |label, state, ui| {
        storage_set(label, state, &id, &key, &value, ui)
    })
    .await
}

#[tauri::command]
pub async fn plugin_storage_delete(
    window: tauri::WebviewWindow,
    id: String,
    key: String,
) -> Result<(), String> {
    off_main(window, move |label, state, ui| {
        storage_delete(label, state, &id, &key, ui)
    })
    .await
}

#[tauri::command]
pub async fn plugin_storage_list(
    window: tauri::WebviewWindow,
    id: String,
) -> Result<Vec<String>, String> {
    off_main(window, move |label, state, _| {
        storage_list(label, state, &id)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[test]
    fn the_boot_script_needs_its_hash_in_script_src() {
        let with = "default-src 'self'; script-src 'self' 'sha256-abc='; style-src 'self'";
        assert!(policy_allows_boot(Some(with), "abc="));
        assert!(!policy_allows_boot(Some(with), "abd="));
        assert!(!policy_allows_boot(Some("script-src 'self'"), "abc="));
        // A hash in another directive allows no script.
        assert!(!policy_allows_boot(Some("style-src 'sha256-abc='"), "abc="));
        assert!(!policy_allows_boot(None, "abc="));
        assert!(!policy_allows_boot(Some(with), ""));
    }

    #[test]
    fn a_shell_without_the_preview_policy_refuses_every_command() {
        let state = PluginState::resolve(Some("script-src 'self'"), "abc=");
        let e = state.dir().expect_err("refused");
        assert_eq!(refusal_name(&e), "plugins_disabled");
    }

    /// A [`Ui`] with a fixed answer that records what it was asked.
    struct Stub {
        answer: bool,
        asked: RefCell<Vec<(String, String, String)>>,
        changed: Cell<usize>,
        stored: RefCell<Vec<StorageChanged>>,
    }

    impl Stub {
        fn answering(answer: bool) -> Stub {
            Stub {
                answer,
                asked: RefCell::new(Vec::new()),
                changed: Cell::new(0),
                stored: RefCell::new(Vec::new()),
            }
        }
        fn asked(&self) -> usize {
            self.asked.borrow().len()
        }
    }

    impl Ui for Stub {
        fn confirm(&self, title: &str, body: &str, accept: &str) -> bool {
            self.asked
                .borrow_mut()
                .push((title.to_string(), body.to_string(), accept.to_string()));
            self.answer
        }
        fn changed(&self) {
            self.changed.set(self.changed.get() + 1);
        }
        fn storage_changed(&self, change: StorageChanged) {
            self.stored.borrow_mut().push(change);
        }
    }

    /// A folder removed when the test ends.
    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Temp {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "hew-shell-plugins-{}-{name}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("temp folder");
            Temp(path)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const NOW: u64 = 1_000_000;

    fn manifest(id: &str, version: &str, permissions: &[&str], optional: &[&str]) -> String {
        serde_json::json!({
            "manifest": 1,
            "id": id,
            "name": "Test Plugin",
            "version": version,
            "api": 1,
            "main": "main.js",
            "permissions": permissions,
            "optionalPermissions": optional,
        })
        .to_string()
    }

    /// An unpacked plugin folder under `root`.
    fn folder(root: &Path, name: &str, manifest: &str) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("hew-plugin.json"), manifest).unwrap();
        std::fs::write(dir.join("main.js"), "// nothing\n").unwrap();
        dir
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &b in bytes {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    /// A stored (uncompressed) zip of `files`, written by hand so the tests
    /// need no zip writer.
    fn stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in files {
            let offset = out.len() as u32;
            let crc = crc32(data);
            let size = data.len() as u32;
            let name_len = name.len() as u16;
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // flags
            out.extend_from_slice(&0u16.to_le_bytes()); // stored
            out.extend_from_slice(&0u16.to_le_bytes()); // time
            out.extend_from_slice(&0x21u16.to_le_bytes()); // 1980-01-01
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&name_len.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(data);

            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes()); // made by (no unix mode)
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0x21u16.to_le_bytes());
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&size.to_le_bytes());
            central.extend_from_slice(&size.to_le_bytes());
            central.extend_from_slice(&name_len.to_le_bytes());
            central.extend_from_slice(&[0; 12]); // extra, comment, disk, attributes
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let central_offset = out.len() as u32;
        let central_size = central.len() as u32;
        let count = files.len() as u16;
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&central_size.to_le_bytes());
        out.extend_from_slice(&central_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    /// A `.hewplugin` file under `root`.
    fn package(root: &Path, file: &str, manifest: &str) -> PathBuf {
        let path = root.join(file);
        let bytes = stored_zip(&[
            ("hew-plugin.json", manifest.as_bytes()),
            ("main.js", b"// nothing\n"),
            ("icon.svg", b"<svg/>"),
        ]);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// A temp folder holding a store and sources, and the state over it.
    fn setup(name: &str) -> (Temp, PluginState) {
        let temp = Temp::new(name);
        let state = PluginState::new(Some(PluginDir::at(temp.0.join("store"))));
        (temp, state)
    }

    fn refusal_name(e: &str) -> &str {
        e.split(':').next().unwrap_or(e)
    }

    /// Picks and stages an unpacked folder from the main window.
    fn stage_folder(state: &PluginState, path: &Path, now: u64) -> Result<PendingView, String> {
        let picked = record_pick(state, path, PickKind::Folder)?;
        stage_dir("main", state, &picked, now)
    }

    /// Installs `id` at `version` from a folder, answering yes.
    fn install(
        state: &PluginState,
        root: &Path,
        id: &str,
        version: &str,
        perms: &[&str],
        opt: &[&str],
    ) -> Installed {
        let src = folder(
            root,
            &format!("{id}-{version}"),
            &manifest(id, version, perms, opt),
        );
        let view = stage_folder(state, &src, NOW).expect("stage");
        commit(
            "main",
            state,
            &view.token,
            true,
            &Stub::answering(true),
            NOW,
        )
        .expect("commit")
        .expect("installed")
    }

    fn installed_ids(state: &PluginState) -> Vec<String> {
        list("main", state)
            .unwrap()
            .into_iter()
            .map(|l| match l {
                Listed::Installed { installed, .. } => installed.id,
                Listed::Unreadable { id, .. } => id,
            })
            .collect()
    }

    #[test]
    fn only_document_windows_and_settings_may_call() {
        for ok in ["main", "main-2", "main-17", "settings"] {
            assert!(window_allowed(ok), "{ok}");
        }
        for refused in [
            "library",
            "plugin-x",
            "",
            "main-",
            "main-x",
            "mainx",
            "Main",
            "settings-2",
        ] {
            assert!(!window_allowed(refused), "{refused:?}");
        }
    }

    #[test]
    fn every_command_refuses_another_window_first() {
        let (temp, state) = setup("label");
        let ui = Stub::answering(true);
        let src = folder(
            &temp.0,
            "src",
            &manifest("com.example.a", "1.0.0", &[], &[]),
        );
        let pkg = package(
            &temp.0,
            "a.hewplugin",
            &manifest("com.example.a", "1.0.0", &[], &[]),
        );
        let file = record_pick(&state, &pkg, PickKind::Package).unwrap();
        let dir_pick = record_pick(&state, &src, PickKind::Folder).unwrap();
        let id = "com.example.a";
        for label in ["library", "plugin-x", ""] {
            let results: Vec<Result<(), String>> = vec![
                list(label, &state).map(drop),
                stage_file(label, &state, &file, NOW).map(drop),
                stage_dir(label, &state, &dir_pick, NOW).map(drop),
                commit(label, &state, "t", true, &ui, NOW).map(drop),
                set_enabled(label, &state, id, true, &ui).map(drop),
                set_grant(label, &state, id, &[], &[], &ui).map(drop),
                uninstall(label, &state, id, &ui),
                rollback(label, &state, id, &ui).map(drop),
                read_file(label, &state, id, "main.js").map(drop),
                storage_get(label, &state, id, "k").map(drop),
                storage_set(label, &state, id, "k", "v", &ui),
                storage_delete(label, &state, id, "k", &ui),
                storage_list(label, &state, id).map(drop),
            ];
            for r in results {
                assert_eq!(refusal_name(&r.unwrap_err()), "window_not_allowed");
            }
        }
        assert_eq!(ui.asked(), 0);
        assert_eq!(ui.changed.get(), 0);
        assert!(ui.stored.borrow().is_empty());
        // A refused window does not use up the user's picks.
        stage_file("settings", &state, &file, NOW).expect("the pick is still there");
        stage_dir("main-2", &state, &dir_pick, NOW).expect("the pick is still there");
        assert!(!temp.0.join("store").join(id).exists());
    }

    #[test]
    fn a_bad_id_is_refused_before_anything_else() {
        // No store folder at all: an id check that came after the folder
        // lookup would answer no_plugin_dir instead.
        let state = PluginState::new(None);
        let ui = Stub::answering(true);
        for bad in [
            "../x", "a", "A.b", "hew.x", "", "a.b/../c", "a..b", "a.b\\c",
        ] {
            let results: Vec<Result<(), String>> = vec![
                set_enabled("main", &state, bad, true, &ui).map(drop),
                set_grant("main", &state, bad, &["model.read".into()], &[], &ui).map(drop),
                uninstall("main", &state, bad, &ui),
                rollback("main", &state, bad, &ui).map(drop),
                read_file("main", &state, bad, "main.js").map(drop),
                storage_get("main", &state, bad, "k").map(drop),
                storage_set("main", &state, bad, "k", "v", &ui),
                storage_delete("main", &state, bad, "k", &ui),
                storage_list("main", &state, bad).map(drop),
            ];
            for r in results {
                let e = r.unwrap_err();
                assert!(
                    matches!(refusal_name(&e), "invalid_plugin_id" | "reserved_plugin_id"),
                    "{bad:?}: {e}"
                );
            }
        }
        assert_eq!(ui.asked(), 0);
        assert_eq!(ui.changed.get(), 0);

        // And with a real store, nothing is created or read for a bad id.
        let (temp, state) = setup("bad-id");
        for bad in ["../x", "a", "A.b", "hew.x"] {
            assert!(uninstall("main", &state, bad, &ui).is_err());
            assert!(set_enabled("main", &state, bad, false, &ui).is_err());
            assert!(rollback("main", &state, bad, &ui).is_err());
            assert!(storage_set("main", &state, bad, "k", "v", &ui).is_err());
        }
        assert!(!temp.0.join("store").exists());
    }

    #[test]
    fn staging_an_unpicked_path_is_refused() {
        let (temp, state) = setup("unpicked");
        let src = folder(
            &temp.0,
            "src",
            &manifest("com.example.a", "1.0.0", &[], &[]),
        );
        let pkg = package(
            &temp.0,
            "a.hewplugin",
            &manifest("com.example.a", "1.0.0", &[], &[]),
        );
        let src_s = src.to_string_lossy().into_owned();
        let pkg_s = pkg.to_string_lossy().into_owned();
        for r in [
            stage_file("main", &state, &pkg_s, NOW),
            stage_dir("main", &state, &src_s, NOW),
            stage_file("main", &state, "/nonexistent/a.hewplugin", NOW),
        ] {
            assert_eq!(refusal_name(&r.unwrap_err()), "path_not_picked");
        }
        // A pick of one kind does not stage as the other.
        record_pick(&state, &pkg, PickKind::Package).unwrap();
        record_pick(&state, &src, PickKind::Folder).unwrap();
        assert_eq!(
            refusal_name(&stage_dir("main", &state, &pkg_s, NOW).unwrap_err()),
            "path_not_picked"
        );
        assert_eq!(
            refusal_name(&stage_file("main", &state, &src_s, NOW).unwrap_err()),
            "path_not_picked"
        );
        // A folder is not recorded as a package pick, nor a file as a folder.
        assert!(record_pick(&state, &src, PickKind::Package).is_err());
        assert!(record_pick(&state, &pkg, PickKind::Folder).is_err());
        assert!(installed_ids(&state).is_empty());
    }

    #[test]
    fn a_pick_is_used_once() {
        let (temp, state) = setup("once");
        let pkg = package(
            &temp.0,
            "a.hewplugin",
            &manifest("com.example.a", "1.0.0", &[], &[]),
        );
        let picked = record_pick(&state, &pkg, PickKind::Package).unwrap();
        let view = stage_file("main", &state, &picked, NOW).expect("first stage");
        assert_eq!(view.consent.id, "com.example.a");
        assert_eq!(
            refusal_name(&stage_file("main", &state, &picked, NOW).unwrap_err()),
            "path_not_picked"
        );

        // Used up even when the package is refused.
        let junk = temp.0.join("junk.hewplugin");
        std::fs::write(&junk, b"not a zip").unwrap();
        let picked = record_pick(&state, &junk, PickKind::Package).unwrap();
        assert_eq!(
            refusal_name(&stage_file("main", &state, &picked, NOW).unwrap_err()),
            "package_unreadable"
        );
        assert_eq!(
            refusal_name(&stage_file("main", &state, &picked, NOW).unwrap_err()),
            "path_not_picked"
        );

        let src = folder(
            &temp.0,
            "src",
            &manifest("com.example.b", "1.0.0", &[], &[]),
        );
        let picked = record_pick(&state, &src, PickKind::Folder).unwrap();
        stage_dir("main", &state, &picked, NOW).expect("first stage");
        assert_eq!(
            refusal_name(&stage_dir("main", &state, &picked, NOW).unwrap_err()),
            "path_not_picked"
        );
    }

    #[test]
    fn a_picked_file_must_be_a_hewplugin() {
        let (temp, state) = setup("ext");
        let text = manifest("com.example.a", "1.0.0", &[], &[]);
        for name in ["a.zip", "a", "a.hewplugin.zip", "a.hewplug"] {
            let path = package(&temp.0, name, &text);
            let picked = record_pick(&state, &path, PickKind::Package).unwrap();
            assert_eq!(
                refusal_name(&stage_file("main", &state, &picked, NOW).unwrap_err()),
                "not_a_package",
                "{name}"
            );
            // Refused and used up.
            assert_eq!(
                refusal_name(&stage_file("main", &state, &picked, NOW).unwrap_err()),
                "path_not_picked"
            );
        }
        let upper = package(&temp.0, "B.HEWPLUGIN", &text);
        let picked = record_pick(&state, &upper, PickKind::Package).unwrap();
        stage_file("main", &state, &picked, NOW).expect("the extension is matched without case");
    }

    #[test]
    fn a_package_over_the_cap_is_refused_on_the_bytes_read() {
        let (temp, state) = setup("big");
        let path = temp.0.join("big.hewplugin");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(plugin_host::MAX_PACKAGE_BYTES as u64 + 1)
            .unwrap();
        drop(file);
        let picked = record_pick(&state, &path, PickKind::Package).unwrap();
        assert_eq!(
            refusal_name(&stage_file("main", &state, &picked, NOW).unwrap_err()),
            "package_too_large"
        );
    }

    #[test]
    fn a_no_in_the_native_dialog_installs_nothing() {
        let (temp, state) = setup("no");
        let pkg = package(
            &temp.0,
            "a.hewplugin",
            &manifest("com.example.a", "1.0.0", &["model.read"], &[]),
        );
        let picked = record_pick(&state, &pkg, PickKind::Package).unwrap();
        let view = stage_file("main", &state, &picked, NOW).unwrap();
        let ui = Stub::answering(false);
        assert_eq!(
            commit("main", &state, &view.token, true, &ui, NOW),
            Ok(None)
        );
        assert_eq!(ui.asked(), 1);
        {
            let asked = ui.asked.borrow();
            let (title, body, accept) = &asked[0];
            assert_eq!(title, "Install this plugin?");
            assert_eq!(accept, "Install");
            // The body is what this side holds, not anything from the page.
            assert_eq!(body, &plugin_host::consent_summary(&view.consent));
            assert!(body.contains(plugin_host::UNSIGNED_LINE));
            assert!(body.contains("Read the model"));
        }
        assert_eq!(ui.changed.get(), 0);
        assert!(installed_ids(&state).is_empty());
        assert!(!temp.0.join("store").join("com.example.a").exists());
        // The pending install is gone.
        assert_eq!(
            refusal_name(
                &commit(
                    "main",
                    &state,
                    &view.token,
                    true,
                    &Stub::answering(true),
                    NOW
                )
                .unwrap_err()
            ),
            "pending_unknown"
        );
    }

    #[test]
    fn a_yes_installs_and_tells_every_window() {
        let (temp, state) = setup("yes");
        let pkg = package(
            &temp.0,
            "a.hewplugin",
            &manifest("com.example.a", "1.0.0", &["model.read"], &[]),
        );
        let picked = record_pick(&state, &pkg, PickKind::Package).unwrap();
        let view = stage_file("main", &state, &picked, NOW).unwrap();
        assert_eq!(
            view.consent.source,
            InstallSource::File {
                name: "a.hewplugin".into()
            }
        );
        let ui = Stub::answering(true);
        let installed = commit("main", &state, &view.token, true, &ui, NOW)
            .unwrap()
            .expect("installed");
        assert_eq!(installed.id, "com.example.a");
        assert_eq!(ui.asked(), 1);
        assert_eq!(ui.changed.get(), 1);
        assert_eq!(installed_ids(&state), ["com.example.a"]);
        assert_eq!(
            read_file("main", &state, "com.example.a", "main.js").unwrap(),
            b"// nothing\n"
        );
        // The token is spent.
        assert_eq!(
            refusal_name(&commit("main", &state, &view.token, true, &ui, NOW).unwrap_err()),
            "pending_unknown"
        );
        assert_eq!(ui.changed.get(), 1);
    }

    #[test]
    fn accept_false_drops_the_pending_without_asking() {
        let (temp, state) = setup("decline");
        let src = folder(
            &temp.0,
            "src",
            &manifest("com.example.a", "1.0.0", &[], &[]),
        );
        let view = stage_folder(&state, &src, NOW).unwrap();
        let ui = Stub::answering(true);
        assert_eq!(
            commit("main", &state, &view.token, false, &ui, NOW),
            Ok(None)
        );
        assert_eq!(ui.asked(), 0);
        assert_eq!(ui.changed.get(), 0);
        assert_eq!(
            refusal_name(&commit("main", &state, &view.token, true, &ui, NOW).unwrap_err()),
            "pending_unknown"
        );
        assert!(installed_ids(&state).is_empty());
    }

    #[test]
    fn an_unknown_token_is_refused_without_asking() {
        let (_temp, state) = setup("unknown");
        let ui = Stub::answering(true);
        for token in ["", "0123456789abcdef0123456789abcdef", "../x"] {
            assert_eq!(
                refusal_name(&commit("main", &state, token, true, &ui, NOW).unwrap_err()),
                "pending_unknown"
            );
        }
        assert_eq!(ui.asked(), 0);
    }

    #[test]
    fn an_expired_pending_is_refused() {
        let (temp, state) = setup("expired");
        let src = folder(
            &temp.0,
            "src",
            &manifest("com.example.a", "1.0.0", &[], &[]),
        );
        let view = stage_folder(&state, &src, NOW).unwrap();
        let ui = Stub::answering(true);
        let late = NOW + PENDING_TTL_MS + 1;
        assert_eq!(
            refusal_name(&commit("main", &state, &view.token, true, &ui, late).unwrap_err()),
            "pending_expired"
        );
        assert_eq!(ui.asked(), 0);
        assert!(installed_ids(&state).is_empty());
        // Expired installs are dropped whenever the waiting set is touched.
        let src_b = folder(&temp.0, "b", &manifest("com.example.b", "1.0.0", &[], &[]));
        let b = stage_folder(&state, &src_b, NOW).unwrap();
        let src_c = folder(&temp.0, "c", &manifest("com.example.c", "1.0.0", &[], &[]));
        stage_folder(&state, &src_c, late).unwrap();
        assert_eq!(lock(&state.pending).len(), 1);
        assert_eq!(
            refusal_name(&commit("main", &state, &b.token, true, &ui, late).unwrap_err()),
            "pending_unknown"
        );
    }

    #[test]
    fn only_a_few_installs_may_wait_at_once() {
        let (temp, state) = setup("cap");
        let mut tokens = Vec::new();
        for i in 0..MAX_PENDING {
            let id = format!("com.example.p{i}");
            let src = folder(&temp.0, &id, &manifest(&id, "1.0.0", &[], &[]));
            tokens.push(stage_folder(&state, &src, NOW).unwrap().token);
        }
        let extra = folder(
            &temp.0,
            "extra",
            &manifest("com.example.extra", "1.0.0", &[], &[]),
        );
        assert_eq!(
            refusal_name(&stage_folder(&state, &extra, NOW).unwrap_err()),
            "too_many_pending"
        );
        // Staging a plugin that is already waiting replaces it.
        let again = folder(
            &temp.0,
            "again",
            &manifest("com.example.p0", "1.0.1", &[], &[]),
        );
        let replaced = stage_folder(&state, &again, NOW).unwrap();
        assert_eq!(lock(&state.pending).len(), MAX_PENDING);
        assert_eq!(
            refusal_name(
                &commit(
                    "main",
                    &state,
                    &tokens[0],
                    false,
                    &Stub::answering(true),
                    NOW
                )
                .unwrap_err()
            ),
            "pending_unknown"
        );
        // Answering one makes room.
        commit(
            "main",
            &state,
            &replaced.token,
            false,
            &Stub::answering(true),
            NOW,
        )
        .unwrap();
        stage_folder(&state, &extra, NOW).expect("room again");
        // So does time.
        let late = NOW + PENDING_TTL_MS + 1;
        let later = folder(
            &temp.0,
            "later",
            &manifest("com.example.later", "1.0.0", &[], &[]),
        );
        stage_folder(&state, &later, late).expect("the old ones expired");
        assert_eq!(lock(&state.pending).len(), 1);
    }

    #[test]
    fn widening_a_grant_asks_and_narrowing_does_not() {
        let (temp, state) = setup("grant");
        let id = "com.example.g";
        let installed = install(
            &state,
            &temp.0,
            id,
            "1.0.0",
            &["model.read", "view"],
            &["documents.save"],
        );
        assert_eq!(installed.grant.len(), 2);

        // Narrowing: no dialog.
        let ui = Stub::answering(false);
        let narrowed = set_grant("main", &state, id, &["model.read".into()], &[], &ui).unwrap();
        assert_eq!(narrowed.grant.len(), 1);
        assert_eq!(ui.asked(), 0);
        assert_eq!(ui.changed.get(), 1);

        // Widening with a no: asked, nothing changes, nothing emitted.
        let no = Stub::answering(false);
        let wide: Vec<String> = vec!["model.read".into(), "view".into()];
        let kept = set_grant("main", &state, id, &wide, &[], &no).unwrap();
        assert_eq!(kept.grant.len(), 1);
        assert_eq!(no.asked(), 1);
        assert_eq!(no.changed.get(), 0);
        {
            let asked = no.asked.borrow();
            let (title, body, accept) = &asked[0];
            assert_eq!(title, "Allow more for this plugin?");
            assert_eq!(accept, "Allow");
            let view_line =
                plugin_host::permission_line(&PermissionName::parse("view").unwrap()).text;
            assert!(body.contains(&view_line), "{body}");
            let read_line =
                plugin_host::permission_line(&PermissionName::parse("model.read").unwrap()).text;
            assert!(!body.contains(&read_line), "only the added lines: {body}");
        }
        let (on_disk, _) = store::installed(state.dir().unwrap(), id).unwrap();
        assert_eq!(on_disk.grant.len(), 1);

        // Widening with a yes.
        let yes = Stub::answering(true);
        let widened =
            set_grant("main", &state, id, &wide, &["documents.save".into()], &yes).unwrap();
        assert_eq!(widened.grant.len(), 2);
        assert_eq!(widened.optional_granted.len(), 1);
        assert_eq!(yes.asked(), 1);
        assert_eq!(yes.changed.get(), 1);

        // The same grant again adds nothing: no dialog.
        let same = Stub::answering(false);
        set_grant("main", &state, id, &wide, &["documents.save".into()], &same).unwrap();
        assert_eq!(same.asked(), 0);

        // Anything the manifest does not declare is refused before asking.
        let ask = Stub::answering(true);
        for (grant, optional) in [
            (vec!["documents.save".to_string()], vec![]),
            (vec![], vec!["view".to_string()]),
            (vec!["network:evil.example".to_string()], vec![]),
            (vec!["not-a-permission".to_string()], vec![]),
        ] {
            let e = set_grant("main", &state, id, &grant, &optional, &ask).unwrap_err();
            assert_eq!(
                refusal_name(&e),
                "unknown_permission",
                "{grant:?} {optional:?}"
            );
        }
        assert_eq!(ask.asked(), 0);
        assert_eq!(ask.changed.get(), 0);
    }

    #[test]
    fn changes_to_the_store_tell_every_window() {
        let (temp, state) = setup("emit");
        let id = "com.example.e";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);

        // An update asks as an update.
        let src = folder(&temp.0, "v2", &manifest(id, "2.0.0", &[], &[]));
        let view = stage_folder(&state, &src, NOW).unwrap();
        let ui = Stub::answering(true);
        commit("main", &state, &view.token, true, &ui, NOW).unwrap();
        assert_eq!(ui.asked.borrow()[0].0, "Update this plugin?");
        assert_eq!(ui.asked.borrow()[0].2, "Update");

        let ui = Stub::answering(true);
        assert_eq!(
            set_enabled("settings", &state, id, false, &ui).unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(ui.changed.get(), 1);
        let back = rollback("settings", &state, id, &ui).unwrap();
        assert_eq!(back.version.to_string(), "1.0.0");
        assert_eq!(ui.changed.get(), 2);
        uninstall("settings", &state, id, &ui).unwrap();
        assert_eq!(ui.changed.get(), 3);
        // The rollback and the uninstall asked; turning it off did not.
        assert_eq!(ui.asked(), 2);
        assert!(installed_ids(&state).is_empty());

        // A refused change tells no one.
        assert!(uninstall("settings", &state, id, &ui).is_err());
        assert!(rollback("settings", &state, id, &ui).is_err());
        assert!(set_enabled("settings", &state, id, true, &ui).is_err());
        assert_eq!(ui.changed.get(), 3);
        assert_eq!(ui.asked(), 2, "refused before asking");
    }

    /// Installs `id` 1.0.0 with `model.read` and `view`, updates it to
    /// 2.0.0, then narrows 2.0.0's grant to `model.read`.
    fn two_versions(state: &PluginState, root: &Path, id: &str) {
        let perms = ["model.read", "view"];
        install(state, root, id, "1.0.0", &perms, &[]);
        install(state, root, id, "2.0.0", &perms, &[]);
        set_grant(
            "main",
            state,
            id,
            &["model.read".into()],
            &[],
            &Stub::answering(true),
        )
        .unwrap();
    }

    #[test]
    fn a_rollback_asks_first_and_a_yes_rolls_back() {
        let (temp, state) = setup("rollback-yes");
        let id = "com.example.r";
        two_versions(&state, &temp.0, id);
        let ui = Stub::answering(true);
        let back = rollback("main", &state, id, &ui).unwrap();
        assert_eq!(back.version.to_string(), "1.0.0");
        assert_eq!(ui.asked(), 1);
        assert_eq!(ui.changed.get(), 1);
        let asked = ui.asked.borrow();
        let (title, body, accept) = &asked[0];
        assert_eq!(title, "Roll back this plugin?");
        assert_eq!(accept, "Roll back");
        // The wider grant the old version brings back is shown.
        let view_line = plugin_host::permission_line(&PermissionName::parse("view").unwrap()).text;
        assert!(body.contains(&view_line), "{body}");
        assert!(body.contains("2.0.0") && body.contains("1.0.0"), "{body}");
    }

    #[test]
    fn a_no_to_a_rollback_changes_nothing() {
        let (temp, state) = setup("rollback-no");
        let id = "com.example.r";
        two_versions(&state, &temp.0, id);
        let dir = state.dir().unwrap();
        let before = (
            store::installed(dir, id).unwrap(),
            store::previous(dir, id).unwrap(),
        );
        let ui = Stub::answering(false);
        assert_eq!(
            rollback("main", &state, id, &ui).unwrap_err(),
            "rollback_declined: The plugin was left as it is."
        );
        assert_eq!(ui.asked(), 1);
        assert_eq!(ui.changed.get(), 0);
        let after = (
            store::installed(dir, id).unwrap(),
            store::previous(dir, id).unwrap(),
        );
        assert_eq!(before, after);
        assert_eq!(after.0 .0.version.to_string(), "2.0.0");
    }

    #[test]
    fn a_rollback_without_a_previous_version_is_refused_before_asking() {
        let (temp, state) = setup("rollback-none");
        let id = "com.example.r";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let ui = Stub::answering(true);
        let e = rollback("main", &state, id, &ui).unwrap_err();
        assert_eq!(refusal_name(&e), "no_previous_version");
        let e = rollback("main", &state, "com.example.missing", &ui).unwrap_err();
        assert_eq!(refusal_name(&e), "not_installed");
        assert_eq!(ui.asked(), 0);
        assert_eq!(ui.changed.get(), 0);
    }

    /// A [`Ui`] that changes the store while its dialog is open, then says
    /// yes.
    struct Meddler<'a> {
        state: &'a PluginState,
        id: &'a str,
    }

    impl Ui for Meddler<'_> {
        fn confirm(&self, _: &str, _: &str, _: &str) -> bool {
            // Another window rolls back while this dialog is up.
            store::rollback(self.state.dir().unwrap(), self.id).unwrap();
            true
        }
        fn changed(&self) {
            panic!("a refused rollback must not emit");
        }
        fn storage_changed(&self, _: StorageChanged) {
            panic!("a rollback changes no saved data");
        }
    }

    #[test]
    fn a_rollback_is_refused_if_the_versions_change_while_asking() {
        let (temp, state) = setup("rollback-race");
        let id = "com.example.r";
        two_versions(&state, &temp.0, id);
        let ui = Meddler { state: &state, id };
        assert_eq!(
            refusal_name(&rollback("main", &state, id, &ui).unwrap_err()),
            "plugin_changed"
        );
        // Only the meddler's own rollback happened.
        let (current, _) = store::installed(state.dir().unwrap(), id).unwrap();
        assert_eq!(current.version.to_string(), "1.0.0");
    }

    #[test]
    fn the_pick_lookup_only_compares_strings() {
        // Pure over strings: none of these exists, and none is resolved.
        let mut picks = VecDeque::new();
        remember_pick(
            &mut picks,
            Pick {
                shown: "/picked/a.hewplugin".into(),
                path: PathBuf::from("/picked/a.hewplugin"),
                kind: PickKind::Package,
            },
        );
        for path in [
            "/nonexistent/a.hewplugin",
            "/picked/./a.hewplugin",
            "/picked//a.hewplugin",
            "/picked/a.hewplugin/",
            "/PICKED/a.hewplugin",
            "",
        ] {
            assert_eq!(
                take_from(&mut picks, path, PickKind::Package),
                None,
                "{path:?}"
            );
        }
        assert_eq!(
            take_from(&mut picks, "/picked/a.hewplugin", PickKind::Folder),
            None
        );
        assert_eq!(picks.len(), 1);
        assert_eq!(
            take_from(&mut picks, "/picked/a.hewplugin", PickKind::Package),
            Some(PathBuf::from("/picked/a.hewplugin"))
        );
        assert!(picks.is_empty());
    }

    #[test]
    fn only_the_exact_picked_string_stages() {
        let (temp, state) = setup("exact");
        let pkg = package(
            &temp.0,
            "a.hewplugin",
            &manifest("com.example.a", "1.0.0", &[], &[]),
        );
        let picked = record_pick(&state, &pkg, PickKind::Package).unwrap();
        let link = temp.0.join("link.hewplugin");
        std::os::unix::fs::symlink(&pkg, &link).unwrap();
        for path in [
            link.to_string_lossy().into_owned(),
            "/nonexistent/a.hewplugin".into(),
        ] {
            assert_eq!(
                refusal_name(&stage_file("main", &state, &path, NOW).unwrap_err()),
                "path_not_picked",
                "{path}"
            );
        }
        stage_file("main", &state, &picked, NOW).expect("the pick was not used up");
    }

    #[test]
    fn only_a_few_picks_are_remembered() {
        let (temp, state) = setup("picks");
        let text = manifest("com.example.a", "1.0.0", &[], &[]);
        let picks: Vec<String> = (0..=MAX_PICKS)
            .map(|i| {
                let path = package(&temp.0, &format!("p{i}.hewplugin"), &text);
                record_pick(&state, &path, PickKind::Package).unwrap()
            })
            .collect();
        assert_eq!(lock(&state.picked).len(), MAX_PICKS);
        assert_eq!(
            refusal_name(&stage_file("main", &state, &picks[0], NOW).unwrap_err()),
            "path_not_picked"
        );
        stage_file("main", &state, &picks[MAX_PICKS], NOW).expect("the newest is kept");
    }

    /// A [`Ui`] that runs `during` while its dialog is open, then answers
    /// `answer`, and counts change events.
    struct During<'a> {
        during: RefCell<Option<Box<dyn FnOnce() + 'a>>>,
        answer: bool,
        changed: Cell<usize>,
    }

    impl<'a> During<'a> {
        fn new(answer: bool, during: impl FnOnce() + 'a) -> During<'a> {
            During {
                during: RefCell::new(Some(Box::new(during))),
                answer,
                changed: Cell::new(0),
            }
        }
    }

    impl Ui for During<'_> {
        fn confirm(&self, _: &str, _: &str, _: &str) -> bool {
            if let Some(f) = self.during.borrow_mut().take() {
                f();
            }
            self.answer
        }
        fn changed(&self) {
            self.changed.set(self.changed.get() + 1);
        }
        fn storage_changed(&self, _: StorageChanged) {}
    }

    #[test]
    fn one_native_confirmation_at_a_time() {
        let (temp, state) = setup("one-dialog");
        let g = "com.example.g";
        install(&state, &temp.0, g, "1.0.0", &["model.read", "view"], &[]);
        set_grant(
            "main",
            &state,
            g,
            &["model.read".into()],
            &[],
            &Stub::answering(true),
        )
        .unwrap();
        let r = "com.example.r";
        two_versions(&state, &temp.0, r);
        let off = "com.example.off";
        install(&state, &temp.0, off, "1.0.0", &[], &[]);
        set_enabled("main", &state, off, false, &Stub::answering(true)).unwrap();
        let gone = "com.example.gone";
        install(&state, &temp.0, gone, "1.0.0", &[], &[]);
        let src = folder(
            &temp.0,
            "waiting",
            &manifest("com.example.w", "1.0.0", &[], &[]),
        );
        let waiting = stage_folder(&state, &src, NOW).unwrap();
        let other = folder(
            &temp.0,
            "first",
            &manifest("com.example.f", "1.0.0", &[], &[]),
        );
        let first = stage_folder(&state, &other, NOW).unwrap();

        let inner: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let ui = During::new(false, || {
            let yes = Stub::answering(true);
            let wide = ["model.read".to_string(), "view".to_string()];
            let results = [
                commit("main", &state, &waiting.token, true, &yes, NOW).map(drop),
                set_grant("main", &state, g, &wide, &[], &yes).map(drop),
                rollback("main", &state, r, &yes).map(drop),
                set_enabled("main", &state, off, true, &yes).map(drop),
                uninstall("main", &state, gone, &yes),
            ];
            // Turning off asks nothing, so it is not held up.
            set_enabled("main", &state, gone, false, &yes).expect("no question");
            for result in results {
                inner.borrow_mut().push(result.unwrap_err());
            }
            assert_eq!(yes.asked(), 0);
            assert_eq!(yes.changed.get(), 1, "only the turn-off");
        });
        assert_eq!(
            commit("main", &state, &first.token, true, &ui, NOW),
            Ok(None)
        );
        drop(ui);
        let inner = inner.into_inner();
        assert_eq!(inner.len(), 5);
        for e in &inner {
            assert_eq!(
                e,
                "confirmation_open: Answer the question Hew is already asking first."
            );
        }
        // The install refused for the open dialog is still waiting, and the
        // flag is released once the first dialog is answered.
        let yes = Stub::answering(true);
        commit("main", &state, &waiting.token, true, &yes, NOW)
            .unwrap()
            .expect("installed");
        assert_eq!(yes.asked(), 1);
        uninstall("main", &state, gone, &yes).expect("asked and removed");
        set_enabled("main", &state, off, true, &yes).expect("asked and on");
        assert_eq!(yes.asked(), 3);
    }

    /// A [`Ui`] whose dialog panics.
    struct Panics;

    impl Ui for Panics {
        fn confirm(&self, _: &str, _: &str, _: &str) -> bool {
            panic!("the dialog failed");
        }
        fn changed(&self) {}
        fn storage_changed(&self, _: StorageChanged) {}
    }

    #[test]
    fn a_panic_in_the_dialog_releases_it() {
        let (temp, state) = setup("panic");
        let r = "com.example.r";
        two_versions(&state, &temp.0, r);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rollback("main", &state, r, &Panics)
        }));
        assert!(panicked.is_err());
        let yes = Stub::answering(true);
        rollback("main", &state, r, &yes).expect("no dialog is held open");
        assert_eq!(yes.asked(), 1);
    }

    #[test]
    fn a_grant_that_changes_while_asking_is_refused() {
        let (temp, state) = setup("grant-race");
        let id = "com.example.g";
        install(&state, &temp.0, id, "1.0.0", &["model.read", "view"], &[]);
        set_grant(
            "main",
            &state,
            id,
            &["model.read".into()],
            &[],
            &Stub::answering(true),
        )
        .unwrap();
        let ui = During::new(true, || {
            // Another window turns it off while the dialog is up.
            store::set_enabled(state.dir().unwrap(), id, false).unwrap();
        });
        let wide = ["model.read".to_string(), "view".to_string()];
        assert_eq!(
            set_grant("main", &state, id, &wide, &[], &ui).unwrap_err(),
            "grant_changed: The plugin's permissions changed while Hew was asking. Try again."
        );
        assert_eq!(ui.changed.get(), 0);
        let (record, _) = store::installed(state.dir().unwrap(), id).unwrap();
        assert_eq!(record.grant.len(), 1);
    }

    #[test]
    fn a_stale_pending_is_refused_after_the_yes() {
        let (temp, state) = setup("stale");
        let id = "com.example.s";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let src = folder(&temp.0, "v2", &manifest(id, "2.0.0", &[], &[]));
        let view = stage_folder(&state, &src, NOW).unwrap();
        set_enabled("main", &state, id, false, &Stub::answering(true)).unwrap();
        let ui = Stub::answering(true);
        let e = commit("main", &state, &view.token, true, &ui, NOW).unwrap_err();
        assert_eq!(refusal_name(&e), "pending_stale");
        assert_eq!(ui.asked(), 1);
        assert_eq!(ui.changed.get(), 0);
        let (record, _) = store::installed(state.dir().unwrap(), id).unwrap();
        assert_eq!(record.version.to_string(), "1.0.0");
    }

    #[test]
    fn reading_a_file_cannot_leave_the_plugin() {
        let (temp, state) = setup("jail");
        let pkg = package(
            &temp.0,
            "a.hewplugin",
            &manifest("com.example.a", "1.0.0", &[], &[]),
        );
        let picked = record_pick(&state, &pkg, PickKind::Package).unwrap();
        let view = stage_file("main", &state, &picked, NOW).unwrap();
        commit(
            "main",
            &state,
            &view.token,
            true,
            &Stub::answering(true),
            NOW,
        )
        .unwrap();
        // A folder install reads from its folder, under the same rules.
        install(&state, &temp.0, "com.example.d", "1.0.0", &[], &[]);
        let outside = temp.0.join("store").join("secret.txt");
        std::fs::write(&outside, b"secret").unwrap();

        for id in ["com.example.a", "com.example.d"] {
            assert_eq!(
                read_file("main", &state, id, "main.js").unwrap(),
                b"// nothing\n"
            );
            let abs = outside.to_string_lossy().into_owned();
            for rel in [
                "../installed.json",
                "../hew-plugin.json",
                "../../secret.txt",
                "files/../../secret.txt",
                abs.as_str(),
                "/etc/passwd",
                "..\\installed.json",
                "a\\..\\..\\installed.json",
                "C:\\Windows\\win.ini",
                "./main.js",
                "",
                "missing.js",
            ] {
                let e = read_file("main", &state, id, rel).unwrap_err();
                assert_eq!(
                    refusal_name(&e),
                    "package_path_invalid",
                    "{id} {rel:?}: {e}"
                );
            }
        }
        // Another plugin's id does not reach this one's files.
        assert!(read_file("main", &state, "com.example.zz", "main.js").is_err());
    }

    #[test]
    fn listing_needs_an_allowed_window_and_a_folder() {
        let (_temp, state) = setup("dir");
        assert!(list("main", &state).unwrap().is_empty());
        let none = PluginState::new(None);
        assert_eq!(
            refusal_name(&list("main", &none).unwrap_err()),
            "no_plugin_dir"
        );
        let ui = Stub::answering(true);
        for r in [
            storage_list("main", &none, "com.example.a").map(drop),
            storage_set("main", &none, "com.example.a", "k", "v", &ui),
        ] {
            assert_eq!(refusal_name(&r.unwrap_err()), "no_plugin_dir");
        }
    }

    #[test]
    fn the_store_folder_is_not_a_command() {
        // The page has no use for where the store is, so no command says.
        let source = include_str!("plugins.rs");
        let gone = ["plugin", "_dir("].concat();
        assert!(!source.contains(&format!("fn {gone}")));
    }

    #[test]
    fn the_running_version_is_this_builds() {
        assert_eq!(running_version().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn reinstalling_the_installed_version_asks_as_a_reinstall() {
        let (temp, state) = setup("again");
        let id = "com.example.again";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let src = folder(&temp.0, "again", &manifest(id, "1.0.0", &[], &[]));
        let view = stage_folder(&state, &src, NOW).unwrap();
        let ui = Stub::answering(true);
        commit("main", &state, &view.token, true, &ui, NOW)
            .unwrap()
            .expect("installed");
        let asked = ui.asked.borrow();
        assert_eq!(asked[0].0, "Install this plugin again?");
        assert_eq!(asked[0].2, "Install");
        assert!(
            asked[0]
                .1
                .contains("Replaces the installed copy of v1.0.0."),
            "{}",
            asked[0].1
        );
    }

    #[test]
    fn uninstalling_asks_and_a_no_removes_nothing() {
        let (temp, state) = setup("uninstall");
        let id = "com.example.u";
        install(&state, &temp.0, id, "1.0.0", &["model.read"], &[]);
        storage_set("main", &state, id, "k", "v", &Stub::answering(true)).unwrap();

        let no = Stub::answering(false);
        assert_eq!(
            uninstall("main", &state, id, &no).unwrap_err(),
            "uninstall_declined: The plugin was left installed."
        );
        assert_eq!(no.asked(), 1);
        assert_eq!(no.changed.get(), 0);
        {
            let asked = no.asked.borrow();
            let (title, body, accept) = &asked[0];
            assert_eq!(title, "Uninstall this plugin?");
            assert_eq!(accept, "Uninstall");
            let (record, manifest) = store::installed(state.dir().unwrap(), id).unwrap();
            assert_eq!(body, &plugin_host::uninstall_summary(&record, &manifest));
        }
        assert_eq!(installed_ids(&state), [id]);
        assert_eq!(
            storage_get("main", &state, id, "k").unwrap().as_deref(),
            Some("v")
        );

        let yes = Stub::answering(true);
        uninstall("main", &state, id, &yes).unwrap();
        assert_eq!(yes.changed.get(), 1);
        assert!(installed_ids(&state).is_empty());
        // Its saved data went with it.
        assert!(!temp.0.join("store").join(id).exists());
        assert_eq!(
            refusal_name(&storage_get("main", &state, id, "k").unwrap_err()),
            "not_installed"
        );
    }

    #[test]
    fn an_uninstall_the_store_would_refuse_is_refused_before_asking() {
        let (temp, state) = setup("uninstall-refused");
        let ui = Stub::answering(true);
        assert_eq!(
            refusal_name(&uninstall("main", &state, "com.example.none", &ui).unwrap_err()),
            "not_installed"
        );
        let base = "com.example.base";
        install(&state, &temp.0, base, "1.0.0", &[], &[]);
        let needs = serde_json::json!({
            "manifest": 1, "id": "com.example.needs", "name": "Needs", "version": "1.0.0",
            "api": 1, "main": "main.js", "requires": [{ "id": base, "range": "^1" }]
        })
        .to_string();
        let src = folder(&temp.0, "needs", &needs);
        let view = stage_folder(&state, &src, NOW).unwrap();
        commit(
            "main",
            &state,
            &view.token,
            true,
            &Stub::answering(true),
            NOW,
        )
        .unwrap();
        assert_eq!(
            refusal_name(&uninstall("main", &state, base, &ui).unwrap_err()),
            "dependency_missing"
        );
        assert_eq!(ui.asked(), 0);
        assert_eq!(installed_ids(&state).len(), 2);
    }

    #[test]
    fn a_plugin_whose_record_is_unreadable_can_be_uninstalled_by_its_id() {
        let (temp, state) = setup("uninstall-unreadable");
        let id = "com.example.newer";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        std::fs::write(
            temp.0.join("store").join(id).join("installed.json"),
            br#"{"schema": 99, "id": "com.example.newer"}"#,
        )
        .unwrap();
        let ui = Stub::answering(true);
        uninstall("main", &state, id, &ui).unwrap();
        assert!(ui.asked.borrow()[0].1.starts_with("com.example.newer\n"));
        assert!(installed_ids(&state).is_empty());
    }

    #[test]
    fn turning_a_plugin_on_asks_and_turning_it_off_does_not() {
        let (temp, state) = setup("enable");
        let id = "com.example.on";
        install(&state, &temp.0, id, "1.0.0", &["model.read"], &[]);

        // Freshly installed: on, but not yet allowed to start with Hew, so
        // turning it on still asks.
        let off = Stub::answering(false);
        assert_eq!(set_enabled("main", &state, id, false, &off), Ok(vec![]));
        assert_eq!(off.asked(), 0, "turning off asks nothing");
        assert_eq!(off.changed.get(), 1);

        let no = Stub::answering(false);
        assert_eq!(
            set_enabled("main", &state, id, true, &no).unwrap_err(),
            "enable_declined: The plugin was left off."
        );
        assert_eq!(no.changed.get(), 0);
        let (record, manifest) = store::installed(state.dir().unwrap(), id).unwrap();
        assert!(!record.enabled);
        {
            let asked = no.asked.borrow();
            let (title, body, accept) = &asked[0];
            assert_eq!(title, "Turn on this plugin?");
            assert_eq!(accept, "Turn On");
            assert_eq!(body, &plugin_host::enable_summary(&record, &manifest));
            assert!(body.contains("Read the model"), "{body}");
        }

        let yes = Stub::answering(true);
        assert_eq!(set_enabled("main", &state, id, true, &yes), Ok(vec![]));
        assert_eq!(yes.asked(), 1);
        assert_eq!(yes.changed.get(), 1);
        let (record, _) = store::installed(state.dir().unwrap(), id).unwrap();
        assert!(record.enabled && record.startup_allowed);

        // Already on and allowed to start: nothing to ask, nothing changes.
        let again = Stub::answering(false);
        assert_eq!(set_enabled("main", &state, id, true, &again), Ok(vec![]));
        assert_eq!(again.asked(), 0);
        assert_eq!(again.changed.get(), 0);

        // Not installed: refused before asking.
        let e = set_enabled("main", &state, "com.example.none", true, &again).unwrap_err();
        assert_eq!(refusal_name(&e), "not_installed");
        assert_eq!(again.asked(), 0);
    }

    #[test]
    fn turning_on_a_plugin_that_changes_while_asking_is_refused() {
        let (temp, state) = setup("enable-race");
        let id = "com.example.on";
        install(&state, &temp.0, id, "1.0.0", &["model.read", "view"], &[]);
        set_enabled("main", &state, id, false, &Stub::answering(true)).unwrap();
        let ui = During::new(true, || {
            // Another window narrows its grant while the dialog is up.
            store::set_grant(
                state.dir().unwrap(),
                id,
                vec![PermissionName::parse("model.read").unwrap()],
                vec![],
            )
            .unwrap();
        });
        assert_eq!(
            refusal_name(&set_enabled("main", &state, id, true, &ui).unwrap_err()),
            "plugin_changed"
        );
        assert_eq!(ui.changed.get(), 0);
        let (record, _) = store::installed(state.dir().unwrap(), id).unwrap();
        assert!(!record.enabled);
    }

    #[test]
    fn saved_data_round_trips_and_tells_every_window() {
        let (temp, state) = setup("storage");
        let id = "com.example.s";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let ui = Stub::answering(false);
        assert_eq!(storage_get("main", &state, id, "a"), Ok(None));
        assert_eq!(storage_list("main-2", &state, id), Ok(vec![]));
        storage_set("main", &state, id, "a", "{\"n\":1}", &ui).unwrap();
        storage_set("settings", &state, id, "b", "\"x\"", &ui).unwrap();
        assert_eq!(
            storage_get("main-2", &state, id, "a").unwrap().as_deref(),
            Some("{\"n\":1}")
        );
        assert_eq!(
            storage_list("main", &state, id).unwrap(),
            ["a".to_string(), "b".to_string()]
        );
        storage_delete("main", &state, id, "a", &ui).unwrap();
        // Deleting what is not there changes nothing and tells no one.
        storage_delete("main", &state, id, "a", &ui).unwrap();
        assert_eq!(
            *ui.stored.borrow(),
            [
                StorageChanged {
                    id: id.into(),
                    keys: vec!["a".into()]
                },
                StorageChanged {
                    id: id.into(),
                    keys: vec!["b".into()]
                },
                StorageChanged {
                    id: id.into(),
                    keys: vec!["a".into()]
                },
            ]
        );
        // Saved data is not a change to the store, and asks nothing.
        assert_eq!(ui.changed.get(), 0);
        assert_eq!(ui.asked(), 0);
        assert_eq!(
            serde_json::to_value(&ui.stored.borrow()[0]).unwrap(),
            serde_json::json!({ "id": id, "keys": ["a"] })
        );
    }

    #[test]
    fn saved_data_refusals_carry_their_names_and_write_nothing() {
        let (temp, state) = setup("storage-refused");
        let id = "com.example.s";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let ui = Stub::answering(true);
        let big = "x".repeat(plugin_host::MAX_STORAGE_BYTES);
        assert_eq!(
            refusal_name(&storage_set("main", &state, id, "a", &big, &ui).unwrap_err()),
            "storage_quota"
        );
        let long = "k".repeat(plugin_host::MAX_STORAGE_KEY_BYTES + 1);
        for key in ["", long.as_str()] {
            assert_eq!(
                refusal_name(&storage_set("main", &state, id, key, "v", &ui).unwrap_err()),
                "storage_key_invalid"
            );
            assert_eq!(
                refusal_name(&storage_get("main", &state, id, key).unwrap_err()),
                "storage_key_invalid"
            );
        }
        // A plugin that is not installed has no saved data, and asking for
        // it creates nothing.
        let none = "com.example.none";
        assert_eq!(
            refusal_name(&storage_set("main", &state, none, "a", "v", &ui).unwrap_err()),
            "not_installed"
        );
        assert!(!temp.0.join("store").join(none).exists());
        assert!(ui.stored.borrow().is_empty());
        assert!(!temp.0.join("store").join(id).join("storage.json").exists());
    }
}
