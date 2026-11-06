/**
 * Server-rendered HTML for `/report/admin/*`. Every value that came from a
 * report — the description, the system info strings, the file names inside
 * a bundle — is attacker-controlled (a submitter writes the whole JSON
 * body), so `escapeHtml` runs on every one of them before it reaches a
 * template string. There is no client-side rendering here at all: no JS
 * ships with these pages, so there is nothing for an unescaped value to
 * execute in beyond what raw HTML injection already gets it, which is why
 * escaping is the whole defense and it has to be applied without
 * exception (`handlers.ts` sends a strict CSP on top, as a second layer,
 * not a substitute for this one).
 */

import type { ReportRow } from './indexStore.ts'
import type { HeadDisplayFields } from './headScan.ts'
import type { ThreadAdminView, ThreadMessage } from './threadStore.ts'

/** The canned replies the detail page's reply form offers as named submit
 *  buttons (`canned=<key>`) — fixed text, so no script is needed to fill
 *  the textarea (docs/design/follow-a-report.md §3.2, D17). */
export const CANNED_REPLIES: Readonly<Record<string, { label: string; text: string }>> = {
  'ask-file': {
    label: 'Ask for the file',
    text:
      "Could you attach the file that wouldn't open? In Hew, open Help ▸ Sent Reports, pick this report, " +
      'and click Attach. Thanks for the report.',
  },
  'ask-steps': {
    label: 'Ask for steps',
    text:
      'Could you tell me the steps that lead to this, as exactly as you can? In Hew, open Help ▸ Sent Reports, ' +
      'pick this report, and type them in the reply box. Thanks for the report.',
  },
}

/** How the admin pages name a status. */
export function statusLabel(status: string | undefined, version: string | null | undefined): string {
  switch (status) {
    case 'need-info':
      return 'needs info'
    case 'fixed':
      return version ? `fixed ${version}` : 'fixed'
    case 'closed':
      return 'closed'
    default:
      return 'open'
  }
}

/** The list's "new" marker: the reporter wrote or attached something since
 *  the maintainer last opened the report. */
export function hasNewActivity(r: Pick<ReportRow, 'reporterActivityAt' | 'adminViewedAt'>): boolean {
  return r.reporterActivityAt != null && r.reporterActivityAt > (r.adminViewedAt ?? 0)
}

export function escapeHtml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;')
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
  return `${(n / (1024 * 1024)).toFixed(1)} MB`
}

function formatDate(ms: number): string {
  return new Date(ms).toISOString().replace('T', ' ').replace(/\.\d+Z$/, ' UTC')
}

/** `system` is optional (a user can untick "App version and system"
 *  entirely — docs/design/report-bug.md §2), so `appVersion`/`platform`
 *  come through as empty strings from `headScan.ts` (the index row) or as
 *  `undefined` from a bundle with no `system` object at all (the detail
 *  page's fuller parse) — either way, this is where that becomes the
 *  word a maintainer actually reads. */
function orUnknown(value: string | undefined): string {
  return value && value.length > 0 ? value : 'unknown'
}

const STYLE = `
  body { font: 14px/1.5 -apple-system, system-ui, sans-serif; max-width: 960px; margin: 2rem auto; padding: 0 1rem; color: #1a1a1a; background: #fff; }
  table { width: 100%; border-collapse: collapse; }
  th, td { text-align: left; padding: 0.4rem 0.6rem; border-bottom: 1px solid #ddd; vertical-align: top; }
  th { font-weight: 600; }
  code { background: #f2f2f2; padding: 0.1rem 0.3rem; border-radius: 3px; }
  .triaged { color: #666; }
  .actions form { display: inline; margin-right: 0.5rem; }
  pre { white-space: pre-wrap; word-break: break-word; background: #f7f7f7; padding: 0.75rem; border-radius: 4px; }
  dt { font-weight: 600; margin-top: 0.5rem; }
  dd { margin: 0; }
  .new { color: #b00020; font-weight: 600; }
  .child { color: #555; }
  .thread { list-style: none; padding: 0; }
  .thread li { border-left: 3px solid #ddd; padding: 0.25rem 0.75rem; margin: 0.5rem 0; }
  .thread li.reporter { border-left-color: #2a6fdb; }
  .thread li.system { border-left-color: #999; color: #555; }
  .thread .meta { font-size: 12px; color: #666; }
  textarea { width: 100%; min-height: 6rem; font: inherit; }
  fieldset { border: 1px solid #ddd; margin: 1rem 0; }
`

