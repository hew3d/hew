/**
 * The update window's shell contract (shells/tauri/src-tauri/src/updater.rs,
 * "IPC contract"): the payload types, thin typed `invoke` wrappers, and the
 * one-line "does this build carry an updater" probe. Everything that talks
 * to the shell about updates goes through here so tests mock one place.
 */

import { isTauri } from '../io/fileHost'

export type UpdateMode = 'selfInstall' | 'selfInstallElevated' | 'notifyOnly'

export type InstallChannel =
  | 'macApp'
  | 'windowsInstaller'
  | 'appImage'
  | 'deb'
  | 'rpm'
  | 'flatpak'
  | 'pacman'
  | 'gentoo'
  | 'otherManaged'
  | 'unknown'

export type UpdatePhase = 'offer' | 'downloading' | 'ready' | 'installed'

export interface UpdatePlanInfo {
  mode: UpdateMode
  channel: InstallChannel
  command: string | null
  helpUrl: string
  closesApp: boolean
}

export interface UpdateInfo {
  version: string
  currentVersion: string
  /** Markdown, or null when the feed carried none. */
  notes: string | null
  date: string | null
  plan: UpdatePlanInfo
  phase: UpdatePhase
  readOnly: boolean
  autoInstall: boolean
  autoSupported: boolean
  changelogUrl: string
  downloadUrl: string
  /** Why the last restart could not run the installer (the window opens on
   *  the failure step), or null. */
  error: string | null
}

export interface InstallResult {
  /** 'ready': downloaded, not installed, because the installer closes Hew
   *  (Windows); "Install and Restart" (`updateRestart`) runs it. */
  status: 'installed' | 'ready' | 'cancelled' | 'failed'
  version: string
  error: string | null
  closesApp: boolean
}

/** `update_restart`'s answer when the restart did not happen: someone
 *  pressed Cancel in a save prompt, or a restart is already closing the
 *  documents. On success the call never returns (Hew restarts). */
export interface RestartOutcome {
  status: 'cancelled' | 'inProgress'
}

export type UpdateProgress =
  | { phase: 'downloading'; version: string; downloaded: number; total: number | null }
  | { phase: 'installing'; version: string; closesApp: boolean }

export interface UpdateReady {
  version: string
  currentVersion: string
  installed: boolean
  closesApp: boolean
}

export interface AutoInstall {
  enabled: boolean
  supported: boolean
}

export async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core')
  return invoke<T>(command, args)
}

/** True only in a desktop build that carries the updater (every shipped
 *  one). The web build and the preview shell answer false. */
export async function updaterAvailable(): Promise<boolean> {
  if (!isTauri) return false
  try {
    return (await call<boolean>('updater_available')) === true
  } catch {
    return false
  }
}

export const updateInfo = (): Promise<UpdateInfo> => call<UpdateInfo>('update_info')
export const updateInstall = (): Promise<InstallResult> => call<InstallResult>('update_install')
export const updateSkip = (): Promise<void> => call<void>('update_skip')
export const updateRemind = (): Promise<void> => call<void>('update_remind')
export const updateRestart = (): Promise<RestartOutcome> => call<RestartOutcome>('update_restart')
export const updateCopyCommand = (): Promise<void> => call<void>('update_copy_command')
export const updateShowNotes = (): Promise<void> => call<void>('update_show_notes')
export const updateReadyGet = (): Promise<UpdateReady | null> => call<UpdateReady | null>('update_ready_get')
export const updateAutoGet = (): Promise<AutoInstall> => call<AutoInstall>('update_auto_get')
export const updateAutoSet = (enabled: boolean): Promise<AutoInstall> =>
  call<AutoInstall>('update_auto_set', { enabled })

/** Subscribe to a shell event; resolves to the unsubscribe function. */
export async function onEvent<T>(name: string, handler: (payload: T) => void): Promise<() => void> {
  const { listen } = await import('@tauri-apps/api/event')
  return listen<T>(name, (event) => handler(event.payload))
}
