/* oxlint-disable lingui/no-unlocalized-strings -- plugin install copy is English only until the feature graduates, and stays out of the catalogs. */
/**
 * Plain-English lines for each permission a plugin can ask for. The same
 * words as `permission_line` in `crates/plugin-host/src/consent.rs`; a test
 * reads that file and fails if the two drift. A name this table does not
 * know falls back to the text the payload carried.
 */

import { cleanText } from './pluginText'

const FIXED: Record<string, string> = {
  'model.read': 'Read the model: every object, component, material, tag, and attribute',
  'model.write': 'Change the model: draw, move, delete, and edit geometry',
  'attributes.write': 'Store its own data on parts, materials, and the document',
  view: 'Change the selection, hide and show parts, and move the camera',
  'documents.save': 'Save, export, and import files',
  'documents.discard': 'Close or revert documents',
  'documents.other': 'Act on every open document, not only the current one',
  files: 'Ask you to pick files, and save files you choose',
  clipboard: 'Read and write the clipboard',
  library: 'Add to and change your Library',
}

/** The line for a permission named as the manifest writes it
 *  (`model.read`, `network:api.example.com`, `attributes.write:com.x`). */
export function permissionText(name: string, fallback?: string): string {
  const fixed = FIXED[name]
  if (fixed !== undefined) return fixed
  if (name.startsWith('attributes.write:')) {
    return `Store data under the "${name.slice('attributes.write:'.length)}" namespace`
  }
  if (name.startsWith('network:')) {
    return `Connect to ${name.slice('network:'.length)} (checked by Hew)`
  }
  return fallback !== undefined ? cleanText(fallback, 200) : cleanText(name, 200)
}

/** Every fixed line, for the drift test. */
export const FIXED_PERMISSION_TEXT: Readonly<Record<string, string>> = FIXED