function page(title: string, body: string): string {
  return `<!doctype html>
<html>
<head><meta charset="utf-8"><title>${escapeHtml(title)}</title><style>${STYLE}</style></head>
<body>
${body}
</body>
</html>`
}

/** The reports list — newest first (`listReports` already orders this way).
 *  `report.id` is server-generated (Crockford base32, `id.ts`) and never
 *  needs escaping, but every other displayed field does. */
export function renderList(reports: ReportRow[]): string {
  const rows = reports
    .map(
      (r) => `<tr${r.triaged ? ' class="triaged"' : ''}>
  <td><a href="/report/admin/${r.id}"><code>${r.id}</code></a>${hasNewActivity(r) ? ' <span class="new">new</span>' : ''}${
    r.parentId ? `<br><span class="child">↳ for <a href="/report/admin/${escapeHtml(r.parentId)}">${escapeHtml(r.parentId)}</a></span>` : ''
  }</td>
  <td>${formatDate(r.receivedAt)}</td>
  <td>${escapeHtml(orUnknown(r.appVersion))}</td>
  <td>${escapeHtml(orUnknown(r.platform))}</td>
  <td>${escapeHtml(r.descriptionPreview)}</td>
  <td>${r.bundleDeleted ? 'expired' : formatBytes(r.sizeBytes)}</td>
  <td>${escapeHtml(statusLabel(r.status, r.statusVersion))}</td>
  <td>${r.following ? 'yes' : ''}</td>
  <td>${r.triaged ? 'yes' : ''}</td>
</tr>`,
    )
    .join('\n')

  return page(
    'Bug reports',
    `<h1>Bug reports</h1>
<p>${reports.length} report${reports.length === 1 ? '' : 's'}.</p>
<table>
<thead><tr><th>ID</th><th>Received</th><th>Version</th><th>Platform</th><th>Description</th><th>Size</th><th>Status</th><th>Following</th><th>Triaged</th></tr></thead>
<tbody>
${rows}
</tbody>
</table>`,
  )
}

function field(label: string, value: string | undefined): string {
  if (value === undefined) return ''
  return `<dt>${escapeHtml(label)}</dt><dd>${escapeHtml(value)}</dd>`
}

/** `head` is the DISPLAY-only parse of just piece 0's decompressed head
 *  (`headScan.ts`'s `parseHeadForDisplay`) — this page never reassembles or
 *  fully decompresses a report's stored bundle any more (see
 *  `handlers.ts`'s `handleAdminDetail`), so there is no `hasRecording`/
 *  `hasModel`/`logLines`/`inputEvents` to show one way or the other; those
 *  parts of the bundle live only in the downloaded file. */
function authorLabel(m: ThreadMessage): string {
  if (m.author === 'maintainer') return 'You'
  if (m.author === 'reporter') return m.kind === 'attachment' ? 'Reporter (attachment)' : 'Reporter'
  return 'Status'
}

function renderMessage(m: ThreadMessage, isNew: boolean): string {
  const attachment =
    m.kind === 'attachment' && m.attachmentReportId
      ? ` &middot; <a href="/report/admin/${escapeHtml(m.attachmentReportId)}"><code>${escapeHtml(m.attachmentReportId)}</code></a>`
      : ''
  return `<li class="${m.author}"><div class="meta">#${m.seq} &middot; ${escapeHtml(authorLabel(m))} &middot; ${formatDate(m.at)}${attachment}${
    isNew ? ' <span class="new">new</span>' : ''
  }</div><pre>${escapeHtml(m.text)}</pre></li>`
}

const STATUS_OPTIONS: ReadonlyArray<[string, string]> = [
  ['open', 'Open'],
  ['need-info', 'Needs info'],
  ['fixed', 'Fixed in…'],
  ['closed', 'Closed'],
]

/** The thread, reply, status, and attachments sections of the detail page
 *  (docs/design/follow-a-report.md §3.2). Plain forms only. Every message
 *  text is escaped, the maintainer's own included. */
