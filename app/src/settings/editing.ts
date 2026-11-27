/**
 * Editing settings — module-level singleton for the Settings ▸ Editing pane.
 *
 * "Copies are linked" (default on): a copy made with Move, Rotate, Paste, or
 * an array shares its shape with the original, so editing one changes all.
 * Off swaps the default — copies are independent and a linked copy is the
 * explicit choice each time. Every gesture reads it at the moment of the
 * copy; the bus does not.
 *
 * Persistence and cross-window sync come from the shared factory in
 * `view.ts`: localStorage under `hew.settings.copiesLinked`, the Tauri
 * 'settings-changed' event under the payload key `copiesLinked`, and the
 * browser 'storage' event for web tabs.
 */

import { makeSetting, parseBool } from './view'

const copiesLinked = makeSetting<boolean>(
  'hew.settings.copiesLinked', 'copiesLinked', true, parseBool, String,
)
export const getCopiesLinked = copiesLinked.get
export const setCopiesLinked = copiesLinked.set
export const subscribeCopiesLinked = copiesLinked.subscribe
