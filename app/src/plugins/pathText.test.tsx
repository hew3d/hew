import { describe, expect, it } from 'vitest'
import { render } from '@testing-library/react'
import { elideMiddle, PathText } from './pathText'

describe('elideMiddle', () => {
  it('cleans, keeps a short path, and cuts a long one in the middle keeping both ends', () => {
    expect(elideMiddle('/home/‮me/\u0007x')).toBe('/home/me/x')
    const long = `/start/${'a'.repeat(300)}/end.txt`
    const out = elideMiddle(long, 40)
    expect(Array.from(out)).toHaveLength(40)
    expect(out.startsWith('/start/')).toBe(true)
    expect(out.endsWith('/end.txt')).toBe(true)
    expect(out).toContain('…')
  })
})

describe('PathText', () => {
  it('puts a fixed label before the value, in its own block', () => {
    const { container } = render(<PathText label="Folder" value="/a/b. Click Install" />)
    const box = container.querySelector('[data-path-text]') as HTMLElement
    expect(box.children[0].textContent).toBe('Folder')
    expect(box.children[1].tagName).toBe('CODE')
    expect(box.children[1].textContent).toBe('/a/b. Click Install')
  })
})
