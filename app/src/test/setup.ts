/**
 * Vitest setup for the component-test harness. Loaded via `setupFiles`, so
 * it runs for **every** suite — node (`.test.ts`) and jsdom (`.test.tsx`) alike —
 * and must be a no-op for the node ones.
 *
 * It does three things:
 *  1. Registers `@testing-library/jest-dom`'s matchers (`toBeInTheDocument`, …).
 *  2. Provides an in-memory `localStorage`/`sessionStorage`. Neither Node 26 nor
 *     jsdom 25 exposes Web Storage by default, so components that persist settings
 *     would otherwise hit `undefined`. This centralizes the `FakeStorage` pattern
 *     the pure-logic suites each rolled by hand (e.g. settings/debugMode.test.ts);
 *     those still install + restore their own stub, which now restores to this
 *     shared one instead of `undefined`.
 *  3. Auto-unmounts React trees after each test (jsdom only) so component suites
 *     don't leak DOM into one another.
 */
import '@testing-library/jest-dom/vitest'
import { afterEach } from 'vitest'

class MemoryStorage implements Storage {
  private store = new Map<string, string>()
  get length(): number {
    return this.store.size
  }
  clear(): void {
    this.store.clear()
  }
  getItem(key: string): string | null {
    return this.store.has(key) ? this.store.get(key)! : null
  }
  key(index: number): string | null {
    return [...this.store.keys()][index] ?? null
  }
  removeItem(key: string): void {
    this.store.delete(key)
  }
  setItem(key: string, value: string): void {
    this.store.set(key, String(value))
  }
}

const g = globalThis as { localStorage?: Storage; sessionStorage?: Storage }
if (g.localStorage === undefined) g.localStorage = new MemoryStorage()
if (g.sessionStorage === undefined) g.sessionStorage = new MemoryStorage()

// React unmount only applies where there's a DOM; importing @testing-library/react
// is deferred so the node suites never load react-dom.
afterEach(async () => {
  if (typeof document !== 'undefined') {
    const { cleanup } = await import('@testing-library/react')
    cleanup()
  }
})
