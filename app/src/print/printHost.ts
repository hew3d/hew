/**
 * Shell seam for the system print dialog (docs/design/printing.md §9) —
 * the `fileHost.ts` triple pattern: `makePrintHost()` picks the Tauri host
 * (dynamic import, so `@tauri-apps/*` stays out of the web bundle) or the
 * web host (`window.print()`).
 */
import { isTauri } from '../io/fileHost'

export interface PrintSetup {
  paperWmm: number
  paperHmm: number
  landscape: boolean
  jobTitle: string
}

export interface PrintHost {
  /**
   * Open the OS print dialog for the document currently in the DOM (the
   * print root). Resolves once the dialog has been handed off — on macOS
   * the sheet is asynchronous, so resolution does not mean "printed".
   */
  print(setup: PrintSetup): Promise<void>
  /** The OS default paper (desktop), to seed the paper preference once;
   * null when the platform can't say (the locale rule applies then). */
  defaults?(): Promise<{ paperWmm: number; paperHmm: number; landscape: boolean } | null>
  /** A second route when the native dialog fails (desktop: the webview's
   * own print dialog) — "Open the browser print dialog instead". */
  fallback?: PrintHost
  /** Physical millimetres per CSS pixel on this display, so the preview's
   * 100 % is a true 100 %; null when unknown (the CSS 96 dpi rule stands). */
  screenMmPerPx?(): Promise<number | null>
}

export class WebPrintHost implements PrintHost {
  async print(): Promise<void> {
    if (typeof window === 'undefined' || typeof window.print !== 'function') throw new Error('Printing is not available here')
    window.print()
  }
}

export function makePrintHost(): PrintHost {
  if (isTauri) {
    const hostPromise = import('./tauriPrintHost').then((m) => new m.TauriPrintHost())
    return {
      print: (setup) => hostPromise.then((h) => h.print(setup)),
      defaults: () => hostPromise.then((h) => h.defaults()),
      screenMmPerPx: () => hostPromise.then((h) => h.screenMmPerPx()),
      fallback: new WebPrintHost(),
    }
  }
  return new WebPrintHost()
}
