/**
 * Build-time version stamping, imported by `vite.config.ts` (and unit-tested).
 *
 * Only the CI job that deploys the beta web app sets `HEW_BETA_SHA`; release,
 * local dev, E2E and self-hosting builds never do, so their version string is
 * exactly package.json's. The beta stamp is semver build metadata
 * (`1.3.1+beta.18eb47ad`), which keeps equal precedence with the plain version.
 */

/** Semver build-metadata identifiers are [0-9A-Za-z-]; dots are the separator. */
const SHA_RE = /^[0-9A-Za-z-]+$/

export interface BuildStamp {
  /** The value for `__HEW_VERSION__`. */
  version: string
  /** The value for `__HEW_BETA__`. */
  beta: boolean
}

/**
 * Resolves the version string and beta flag from package.json's version and
 * the raw `HEW_BETA_SHA` env value. Unset, empty and whitespace-only all mean
 * "not a beta build". Anything else must be a valid build-metadata identifier;
 * otherwise this throws so the build fails instead of emitting an invalid
 * version.
 */
export function resolveBuildStamp(pkgVersion: string, betaSha: string | undefined): BuildStamp {
  const sha = betaSha?.trim() ?? ''
  if (sha === '') return { version: pkgVersion, beta: false }
  if (!SHA_RE.test(sha)) {
    throw new Error(
      `HEW_BETA_SHA must contain only letters, digits and hyphens (got ${JSON.stringify(sha)}); ` +
        'it becomes semver build metadata in the version string.',
    )
  }
  return { version: `${pkgVersion}+beta.${sha}`, beta: true }
}
