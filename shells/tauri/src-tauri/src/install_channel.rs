//! How this copy of Hew was installed, and so how it can be updated.
//!
//! The updater plugin picks its installer from a marker compiled into the
//! binary. That marker says how the binary was *bundled*, not how it was
//! *installed*: the Flatpak, pacman, and Gentoo packages all unpack the
//! release `.deb`, so all three carry the Deb marker even though dpkg owns
//! none of them. Running `dpkg -i` for them fails at best and, on a distro
//! that has dpkg installed, overwrites files the real package manager owns.
//!
//! This module is the one place that decides. Detection is pure: it takes
//! its inputs as a parameter struct and an ownership callback, so the tests
//! never touch the real filesystem or run a package manager. The update flow
//! in `main.rs` reads the resulting [`UpdatePlan`] and nothing else.
//!
//! Linux detection, first match wins:
//!
//! 1. Running inside a Flatpak sandbox (`/.flatpak-info` or `FLATPAK_ID`).
//! 2. A marker file `<exe dir>/../share/hew/install-channel`, written by the
//!    package recipes in `packaging/`. Its first line names the channel
//!    (`flatpak`, `pacman`, `gentoo`); any other non-empty value means "some
//!    other package manager" and is the hook a third-party packager uses.
//! 3. The bundle marker compiled into the binary: an AppImage self-installs;
//!    a Deb or Rpm self-installs only when dpkg or rpm really owns the
//!    running binary, so a repackager we have never heard of is never handed
//!    to an installer that does not own it.
//! 4. Anything else (a dev build) is unknown and only notifies.
//!
//! macOS and Windows always self-install, as they did before this module
//! existed.

// Most of this module is exercised only on Linux; macOS and Windows compile
// it for the shared types and the unit tests.
#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The package name the release `.deb` and `.rpm` install under, compared
/// without regard to case.
const PACKAGE_NAME: &str = "hew";

/// The Download page; also the failure dialog's fallback.
pub const DOWNLOAD_URL: &str = "https://hew3d.com/download";

/// The "how do I update" help page. Anchors per channel are added by
/// [`plan_for`].
pub const HELP_URL: &str = "https://hew3d.com/download/update-help";

/// File, relative to the executable's directory, that a package recipe writes.
pub const MARKER_RELATIVE_PATH: &str = "../share/hew/install-channel";

/// A marker is one short word; never read an oversized file into memory.
const MARKER_MAX_BYTES: u64 = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallChannel {
    MacApp,
    WindowsInstaller,
    AppImage,
    Deb,
    Rpm,
    Flatpak,
    Pacman,
    Gentoo,
    /// Installed by a package manager Hew does not know. The string is the
    /// marker's value, or empty when it was inferred (a Deb/Rpm-marked
    /// binary that dpkg/rpm does not own).
    OtherManaged(String),
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateMode {
    /// The updater downloads and installs by itself.
    SelfInstall,
    /// The updater installs a native package; the system asks for a password.
    SelfInstallElevated,
    /// Hew only announces the update; the package manager installs it.
    NotifyOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePlan {
    pub channel: InstallChannel,
    pub mode: UpdateMode,
    /// What to run in a terminal, in notify-only mode when one is known.
    pub command: Option<String>,
    pub help_url: String,
}

// Each platform builds only its own variant; the tests build them all.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    Mac,
    Windows,
    Other,
}

/// What the compiled-in bundle marker says (Linux only matters).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bundle {
    AppImage,
    Deb,
    Rpm,
    /// No marker, or one that is not a Linux package type.
    None,
}

/// Which package manager to ask about file ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    Dpkg,
    Rpm,
}

/// Everything detection reads, gathered by the caller.
#[derive(Debug, Clone)]
pub struct Inputs {
    pub os: Os,
    pub flatpak_info_exists: bool,
    pub flatpak_id_set: bool,
    /// The marker file's first line, raw, when the file exists.
    pub marker: Option<String>,
    pub bundle: Bundle,
}

