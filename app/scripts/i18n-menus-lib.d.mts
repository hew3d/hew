export interface NativeMenuRow {
  key: string
  context: string
  message: string
}
export const APP_DIR: string
export const MENU_LOCALES_DIR: string
export function decodeTemplateEscapes(text: string): string
export function readNativeMenuTable(source: string): NativeMenuRow[]
export function readTableRows(): NativeMenuRow[]
export function labelsFor(rows: NativeMenuRow[], lookup: Map<string, string> | null): Record<string, string>
export function render(labels: Record<string, string>): string
export function menuFileFor(rows: NativeMenuRow[], locale: string): string
