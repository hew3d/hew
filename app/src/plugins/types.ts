/**
 * The shapes the install store passes between the shell (or the web store),
 * the consent sheet and the Plugins pane. They mirror the JSON `plugin-host`
 * writes and reads; a field the app does not know is carried along, never
 * dropped.
 */

/** Where an installed plugin came from. Only files and folders install today. */
export type InstallSource =
  | { kind: 'file'; name: string }
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
  [unknown: string]: unknown
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
}

export interface PendingView {
  token: string
  consent: ConsentPayload
}

export type Listed =
  | { kind: 'installed'; installed: Installed; manifest: PluginManifest; previous_version: string | null }
  | { kind: 'unreadable'; id: string; reason: string }

/** The pane's only door to the install store. */
export interface PluginInstaller {
  /** Desktop installs from a file or an unpacked folder through native
   *  pickers; the web build installs from a file the page already holds. */
  readonly tier: 'desktop' | 'web'
  list(): Promise<Listed[]>
  /** Desktop: open the native picker and stage what was picked, or null when
   *  it was cancelled. Web: not available (use `stageFile`). */
  pickAndStage(kind: 'file' | 'dir'): Promise<PendingView | null>
  /** Web: stage a package the person chose in a file input. */
  stageFile(file: File): Promise<PendingView>
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
