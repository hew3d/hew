//! Why an install, or an operation on an installed plugin, was refused.

use plugin_manifest::ManifestError;

/// Why an install, or an operation on an installed plugin, was refused.
/// [`InstallError::name`] is the snake_case refusal name every host
/// reports; [`InstallError::explanation`] is one sentence a person can act
/// on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallError {
    /// `hew-plugin.json` did not parse or validate; named by the manifest
    /// error itself.
    Manifest(ManifestError),
    /// An entry is compressed or encrypted. Packages are stored zips.
    PackageNotStoreOnly,
    /// Too many entries, an entry over [`crate::MAX_ENTRY_BYTES`], the
    /// whole over [`crate::MAX_PACKAGE_BYTES`], or `agent.notes` over its
    /// cap. Always measured on the bytes read.
    PackageTooLarge,
    /// An entry's path is not a plain relative path Hew accepts on every
    /// platform, two entries collide, or an entry is not a regular file or
    /// folder (a symbolic link, say). Carries the offending path.
    PackagePathInvalid(String),
    /// The bytes are not a zip archive this reader can make sense of, or
    /// the archive's two directories disagree.
    PackageUnreadable(String),
    /// No `hew-plugin.json` at the package root.
    ManifestMissing,
    /// The manifest names a file (`main`, `icon`, `locales.catalogs`,
    /// `agent.notes`) the package does not contain.
    ManifestFileMissing(String),
    /// A file or folder install claims an id under `org.hew3d.`, which is
    /// kept for plugins Hew itself publishes.
    ReservedId(String),
    HewVersionUnsupported {
        required: String,
        running: String,
    },
    /// Plugins named in `requires` that are not installed in a matching
    /// version; or, from `uninstall`, the enabled plugins that still
    /// require this one.
    DependencyMissing(Vec<String>),
    /// The `requires` graph would contain this cycle.
    DependencyCycle(Vec<String>),
    NotInstalled(String),
    NoPreviousVersion(String),
    PendingExpired,
    PendingUnknown,
    /// What is installed under this id changed between `stage` and
    /// `commit` (installed, removed, updated, or its record rewritten).
    PendingStale,
    /// `permissions` and `optionalPermissions` together declare more than
    /// [`crate::MAX_PERMISSIONS`]; carries the count.
    TooManyPermissions(usize),
    /// The plugin's `installed.json` was written by a newer Hew (a higher
    /// `schema`) or cannot be read. It is listed but never rewritten.
    InstalledUnreadable {
        id: String,
        reason: String,
    },
    /// A storage write would make the plugin's `storage.json` larger than
    /// [`MAX_STORAGE_BYTES`].
    StorageQuota,
    /// A write would take everything the plugin keeps beside its package
    /// (`plugin-data/<id>/`) past [`MAX_PLUGIN_DATA_BYTES`].
    /// Named `storage_quota`, like [`InstallError::StorageQuota`].
    DataQuota,
    /// A document id is not the 36-character lowercase hyphenated UUID
    /// form. Nothing is built from it.
    DocumentIdInvalid,
    /// A storage key is empty or longer than
    /// [`MAX_STORAGE_KEY_BYTES`].
    StorageKeyInvalid,
    /// The plugin's `storage.json` is not a JSON object of strings, is not
    /// a regular file, or is over the cap. It is left as it is.
    StorageUnreadable,
    /// A folder plugin's folder is gone, is no longer a folder, or cannot
    /// be opened. Carries the recorded path.
    FolderMissing(String),
    /// Developer mode is off, so nothing is read from a plugin folder.
    DeveloperModeOff,
    /// A folder plugin's manifest changed in a way the person must review
    /// before anything is read from the folder again. Carries the id.
    DevConsentPending(String),
    /// A review was asked for a plugin with nothing waiting to be reviewed.
    NoReviewPending(String),
    /// The folder's manifest now names another plugin; carries the id it
    /// names, cleaned.
    FolderIdChanged(String),
    /// One of the plugin's commands has the namespaced name
    /// (`<plugin-id>.<command-id>`) of another installed plugin's command;
    /// carries the name.
    CommandNameTaken(String),
    Io(String),
}

