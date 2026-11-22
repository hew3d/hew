/**
 * Build-time version stamping, imported by `vite.config.ts` (and unit-tested).
 *
 * Only the CI job that deploys the beta web app sets `HEW_BETA_SHA`; release,
 * local dev, E2E and self-hosting builds never do, so their version string is
 * exactly package.json's. The beta stamp is semver build metadata
 * (`1.3.1+beta.18eb47ad`), which keeps equal precedence with the plain version.
 */

/** A release version: `1.5.0` or `1.5.0-pre.2`, no build metadata and no
 *  leading zeros in the numbers (the kernel's semver parser refuses them). */
const RELEASE_RE = /^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/

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
export function resolveBuildStamp(
  pkgVersion: string,
  betaSha: string | undefined,
  releaseVersion?: string,
): BuildStamp {
  // The release workflow stamps the tag's version (`HEW_RELEASE_VERSION`),
  // because a preview tag does not edit package.json: without it a
  // `1.5.0-pre.2` build would report the last release's version, and the
  // plugin host would refuse every plugin written for the preview.
  const stamped = releaseVersion?.trim() ?? ''
  if (stamped !== '') {
    if (!RELEASE_RE.test(stamped)) {
      throw new Error(
        `HEW_RELEASE_VERSION must look like 1.5.0 or 1.5.0-pre.2 (got ${JSON.stringify(stamped)}).`,
      )
    }
    pkgVersion = stamped
  }
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
