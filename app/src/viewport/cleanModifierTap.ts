/**
 * cleanModifierTap — pure decision logic for a "clean tap" of a modifier key
 * (Control/Meta), scoped to whichever tool instance was active when the key
 * went down. Pure data in, pure data out — unit-testable directly.
 *
 * Two independent features share this shape at window scope: Scale's
 * `toggleCenterAnchor` and Push/Pull's `toggleExtrudeAsNew` both fire on a
 * bare Ctrl/Meta tap — pressed and released with no other key in between (a
 * bare Control/Meta keydown reports ctrlKey/metaKey: true on itself, so
 * Viewport's generic modifier-combo key path never carries it, and toggling
 * on the leading keydown would also fire on every Ctrl chord — Ctrl+Z,
 * Ctrl+A, …). Both listeners live at window scope and both would otherwise
 * arm on the SAME bare keydown regardless of which tool is active. Without
 * scoping, holding Ctrl while Scale is active, clicking over to Push/Pull,
 * then releasing Ctrl would fire Push/Pull's toggle (and the reverse fires
 * Scale's) — a tap started on one tool leaking its release onto another.
 *
 * `CleanModifierTap` fixes this structurally: `onKeyDown` records which tool
 * instance was active when the tap armed; `onKeyUp` only reports a fired tap
 * if that SAME instance is still active. `reset()` additionally clears the
 * arm on any tool switch, covering switches that happen through a path other
 * than the normal keyup (e.g. a drag-to-move handoff mid-hold).
 */
export class CleanModifierTap<Tool> {
  private clean = false
  private tool: Tool | null = null

  /** `matchesKey` identifies the modifier this tap watches (e.g. `key ===
   *  'Control'`, or `key === 'Control' || key === 'Meta'`). */
  constructor(private readonly matchesKey: (key: string) => boolean) {}

  /** Call from a window `keydown` listener with the CURRENTLY active tool. */
  onKeyDown(ev: { key: string; repeat: boolean }, activeTool: Tool): void {
    if (this.matchesKey(ev.key)) {
      if (!ev.repeat) {
        this.clean = true
        this.tool = activeTool
      }
      return
    }
    this.clean = false // another key joined the press → it's a chord, not a tap
    this.tool = null
  }

  /**
   * Call from a window `keyup` listener with the CURRENTLY active tool.
   * Returns the tool instance the tap should fire against, or null if this
   * keyup doesn't resolve an armed clean tap, or the active tool changed
   * between the arming keydown and this keyup.
   */
  onKeyUp(ev: { key: string }, activeTool: Tool): Tool | null {
    if (!this.matchesKey(ev.key) || !this.clean) return null
    this.clean = false
    const armedTool = this.tool
    this.tool = null
    return armedTool === activeTool ? armedTool : null
  }

  /** Clear the arm — call on every tool switch, including ones that bypass
   *  the normal switch path. */
  reset(): void {
    this.clean = false
    this.tool = null
  }
}