impl InstallError {
    /// The snake_case refusal name.
    pub fn name(&self) -> &'static str {
        match self {
            InstallError::Manifest(e) => e.name(),
            InstallError::PackageNotStoreOnly => "package_not_store_only",
            InstallError::PackageTooLarge => "package_too_large",
            InstallError::PackagePathInvalid(_) => "package_path_invalid",
            InstallError::PackageUnreadable(_) => "package_unreadable",
            InstallError::ManifestMissing => "manifest_missing",
            InstallError::ManifestFileMissing(_) => "manifest_file_missing",
            InstallError::ReservedId(_) => "reserved_id",
            InstallError::HewVersionUnsupported { .. } => "hew_version_unsupported",
            InstallError::DependencyMissing(_) => "dependency_missing",
            InstallError::DependencyCycle(_) => "dependency_cycle",
            InstallError::NotInstalled(_) => "not_installed",
            InstallError::NoPreviousVersion(_) => "no_previous_version",
            InstallError::PendingExpired => "pending_expired",
            InstallError::PendingUnknown => "pending_unknown",
            InstallError::PendingStale => "pending_stale",
            InstallError::TooManyPermissions(_) => "too_many_permissions",
            InstallError::InstalledUnreadable { .. } => "installed_unreadable",
            InstallError::StorageQuota | InstallError::DataQuota => "storage_quota",
            InstallError::DocumentIdInvalid => "document_id_invalid",
            InstallError::StorageKeyInvalid => "storage_key_invalid",
            InstallError::StorageUnreadable => "storage_unreadable",
            InstallError::FolderMissing(_) => "folder_missing",
            InstallError::DeveloperModeOff => "developer_mode_off",
            InstallError::DevConsentPending(_) => "dev_consent_pending",
            InstallError::NoReviewPending(_) => "no_review_pending",
            InstallError::FolderIdChanged(_) => "folder_id_changed",
            InstallError::CommandNameTaken(_) => "command_name_taken",
            InstallError::Io(_) => "io",
        }
    }

    /// [`InstallError::explanation`] for a failure while reading a folder
    /// Hew watches. Where the plain text tells the person to install again
    /// or says "package", this says what is true of a folder: fixing the
    /// file is enough, because the folder is read again by itself. Every
    /// other refusal reads the same for either source.
    pub fn explanation_for_folder(&self) -> String {
        match self {
            InstallError::Manifest(ManifestError::Json(text)) => format!(
                "hew-plugin.json is not a valid manifest: {text}. Fix the file and Hew reads it again."
            ),
            InstallError::ManifestMissing => {
                "This folder has no hew-plugin.json at its top level.".to_string()
            }
            other => other.explanation(),
        }
    }

    /// One sentence a person can act on.
    pub fn explanation(&self) -> String {
        match self {
            InstallError::Manifest(e) => e.explanation(),
            InstallError::PackageNotStoreOnly => {
                "This package is compressed. Hew installs packages made with no compression (zip -0)."
                    .to_string()
            }
            InstallError::PackageTooLarge => format!(
                "This package is too large. A package may hold at most {} files, {} MB per file and {} MB in all.",
                crate::MAX_PACKAGE_ENTRIES,
                crate::MAX_ENTRY_BYTES >> 20,
                crate::MAX_PACKAGE_BYTES >> 20
            ),
            InstallError::PackagePathInvalid(path) => format!(
                "The package holds \"{path}\", which is not a file name Hew accepts. Use plain relative names that work on every system, with no links."
            ),
            InstallError::PackageUnreadable(why) => format!(
                "This file is not a plugin package Hew can read ({why})."
            ),
            InstallError::ManifestMissing => {
                "The package has no hew-plugin.json at its top level.".to_string()
            }
            InstallError::ManifestFileMissing(path) => format!(
                "hew-plugin.json names \"{path}\", which is not in the package."
            ),
            InstallError::ReservedId(id) => format!(
                "\"{id}\" is kept for plugins Hew publishes, so it cannot be installed from a file or folder."
            ),
            InstallError::HewVersionUnsupported { required, running } => format!(
                "This plugin needs Hew {required}, and this is Hew {running}."
            ),
            InstallError::DependencyMissing(ids) => format!(
                "This needs other plugins first, or other plugins still need it: {}.",
                ids.join(", ")
            ),
            InstallError::DependencyCycle(ids) => format!(
                "These plugins would each need the other: {}.",
                ids.join(" → ")
            ),
            InstallError::NotInstalled(id) => format!("\"{id}\" is not installed."),
            InstallError::NoPreviousVersion(id) => {
                format!("\"{id}\" has no earlier version to go back to.")
            }
            InstallError::PendingExpired => {
                "This install waited too long for an answer. Start it again.".to_string()
            }
            InstallError::PendingUnknown => {
                "This install is no longer waiting for an answer. Start it again.".to_string()
            }
            InstallError::PendingStale => {
                "This plugin changed while the install was waiting. Start the install again."
                    .to_string()
            }
            InstallError::TooManyPermissions(n) => format!(
                "This plugin asks for {n} permissions, and a plugin may ask for at most {}.",
                crate::MAX_PERMISSIONS
            ),
            InstallError::InstalledUnreadable { id, reason } => format!(
                "Hew cannot read the record of \"{id}\" ({reason}). A newer Hew may have written it; it is left as it is."
            ),
            InstallError::StorageQuota => format!(
                "This plugin's saved data would pass {} MB, the most a plugin may keep.",
                MAX_STORAGE_BYTES >> 20
            ),
            InstallError::DataQuota => format!(
                "This plugin's saved data would pass {} MB in all, the most a plugin may keep.",
                MAX_PLUGIN_DATA_BYTES >> 20
            ),
            InstallError::DocumentIdInvalid => {
                "That is not a document id Hew uses.".to_string()
            }
            InstallError::StorageKeyInvalid => format!(
                "A storage key must be 1 to {} bytes long.",
                MAX_STORAGE_KEY_BYTES
            ),
            InstallError::StorageUnreadable => {
                "Hew cannot read this plugin's saved data; it is left as it is.".to_string()
            }
            InstallError::FolderMissing(path) => format!(
                "Hew cannot open the plugin's folder \"{}\". It may have been moved or deleted.",
                crate::clean_text(path, 512)
            ),
            InstallError::DeveloperModeOff => {
                "Turn on developer mode in Settings > Plugins to run a plugin from a folder."
                    .to_string()
            }
            InstallError::DevConsentPending(_) => {
                "This plugin's folder changed in a way you need to review. Review the change in Settings > Plugins."
                    .to_string()
            }
            InstallError::NoReviewPending(id) => {
                format!("\"{id}\" has no change waiting for review.")
            }
            InstallError::FolderIdChanged(found) => format!(
                "The folder's hew-plugin.json now names \"{found}\", another plugin. Change it back, or remove this plugin and install the folder again."
            ),
            InstallError::CommandNameTaken(name) => format!(
                "Another installed plugin already has a command named \"{}\". Rename this plugin's command.",
                crate::clean_text(name, 256)
            ),
            InstallError::Io(why) => format!("The plugin folder could not be changed: {why}."),
        }
    }
}

impl From<ManifestError> for InstallError {
    fn from(e: ManifestError) -> InstallError {
        InstallError::Manifest(e)
    }
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.name(), self.explanation())
    }
}

impl std::error::Error for InstallError {}

/// The most a plugin's `storage.json` may hold, in bytes, as written.
pub const MAX_STORAGE_BYTES: usize = 5 * 1024 * 1024;
/// The longest storage key, in bytes.
pub const MAX_STORAGE_KEY_BYTES: usize = 256;
/// The most a plugin may keep beside its package (`plugin-data/<id>/`:
/// per-document storage, its console log and its activity), in bytes.
pub const MAX_PLUGIN_DATA_BYTES: u64 = 50 * 1024 * 1024;
