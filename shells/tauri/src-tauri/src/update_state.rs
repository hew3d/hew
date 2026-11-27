//! What the person told the update window to remember: `update.json` in the
//! app config folder (docs/design D3, "Skip, remind, automatic").
//!
//! The shell owns the file because the launch check runs in Rust before any
//! window's scripts do. Everything in this module is pure (no Tauri types),
//! so it is unit tested directly, and it compiles in builds without the
//! updater, where the "automatic updates" preference is still stored.
//!
//! ```json
//! { "skippedVersion": "1.6.0", "remindAfter": 1795000000000, "autoInstall": false }
//! ```

// Builds without the updater keep this module for its tests only.
#![cfg_attr(not(hew_updater), allow(dead_code))]

use crate::install_channel::{UpdateMode, UpdatePlan};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const STATE_FILE: &str = "update.json";

/// The retired one-line file that remembered the last announced version.
const LEGACY_NOTICE_FILE: &str = "last-update-notice";

/// "Remind Me Later" and closing the window: a day, in milliseconds.
pub const REMIND_AFTER_MS: u64 = 24 * 60 * 60 * 1000;

/// A state file is a few dozen bytes; never read an oversized one.
const STATE_MAX_BYTES: u64 = 4096;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateState {
    /// The launch check stays quiet for exactly this version.
    pub skipped_version: Option<String>,
    /// The launch check stays quiet until this time (ms since the epoch).
    pub remind_after: Option<u64>,
    /// Download updates on launch without asking (self-install copies only).
    pub auto_install: bool,
}

/// What a launch check does with an update it found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchAction {
    /// Say nothing.
    Quiet,
    /// Open the update window.
    ShowWindow,
    /// Download without asking, then show the notice bar.
    AutoInstall,
}

/// The one decision the launch check makes. A skipped version and a pending
/// reminder both keep it quiet (and win over automatic installs: a skipped
/// version is never installed). Automatic installs happen only where the copy
/// can replace itself without a password prompt. A manual "Check for
/// Updates" does not call this: it ignores both and always shows the window.
pub fn launch_action(
    state: &UpdateState,
    offered_version: &str,
    now_ms: u64,
    plan: &UpdatePlan,
) -> LaunchAction {
    if state.skipped_version.as_deref() == Some(offered_version) {
        return LaunchAction::Quiet;
    }
    if state
        .remind_after
        .is_some_and(|until| reminder_pending(until, now_ms))
    {
        return LaunchAction::Quiet;
    }
    if state.auto_install && plan.mode == UpdateMode::SelfInstall {
        return LaunchAction::AutoInstall;
    }
    LaunchAction::ShowWindow
}

/// Whether a "Remind Me Later" set for `until` still keeps the launch check
/// quiet at `now_ms`. A reminder is only ever set a day ahead, so one more
/// than a day in the future was written by a clock that was wrong (or the
/// clock is wrong now); it counts as expired, so a bad clock can never
/// silence the check for good.
pub fn reminder_pending(until: u64, now_ms: u64) -> bool {
    now_ms < until && until - now_ms <= REMIND_AFTER_MS
}

/// Whether the "automatic updates" preference can work on this copy. It is
/// shown (window checkbox, Settings) only where this is true.
pub fn auto_install_supported(plan: &UpdatePlan) -> bool {
    plan.mode == UpdateMode::SelfInstall
}

