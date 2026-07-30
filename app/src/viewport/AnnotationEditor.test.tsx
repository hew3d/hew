import { render, screen, fireEvent } from '@testing-library/react'
import { describe, it, expect, vi } from 'vitest'
import { AnnotationEditor } from './AnnotationEditor'

describe('AnnotationEditor', () => {
  it('is prefilled with initialText and positioned at screenX/screenY', () => {
    const { container } = render(
      <AnnotationEditor
        screenX={100}
        screenY={200}
        initialText="3.500 m"
        onCommit={() => {}}
        onCancel={() => {}}
      />,
    )
    const input = screen.getByDisplayValue('3.500 m') as HTMLInputElement
    expect(input).toBeInTheDocument()
    expect((container.firstChild as HTMLElement).style.left).toBe('100px')
    expect((container.firstChild as HTMLElement).style.top).toBe('200px')
  })

  it('Enter commits the current field value, not the initial one', () => {
    const onCommit = vi.fn()
    const onCancel = vi.fn()
    render(<AnnotationEditor screenX={0} screenY={0} initialText="" onCommit={onCommit} onCancel={onCancel} />)
    const input = screen.getByRole('textbox')
    fireEvent.change(input, { target: { value: 'Note here' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(onCommit).toHaveBeenCalledWith('Note here')
    expect(onCancel).not.toHaveBeenCalled()
  })

  it('Escape cancels and discards the typed text', () => {
    const onCommit = vi.fn()
    const onCancel = vi.fn()
    render(<AnnotationEditor screenX={0} screenY={0} initialText="" onCommit={onCommit} onCancel={onCancel} />)
    const input = screen.getByRole('textbox')
    fireEvent.change(input, { target: { value: 'discard me' } })
    fireEvent.keyDown(input, { key: 'Escape' })
    expect(onCancel).toHaveBeenCalledTimes(1)
    expect(onCommit).not.toHaveBeenCalled()
  })

  it('blur commits (clicking away saves rather than silently discarding)', async () => {
    const onCommit = vi.fn()
    const onCancel = vi.fn()
    render(<AnnotationEditor screenX={0} screenY={0} initialText="" onCommit={onCommit} onCancel={onCancel} />)
    const input = screen.getByRole('textbox')
    fireEvent.change(input, { target: { value: 'typed then blurred' } })
    // A REAL blur (well after mount, past the one-frame grace window — see
    // the next test) commits.
    await new Promise((r) => requestAnimationFrame(r))
    fireEvent.blur(input)
    expect(onCommit).toHaveBeenCalledWith('typed then blurred')
    expect(onCancel).not.toHaveBeenCalled()
  })

  it('ignores a blur that fires in the same frame as mount (the canvas-refocus race)', () => {
    // The editor mounts at the tail of a canvas click gesture; the canvas can
    // re-claim focus a tick after this component's own mount-time focus()
    // call, firing a spurious blur before the user has touched anything.
    // Committing an EMPTY value there would silently close the editor with
    // no annotation created and no visible error — see AnnotationEditor.tsx's
    // `readyForBlurRef` doc comment.
    const onCommit = vi.fn()
    const onCancel = vi.fn()
    render(<AnnotationEditor screenX={0} screenY={0} initialText="" onCommit={onCommit} onCancel={onCancel} />)
    const input = screen.getByRole('textbox')
    fireEvent.blur(input)
    expect(onCommit).not.toHaveBeenCalled()
    expect(onCancel).not.toHaveBeenCalled()
  })

  it('Escape then blur only cancels once (settled guard)', () => {
    const onCommit = vi.fn()
    const onCancel = vi.fn()
    render(<AnnotationEditor screenX={0} screenY={0} initialText="" onCommit={onCommit} onCancel={onCancel} />)
    const input = screen.getByRole('textbox')
    fireEvent.keyDown(input, { key: 'Escape' })
    fireEvent.blur(input)
    expect(onCancel).toHaveBeenCalledTimes(1)
    expect(onCommit).not.toHaveBeenCalled()
  })
})
