// Types for po.mjs, so the tests that import it typecheck.

export interface PoEntry {
  msgctxt: string
  msgid: string
  msgstr: string
  forms: string[]
  untranslated: boolean
}

export function parsePo(text: string): PoEntry[]
export function poLookup(entries: PoEntry[]): Map<string, string>
export function catalogCoverage(entries: PoEntry[]): {
  total: number
  translated: number
  missing: PoEntry[]
  percent: number
}
export function coverageReport(opts: {
  locales: readonly string[]
  read: (locale: string) => string
  list?: boolean
}): { lines: { text: string; error: boolean }[]; failed: boolean }
