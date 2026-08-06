import { describe, it, expect } from 'vitest'
import { itemFileName } from './fileNaming'

describe('itemFileName', () => {
  it('slugifies the name and appends a 6-char id suffix, under the category folder', () => {
    expect(
      itemFileName('Patio Chair', '12345678-aaaa-bbbb-cccc-ddddeeeeffff', 'component'),
    ).toBe('Components/patio-chair-123456.hew')
  })

  it('collapses non-alphanumeric runs to single hyphens and trims edges', () => {
    expect(
      itemFileName('  Fancy!!  Door//Frame  ', 'abcdef00-0000-0000-0000-000000000000', 'model'),
    ).toBe('Models/fancy-door-frame-abcdef.hew')
  })

  it('falls back to "item" for an empty/symbol-only name', () => {
    expect(itemFileName('###', '11112222-0000-0000-0000-000000000000', 'component')).toBe(
      'Components/item-111122.hew',
    )
  })

  it('always ends in .hew', () => {
    expect(itemFileName('Box', 'aaaaaaaa-0000-0000-0000-000000000000', 'model')).toMatch(/\.hew$/)
  })

  it('two different ids for the same name never collide', () => {
    const a = itemFileName('Box', '11111111-0000-0000-0000-000000000000', 'model')
    const b = itemFileName('Box', '22222222-0000-0000-0000-000000000000', 'model')
    expect(a).not.toBe(b)
  })

  it('is case-insensitive and lowercases the name', () => {
    expect(
      itemFileName('Patio Chair', 'ABCDEF00-0000-0000-0000-000000000000', 'component'),
    ).toBe('Components/patio-chair-abcdef.hew')
  })

  it('maps each category to its own subfolder', () => {
    const id = '11111111-0000-0000-0000-000000000000'
    expect(itemFileName('X', id, 'component')).toMatch(/^Components\//)
    expect(itemFileName('X', id, 'material')).toMatch(/^Materials\//)
    expect(itemFileName('X', id, 'model')).toMatch(/^Models\//)
  })
})