export function renderThreadSection(id: string, thread: ThreadAdminView | null, children: ReportRow[]): string {
  const following = thread?.following === true
  const status = thread?.status ?? 'open'
  const messages =
    thread && thread.messages.length > 0
      ? `<ol class="thread">\n${thread.messages.map((m) => renderMessage(m, m.seq > thread.previousAdminSeenUpTo && m.author !== 'maintainer')).join('\n')}\n</ol>`
      : '<p>No messages yet.</p>'
  const cannedButtons = Object.entries(CANNED_REPLIES)
    .map(([key, c]) => `<button type="submit" name="canned" value="${escapeHtml(key)}">${escapeHtml(c.label)}</button>`)
    .join('\n')
  const options = STATUS_OPTIONS.map(
    ([value, label]) => `<option value="${value}"${value === status ? ' selected' : ''}>${escapeHtml(label)}</option>`,
  ).join('')
  const attachments =
    children.length > 0
      ? `<ul>\n${children
          .map(
            (c) =>
              `<li><a href="/report/admin/${escapeHtml(c.id)}"><code>${escapeHtml(c.id)}</code></a> &middot; ${formatDate(c.receivedAt)} &middot; ${
                c.bundleDeleted ? 'expired' : `${formatBytes(c.sizeBytes)} &middot; <a href="/report/admin/${escapeHtml(c.id)}/download">Download</a>`
              } &middot; ${escapeHtml(c.descriptionPreview)}</li>`,
          )
          .join('\n')}\n</ul>`
      : '<p>None.</p>'

  return `<h2>Following: ${following ? 'yes' : 'no'}</h2>
<p>Status: <strong>${escapeHtml(statusLabel(status, thread?.statusVersion))}</strong></p>
<h2>Thread</h2>
${messages}
<form method="post" action="/report/admin/${id}/reply">
<fieldset><legend>Reply</legend>
${following ? '' : '<p>No one is following this report. A reply is kept with it but will not reach anyone.</p>'}
<textarea name="text" maxlength="4000"></textarea>
<p><button type="submit">Send reply</button>
${cannedButtons}</p>
</fieldset>
</form>
<form method="post" action="/report/admin/${id}/status">
<fieldset><legend>Status</legend>
<label>Status <select name="status">${options}</select></label>
<label>Version <input type="text" name="version" maxlength="64" value="${escapeHtml(thread?.statusVersion ?? '')}"></label>
<label>Note <input type="text" name="note" maxlength="4000"></label>
<button type="submit">Set status</button>
</fieldset>
</form>
<h2>Attachments</h2>
${attachments}`
}

export interface DetailExtras {
  thread: ThreadAdminView | null
  children: ReportRow[]
}

export function renderDetail(entry: ReportRow, head: HeadDisplayFields | null, extras?: DetailExtras): string {
  const body = head
    ? `<dl>
${field('App version', orUnknown(head.appVersion))}
${field('Platform', orUnknown(head.platform))}
${field('OS', head.os)}
${field('GPU', head.gpu)}
${field('User agent', head.userAgent)}
${field('UI language', head.locale)}
${field('Contact', head.contact)}
</dl>
<h2>What happened</h2>
<pre>${escapeHtml(head.description ?? '(no description)')}</pre>
${head.expected ? `<h2>What was expected</h2><pre>${escapeHtml(head.expected)}</pre>` : ''}
${head.crash ? `<h2>Crash</h2><dl>${field('At', head.crash.at)}${field('Message', head.crash.message)}</dl>` : ''}
<p>Any recorded steps, imported files, the model file, and the diagnostic
log are in the downloaded bundle.</p>`
    : entry.bundleDeleted
      ? `<p>This report's bundle has expired; its metadata and thread remain.</p>`
      : `<p>This report's head could not be read.</p>`

  return page(
    entry.id,
    `<p><a href="/report/admin/">&larr; All reports</a></p>
<h1><code>${entry.id}</code></h1>
<p>Received ${formatDate(entry.receivedAt)} &middot; ${entry.bundleDeleted ? 'bundle expired' : formatBytes(entry.sizeBytes)}${
      entry.parentId
        ? ` &middot; <span class="child">↳ for <a href="/report/admin/${escapeHtml(entry.parentId)}"><code>${escapeHtml(entry.parentId)}</code></a></span>`
        : ''
    }</p>
<div class="actions">
${entry.bundleDeleted ? '' : `<a href="/report/admin/${entry.id}/download">Download bundle</a>`}
<form method="post" action="/report/admin/${entry.id}/triage"><button type="submit">${entry.triaged ? 'Mark untriaged' : 'Mark triaged'}</button></form>
<form method="post" action="/report/admin/${entry.id}/delete"><button type="submit">Delete (cannot be undone)</button></form>
</div>
${body}
${extras ? renderThreadSection(entry.id, extras.thread, extras.children) : ''}`,
  )
}

/** A small error page for a refused admin form (empty reply, bad status). */
export function renderFormError(id: string, message: string): string {
  return page('Not saved', `<p>${escapeHtml(message)}</p><p><a href="/report/admin/${id}">&larr; Back to <code>${id}</code></a></p>`)
}
