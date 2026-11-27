// The words the Library dialog uses when something about a source goes
// wrong (docs/design/shared-library-sources.md 6.6): one message per
// condition, each saying what the user can do. Error codes never reach the
// screen as text (decision 28's neighbor: the lingui lint forbids it), so
// every code maps to a sentence here.
//
// Plain words for SketchUp users: "source", "catalog", "add". Never
// "repository", "index", "fetch" or "download".

import { t } from '@lingui/core/macro'
import { formatBytes } from './libraryModel'
import type { Failure, SourceErrorCode } from './sources'

export interface SourceErrorContext {
  /** The host the failed request went to, when the caller knows it. */
  host?: string | null
  /** True in the web build, which cannot tell a dead host from one that
   * sends no cross-origin header (6.6, `network`, web). */
  web?: boolean
  /** The browser reports it is offline. */
  offline?: boolean
  /** HTTP status, for `http_status`. */
  status?: number
  /** `source_removed`: the catalog's reason. */
  reason?: string
  /** `not_enough_space`: bytes needed and bytes free. */
  needed?: number
  free?: number
  /** `unavailable_other_source`: the name in the other note. */
  otherSource?: string
  /** `no_source`: each candidate's code. */
  causes?: SourceErrorCode[]
}

const NETWORK_LIKE: ReadonlySet<SourceErrorCode> = new Set<SourceErrorCode>(['network', 'timeout', 'offline'])

function couldNotReach(host: string | null | undefined): string {
  return host ? t`Hew could not reach ${host}. Check your connection and try again.` : t`Hew could not reach this site. Check your connection and try again.`
}

/** The sentence for one condition. */
export function sourceErrorMessage(code: SourceErrorCode, ctx: SourceErrorContext = {}): string {
  switch (code) {
    case 'offline':
      return t`You are offline. Your Library still works.`
    case 'network':
      if (ctx.offline === true) return t`You are offline. Your Library still works.`
      return ctx.web === true
        ? t`Hew could not read this site from the browser. The site may be down, or it may not allow web apps to read it. The desktop app can read it either way.`
        : couldNotReach(ctx.host)
    case 'timeout':
      return ctx.offline === true ? t`You are offline. Your Library still works.` : couldNotReach(ctx.host)
    case 'not_https':
      return t`Sources must use https.`
    case 'rate_limited': {
      const host = ctx.host
      return host ? t`${host} is limiting requests right now. Try again later.` : t`This site is limiting requests right now. Try again later.`
    }
    case 'too_many_redirects': {
      const host = ctx.host
      return host ? t`${host} keeps sending Hew somewhere else. Check the address and try again.` : t`This address keeps sending Hew somewhere else. Check the address and try again.`
    }
    case 'http_status': {
      if (ctx.status === 404 || ctx.status === 410) return t`This address does not have a Hew source.`
      const status = ctx.status
      const host = ctx.host
      // An entry stored before the status was kept has none: no number then.
      if (status === undefined) return host ? t`${host} answered with an error. Try again later.` : t`The site answered with an error. Try again later.`
      return host ? t`${host} answered with an error (${status}). Try again later.` : t`The site answered with an error (${status}). Try again later.`
    }
    case 'too_large':
      return t`This is bigger than Hew allows for a source.`
    case 'index_too_new':
      return t`This source needs a newer version of Hew.`
    case 'no_source': {
      const causes = ctx.causes ?? []
      if (causes.includes('index_too_new')) return t`This source needs a newer version of Hew.`
      if (causes.length > 0 && causes.every((c) => NETWORK_LIKE.has(c))) return sourceErrorMessage(causes[0], ctx)
      return t`This address does not have a Hew source.`
    }
    case 'ref_invalid':
    case 'resolve_invalid':
    case 'index_invalid':
    case 'index_too_large':
      return t`This address does not have a Hew source.`
    case 'catalog_invalid':
      return t`Hew could not read the catalog right now.`
    case 'catalog_disabled':
      return t`The catalog is not available.`
    case 'hash_mismatch':
      return t`This item changed while Hew was reading it. Try again in a few minutes.`
    case 'listing_incorrect':
    case 'file_invalid':
      return t`The source lists this item incorrectly. Hew did not add it.`
    case 'name_clash':
      return t`An item with the same file name is already in your Library.`
    case 'open_for_editing':
      return t`Open for editing. Close it and update again.`
    case 'not_enough_space': {
      const needed = formatBytes(ctx.needed ?? 0)
      const free = formatBytes(ctx.free ?? 0)
      return t`This source needs ${needed}. Your browser has ${free} free.`
    }
    case 'source_removed':
      const reason = ctx.reason
      return reason ? t`The Hew catalog removed this source: ${reason}` : t`The Hew catalog removed this source.`
    case 'unavailable_other_source': {
      const other = ctx.otherSource ?? ''
      return other ? t`This item is already in your Library from ${other}.` : t`This item is already in your Library from another source.`
    }
    case 'already_in_library':
      return t`This item is already in your Library.`
    case 'busy':
      return t`Another window is already working on this source.`
    case 'unknown_source':
      return t`This source is not in your list.`
    case 'not_checked':
      return t`Hew has not read this source yet. Press Refresh to check it.`
    case 'item_not_found':
    case 'not_found':
      return t`Hew could not find this item.`
    case 'not_updatable':
    case 'not_linked':
      return t`This item cannot be updated from this source.`
    case 'needs_confirm':
      return t`This needs your answer first.`
    case 'write_failed':
      return t`Hew could not save to your Library. Check that the folder is available and try again.`
  }
}

/** True when the browser reports it has no connection. */
export function browserOffline(): boolean {
  return typeof navigator !== 'undefined' && navigator.onLine === false
}

/** A message for the screen, and the catalog's link when it gave one. */
export interface SourceMessage {
  text: string
  /** An `https` link only; anything else is dropped. */
  link?: string
}

function httpsLink(link: string | undefined): string | undefined {
  if (link === undefined) return undefined
  try {
    return new URL(link).protocol === 'https:' ? link : undefined
  } catch {
    return undefined
  }
}

/** The message for a failed result, given what the caller knows about where
 * it happened (`host`) and which build it is (`web`). */
export function failureMessage(f: Failure, where: { host?: string | null; web: boolean }): SourceMessage {
  const ctx: SourceErrorContext = {
    host: where.host ?? null,
    web: where.web,
    offline: browserOffline(),
    ...(f.status !== undefined ? { status: f.status } : {}),
    ...(f.reason !== undefined ? { reason: f.reason } : {}),
    ...(f.needed !== undefined ? { needed: f.needed } : {}),
    ...(f.free !== undefined ? { free: f.free } : {}),
    ...(f.otherSource !== undefined ? { otherSource: f.otherSource } : {}),
    ...(f.causes !== undefined ? { causes: f.causes } : {}),
  }
  const link = f.code === 'source_removed' ? httpsLink(f.link) : undefined
  return { text: sourceErrorMessage(f.code, ctx), ...(link !== undefined ? { link } : {}) }
}

/** The one line the Sources block shows where sources are Off. */
export function sourcesOffText(): string {
  return t`Sources are turned off on this server.`
}

/** A landed update left the old copy behind (design 7.4): both files carry
 * the item until the user deletes the old one. */
export function removeFailedText(): string {
  return t`The item was updated, but Hew could not remove the old copy. Delete the old copy from your Library by hand.`
}
