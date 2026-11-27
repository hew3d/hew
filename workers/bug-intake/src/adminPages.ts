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
  .notice { background: #f2f7ee; border: 1px solid #b9d3a8; padding: 0.5rem 0.75rem; border-radius: 4px; }
  .toolbar button { margin-right: 0.5rem; }
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

export const SELECT_PRESETS = ['all', 'untriaged', 'triaged', 'expired', 'none'] as const
export type SelectPreset = (typeof SELECT_PRESETS)[number]

export function isSelectPreset(value: unknown): value is SelectPreset {
  return typeof value === 'string' && (SELECT_PRESETS as readonly string[]).includes(value)
}

/** Whether a preset checks `r`'s box. A follow-up attachment never has a
 *  box (it travels with its parent), so it is never selected. */
function presetSelects(preset: SelectPreset | undefined, r: ReportRow): boolean {
  if (r.parentId) return false
  switch (preset) {
    case 'all':
      return true
    case 'untriaged':
      return !r.triaged
    case 'triaged':
      return r.triaged
    case 'expired':
      return r.bundleDeleted
    default:
      return false
  }
}

/** What a finished batch action reports back on the list. Built by the
 *  handler from VALIDATED values only (a known action name, a count, report
 *  ids that matched the id shape); this still escapes every one. */
export interface BatchNotice {
  action: 'triaged' | 'untriaged' | 'deleted'
  ok: number
  failed: string[]
  /** A delete destroyed the data but could not remove the list entries. */
  rowsKept?: boolean
}

export const NOTICE_ACTIONS: ReadonlyArray<BatchNotice['action']> = ['triaged', 'untriaged', 'deleted']

function renderNotice(n: BatchNotice): string {
  const verb = n.action === 'deleted' ? 'Deleted' : n.action === 'triaged' ? 'Marked triaged' : 'Marked untriaged'
  const failed =
    n.failed.length > 0
      ? ` <strong>Not done for ${n.failed.length}:</strong> ${n.failed.map((id) => `<code>${escapeHtml(id)}</code>`).join(', ')}.`
      : ''
  const kept = n.rowsKept
    ? ' The data for the reports listed above was removed, but their entries are still in this list; delete them again to clear them.'
    : ''
  return `<p class="notice">${verb} ${Math.trunc(n.ok)} report${n.ok === 1 ? '' : 's'}.${failed}${kept}</p>`
}

const BATCH_BUTTONS = `<p class="toolbar">
<button type="submit" formmethod="post" formaction="/report/admin/batch/triage" name="value" value="1">Mark triaged</button>
<button type="submit" formmethod="post" formaction="/report/admin/batch/triage" name="value" value="0">Mark untriaged</button>
<button type="submit" formmethod="post" formaction="/report/admin/batch/delete">Delete&hellip;</button>
<button type="submit" formmethod="get" formaction="/report/admin/batch/download">Download</button>
</p>`

export interface ListOptions {
  select?: SelectPreset
  notice?: BatchNotice
}

/** The reports list — newest first (`listReports` already orders this way).
 *  `report.id` is server-generated (Crockford base32, `id.ts`) and never
 *  needs escaping, but every other displayed field does.
 *
 *  The table sits in ONE form so checkboxes and the toolbar need no script:
 *  each toolbar button carries its own `formaction`/`formmethod`, and the
 *  "Select" presets are plain links that re-render the page with the boxes
 *  already checked (`?select=`). */
export function renderList(reports: ReportRow[], options: ListOptions = {}): string {
  const rows = reports
    .map(
      (r) => `<tr${r.triaged ? ' class="triaged"' : ''}>
  <td>${
    r.parentId
      ? ''
      : `<input type="checkbox" name="id" value="${r.id}" aria-label="Select ${r.id}"${presetSelects(options.select, r) ? ' checked' : ''}>`
  }</td>
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

  const selectLinks = SELECT_PRESETS.map((p) => `<a href="/report/admin/?select=${p}">${p}</a>`).join(' &middot; ')

  return page(
    'Bug reports',
    `<h1>Bug reports</h1>
${options.notice ? renderNotice(options.notice) : ''}<p>${reports.length} report${reports.length === 1 ? '' : 's'}.</p>
<p>Select: ${selectLinks}</p>
<form method="post" action="/report/admin/batch/triage">
${BATCH_BUTTONS}
<table>
<thead><tr><th></th><th>ID</th><th>Received</th><th>Version</th><th>Platform</th><th>Description</th><th>Size</th><th>Status</th><th>Following</th><th>Triaged</th></tr></thead>
<tbody>
${rows}
</tbody>
</table>
${BATCH_BUTTONS}
</form>`,
  )
}

/** The step before a batch delete: nothing has changed yet. `ids` are every
 *  validated selected id (carried as hidden inputs so the second request is
 *  self-contained); `found` the reports that exist, `attachments` their
 *  follow-up attachments, which are deleted with them. */
export function renderDeleteConfirm(ids: string[], found: ReportRow[], attachments: ReportRow[]): string {
  const hidden = ids.map((id) => `<input type="hidden" name="id" value="${escapeHtml(id)}">`).join('\n')
  const n = found.length
  const m = attachments.length
  const list = found.map((r) => `<li><code>${escapeHtml(r.id)}</code> &middot; ${escapeHtml(r.descriptionPreview)}</li>`).join('\n')
  const gone = ids.length - n
  return page(
    'Delete reports?',
    `<h1>Delete ${n} report${n === 1 ? '' : 's'}${m > 0 ? ` and their ${m} attachment${m === 1 ? '' : 's'}` : ''}?</h1>
<p>This cannot be undone.</p>
${gone > 0 ? `<p>${gone} selected id${gone === 1 ? ' is' : 's are'} not a report any more and will be skipped.</p>` : ''}<ul>
${list}
</ul>
<form method="post" action="/report/admin/batch/delete">
${hidden}
<input type="hidden" name="confirm" value="1">
<button type="submit">Delete ${n} report${n === 1 ? '' : 's'}</button>
<a href="/report/admin/">Back</a>
</form>`,
  )
}

/** A refused batch request: a plain page, nothing changed. */
export function renderBatchError(message: string): string {
  return page('Not done', `<p>${escapeHtml(message)}</p><p><a href="/report/admin/">&larr; All reports</a></p>`)
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
