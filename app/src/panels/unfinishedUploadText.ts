import { plural, t } from '@lingui/core/macro'

/** Why a report or attachment could not start: an earlier upload from the
 *  same network never finished and still holds its share of the service.
 *  `retryAfter` is the service's wait in seconds, when it sent one. */
export function unfinishedUploadText(retryAfter: number | undefined): string {
  if (retryAfter === undefined) return t`An earlier upload from your network hasn’t finished. Try again in a few minutes.`
  const minutes = Math.ceil(retryAfter / 60)
  return plural(minutes, {
    one: 'An earlier upload from your network hasn’t finished. Try again in about # minute.',
    other: 'An earlier upload from your network hasn’t finished. Try again in about # minutes.',
  })
}
