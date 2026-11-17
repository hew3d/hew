/**
 * The shapes the install store passes between the shell (or the web store),
 * the consent sheet and the Plugins pane. They mirror the JSON `plugin-host`
 * writes and reads; a field the app does not know is carried along, never
 * dropped.
 */

/** Where an installed plugin came from. Only files and folders install today.
 *  On the web, a folder copied in is a file whose `origin` is `'folder'`. */
export type InstallSource =
  | { kind: 'file'; name: string; origin?: 'folder' }
  | { kind: 'dir'; path: string }
  | { kind: 'index'; id: string; url: string }
  | { kind: 'git'; url: string; reference: string }
  | { kind: 'url'; url: string }

/** `installed.json`. */
export interface Installed {
  schema: number
  id: string
  version: string
  source: InstallSource
  installed_at_ms: number
  /** Permissions the user accepted at install time. */
  grant: string[]
  /** Optional permissions granted later. */
  optional_granted: string[]
  enabled: boolean
  /** False until the user first turns the plugin on by hand. */
  startup_allowed: boolean
  data_version: number
  signer?: string | null
  /** Set by the desktop shell when a folder plugin's manifest changed in a
   *  way the person must review; anything but absent or `false` means it
   *  is waiting. While it waits nothing is read from the folder. */
  needs_consent?: unknown
  /** Why it waits: `permissions_changed`, `startup_added`,
   *  `manifest_unreadable`, `id_changed`, `requirements_unmet`,
   *  `consent_text_changed`, `authority_changed` or `command_name_taken`. */
  needs_consent_reason?: unknown
  [unknown: string]: unknown
}

/** Whether a record waits for the person to review a change to its
 *  folder's manifest: anything in `needs_consent` but absent or `false`. */
export function needsConsent(installed: Installed): boolean {
  return installed.needs_consent !== undefined && installed.needs_consent !== false
}

/** A string a manifest can write in one language, several, or as a catalog key. */
export type LocalizableString = string | Record<string, string> | { $t: string }

/** The parts of a manifest the app reads. */
export interface PluginManifest {
  id: string
  name: LocalizableString
  version: string
  description?: LocalizableString | null
  icon?: string | null
  permissions?: string[]
  optionalPermissions?: string[]
  requires?: { id: string; range: string }[]
  [unknown: string]: unknown
}

export interface PermissionLine {
  name: string
  text: string
}

export interface UpdateDiff {
  added: PermissionLine[]
  new_optional: PermissionLine[]
  /** What the installed version asked for that this one no longer does
   *  (absent from a kernel that does not say). */
  removed?: PermissionLine[]
  removed_optional?: PermissionLine[]
  startup_newly: boolean
  from_version: string
}

/** Everything the consent sheet shows. Text from the plugin (`name`,
 *  `activation_reason`, the source text) is already cleaned. */
export interface ConsentPayload {
  id: string
  name: string
  version: string
  permissions: PermissionLine[]
  optional: PermissionLine[]
  warnings: string[]
  on_startup: boolean
  activation_reason: string | null
  signer: string | null
  unsigned: boolean
  source: InstallSource
  replaces: { name: string; version: string } | null
  update: UpdateDiff | null
  /** A review of a folder plugin whose manifest changed (desktop). */
  review?: boolean
  /** For a review: why, in Hew's words, set by the host that staged it. */
  review_line?: string | null
}

export interface PendingView {
  token: string
  consent: ConsentPayload
}

export type Listed =
  | {
      kind: 'installed'
      installed: Installed
      manifest: PluginManifest
      previous_version: string | null
      /** Desktop, folder plugins: the problem the watcher has with the
       *  folder now (`folder_missing`, `package_path_invalid`,
       *  `package_too_large`), or null. Absent elsewhere. */
      dev_problem?: string | null
    }
  | { kind: 'unreadable'; id: string; reason: string }