/// The channel for `inputs`. `owns` answers "does this package manager own
/// the running binary?" and is only called for a Deb or Rpm marker.
pub fn detect(inputs: &Inputs, owns: &dyn Fn(PackageManager) -> bool) -> InstallChannel {
    match inputs.os {
        Os::Mac => return InstallChannel::MacApp,
        Os::Windows => return InstallChannel::WindowsInstaller,
        Os::Other => return InstallChannel::Unknown,
        Os::Linux => {}
    }
    if inputs.flatpak_info_exists || inputs.flatpak_id_set {
        return InstallChannel::Flatpak;
    }
    if let Some(channel) = inputs.marker.as_deref().and_then(parse_marker) {
        return channel;
    }
    match inputs.bundle {
        Bundle::AppImage => InstallChannel::AppImage,
        Bundle::Deb if owns(PackageManager::Dpkg) => InstallChannel::Deb,
        Bundle::Rpm if owns(PackageManager::Rpm) => InstallChannel::Rpm,
        Bundle::Deb | Bundle::Rpm => InstallChannel::OtherManaged(String::new()),
        Bundle::None => InstallChannel::Unknown,
    }
}

/// A marker file's channel: the first line, trimmed and lowercased. Empty
/// means "no marker".
pub fn parse_marker(text: &str) -> Option<InstallChannel> {
    // `trim` leaves a byte-order mark in place; an editor may have added one.
    let first_line = text
        .trim_start_matches('\u{feff}')
        .lines()
        .next()
        .unwrap_or("");
    let value = first_line.trim().to_lowercase();
    // Keep what reaches a dialog short and printable.
    let value: String = value.chars().filter(|c| !c.is_control()).take(40).collect();
    match value.as_str() {
        "" => None,
        "flatpak" => Some(InstallChannel::Flatpak),
        "pacman" => Some(InstallChannel::Pacman),
        "gentoo" => Some(InstallChannel::Gentoo),
        _ => Some(InstallChannel::OtherManaged(value)),
    }
}

/// Read the marker file at `path`, capped in size. A missing, oversized, or
/// unreadable file is no marker.
pub fn read_marker(path: &Path) -> Option<String> {
    if std::fs::metadata(path).ok()?.len() > MARKER_MAX_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// The update plan for a channel.
pub fn plan_for(channel: InstallChannel) -> UpdatePlan {
    let (mode, command, anchor) = match &channel {
        InstallChannel::MacApp | InstallChannel::WindowsInstaller => {
            (UpdateMode::SelfInstall, None, "")
        }
        InstallChannel::AppImage => (UpdateMode::SelfInstall, None, "#appimage"),
        InstallChannel::Deb | InstallChannel::Rpm => {
            (UpdateMode::SelfInstallElevated, None, "#deb-rpm")
        }
        InstallChannel::Flatpak => (
            UpdateMode::NotifyOnly,
            Some("flatpak update com.hew3d.Hew"),
            "#flatpak",
        ),
        // Arch does not support partial upgrades, so the command is the
        // full system upgrade.
        InstallChannel::Pacman => (UpdateMode::NotifyOnly, Some("sudo pacman -Syu"), "#arch"),
        InstallChannel::Gentoo => (
            UpdateMode::NotifyOnly,
            Some("sudo emaint sync -r hew3d && sudo emerge --update media-gfx/hew-bin"),
            "#gentoo",
        ),
        InstallChannel::OtherManaged(_) | InstallChannel::Unknown => {
            (UpdateMode::NotifyOnly, None, "")
        }
    };
    UpdatePlan {
        channel,
        mode,
        command: command.map(str::to_string),
        help_url: format!("{HELP_URL}{anchor}"),
    }
}

/// Whether the package manager's answer says the Hew package owns `exe`.
///
/// `stdout` is what `dpkg-query -S <exe>` or
/// `rpm -qf --queryformat '%{NAME}\n' <exe>` printed. Ownership by some other
/// package is "no": a distro or third-party rebuild that happens to carry the
/// Deb or Rpm marker must not be handed the release package.
pub fn owner_is_hew(pm: PackageManager, stdout: &str, exe: &str) -> bool {
    let is_hew = |name: &str| name.trim().eq_ignore_ascii_case(PACKAGE_NAME);
    match pm {
        // `hew: /usr/bin/hew`, `hew:amd64: /usr/bin/hew`, or several owners
        // as `a, b: /path`. `-S` takes a glob pattern, so only a line naming
        // this exact path counts.
        PackageManager::Dpkg => stdout.lines().any(|line| {
            line.rsplit_once(": ").is_some_and(|(owners, path)| {
                path == exe
                    && owners
                        .split(',')
                        .any(|owner| is_hew(owner.trim().split(':').next().unwrap_or("")))
            })
        }),
        PackageManager::Rpm => stdout.lines().any(is_hew),
    }
}

/// Whether `pm` says the Hew package owns the file at `exe`: asks the package
/// database, read only. A missing tool, an unowned file, or another owner is
/// "no".
#[cfg(target_os = "linux")]
fn package_manager_owns(pm: PackageManager, exe: &Path) -> bool {
    use std::process::{Command, Stdio};
    let Some(exe_str) = exe.to_str() else {
        return false;
    };
    let mut cmd = match pm {
        PackageManager::Dpkg => {
            let mut c = Command::new("dpkg-query");
            c.arg("-S");
            c
        }
        PackageManager::Rpm => {
            let mut c = Command::new("rpm");
            c.args(["-qf", "--queryformat", "%{NAME}\n"]);
            c
        }
    };
    cmd.arg(exe)
        // Parse a stable, untranslated answer.
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|out| out.status.success())
        .is_some_and(|out| owner_is_hew(pm, &String::from_utf8_lossy(&out.stdout), exe_str))
}

