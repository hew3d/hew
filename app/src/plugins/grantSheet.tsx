/* oxlint-disable lingui/no-unlocalized-strings -- plugin permission copy; English only until the feature graduates, and stays out of the catalogs. */
/**
 * Widening a plugin's optional permissions, asked of the person.
 *
 * `requestGrant` is the one path both askers take: a person turning a switch
 * on in Settings > Plugins, and (on the web) a running plugin that asked for
 * more through `hew.permissions.request`. On the desktop the shell asks in
 * its own native confirmation (`plugin_set_grant`); on the web this shows the
 * consent sheet in its grant mode, with the lines the kernel wrote
 * (`plugin_grant_lines`; no permission text is composed here), and writes the
 * grant only after "Allow". Cancel has focus, Enter accepts nothing and
 * Escape cancels. The answer is read from the record as stored afterwards,
 * never assumed: a "no" leaves it as it was.
 *
 * The sheet is mounted on its own, so it shows over a document window as well
 * as over Settings. Only one is open at a time in a window.
 */

import { createElement } from 'react'
import { createRoot } from 'react-dom/client'
import { ConsentSheet } from './ConsentSheet'
import { grantCapable, pluginInstaller } from './installer'
import { PluginError } from './pluginError'
import { displayName } from './pluginText'
import type { GrantDecider } from './storeAccess'
import type { ConsentPayload, PermissionLine, PluginInstaller } from './types'
import { recordFingerprint } from './webPluginStore'

export type GrantOutcome = 'granted' | 'declined'

let sheetOpen = false

/** Shows the grant sheet; resolves `true` on "Allow", `false` on a cancel. */
function askOnSheet(consent: ConsentPayload, grant: { added: PermissionLine[]; warnings: string[] }): Promise<boolean> {
  return new Promise((resolve) => {
    const box = document.createElement('div')
    document.body.appendChild(box)
    const root = createRoot(box)
    const finish = (answer: boolean): void => {
      sheetOpen = false
      root.unmount()
      box.remove()
      resolve(answer)
    }
    sheetOpen = true
    root.render(createElement(ConsentSheet, { consent, grant, onInstall: () => finish(true), onCancel: () => finish(false) }))
  })
}

/**
 * Asks whether plugin `id` may also have the optional permissions `add` and
 * writes the wider grant when the person says yes. Rejects (a `PluginError`)
 * when the plugin is not installed or does not declare one of the names,
 * when another question is open, when the record changed while the person
 * was deciding, or where there is nobody to ask.
 */
export async function requestGrant(inst: PluginInstaller, id: string, add: string[]): Promise<GrantOutcome> {
  const cap = grantCapable(inst)
  if (cap === null) throw new PluginError('unsupported', 'Permissions cannot be changed here.')
  const row = (await inst.list()).find((l) => l.kind === 'installed' && l.installed.id === id)
  if (row === undefined || row.kind !== 'installed') throw new PluginError('not_installed', 'The plugin is not installed.')
  const declared = row.manifest.optionalPermissions ?? []
  if (add.some((n) => !declared.includes(n))) {
    throw new PluginError('unknown_permission', 'This plugin does not declare one of the permissions asked for.')
  }
  const held = row.installed.optional_granted
  const optional = [...new Set([...held, ...add])]
  const grant = [...row.installed.grant]
  const holdsAll = (granted: readonly string[]): boolean => add.every((n) => granted.includes(n))
  if (holdsAll(held)) return 'granted'

  if (inst.tier === 'desktop') {
    // The shell asks, and answers with the record as it stands.
    return holdsAll((await cap.setGrant(id, grant, optional)).optional_granted) ? 'granted' : 'declined'
  }

  if (sheetOpen) throw new PluginError('confirmation_open', 'Another confirmation is already open. Answer it first, then try again.')
  if (cap.grantLines === undefined) throw new PluginError('unsupported', 'This build of Hew cannot say what a permission would allow.')
  const lines = await cap.grantLines(id, grant, optional)
  const consent: ConsentPayload = {
    id,
    name: displayName(row.manifest),
    version: row.installed.version,
    permissions: [],
    optional: [],
    warnings: [],
    on_startup: false,
    activation_reason: null,
    signer: null,
    unsigned: false,
    source: row.installed.source,
    replaces: null,
    update: null,
  }
  if (sheetOpen) throw new PluginError('confirmation_open', 'Another confirmation is already open. Answer it first, then try again.')
  if (!(await askOnSheet(consent, lines))) return 'declined'
  // What was shown is what is written, or nothing is.
  const now = (await inst.list()).find((l) => l.kind === 'installed' && l.installed.id === id)
  if (now === undefined || now.kind !== 'installed' || recordFingerprint(now.installed) !== recordFingerprint(row.installed)) {
    throw new PluginError('plugin_changed', 'This plugin changed while you were deciding. Check it, then try again.')
  }
  return holdsAll((await cap.setGrant(id, grant, optional)).optional_granted) ? 'granted' : 'declined'
}

/** The web's answer to a running plugin that asks for more
 *  (`hew.permissions.request`): the sheet, over the document window. */
export function decideGrant(id: string, add: string[]): Promise<GrantOutcome> {
  return requestGrant(pluginInstaller(), id, add)
}

/** `decideGrant` as the host's `GrantDecider`. */
export const webGrantDecider: GrantDecider = { decide: decideGrant }
