// A job on a source (check, Add all, Follow, Update all) holds a Web Lock
// named for the source (`hew-library-source:<sourceRef>`), so two tabs do
// not run the same job (docs/design/shared-library-sources.md 7.3). On the
// desktop the one Library window is the only place a job runs, so there is
// no lock to take there.

/** `{ ran: true, value }` when `fn` ran to completion (or `fn` rejected:
 * the rejection propagates and the lock is released); `{ ran: false }`
 * when another holder already has the source's lock - `fn` is not called
 * and nothing waits. Different sources never block each other. */
export type SourceLockResult<T> = { ran: true; value: T } | { ran: false }

/* oxlint-disable-next-line lingui/no-unlocalized-strings -- Web Lock name, an identifier */
const LOCK_PREFIX = 'hew-library-source:'

export async function withSourceLock<T>(
  sourceRef: string,
  fn: () => Promise<T>,
): Promise<SourceLockResult<T>> {
  const locks = typeof navigator !== 'undefined' ? navigator.locks : undefined
  if (!locks || typeof locks.request !== 'function') {
    return { ran: true, value: await fn() }
  }
  return locks.request(`${LOCK_PREFIX}${sourceRef}`, { ifAvailable: true }, async (lock) => {
    if (lock === null) return { ran: false } as const
    return { ran: true, value: await fn() } as const
  })
}
