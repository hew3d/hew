/**
 * Detached mode — the draw tools' (Line, Rectangle, Circle, Polygon, Arc)
 * durable "keep the next shape apart from the face or drawing under it"
 * toggle (separate-shapes design §6.1). A clean tap of Ctrl/Cmd flips it
 * (Viewport-driven, like Push/Pull's extrude-as-new); it shows as a
 * "Detached ·" readout prefix, a status-bar sentence and a "+" cursor
 * badge (the `onChange` callback), and it sticks until tapped again or the tool changes.
 *
 * One small object per tool owns the flag, the callback to the cursor,
 * and the readout decoration, so the five tools stay identical.
 */

import { t } from '@lingui/core/macro'
import { DETACH_MODIFIER_LABEL } from '../platform'
import { PREVIEW_LINE_STYLE, type FatSegmentsOpts } from '../viewport/fatLine'
import {
  readPreviewTint,
  PREVIEW_DETACHED_TOKEN,
  PREVIEW_DETACHED_FALLBACK,
  PREVIEW_DRAW_JOINED_TOKEN,
  PREVIEW_DRAW_JOINED_FALLBACK,
} from './previewTint'

export type OnDetachedChange = (on: boolean) => void

export class DetachedMode {
  private _on = false
  /** The last UNdecorated readout text, so a toggle re-reports at once. */
  private lastText = ''

  constructor(
    private readonly sink: (text: string) => void,
    private readonly onChange: OnDetachedChange,
  ) {}

  get on(): boolean {
    return this._on
  }

  /** The tool's readout callback: routes every readout through the prefix. */
  readonly report = (text: string): void => {
    this.lastText = text
    this.sink(this.decorate(text))
  }

  /** Prefix "Detached ·" onto a non-empty readout while the mode is on. */
  decorate(text: string): string {
    if (!this._on || text === '') return text
    return t({
      comment: 'Measurements box readout while Detach is on, followed by the measured value',
      message: `Detached · ${text}`,
    })
  }

  /** The rubber-band style under the draw color language: orange (the
   *  "part of a solid's face" ink) only when the shape would join a face
   *  (`joinsFace`: the tool is anchored on a solid's face) and Detach is
   *  off; blue (a free drawing that pushes into a new object) otherwise,
   *  including on the ground and in the void. `detached` overrides the
   *  live toggle for a caller that has latched the mode (Line's chain). */
  previewStyle(joinsFace: boolean, detached: boolean = this._on): FatSegmentsOpts {
    const color =
      joinsFace && !detached
        ? readPreviewTint(PREVIEW_DRAW_JOINED_TOKEN, PREVIEW_DRAW_JOINED_FALLBACK)
        : readPreviewTint(PREVIEW_DETACHED_TOKEN, PREVIEW_DETACHED_FALLBACK)
    return { ...PREVIEW_LINE_STYLE, color }
  }

  /** Flip the mode, tell the cursor, and re-report the readout. */
  toggle(): void {
    this._on = !this._on
    this.onChange(this._on)
    if (this.lastText !== '') this.sink(this.decorate(this.lastText))
  }

  /** Back to off (a new or loaded document); tells the cursor. */
  reset(): void {
    if (!this._on) return
    this._on = false
    this.onChange(false)
    if (this.lastText !== '') this.sink(this.lastText)
  }

  /**
   * The status-bar sentence for the current stage. `idle` is "no gesture in
   * progress"; while on, the sentence says what the mode means, and while
   * off the idle hint gains the discoverable clause. `nextLine` is Line's
   * mid-chain wording (D12: a mid-chain tap applies to the next chain).
   */
  hint(base: string, idle: boolean, nextLine = false): string {
    if (this._on) {
      const tail = nextLine
        ? t`Detached applies to the next line.`
        : t`Detach is on — the shape stays apart from the face until you push it.`
      return `${base} ${tail}`
    }
    if (!idle) return base
    const key = DETACH_MODIFIER_LABEL
    return `${base} ${t`Tap ${key} to detach the shape from the face.`}`
  }
}
