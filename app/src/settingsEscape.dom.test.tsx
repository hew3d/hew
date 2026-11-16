import { describe, it, expect, afterEach } from 'vitest'
import { childPopupOpen } from './settingsEscape'

afterEach(() => {
  document.body.innerHTML = ''
})

describe('childPopupOpen', () => {
  it('is false with only the settings dialog open', () => {
    document.body.innerHTML = '<div role="dialog" id="s"></div>'
    expect(childPopupOpen(document.getElementById('s'))).toBe(false)
  })

  it('is true when a child dialog, menu or listbox is open, even portaled outside Settings', () => {
    for (const role of ['dialog', 'alertdialog', 'menu', 'listbox']) {
      document.body.innerHTML = `<div role="dialog" id="s"></div><div role="${role}"></div>`
      expect(childPopupOpen(document.getElementById('s'))).toBe(true)
    }
  })
})
