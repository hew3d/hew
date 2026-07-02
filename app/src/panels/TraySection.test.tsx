import { render, screen, fireEvent } from '@testing-library/react'
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