// --- Installing a native package ourselves ---------------------------------
//
// For a `.deb` or `.rpm` copy the update flow downloads and verifies the
// package through the updater plugin, then installs it here with one
// `pkexec` call. The plugin's own installer keeps asking after the user
// dismisses the password prompt (a second password box, then bare `sudo`)
// and reports the cancel as a failure; here a cancel is a cancel.

/// How an elevated package install ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageInstall {
    Installed,
    /// The user dismissed the system's password prompt.
    Cancelled,
    Failed(String),
}

/// Whether `bytes` is the kind of package `channel` installs: a `.deb` is an
/// `ar` archive, an `.rpm` starts with the RPM lead magic. Anything else is
/// never handed to an installer.
pub fn package_matches_channel(channel: &InstallChannel, bytes: &[u8]) -> bool {
    match channel {
        InstallChannel::Deb => bytes.starts_with(b"!<arch>\ndebian-binary"),
        InstallChannel::Rpm => bytes.starts_with(&[0xed, 0xab, 0xee, 0xdb]),
        _ => false,
    }
}

/// What a `pkexec <installer>` exit status means. pkexec returns 126 when the
/// user dismisses the authentication dialog and 127 when authorization fails
/// or it cannot run the program; any other status is the installer's own.
pub fn classify_pkexec_exit(code: Option<i32>) -> PackageInstall {
    match code {
        Some(0) => PackageInstall::Installed,
        Some(126) => PackageInstall::Cancelled,
        Some(127) => PackageInstall::Failed(
            "The system did not allow the installer to run (pkexec status 127).".to_string(),
        ),
        Some(n) => PackageInstall::Failed(format!("The package installer exited with status {n}.")),
        None => PackageInstall::Failed("The package installer was stopped.".to_string()),
    }
}

/// Install the downloaded package `bytes` for a `Deb` or `Rpm` copy: write it
/// to a private file under `work_dir` and run `pkexec dpkg -i` or
/// `pkexec rpm -U` on it, once. Blocks until the installer exits.
#[cfg(target_os = "linux")]
pub fn install_package_elevated(
    channel: &InstallChannel,
    bytes: &[u8],
    work_dir: &Path,
) -> PackageInstall {
    install_package_with(channel, bytes, work_dir, std::ffi::OsStr::new("pkexec"))
}

