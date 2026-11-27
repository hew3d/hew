/* oxlint-disable lingui/no-unlocalized-strings -- plugin install copy is English only until the feature graduates, and stays out of the catalogs. */
/**
 * Refusals from the install store. The shell reports `"<name>: <text>"`
 * (snake_case name); the kernel reports `"INSTALL_<NAME>: <text>"` or
 * `"MANIFEST_<NAME>: <text>"`. Either way the page keeps the name for
 * branching and shows only the text.
 */

export class PluginError extends Error {
  /** The refusal name, snake_case (`reserved_id`, `package_not_store_only`). */
  readonly refusal: string
  constructor(refusal: string, text: string) {
    super(text)
    this.name = 'PluginError'
    this.refusal = refusal
  }
}

const PREFIXED = /^([A-Za-z][A-Za-z0-9_]*): ([\s\S]*)$/

/** Turns whatever a command or the kernel threw into a `PluginError`. */
export function toPluginError(e: unknown): PluginError {
  if (e instanceof PluginError) return e
  const raw = typeof e === 'string' ? e : e instanceof Error ? e.message : String(e)
  const m = PREFIXED.exec(raw)
  if (m === null) return new PluginError('unknown', raw)
  let name = m[1]
  if (name === name.toUpperCase()) {
    name = name.replace(/^(INSTALL|MANIFEST)_/, '').toLowerCase()
  }
  return new PluginError(name, m[2])
}

/** The sentence to show a person: the text after the `name:` prefix. */
export function errorText(e: unknown): string {
  return toPluginError(e).message
}
