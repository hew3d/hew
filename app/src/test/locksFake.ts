/**
 * In-memory stand-in for `navigator.locks` (the Web Locks API): exclusive
 * locks by name, `ifAvailable`, queued waiters, and `query()`.
 */

type Lock = { name: string; mode: 'exclusive' }
type Callback<T> = (lock: Lock | null) => T | Promise<T>

export function fakeLocks() {
  const held = new Set<string>()
  const waiting = new Map<string, Array<() => void>>()

  async function run<T>(name: string, cb: Callback<T>): Promise<T> {
    held.add(name)
    try {
      return await cb({ name, mode: 'exclusive' })
    } finally {
      held.delete(name)
      waiting.get(name)?.shift()?.()
    }
  }

  const locks = {
    async request<T>(
      name: string,
      a: { ifAvailable?: boolean } | Callback<T>,
      b?: Callback<T>,
    ): Promise<T> {
      const options = typeof a === 'function' ? {} : a
      const cb = (typeof a === 'function' ? a : b) as Callback<T>
      if (held.has(name)) {
        if (options.ifAvailable) return await cb(null)
        await new Promise<void>((resolve) => {
          const q = waiting.get(name) ?? []
          q.push(resolve)
          waiting.set(name, q)
        })
      }
      return run(name, cb)
    },
    async query() {
      return {
        held: [...held].map((name) => ({ name, mode: 'exclusive' })),
        pending: [],
      }
    },
  }
  return locks
}
