// Minimal gettext .po reader shared by the i18n scripts: enough of the
// format to read msgctxt/msgid/msgstr (multi-line strings concatenated,
// header and obsolete `#~` entries skipped). Not a general parser.

export function parsePo(text) {
  const entries = []
  let cur = null
  let field = null
  const flush = () => {
    if (cur && cur.msgid !== '') entries.push(cur)
    cur = null
    field = null
  }
  for (const rawLine of text.split('\n')) {
    const line = rawLine.trim()
    if (line === '') {
      flush()
      continue
    }
    if (line.startsWith('#')) continue
    const m = /^(msgctxt|msgid|msgid_plural|msgstr(?:\[\d+\])?)\s+"(.*)"$/.exec(line)
    if (m) {
      cur ??= { msgctxt: '', msgid: '', msgstr: '', forms: [] }
      if (m[1].startsWith('msgstr')) {
        // Each msgstr[n] is its own form: a plural counts as translated only
        // when every form is filled, so the check keeps them apart.
        cur.forms.push('')
        field = 'msgstr'
      } else {
        field = m[1] === 'msgid' ? 'msgid' : m[1] === 'msgctxt' ? 'msgctxt' : null
      }
      if (field === 'msgstr') cur.forms[cur.forms.length - 1] += unescape(m[2])
      else if (field) cur[field] += unescape(m[2])
      continue
    }
    const s = /^"(.*)"$/.exec(line)
    if (s && cur && field === 'msgstr') cur.forms[cur.forms.length - 1] += unescape(s[1])
    else if (s && cur && field) cur[field] += unescape(s[1])
  }
  flush()
  for (const e of entries) {
    // `msgstr` stays the joined text for lookups; `untranslated` is true when
    // any form is empty.
    e.msgstr = e.forms.join('')
    e.untranslated = e.forms.length === 0 || e.forms.some((f) => f === '')
  }
  return entries
}

function unescape(s) {
  return s.replace(/\\n/g, '\n').replace(/\\"/g, '"').replace(/\\\\/g, '\\')
}

/** Map "<msgctxt>\u0004<msgid>" → msgstr for quick lookup. */
export function poLookup(entries) {
  const map = new Map()
  for (const e of entries) map.set(`${e.msgctxt}\u0004${e.msgid}`, e.msgstr)
  return map
}

/**
 * How much of a parsed catalog is translated. `percent` is rounded down
 * to one decimal so a catalog with any gap never reads as 100.
 */
export function catalogCoverage(entries) {
  const missing = entries.filter((e) => e.untranslated)
  const total = entries.length
  const translated = total - missing.length
  const percent = total === 0 ? 0 : Math.floor((translated / total) * 1000) / 10
  return { total, translated, missing, percent }
}

/**
 * The i18n-check report: one line per locale, plus one per untranslated
 * message when `list` is set. `read(locale)` returns the catalog text and
 * may throw. `failed` is true only for a catalog that cannot be read or
 * holds no messages; an untranslated message never fails.
 */
export function coverageReport({ locales, read, list = false }) {
  const lines = []
  let failed = false
  const fail = (text) => {
    failed = true
    lines.push({ text, error: true })
  }
  for (const locale of locales) {
    let entries
    try {
      entries = parsePo(read(locale))
    } catch (err) {
      fail(`i18n-check: ${locale} is shipped but its catalog cannot be read: ${err.message}`)
      continue
    }
    const { total, translated, missing, percent } = catalogCoverage(entries)
    if (total === 0) {
      fail(`i18n-check: ${locale} is shipped but its catalog holds no messages`)
      continue
    }
    if (missing.length === 0) {
      lines.push({ text: `i18n-check: ${locale} complete (${total} messages)`, error: false })
      continue
    }
    lines.push({
      text: `i18n-check: ${locale} ${translated} of ${total} messages translated (${percent}%), ${missing.length} show in English`,
      error: false,
    })
    if (list) {
      for (const e of missing) {
        lines.push({ text: `  ${e.msgctxt ? `[${e.msgctxt}] ` : ''}${JSON.stringify(e.msgid)}`, error: false })
      }
    }
  }
  return { lines, failed }
}
