/**
 * The loaded kernel's plugin exports. They exist only in a preview kernel;
 * a kernel without them (a production module) gets a plain refusal rather
 * than an `undefined is not a function`.
 */

import { loadKernel } from '../wasm/loader'
import * as kernelModule from '../wasm/pkg/wasm_api.js'
import type { PluginWasm } from './webPluginStore'

const NAMES = [
  'plugin_package_from_zip',
  'plugin_consent',
  'plugin_manifest_parse',
  'plugin_hew_version_ok',
  'plugin_install_check',
  // What a widened grant would add, for the web's grant sheet.
  'plugin_grant_lines',
] as const

export async function pluginWasm(): Promise<PluginWasm> {
  await loadKernel()
  const mod = kernelModule as unknown as Record<string, unknown>
  for (const name of NAMES) {
    if (typeof mod[name] !== 'function') {
      throw new Error('This build of Hew cannot read plugin packages.')
    }
  }
  return mod as unknown as PluginWasm
}
