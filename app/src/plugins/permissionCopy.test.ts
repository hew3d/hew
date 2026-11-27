import { readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { beforeEach, describe, expect, it } from 'vitest'
import { requiresPreview } from '../test/requiresPreview'
import { FIXED_PERMISSION_TEXT, permissionText } from './permissionCopy'
import { UNSIGNED_LINE } from './ConsentSheet'

beforeEach((ctx) => requiresPreview(ctx, 'plugins'))

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../../..')
const consentRs = readFileSync(join(ROOT, 'crates/plugin-host/src/consent.rs'), 'utf8')

describe('permissionText', () => {
  it('gives each fixed permission its line', () => {
    expect(permissionText('model.read')).toBe('Read the model: every object, component, material, tag, and attribute')
    expect(permissionText('files')).toBe('Ask you to pick files, and save files you choose')
  })

  it('fills in the host and the namespace', () => {
    expect(permissionText('network:api.example.com')).toBe('Connect to api.example.com (checked by Hew)')
    expect(permissionText('attributes.write:com.x')).toBe('Store data under the "com.x" namespace')
  })

  it('falls back to the payload text, cleaned, then to the name', () => {
    expect(permissionText('future.thing', 'Do a‮ future thing')).toBe('Do a future thing')
    expect(permissionText('future.thing')).toBe('future.thing')
  })

  it('says the same words as the Rust table (consent.rs)', () => {
    for (const [name, text] of Object.entries(FIXED_PERMISSION_TEXT)) {
      expect(consentRs, name).toContain(`"${text}"`)
    }
    expect(consentRs).toContain('"Store data under the \\"{ns}\\" namespace"')
    expect(consentRs).toContain('"Connect to {host} (checked by Hew)"')
    expect(consentRs).toContain(`"${UNSIGNED_LINE}"`)
    // Every fixed line in the Rust table is in this one, so a new permission
    // cannot be added on one side only.
    const rustLines = [...consentRs.matchAll(/^\s*\("[a-z.]+", None\) =>\s*(?:\{\s*)?"([^"]+)"/gm)].map((m) => m[1])
    const ours = new Set(Object.values(FIXED_PERMISSION_TEXT))
    expect(rustLines.length).toBe(ours.size)
    for (const line of rustLines) expect(ours.has(line), line).toBe(true)
  })
})
