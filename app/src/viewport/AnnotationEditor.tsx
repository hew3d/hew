/**
 * AnnotationEditor — the lightweight in-viewport text editor for creating a
 * new leader text (TextTool's second click) and for double-click-editing an
 * existing annotation (a leader's content, or a dimension's `text_override`
 * — docs/design/dimensions-text.md "Tools & UX"). A single positioned DOM
 * `<input>`, the `InferenceTooltip` cursor-anchored DOM-positioning pattern
 * (container-relative `screenX`/`screenY`), NOT an in-scene billboard — text
 * entry needs a real text-input widget (IME composition, selection, paste),
 * which a canvas quad can't provide.
 *
 * Enter (or blur — clicking away commits rather than silently discarding
 * typed text) calls `onCommit` with the current field value; Esc calls
 * `onCancel` and discards it. The caller decides what an empty commit means
 * (TextTool: no annotation created; an existing dimension's override: empty
 * clears the override back to the computed value, per the design doc's
 * simplified SketchUp `<>` semantics).
 */
import { useEffect, useRef, useState } from 'react'

export interface AnnotationEditorProps {
  /** Container-relative CSS px, top-left origin (matches InferenceTooltip). */
  screenX: number
  screenY: number
  initialText: string
  placeholder?: string
  onCommit: (text: string) => void
  onCancel: () => void
}

export function AnnotationEditor({ screenX, screenY, initialText, placeholder, onCommit, onCancel }: AnnotationEditorProps) {
  const [text, setText] = useState(initialText)
  const inputRef = useRef<HTMLInputElement>(null)
  // Committed via a ref (not just relying on onBlur's closure) so Enter and a
  // subsequent blur (Enter typically blurs the field too, depending on the
  // browser) can't double-commit or race with Escape's cancel.
  const settledRef = useRef(false)
  // The mount that places this editor is itself the tail of the SAME click
  // gesture (a tool's second click, or a double-click) that is still
  // resolving focus: the canvas can re-claim focus a tick after this
  // component's own `el.focus()` call, firing a spurious blur before the
  // user has touched anything. Ignore `onBlur` until one animation frame
  // after mount so only a REAL user-initiated blur (always many frames
  // later) commits — Enter/Escape are unaffected, they work immediately.
  const readyForBlurRef = useRef(false)

  useEffect(() => {
    const el = inputRef.current
    if (el === null) return
    el.focus()
    el.select()
    const raf = requestAnimationFrame(() => {
      readyForBlurRef.current = true
    })
    return () => cancelAnimationFrame(raf)
  }, [])

  const commit = (value: string): void => {
    if (settledRef.current) return
    settledRef.current = true
    onCommit(value)
  }
  const cancel = (): void => {
    if (settledRef.current) return
    settledRef.current = true
    onCancel()
  }

  return (
    <input
      ref={inputRef}
      data-testid="annotation-editor"
      value={text}
      placeholder={placeholder}
      onChange={(ev) => setText(ev.target.value)}
      onKeyDown={(ev) => {
        // Stop every key from reaching the viewport's global shortcut/tool
        // routing while this field owns keyboard focus (mirrors how a tool's
        // `capturingInput()` shields VCB entry, but for a real DOM input).
        ev.stopPropagation()
        if (ev.key === 'Enter') {
          ev.preventDefault()
          commit(text)
        } else if (ev.key === 'Escape') {
          ev.preventDefault()
          cancel()
        }
      }}
      onBlur={() => {
        if (readyForBlurRef.current) commit(text)
      }}
      style={{
        position: 'absolute',
        left: `${screenX}px`,
        top: `${screenY}px`,
        transform: 'translate(-50%, -50%)',
        minWidth: '80px',
        padding: '3px 7px',
        background: 'var(--surface-overlay)',
        border: '1px solid var(--accent-base, #4d90ff)',
        borderRadius: '5px',
        fontFamily: 'var(--font-family-ui)',
        fontSize: '12px',
        color: 'var(--text-primary)',
        zIndex: 30,
      }}
    />
  )
}
