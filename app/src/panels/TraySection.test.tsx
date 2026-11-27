import { render, screen, fireEvent, act } from '@testing-library/react'
import { describe, it, expect, vi } from 'vitest'
import { TraySection } from './TraySection'

describe('TraySection', () => {
  it('renders the title and children when expanded', () => {
    render(
      <TraySection title="Materials" collapsed={false} onToggle={vi.fn()}>
        <p>swatches here</p>
      </TraySection>,
    )
    expect(screen.getByText('Materials')).toBeInTheDocument()
    expect(screen.getByText('swatches here')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /materials/i })).toHaveAttribute('aria-expanded', 'true')
  })

  it('hides children when collapsed', () => {
    render(
      <TraySection title="Materials" collapsed onToggle={vi.fn()}>
        <p>swatches here</p>
      </TraySection>,
    )
    expect(screen.queryByText('swatches here')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /materials/i })).toHaveAttribute('aria-expanded', 'false')
  })

  it('clicking the header calls onToggle', () => {
    const onToggle = vi.fn()
    render(
      <TraySection title="Materials" collapsed={false} onToggle={onToggle}>
        <p>swatches here</p>
      </TraySection>,
    )
    fireEvent.click(screen.getByRole('button', { name: /materials/i }))
    expect(onToggle).toHaveBeenCalledOnce()
  })
})

// jsdom has no layout; use a measured starting section height for drag deltas.
function measureSection(handle: HTMLElement, height = 240) {
  const section = handle.parentElement!
  vi.spyOn(section, 'getBoundingClientRect').mockReturnValue({ height } as DOMRect)
  return section
}

function pointer(target: Element | Window, type: string, y: number, pointerId = 1) {
  const event = new MouseEvent(type, { bubbles: true, clientY: y, button: 0 })
  Object.defineProperty(event, 'pointerId', { value: pointerId })
  fireEvent(target, event)
}

describe('TraySection resizing', () => {
  it('keeps automatic sizing until resized, then grows and shrinks from its measured height', () => {
    render(<TraySection title="Outliner" collapsed={false} onToggle={vi.fn()}>tree</TraySection>)
    const handle = screen.getByRole('separator', { name: /Outliner/ })
    const section = measureSection(handle)
    expect(section.style.flex).toBe('1 1 0px')
    fireEvent.keyDown(handle, { key: 'ArrowDown' })
    expect(section.style.height).toBe('260px')
    fireEvent.keyDown(handle, { key: 'ArrowUp' })
    expect(section.style.height).toBe('240px')
    expect(handle).toHaveAttribute('aria-orientation', 'horizontal')
    expect(handle).toHaveAttribute('aria-valuenow', '240')
    expect(handle).toHaveAttribute('tabindex', '0')
  })

  it('drags independently, clamps to a usable minimum, and stops on cancellation', () => {
    render(<><TraySection title="Outliner" collapsed={false} onToggle={vi.fn()}>tree</TraySection>
      <TraySection title="Tags" collapsed={false} onToggle={vi.fn()}>tags</TraySection></>)
    const handle = screen.getByRole('separator', { name: /Outliner/ })
    const section = measureSection(handle)
    const tags = screen.getByRole('separator', { name: /Tags/ }).parentElement!
    pointer(handle, 'pointerdown', 100)
    pointer(window, 'pointermove', 200, 2)
    expect(section.style.height).toBe('')
    pointer(window, 'pointermove', 200)
    expect(section.style.height).toBe('340px')
    expect(tags.style.height).toBe('')
    pointer(window, 'pointermove', 100000)
    expect(section.style.height).toBe('10000px')
    expect(handle).toHaveAttribute('aria-valuemax', '10000')
    pointer(window, 'pointermove', -500)
    expect(Number.parseFloat(section.style.height)).toBeGreaterThanOrEqual(100)
    const minimum = section.style.height
    pointer(window, 'pointercancel', -500)
    pointer(window, 'pointermove', 500)
    expect(section.style.height).toBe(minimum)
  })

  it('retains the chosen height after collapse and removes an active drag on collapse or unmount', () => {
    const renderSection = (collapsed: boolean) => <TraySection title="Outliner" collapsed={collapsed} onToggle={vi.fn()}>tree</TraySection>
    const { rerender, unmount } = render(renderSection(false))
    let handle = screen.getByRole('separator')
    const section = measureSection(handle)
    pointer(handle, 'pointerdown', 100)
    pointer(window, 'pointermove', 160)
    expect(section.style.height).toBe('300px')
    rerender(renderSection(true))
    expect(screen.queryByRole('separator')).not.toBeInTheDocument()
    pointer(window, 'pointermove', 500)
    rerender(renderSection(false))
    expect(section.style.height).toBe('300px')
    handle = screen.getByRole('separator')
    pointer(handle, 'pointerdown', 100)
    const remove = vi.spyOn(window, 'removeEventListener')
    unmount()
    expect(remove).toHaveBeenCalledWith('pointermove', expect.any(Function))
    remove.mockRestore()
  })
})

it.each(['pointerup', 'lostpointercapture', 'blur'])('stops a section drag on %s', (end) => {
  render(<TraySection title="Outliner" collapsed={false} onToggle={vi.fn()}>tree</TraySection>)
  const handle = screen.getByRole('separator')
  const section = measureSection(handle)
  pointer(handle, 'pointerdown', 100)
  pointer(window, 'pointermove', 150)
  expect(section.style.height).toBe('290px')
  if (end === 'blur') fireEvent(window, new Event('blur'))
  else pointer(end === 'lostpointercapture' ? handle : window, end, 150)
  pointer(window, 'pointermove', 250)
  expect(section.style.height).toBe('290px')
})


it('reports automatic height changes without locking the shared flex layout', () => {
  let notifyResize = () => {}
  vi.stubGlobal('ResizeObserver', class {
    constructor(callback: () => void) { notifyResize = callback }
    observe() {}
    disconnect() {}
  })
  try {
    render(<TraySection title="Outliner" collapsed={false} onToggle={vi.fn()}>tree</TraySection>)
    const handle = screen.getByRole('separator')
    const section = measureSection(handle, 240)
    act(() => notifyResize())
    expect(handle).toHaveAttribute('aria-valuenow', '240')
    vi.mocked(section.getBoundingClientRect).mockReturnValue({ height: 320 } as DOMRect)
    act(() => notifyResize())
    expect(handle).toHaveAttribute('aria-valuenow', '320')
    expect(section.style.height).toBe('')
    expect(section.style.flex).toBe('1 1 0px')
  } finally {
    vi.unstubAllGlobals()
  }
})