/// Read the state. A missing, oversized, or corrupt file is an empty state.
pub fn load(config_dir: &Path) -> UpdateState {
    let path = config_dir.join(STATE_FILE);
    match std::fs::metadata(&path) {
        Ok(meta) if meta.len() <= STATE_MAX_BYTES => {}
        _ => return UpdateState::default(),
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Write the state atomically (temp file + rename).
pub fn save(config_dir: &Path, state: &UpdateState) -> std::io::Result<()> {
    std::fs::create_dir_all(config_dir)?;
    let path = config_dir.join(STATE_FILE);
    let tmp = config_dir.join(format!("{STATE_FILE}.tmp"));
    let text = serde_json::to_string_pretty(state).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// One-time handover of the retired `last-update-notice` file: the version
/// it names becomes `skippedVersion` (the same effect it had: that version
/// is not announced again at launch), unless a skipped version is already
/// set. The old file is then deleted. Returns the state to use.
pub fn load_with_handover(config_dir: &Path) -> UpdateState {
    let mut state = load(config_dir);
    let legacy = config_dir.join(LEGACY_NOTICE_FILE);
    if !legacy.exists() {
        return state;
    }
    let old = std::fs::metadata(&legacy)
        .ok()
        .filter(|meta| meta.len() <= 64)
        .and_then(|_| std::fs::read_to_string(&legacy).ok())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());
    if let Some(version) = old {
        if state.skipped_version.is_none() {
            state.skipped_version = Some(version);
            if save(config_dir, &state).is_err() {
                // Keep the old file so the next launch tries again.
                return state;
            }
        }
    }
    let _ = std::fs::remove_file(&legacy);
    state
}

// ---------------------------------------------------------------------------
// The update window's IPC payloads (see the "IPC contract" in main.rs's
// `updater` module doc). Plain data, no Tauri types, so the shapes are tested
// here and the React side codes against them.
// ---------------------------------------------------------------------------

/// The release notes link shown under the notes box.
pub const CHANGELOG_URL: &str = "https://github.com/hew3d/hew/releases";

/// What the shell knows about the update it found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub version: String,
    pub current_version: String,
    /// `Update.body`: the version's CHANGELOG.md section as Markdown. Absent
    /// or empty for feeds published before the field existed.
    pub notes: Option<String>,
    /// Publish date, `YYYY-MM-DD`.
    pub date: Option<String>,
}

/// Where the pending update is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// Found; nothing downloaded.
    Offer,
    /// Download or install running.
    Downloading,
    /// Downloaded and verified, not installed (Windows automatic mode: the
    /// installer must close Hew, so it waits for a click).
    Ready,
    /// Installed; running copy still the old version. Restart to use it.
    Installed,
}

/// How this copy installs, for the window's three rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanInfo {
    /// `"selfInstall"`, `"selfInstallElevated"` (system asks for a
    /// password), or `"notifyOnly"` (tell-only: show `command`/`helpUrl`).
    pub mode: &'static str,
    /// `macApp`, `windowsInstaller`, `appImage`, `deb`, `rpm`, `flatpak`,
    /// `pacman`, `gentoo`, `otherManaged`, `unknown`.
    pub channel: &'static str,
    /// Terminal command to copy, tell-only copies that have one.
    pub command: Option<String>,
    pub help_url: String,
    /// The installer closes Hew itself (Windows): the last step reads "Hew
    /// will close to finish installing" and nothing returns after install.
    pub closes_app: bool,
}

/// `update_info`'s return value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    pub notes: Option<String>,
    pub date: Option<String>,
    pub plan: PlanInfo,
    pub phase: Phase,
    /// "What's New" form: notes and a Close button only.
    pub read_only: bool,
    /// The saved "automatically download and install" preference.
    pub auto_install: bool,
    /// Whether that preference can work on this copy (the checkbox is hidden
    /// when false).
    pub auto_supported: bool,
    pub changelog_url: &'static str,
    pub download_url: &'static str,
    /// Why the last restart could not run the installer, or null. When set
    /// (with `phase: "offer"`), the window opens on the failure step with
    /// Open Download Page.
    pub error: Option<String>,
}

/// `update_auto_get` / `update_auto_set` return value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoInstall {
    pub enabled: bool,
    pub supported: bool,
}

/// Payload of the `update-ready` event and `update_ready_get`: a launch check
/// downloaded an update on its own and the document windows should show the
/// notice bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateReady {
    pub version: String,
    pub current_version: String,
    /// true: installed in place (Mac, AppImage); the bar says it opens next
    /// start. false: downloaded only (Windows); the bar offers "Install and
    /// Restart", and nothing is installed until that click.
    pub installed: bool,
    pub closes_app: bool,
}

