/**
 * ExternalLink — https only, rel noopener, and the desktop opener (design 7.5).
 */
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { ExternalLink, isHttpsUrl } from './ExternalLink'

describe('ExternalLink', () => {
  it('an https address is a link that opens a new tab without a back-reference', () => {
    render(<ExternalLink href="https://example.org/a">Homepage</ExternalLink>)
    const a = screen.getByRole('link', { name: 'Homepage' })
    expect(a).toHaveAttribute('href', 'https://example.org/a')
    expect(a).toHaveAttribute('target', '_blank')
    expect(a).toHaveAttribute('rel', 'noopener noreferrer')
  })

  it.each(['http://example.org', 'javascript:alert(1)', 'data:text/html,hi', 'ftp://example.org', 'example.org', ''])('%s is plain text, never a link', (href) => {
    render(<ExternalLink href={href}>Homepage</ExternalLink>)
    expect(screen.queryByRole('link')).not.toBeInTheDocument()
    expect(screen.getByText('Homepage')).toBeInTheDocument()
  })

  it('a click does not reach the row it sits in', () => {
    let rowClicks = 0
    render(
      <div onClick={() => rowClicks++}>
        <ExternalLink href="https://example.org">Go</ExternalLink>
      </div>,
    )
    fireEvent.click(screen.getByRole('link'))
    expect(rowClicks).toBe(0)
  })

  it('isHttpsUrl', () => {
    expect(isHttpsUrl('https://a.b')).toBe(true)
    expect(isHttpsUrl('HTTPS://A.B')).toBe(true)
    expect(isHttpsUrl('http://a.b')).toBe(false)
    expect(isHttpsUrl('not a url')).toBe(false)
  })
})
