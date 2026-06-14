/**
 * consoleCapture — wrap console.error and console.warn to forward messages
 * into LogStore, while calling through to the original implementations.
 *
 * Call install() once at app startup.
 * Call restore() to undo (e.g., in test teardown or React effect cleanup).
 */

import { append } from './LogStore'

let originalError: typeof console.error | null = null
let originalWarn: typeof console.warn | null = null
let installed = false

/** Format console arguments to a single string (mirrors what browsers do). */
function argsToMessage(args: unknown[]): string {
  return args
    .map((a) => {
      if (typeof a === 'string') return a
      if (a instanceof Error) return `${a.name}: ${a.message}`
      try {
        return JSON.stringify(a)
      } catch {
        return String(a)
      }
    })
    .join(' ')
}

export function install(): void {
  if (installed) return
  installed = true

  originalError = console.error
  originalWarn = console.warn

  console.error = (...args: unknown[]) => {
    append('error', 'console', argsToMessage(args))
    originalError!(...args)
  }

  console.warn = (...args: unknown[]) => {
    append('warn', 'console', argsToMessage(args))
    originalWarn!(...args)
  }
}

export function restore(): void {
  if (!installed) return
  if (originalError !== null) {
    console.error = originalError
    originalError = null
  }
  if (originalWarn !== null) {
    console.warn = originalWarn
    originalWarn = null
  }
  installed = false
}