/** The live API's waiting request to load a folder: its id, and its path,
 *  cleaned, for display only. */
export interface DevRequest {
  id: string
  path: string
}

/** `plugins-dev-changed`, checked: a folder plugin's folder changed. A hint
 *  only; what is true is what the store lists. */
export interface DevChange {
  id: string
  /** True: the record is now marked and waits for a review. */
  manifest_changed: boolean
  reason: string | null
  /** The folder cannot be watched right now (`folder_missing`,
   *  `package_path_invalid`, `package_too_large`). */
  problem: string | null
}

/** The pane's only door to the install store. */
export interface PluginInstaller {
  /** Desktop installs from a file or (in developer mode) an unpacked folder
   *  through native pickers; the web build installs from a file the page
   *  already holds, or a copy of a folder the browser's picker gave it. */
  readonly tier: 'desktop' | 'web'
  list(): Promise<Listed[]>
  /** Desktop: open the native picker and stage what was picked, or null when
   *  it was cancelled. Web: not available (use `stageFile`). `useRequest`,
   *  the id of the live API request the window shows, opens the folder
   *  picker near that request's folder (refused `request_gone` when it is no
   *  longer the one waiting); no path ever goes from the page to the shell. */
  pickAndStage(kind: 'file' | 'dir', opts?: { useRequest?: string }): Promise<PendingView | null>
  /** Web: stage a package the person chose in a file input. */
  stageFile(file: File): Promise<PendingView>
  /** Web, where the browser has a folder picker: stage a copy of a folder,
   *  packed in the page and read by the same package reader. */
  stageFolder?(dir: FileSystemDirectoryHandle): Promise<PendingView>
  /** Desktop: whether developer mode is on, as the shell says (display
   *  only; the shell enforces it). */
  devMode?(): Promise<boolean>
  /** Desktop: turns developer mode on (the shell asks natively) or off;
   *  answers what it is now. */
  setDevMode?(on: boolean): Promise<boolean>
  /** Desktop: stages the review of a folder plugin whose manifest changed. */
  review?(id: string): Promise<PendingView>
  /** Desktop: the waiting live API request to load a folder, its id and its
   *  path cleaned for display, or null. Looking does not take it. */
  devRequest?(): Promise<DevRequest | null>
  /** Desktop: the person dismissed request `id`; every window forgets it. */
  dismissDevRequest?(id: string): Promise<void>
  /** Desktop: calls `fn` with each checked `plugins-dev-request` (null:
   *  the request was taken, dismissed or dropped). */
  onDevRequest?(fn: (request: DevRequest | null) => void): () => void
  /** Desktop: calls `fn` with each checked `plugins-dev-changed`. */
  onDevChanged?(fn: (change: DevChange) => void): () => void
  commit(token: string, accept: boolean): Promise<Installed | null>
  /** Returns the ids of the other plugins that were also turned off. On the
   *  web, `shown` fingerprints the record the person was shown when turning
   *  a plugin on (`recordFingerprint`), and a different one is refused
   *  `plugin_changed`; the desktop shell compares what it showed itself. */
  setEnabled(id: string, enabled: boolean, shown?: string): Promise<string[]>
  uninstall(id: string): Promise<void>
  /** `shown`, on the web, fingerprints the two records the person was shown
   *  (`rollbackFingerprint`); a different pair is refused `plugin_changed`. */
  rollback(id: string, shown?: string): Promise<Installed>
  /** The version `rollback` would restore, to show before going back. Web
   *  only: the desktop shell confirms natively instead. */
  previous?(id: string): Promise<{ installed: Installed; manifest: PluginManifest }>
  /** A blob URL for an icon, for an `<img>` and nothing else; null when the
   *  plugin has none or it cannot be read. The caller revokes it. */
  iconUrl(id: string, rel: string): Promise<string | null>
  /** Calls `fn` when the store changed, here or in another window. */
  onChanged(fn: () => void): () => void
}
