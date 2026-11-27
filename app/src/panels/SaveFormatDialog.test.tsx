import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'
import { SaveFormatDialog } from './SaveFormatDialog'

describe('SaveFormatDialog', () => {
  it('lists Hew, the 2021-and-later container and one older-versions entry with its release list', () => {
    render(<SaveFormatDialog initial="hew" onChoose={vi.fn()} onCancel={vi.fn()} />)
    const radios = screen.getAllByRole('radio').map((r) => (r as HTMLInputElement).value)
    expect(radios).toEqual(['hew', 'skp2026', 'older'])
    const list = screen.getByTestId('save-older-release') as HTMLSelectElement
    expect([...list.options].map((o) => o.value)).toEqual([
      'skp2020', 'skp2019', 'skp2018', 'skp2017', 'skp2016', 'skp2015', 'skp2014', 'skp2013',
      'skp8', 'skp7', 'skp6', 'skp5', 'skp4', 'skp3',
    ])
    expect(screen.getByText('SketchUp 2021 and later (.skp)')).toBeInTheDocument()
  })

  it('preselects the file\'s own older release when saving back', () => {
    const onChoose = vi.fn()
    render(<SaveFormatDialog initial="skp2014" onChoose={onChoose} onCancel={vi.fn()} />)
    expect(screen.getByRole('radio', { name: /Older SketchUp versions/ })).toBeChecked()
    expect((screen.getByTestId('save-older-release') as HTMLSelectElement).value).toBe('skp2014')
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }))
    expect(onChoose).toHaveBeenCalledWith({ format: 'skp2014', compat: false })
  })

  it('picking a release from the list chooses that release', () => {
    const onChoose = vi.fn()
    render(<SaveFormatDialog initial="hew" onChoose={onChoose} onCancel={vi.fn()} />)
    fireEvent.change(screen.getByTestId('save-older-release'), { target: { value: 'skp8' } })
    expect(screen.getByRole('radio', { name: /Older SketchUp versions/ })).toBeChecked()
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }))
    expect(onChoose).toHaveBeenCalledWith({ format: 'skp8', compat: false })
  })

  it('shows the avoid-newer-features box for the 2021-and-later container only', () => {
    const onChoose = vi.fn()
    render(<SaveFormatDialog initial="skp2026" initialCompat onChoose={onChoose} onCancel={vi.fn()} />)
    const box = screen.getByTestId('save-skp-compat') as HTMLInputElement
    expect(box.checked).toBe(true)
    expect(screen.getByText('Avoid features older SketchUp versions cannot open')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }))
    expect(onChoose).toHaveBeenCalledWith({ format: 'skp2026', compat: true })

    fireEvent.click(screen.getByRole('radio', { name: /Hew document/ }))
    expect(screen.queryByTestId('save-skp-compat')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }))
    expect(onChoose).toHaveBeenLastCalledWith({ format: 'hew', compat: false })
  })
})