pub fn channel_id(channel: &crate::install_channel::InstallChannel) -> &'static str {
    use crate::install_channel::InstallChannel as C;
    match channel {
        C::MacApp => "macApp",
        C::WindowsInstaller => "windowsInstaller",
        C::AppImage => "appImage",
        C::Deb => "deb",
        C::Rpm => "rpm",
        C::Flatpak => "flatpak",
        C::Pacman => "pacman",
        C::Gentoo => "gentoo",
        C::OtherManaged(_) => "otherManaged",
        C::Unknown => "unknown",
    }
}

pub fn plan_info(plan: &UpdatePlan) -> PlanInfo {
    PlanInfo {
        mode: match plan.mode {
            UpdateMode::SelfInstall => "selfInstall",
            UpdateMode::SelfInstallElevated => "selfInstallElevated",
            UpdateMode::NotifyOnly => "notifyOnly",
        },
        channel: channel_id(&plan.channel),
        command: plan.command.clone(),
        help_url: plan.help_url.clone(),
        closes_app: plan.channel == crate::install_channel::InstallChannel::WindowsInstaller,
    }
}

pub fn build_info(
    offer: &Offer,
    plan: &UpdatePlan,
    phase: Phase,
    read_only: bool,
    state: &UpdateState,
) -> UpdateInfo {
    UpdateInfo {
        version: offer.version.clone(),
        current_version: offer.current_version.clone(),
        notes: offer.notes.clone().filter(|n| !n.trim().is_empty()),
        date: offer.date.clone(),
        plan: plan_info(plan),
        phase,
        read_only,
        auto_install: state.auto_install && auto_install_supported(plan),
        auto_supported: auto_install_supported(plan),
        changelog_url: CHANGELOG_URL,
        download_url: crate::install_channel::DOWNLOAD_URL,
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install_channel::{plan_for, InstallChannel};

    fn plan(mode_channel: InstallChannel) -> UpdatePlan {
        plan_for(mode_channel)
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("hew-update-state-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const NOW: u64 = 1_000_000;

    #[test]
    fn launch_action_table() {
        let plans = [
            (
                "self",
                plan(InstallChannel::MacApp),
                UpdateMode::SelfInstall,
            ),
            (
                "self-appimage",
                plan(InstallChannel::AppImage),
                UpdateMode::SelfInstall,
            ),
            (
                "elevated",
                plan(InstallChannel::Deb),
                UpdateMode::SelfInstallElevated,
            ),
            (
                "notify",
                plan(InstallChannel::Flatpak),
                UpdateMode::NotifyOnly,
            ),
        ];
        for (name, p, mode) in &plans {
            assert_eq!(p.mode, *mode, "{name}");
            let can_auto = *mode == UpdateMode::SelfInstall;
            // (skipped, remind_after, auto) -> expected
            let cases: [(Option<&str>, Option<u64>, bool, LaunchAction); 12] = [
                (None, None, false, LaunchAction::ShowWindow),
                (
                    None,
                    None,
                    true,
                    if can_auto {
                        LaunchAction::AutoInstall
                    } else {
                        LaunchAction::ShowWindow
                    },
                ),
                // Skip: exactly the offered version stays quiet; another shows.
                (Some("1.6.0"), None, false, LaunchAction::Quiet),
                (Some("1.6.0"), None, true, LaunchAction::Quiet),
                (Some("1.5.9"), None, false, LaunchAction::ShowWindow),
                (
                    Some("1.5.9"),
                    None,
                    true,
                    if can_auto {
                        LaunchAction::AutoInstall
                    } else {
                        LaunchAction::ShowWindow
                    },
                ),
                // Remind: quiet until the time passes (the boundary is "due").
                (None, Some(NOW + 1), false, LaunchAction::Quiet),
                (None, Some(NOW + 1), true, LaunchAction::Quiet),
                (None, Some(NOW), false, LaunchAction::ShowWindow),
                (
                    None,
                    Some(NOW - 1),
                    true,
                    if can_auto {
                        LaunchAction::AutoInstall
                    } else {
                        LaunchAction::ShowWindow
                    },
                ),
                // Both set.
                (Some("1.6.0"), Some(NOW - 1), false, LaunchAction::Quiet),
                (Some("1.5.9"), Some(NOW + 5), true, LaunchAction::Quiet),
            ];
            for (skipped, remind, auto, expected) in cases {
                let state = UpdateState {
                    skipped_version: skipped.map(str::to_string),
                    remind_after: remind,
                    auto_install: auto,
                };
                assert_eq!(
                    launch_action(&state, "1.6.0", NOW, p),
                    expected,
                    "{name}: {state:?}"
                );
            }
        }
    }

    #[test]
    fn a_reminder_more_than_a_day_ahead_counts_as_expired() {
        let p = plan(InstallChannel::MacApp);
        let at = |remind_after: u64| {
            launch_action(
                &UpdateState {
                    remind_after: Some(remind_after),
                    ..Default::default()
                },
                "1.6.0",
                NOW,
                &p,
            )
        };
        // Just set (exactly a day ahead) and anything sooner: quiet.
        assert_eq!(at(NOW + REMIND_AFTER_MS), LaunchAction::Quiet);
        assert_eq!(at(NOW + 1), LaunchAction::Quiet);
        // Written by a clock running ahead, or read by one set back: a
        // reminder further out than any "Remind Me Later" could set.
        assert_eq!(at(NOW + REMIND_AFTER_MS + 1), LaunchAction::ShowWindow);
        assert_eq!(at(u64::MAX), LaunchAction::ShowWindow);
        assert!(!reminder_pending(NOW, NOW), "due at the boundary");
        assert!(!reminder_pending(NOW - 1, NOW));
    }

    #[test]
    fn info_shape_per_install_row() {
        let offer = Offer {
            version: "1.6.0".into(),
            current_version: "1.5.0".into(),
            notes: Some("   ".into()),
            date: Some("2026-12-01".into()),
        };
        let state = UpdateState {
            auto_install: true,
            ..Default::default()
        };
        let mac = build_info(
            &offer,
            &plan(InstallChannel::MacApp),
            Phase::Offer,
            false,
            &state,
        );
        assert_eq!(mac.notes, None, "blank notes read as missing");
        assert!(mac.auto_supported && mac.auto_install && !mac.plan.closes_app);
        let json = serde_json::to_value(&mac).unwrap();
        assert_eq!(json["plan"]["mode"], "selfInstall");
        assert_eq!(json["plan"]["channel"], "macApp");
        assert_eq!(json["phase"], "offer");
        assert_eq!(json["currentVersion"], "1.5.0");
        assert_eq!(json["autoSupported"], true);
        assert_eq!(json["error"], serde_json::Value::Null);

        let win = build_info(
            &offer,
            &plan(InstallChannel::WindowsInstaller),
            Phase::Ready,
            false,
            &state,
        );
        assert!(win.plan.closes_app);
        assert_eq!(serde_json::to_value(&win).unwrap()["phase"], "ready");

        let deb = build_info(
            &offer,
            &plan(InstallChannel::Deb),
            Phase::Offer,
            false,
            &state,
        );
        assert_eq!(deb.plan.mode, "selfInstallElevated");
        assert!(
            !deb.auto_supported && !deb.auto_install,
            "stored flag is hidden where it cannot work"
        );

        let flat = build_info(
            &offer,
            &plan(InstallChannel::Flatpak),
            Phase::Offer,
            true,
            &state,
        );
        assert_eq!(flat.plan.mode, "notifyOnly");
        assert_eq!(
            flat.plan.command.as_deref(),
            Some("flatpak update com.hew3d.Hew")
        );
        assert!(flat.read_only);
        let other = build_info(
            &offer,
            &plan(InstallChannel::Unknown),
            Phase::Offer,
            false,
            &state,
        );
        assert_eq!(other.plan.command, None);
        assert!(other
            .plan
            .help_url
            .starts_with("https://hew3d.com/download/update-help"));
    }

    #[test]
    fn auto_install_is_supported_only_for_plain_self_install() {
        assert!(auto_install_supported(&plan(InstallChannel::MacApp)));
        assert!(auto_install_supported(&plan(
            InstallChannel::WindowsInstaller
        )));
        assert!(!auto_install_supported(&plan(InstallChannel::Rpm)));
        assert!(!auto_install_supported(&plan(InstallChannel::Pacman)));
    }

    #[test]
    fn state_round_trips_and_uses_camel_case_keys() {
        let dir = scratch("roundtrip");
        assert_eq!(load(&dir), UpdateState::default(), "missing file is empty");
        let state = UpdateState {
            skipped_version: Some("1.6.0".into()),
            remind_after: Some(1_795_000_000_000),
            auto_install: true,
        };
        save(&dir, &state).unwrap();
        assert_eq!(load(&dir), state);
        let text = std::fs::read_to_string(dir.join(STATE_FILE)).unwrap();
        assert!(text.contains("skippedVersion") && text.contains("remindAfter"));
        assert!(!dir.join("update.json.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn corrupt_or_partial_files_read_as_empty_or_defaulted() {
        let dir = scratch("corrupt");
        std::fs::write(dir.join(STATE_FILE), "{ not json").unwrap();
        assert_eq!(load(&dir), UpdateState::default());
        std::fs::write(dir.join(STATE_FILE), "[1,2]").unwrap();
        assert_eq!(load(&dir), UpdateState::default());
        std::fs::write(dir.join(STATE_FILE), "").unwrap();
        assert_eq!(load(&dir), UpdateState::default());
        // Missing keys default; unknown keys are ignored.
        std::fs::write(dir.join(STATE_FILE), r#"{"autoInstall":true,"future":1}"#).unwrap();
        assert_eq!(
            load(&dir),
            UpdateState {
                auto_install: true,
                ..Default::default()
            }
        );
        // Wrong types are a corrupt file, not a panic.
        std::fs::write(dir.join(STATE_FILE), r#"{"remindAfter":"soon"}"#).unwrap();
        assert_eq!(load(&dir), UpdateState::default());
        // Oversized is ignored.
        std::fs::write(dir.join(STATE_FILE), " ".repeat(10_000)).unwrap();
        assert_eq!(load(&dir), UpdateState::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn handover_turns_the_old_notice_into_a_skipped_version_once() {
        let dir = scratch("handover");
        std::fs::write(dir.join(LEGACY_NOTICE_FILE), "1.6.0\n").unwrap();
        let state = load_with_handover(&dir);
        assert_eq!(state.skipped_version.as_deref(), Some("1.6.0"));
        assert!(!dir.join(LEGACY_NOTICE_FILE).exists(), "old file deleted");
        assert_eq!(
            load(&dir).skipped_version.as_deref(),
            Some("1.6.0"),
            "persisted"
        );
        // A second call changes nothing.
        assert_eq!(load_with_handover(&dir), state);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn handover_never_overrides_an_existing_skip_and_keeps_other_fields() {
        let dir = scratch("handover-keep");
        save(
            &dir,
            &UpdateState {
                skipped_version: Some("1.7.0".into()),
                remind_after: Some(5),
                auto_install: true,
            },
        )
        .unwrap();
        std::fs::write(dir.join(LEGACY_NOTICE_FILE), "1.6.0\n").unwrap();
        let state = load_with_handover(&dir);
        assert_eq!(state.skipped_version.as_deref(), Some("1.7.0"));
        assert_eq!(state.remind_after, Some(5));
        assert!(state.auto_install);
        assert!(!dir.join(LEGACY_NOTICE_FILE).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn handover_deletes_an_empty_or_oversized_old_file_without_a_skip() {
        let dir = scratch("handover-junk");
        std::fs::write(dir.join(LEGACY_NOTICE_FILE), "  \n").unwrap();
        assert_eq!(load_with_handover(&dir), UpdateState::default());
        assert!(!dir.join(LEGACY_NOTICE_FILE).exists());
        std::fs::write(dir.join(LEGACY_NOTICE_FILE), "x".repeat(500)).unwrap();
        assert_eq!(load_with_handover(&dir), UpdateState::default());
        assert!(!dir.join(LEGACY_NOTICE_FILE).exists());
        assert!(!dir.join(STATE_FILE).exists(), "nothing to save");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
