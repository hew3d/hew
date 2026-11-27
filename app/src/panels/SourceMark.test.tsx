/**
 * SourceMark — drawn like LibraryMark, says "Newer at the source" (design 7.5).
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { LibraryMark } from './LibraryMark'
import { SourceMark } from './SourceMark'

describe('SourceMark', () => {
  it('is a labelled image with the "newer at the source" tooltip', () => {
    render(<SourceMark />)
    const mark = screen.getByTestId('source-mark')
    expect(mark).toHaveAttribute('role', 'img')
    expect(mark).toHaveAttribute('aria-label', 'Newer at the source')
    expect(mark).toHaveAttribute('title', 'Newer at the source')
  })

  it('wears the same border and color as LibraryMark', () => {
    render(
      <>
        <SourceMark />
        <LibraryMark state="library_newer" />
      </>,
    )
    const a = screen.getByTestId('source-mark')
    const b = screen.getByTestId('library-mark')
    for (const prop of ['borderRadius', 'border', 'color', 'fontSize', 'padding'] as const) {
      expect(a.style[prop], prop).toBe(b.style[prop])
    }
  })
})
