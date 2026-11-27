/**
 * ExternalLink — a link that leaves the app, for the Library's source UI
 * (a source's homepage, the publishing guide, a catalog removal link). Only
 * `https` addresses become links (anything else is shown as plain text), and
 * the link carries `rel="noopener noreferrer"`. A Tauri webview swallows a
 * blank-target navigation, so on the desktop the click goes to the OS default
 * browser through the opener plugin; the web build opens a tab.
 */

import { isTauri } from '../../io/fileHost'

/** True for an absolute `https` URL. */
export function isHttpsUrl(url: string): boolean {
  try {
    return new URL(url).protocol === 'https:'
  } catch {
    return false
  }
}

export function ExternalLink({ href, children, className }: { href: string; children: React.ReactNode; className?: string }) {
  if (!isHttpsUrl(href)) return <span className={className}>{children}</span>
  return (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      className={className ?? 'hwlib__src-link'}
      onClick={(e) => {
        e.stopPropagation()
        if (!isTauri) return
        e.preventDefault()
        void import('@tauri-apps/plugin-opener')
          .then(({ openUrl }) => openUrl(href))
          .catch(() => {
            /* best effort: nothing useful to do if the OS cannot open it */
          })
      }}
    >
      {children}
    </a>
  )
}
