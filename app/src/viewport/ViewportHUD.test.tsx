import { render, screen, fireEvent } from '@testing-library/react'
import { describe, it, expect, vi } from 'vitest'
import { ViewportHUD } from './ViewportHUD'

describe('ViewportHUD', () => {
  it('renders the Orbit, Top, Iso, Front chips', () => {
    render(<ViewportHUD onSelectView={vi.fn()} onOrbit={vi.fn()} />)
    expect(screen.getByText('Orbit')).toBeInTheDocument()
    expect(screen.getByText('Top')).toBeInTheDocument()
    expect(screen.getByText('Iso')).toBeInTheDocument()
    expect(screen.getByText('Front')).toBeInTheDocument()
  })

  it('clicking Top calls onSelectView("top")', () => {
    const onSelectView = vi.fn()
    render(<ViewportHUD onSelectView={onSelectView} onOrbit={vi.fn()} />)
    fireEvent.click(screen.getByText('Top'))
    expect(onSelectView).toHaveBeenCalledWith('top')
  })

  it('clicking Iso calls onSelectView("iso")', () => {
    const onSelectView = vi.fn()
    render(<ViewportHUD onSelectView={onSelectView} onOrbit={vi.fn()} />)
    fireEvent.click(screen.getByText('Iso'))
    expect(onSelectView).toHaveBeenCalledWith('iso')
  })

  it('clicking Front calls onSelectView("front")', () => {
    const onSelectView = vi.fn()
    render(<ViewportHUD onSelectView={onSelectView} onOrbit={vi.fn()} />)
    fireEvent.click(screen.getByText('Front'))
    expect(onSelectView).toHaveBeenCalledWith('front')
  })

  it('clicking Orbit calls onOrbit', () => {
    const onOrbit = vi.fn()
    render(<ViewportHUD onSelectView={vi.fn()} onOrbit={onOrbit} />)
    fireEvent.click(screen.getByText('Orbit'))
    expect(onOrbit).toHaveBeenCalledOnce()
  })
})