/// [`install_package_elevated`] with the elevation program named by the
/// caller, so tests can stand one in.
#[cfg(target_os = "linux")]
fn install_package_with(
    channel: &InstallChannel,
    bytes: &[u8],
    work_dir: &Path,
    pkexec: &std::ffi::OsStr,
) -> PackageInstall {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    use std::process::{Command, Stdio};

    let (tool, flag, file_name) = match channel {
        InstallChannel::Deb => ("dpkg", "-i", "package.deb"),
        InstallChannel::Rpm => ("rpm", "-U", "package.rpm"),
        _ => return PackageInstall::Failed("This copy does not install packages.".to_string()),
    };
    if !package_matches_channel(channel, bytes) {
        return PackageInstall::Failed("The download is not the expected package.".to_string());
    }
    let failed = |what: &str, err: std::io::Error| PackageInstall::Failed(format!("{what}: {err}"));
    // A directory only this user can write, so nothing else can swap the
    // file between the write and the install.
    if let Err(err) = std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(work_dir)
    {
        return failed("Could not prepare the download folder", err);
    }
    let path = work_dir.join(file_name);
    let _ = std::fs::remove_file(&path);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .and_then(|mut file| file.write_all(bytes).and_then(|()| file.sync_all()));
    if let Err(err) = written {
        let _ = std::fs::remove_file(&path);
        return failed("Could not save the package", err);
    }
    let status = Command::new(pkexec)
        .arg(tool)
        .arg(flag)
        .arg(&path)
        .stdin(Stdio::null())
        .status();
    let _ = std::fs::remove_file(&path);
    match status {
        Ok(status) => classify_pkexec_exit(status.code()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => PackageInstall::Failed(
            "pkexec was not found, so Hew cannot ask for permission to install.".to_string(),
        ),
        Err(err) => failed("Could not start the installer", err),
    }
}

#[cfg(target_os = "linux")]
fn bundle_from_binary() -> Bundle {
    use tauri::utils::config::BundleType;
    use tauri::utils::platform::bundle_type;
    match bundle_type() {
        Some(BundleType::AppImage) => Bundle::AppImage,
        Some(BundleType::Deb) => Bundle::Deb,
        Some(BundleType::Rpm) => Bundle::Rpm,
        _ => Bundle::None,
    }
}

/// The running copy's channel. Detected once and kept: how a process was
/// installed does not change while it runs, and after a package update
/// replaces the binary on disk the path of the running one no longer
/// resolves, which would make a later ownership check wrongly say "no".
pub fn current_channel() -> InstallChannel {
    static CHANNEL: OnceLock<InstallChannel> = OnceLock::new();
    CHANNEL.get_or_init(detect_current_channel).clone()
}

/// Detect the running copy's channel from the real environment.
fn detect_current_channel() -> InstallChannel {
    #[cfg(target_os = "linux")]
    {
        let exe = std::env::current_exe().ok();
        let marker = exe
            .as_deref()
            .and_then(Path::parent)
            .and_then(|dir| read_marker(&dir.join(MARKER_RELATIVE_PATH)));
        let inputs = Inputs {
            os: Os::Linux,
            flatpak_info_exists: Path::new("/.flatpak-info").exists(),
            flatpak_id_set: std::env::var_os("FLATPAK_ID").is_some(),
            marker,
            bundle: bundle_from_binary(),
        };
        let canonical: Option<PathBuf> = exe.as_deref().and_then(|p| p.canonicalize().ok());
        detect(&inputs, &|pm| {
            canonical
                .as_deref()
                .is_some_and(|p| package_manager_owns(pm, p))
        })
    }
    #[cfg(target_os = "macos")]
    {
        detect(&no_linux_inputs(Os::Mac), &|_| false)
    }
    #[cfg(target_os = "windows")]
    {
        detect(&no_linux_inputs(Os::Windows), &|_| false)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        detect(&no_linux_inputs(Os::Other), &|_| false)
    }
}

#[cfg(not(target_os = "linux"))]
fn no_linux_inputs(os: Os) -> Inputs {
    Inputs {
        os,
        flatpak_info_exists: false,
        flatpak_id_set: false,
        marker: None,
        bundle: Bundle::None,
    }
}

/// The plan for this running copy.
pub fn current_plan() -> UpdatePlan {
    plan_for(current_channel())
}

// --- Launch-notice quieting (notify-only channels) ------------------------
//
// A package-manager copy cannot act on the notice, so the launch check names
// each version once instead of on every start. The last version announced is
// one line in the app config dir.

const ANNOUNCED_FILE: &str = "last-update-notice";

/// The version last announced, if one was recorded.
pub fn read_announced(config_dir: &Path) -> Option<String> {
    let path = config_dir.join(ANNOUNCED_FILE);
    if std::fs::metadata(&path).ok()?.len() > 64 {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    let version = text.trim();
    (!version.is_empty()).then(|| version.to_string())
}

/// Remember `version` as announced. Best effort: a failure only means the
/// next launch announces it once more.
pub fn write_announced(config_dir: &Path, version: &str) {
    if std::fs::create_dir_all(config_dir).is_err() {
        return;
    }
    let _ = std::fs::write(config_dir.join(ANNOUNCED_FILE), format!("{version}\n"));
}

/// Whether a launch check should announce `version`: true unless it is the
/// version already announced.
pub fn should_announce(config_dir: Option<&Path>, version: &str) -> bool {
    match config_dir.and_then(read_announced) {
        Some(last) => last != version,
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory holding a stand-in for pkexec that records its
    /// arguments, the package file's and folder's modes, and the file's
    /// first bytes, then exits with `status`.
    #[cfg(target_os = "linux")]
    fn fake_pkexec(name: &str, status: i32) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("hew-install-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let script = root.join("fake-pkexec");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\n{{ echo \"$1 $2 $3\"; stat -c %a \"$3\"; stat -c %a \"$(dirname \"$3\")\"; head -c 7 \"$3\"; }} > '{}'\nexit {status}\n",
                root.join("record").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        (root, script)
    }

    #[cfg(target_os = "linux")]
    const DEB: &[u8] = b"!<arch>\ndebian-binary   rest";
    #[cfg(target_os = "linux")]
    const RPM: &[u8] = &[0xed, 0xab, 0xee, 0xdb, 3, 0];

    #[cfg(target_os = "linux")]
    #[test]
    fn package_install_runs_one_private_installer_call_and_cleans_up() {
        let (root, script) = fake_pkexec("ok", 0);
        let work = root.join("cache").join("update");
        let outcome = install_package_with(&InstallChannel::Deb, DEB, &work, script.as_os_str());
        assert_eq!(outcome, PackageInstall::Installed);
        let record = std::fs::read_to_string(root.join("record")).unwrap();
        let file = work.join("package.deb");
        assert_eq!(
            record,
            format!("dpkg -i {}\n600\n700\n!<arch>", file.display())
        );
        assert!(!file.exists(), "the package file is removed after the run");

        let outcome = install_package_with(&InstallChannel::Rpm, RPM, &work, script.as_os_str());
        assert_eq!(outcome, PackageInstall::Installed);
        // The recorded first bytes of an .rpm are not text.
        let record = std::fs::read(root.join("record")).unwrap();
        let record = String::from_utf8_lossy(&record);
        assert!(record.starts_with(&format!("rpm -U {}\n", work.join("package.rpm").display())));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn package_install_cancel_failure_and_missing_pkexec() {
        let (root, script) = fake_pkexec("cancel", 126);
        let work = root.join("update");
        assert_eq!(
            install_package_with(&InstallChannel::Deb, DEB, &work, script.as_os_str()),
            PackageInstall::Cancelled
        );
        assert!(!work.join("package.deb").exists());
        std::fs::remove_dir_all(&root).unwrap();

        let (root, script) = fake_pkexec("fail", 1);
        let work = root.join("update");
        let outcome = install_package_with(&InstallChannel::Deb, DEB, &work, script.as_os_str());
        assert!(
            matches!(&outcome, PackageInstall::Failed(m) if m.contains("status 1")),
            "{outcome:?}"
        );

        let missing = root.join("no-such-pkexec");
        let outcome = install_package_with(&InstallChannel::Deb, DEB, &work, missing.as_os_str());
        assert!(
            matches!(&outcome, PackageInstall::Failed(m) if m.contains("was not found")),
            "{outcome:?}"
        );
        assert!(!work.join("package.deb").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn package_install_refuses_the_wrong_kind_before_writing_anything() {
        let (root, script) = fake_pkexec("kind", 0);
        let work = root.join("update");
        for (channel, bytes) in [
            (InstallChannel::Deb, RPM),
            (InstallChannel::Rpm, DEB),
            (InstallChannel::AppImage, DEB),
            (InstallChannel::Flatpak, DEB),
        ] {
            let outcome = install_package_with(&channel, bytes, &work, script.as_os_str());
            assert!(matches!(outcome, PackageInstall::Failed(_)), "{channel:?}");
        }
        assert!(!work.exists(), "nothing is written");
        assert!(!root.join("record").exists(), "the installer never runs");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn pkexec_exit_status_separates_cancel_from_failure() {
        assert_eq!(classify_pkexec_exit(Some(0)), PackageInstall::Installed);
        assert_eq!(classify_pkexec_exit(Some(126)), PackageInstall::Cancelled);
        for code in [Some(1), Some(2), Some(127), None] {
            assert!(
                matches!(classify_pkexec_exit(code), PackageInstall::Failed(_)),
                "{code:?}"
            );
        }
    }

    #[test]
    fn only_the_matching_package_kind_reaches_an_installer() {
        let deb = b"!<arch>\ndebian-binary   1700000000  0     0     100644  4         `\n2.0\n";
        let rpm = [0xed, 0xab, 0xee, 0xdb, 3, 0, 0, 0];
        let appimage = b"\x7fELF\x02\x01\x01\x00AI\x02";
        assert!(package_matches_channel(&InstallChannel::Deb, deb));
        assert!(package_matches_channel(&InstallChannel::Rpm, &rpm));
        assert!(!package_matches_channel(&InstallChannel::Deb, &rpm));
        assert!(!package_matches_channel(&InstallChannel::Rpm, deb));
        assert!(!package_matches_channel(&InstallChannel::Deb, appimage));
        // A plain `ar` archive that is not a Debian package.
        assert!(!package_matches_channel(
            &InstallChannel::Deb,
            b"!<arch>\nfoo.o/"
        ));
        assert!(!package_matches_channel(
            &InstallChannel::AppImage,
            appimage
        ));
        assert!(!package_matches_channel(&InstallChannel::Deb, b""));
    }

    #[test]
    fn dpkg_ownership_needs_the_hew_package_and_the_exact_path() {
        let exe = "/usr/bin/hew";
        let dpkg = PackageManager::Dpkg;
        assert!(owner_is_hew(dpkg, "hew: /usr/bin/hew\n", exe));
        assert!(owner_is_hew(dpkg, "hew:amd64: /usr/bin/hew\n", exe));
        assert!(owner_is_hew(dpkg, "Hew: /usr/bin/hew\n", exe));
        assert!(owner_is_hew(dpkg, "other, hew: /usr/bin/hew\n", exe));
        // Another package owns the binary: not ours to replace.
        assert!(!owner_is_hew(dpkg, "hew-distro: /usr/bin/hew\n", exe));
        assert!(!owner_is_hew(dpkg, "hewn: /usr/bin/hew\n", exe));
        // `-S` matched a different file through a glob in the path.
        assert!(!owner_is_hew(dpkg, "hew: /usr/bin/hex\n", "/usr/bin/he?"));
        assert!(!owner_is_hew(dpkg, "hew: /usr/bin/hew-cli\n", exe));
        assert!(!owner_is_hew(dpkg, "", exe));
        assert!(!owner_is_hew(
            dpkg,
            "diversion by hew from: /usr/bin/hew\n",
            exe
        ));
    }

    #[test]
    fn rpm_ownership_needs_the_hew_package() {
        let rpm = PackageManager::Rpm;
        assert!(owner_is_hew(rpm, "Hew\n", "/usr/bin/hew"));
        assert!(owner_is_hew(rpm, "hew\n", "/usr/bin/hew"));
        assert!(!owner_is_hew(rpm, "hew-nightly\n", "/usr/bin/hew"));
        assert!(!owner_is_hew(rpm, "", "/usr/bin/hew"));
    }

    #[test]
    fn marker_with_a_byte_order_mark_still_names_its_channel() {
        assert_eq!(
            parse_marker("\u{feff}pacman\n"),
            Some(InstallChannel::Pacman)
        );
    }

    fn linux(marker: Option<&str>, bundle: Bundle) -> Inputs {
        Inputs {
            os: Os::Linux,
            flatpak_info_exists: false,
            flatpak_id_set: false,
            marker: marker.map(str::to_string),
            bundle,
        }
    }

    fn never(_: PackageManager) -> bool {
        panic!("ownership must not be asked here");
    }

    #[test]
    fn flatpak_beats_everything() {
        let mut i = linux(Some("pacman"), Bundle::Deb);
        i.flatpak_info_exists = true;
        assert_eq!(detect(&i, &never), InstallChannel::Flatpak);
        let mut i = linux(None, Bundle::AppImage);
        i.flatpak_id_set = true;
        assert_eq!(detect(&i, &never), InstallChannel::Flatpak);
    }

    #[test]
    fn marker_names_the_channel_without_asking_ownership() {
        let by = |m: &str| detect(&linux(Some(m), Bundle::Deb), &never);
        assert_eq!(by("flatpak\n"), InstallChannel::Flatpak);
        assert_eq!(
            by("  Pacman  \nignored second line"),
            InstallChannel::Pacman
        );
        assert_eq!(by("GENTOO"), InstallChannel::Gentoo);
        assert_eq!(by("nix"), InstallChannel::OtherManaged("nix".into()));
    }

    #[test]
    fn empty_marker_falls_through_to_the_bundle() {
        assert_eq!(
            detect(&linux(Some("\n"), Bundle::AppImage), &never),
            InstallChannel::AppImage
        );
        assert_eq!(parse_marker(""), None);
        assert_eq!(parse_marker("   \n"), None);
    }

    #[test]
    fn marker_text_is_bounded_and_printable() {
        let long = "x".repeat(500);
        match parse_marker(&long) {
            Some(InstallChannel::OtherManaged(v)) => assert_eq!(v.len(), 40),
            other => panic!("unexpected {other:?}"),
        }
        match parse_marker("a\u{7}b") {
            Some(InstallChannel::OtherManaged(v)) => assert_eq!(v, "ab"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn appimage_self_installs_without_asking_ownership() {
        assert_eq!(
            detect(&linux(None, Bundle::AppImage), &never),
            InstallChannel::AppImage
        );
    }

    #[test]
    fn deb_needs_dpkg_to_own_the_binary() {
        let i = linux(None, Bundle::Deb);
        assert_eq!(
            detect(&i, &|pm| pm == PackageManager::Dpkg),
            InstallChannel::Deb
        );
        // Asked about dpkg only; rpm owning it must not matter.
        assert_eq!(
            detect(&i, &|pm| pm == PackageManager::Rpm),
            InstallChannel::OtherManaged(String::new())
        );
    }

    #[test]
    fn rpm_needs_rpm_to_own_the_binary() {
        let i = linux(None, Bundle::Rpm);
        assert_eq!(
            detect(&i, &|pm| pm == PackageManager::Rpm),
            InstallChannel::Rpm
        );
        assert_eq!(
            detect(&i, &|_| false),
            InstallChannel::OtherManaged(String::new())
        );
    }

    #[test]
    fn no_marker_and_no_bundle_is_unknown() {
        assert_eq!(
            detect(&linux(None, Bundle::None), &never),
            InstallChannel::Unknown
        );
    }

    #[test]
    fn mac_and_windows_ignore_linux_signals() {
        let mut i = linux(Some("flatpak"), Bundle::Deb);
        i.flatpak_info_exists = true;
        i.os = Os::Mac;
        assert_eq!(detect(&i, &never), InstallChannel::MacApp);
        i.os = Os::Windows;
        assert_eq!(detect(&i, &never), InstallChannel::WindowsInstaller);
        i.os = Os::Other;
        assert_eq!(detect(&i, &never), InstallChannel::Unknown);
    }

    #[test]
    fn modes_and_commands_per_channel() {
        use InstallChannel::*;
        let mode = |c| plan_for(c).mode;
        assert_eq!(mode(MacApp), UpdateMode::SelfInstall);
        assert_eq!(mode(WindowsInstaller), UpdateMode::SelfInstall);
        assert_eq!(mode(AppImage), UpdateMode::SelfInstall);
        assert_eq!(mode(Deb), UpdateMode::SelfInstallElevated);
        assert_eq!(mode(Rpm), UpdateMode::SelfInstallElevated);
        for c in [Flatpak, Pacman, Gentoo, OtherManaged("x".into()), Unknown] {
            assert_eq!(mode(c), UpdateMode::NotifyOnly);
        }
        assert_eq!(
            plan_for(Flatpak).command.as_deref(),
            Some("flatpak update com.hew3d.Hew")
        );
        assert_eq!(
            plan_for(Pacman).command.as_deref(),
            Some("sudo pacman -Syu")
        );
        assert_eq!(
            plan_for(Gentoo).command.as_deref(),
            Some("sudo emaint sync -r hew3d && sudo emerge --update media-gfx/hew-bin")
        );
        assert_eq!(plan_for(OtherManaged(String::new())).command, None);
        assert_eq!(plan_for(Unknown).command, None);
        // Self-installing channels never carry a command to copy.
        assert_eq!(plan_for(Deb).command, None);
    }

    #[test]
    fn help_urls_carry_the_channel_anchor() {
        use InstallChannel::*;
        let url = |c| plan_for(c).help_url;
        assert_eq!(url(Flatpak), format!("{HELP_URL}#flatpak"));
        assert_eq!(url(Pacman), format!("{HELP_URL}#arch"));
        assert_eq!(url(Gentoo), format!("{HELP_URL}#gentoo"));
        assert_eq!(url(Deb), format!("{HELP_URL}#deb-rpm"));
        assert_eq!(url(Rpm), format!("{HELP_URL}#deb-rpm"));
        assert_eq!(url(AppImage), format!("{HELP_URL}#appimage"));
        assert_eq!(url(Unknown), HELP_URL);
        assert_eq!(url(OtherManaged("x".into())), HELP_URL);
    }

    #[test]
    fn marker_file_reads_with_a_size_cap() {
        let dir = std::env::temp_dir().join(format!("hew-install-channel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("install-channel");
        assert_eq!(read_marker(&path), None);
        std::fs::write(&path, "flatpak\n").unwrap();
        assert_eq!(read_marker(&path).as_deref(), Some("flatpak\n"));
        std::fs::write(&path, "x".repeat(10_000)).unwrap();
        assert_eq!(read_marker(&path), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn launch_notice_is_announced_once_per_version() {
        let dir = std::env::temp_dir().join(format!("hew-announce-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(should_announce(Some(&dir), "1.5.0"));
        assert!(should_announce(None, "1.5.0"));
        write_announced(&dir, "1.5.0");
        assert_eq!(read_announced(&dir).as_deref(), Some("1.5.0"));
        assert!(!should_announce(Some(&dir), "1.5.0"));
        assert!(should_announce(Some(&dir), "1.5.1"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
