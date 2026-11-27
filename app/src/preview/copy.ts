/* oxlint-disable lingui/no-unlocalized-strings -- Preview-build copy is English only by design: the Settings > Preview section, the mark and the tags exist only in a preview build, and are exempt from localization (docs/design/preview-channel.md section 3). Nothing here ships in a production build. */
/**
 * The words of the Preview surfaces. English only, and only ever shown in a
 * preview build. A production build never renders these (each use sits behind
 * the build flag), and the catalogs never carry them.
 */

export const PREVIEW_TAG = 'Preview'

export const PREVIEW_MARK_TIP = 'This build contains unfinished features. Settings > Preview lists them.'

export const PREVIEW_SETTINGS = {
  heading: 'Preview',
  intro:
    'This is a preview build. These features are unfinished: they may be rough, change, or be removed. Turn off any that get in your way.',
  restart: 'Takes effect the next time Hew starts.',
  on: 'On',
  off: 'Off',
  pending: (nowOn: boolean) => (nowOn ? 'Turns on the next time Hew starts.' : 'Turns off the next time Hew starts.'),
} as const

/** "Fillet (Shift+F) · Preview": a tooltip with the tag added. */
export function withPreviewTag(text: string): string {
  return `${text} · ${PREVIEW_TAG}`
}
