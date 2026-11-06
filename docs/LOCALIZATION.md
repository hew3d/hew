# Hew — Localization

Hew's interface is written in English and translated through gettext
catalogs. The English text in the source code is the key. A command
extracts every string into one `.po` file per language, translators fill
those files in, and the app loads the catalog for the language the user
picks. This document covers the three jobs that touch it: writing UI
text, translating it, and adding a language.

Status: the plumbing lands with the branch that introduces this document.
Until then the app is English-only.

## What is translated

The desktop and web app: menus, panels, dialogs, tooltips, accessibility
labels, tool prompts, the Measurements box, error messages, the Settings
window, Shop Mode, and the Print dialog. The native macOS menu bar is
generated from the same catalogs.

Not translated, by design:

- The headless API, the MCP server, `hew-cli`, the API reference, and log
  output. These are developer surfaces and stay English.
- The `.hew` file format. Names a user types, tag names, material names,
  and scene names are stored verbatim and never pass through a catalog.
  Defaults the app invents are stored as identifiers and translated only
  when shown.
- Keyboard shortcuts, command IDs, tool IDs, and file names.
- The product name.

The website and the user guide are a separate project and are English
today.

## Writing UI text

Every user-facing string in `app/src` goes through a Lingui macro. Three
forms cover everything:

```tsx
import { t, msg } from '@lingui/core/macro'
import { Trans } from '@lingui/react/macro'

// Attribute or plain TypeScript value
<button title={t`Push/Pull`}>…</button>

// JSX content, including markup inside the sentence
<p><Trans>That face has no usable area. Pick a different face.</Trans></p>

// A module-level table resolved later, at render time
const NAME = msg`Follow Me`
```

Counts use ICU plural syntax inside the message so each language supplies
its own forms:

```tsx
import { plural } from '@lingui/core/macro'

plural(count, { one: 'Moved # object', other: 'Moved # objects' })
```

Rules:

- Keep whole sentences in one message. A translator cannot reorder
  fragments that are concatenated in code.
- Named placeholders over positional ones. `t\`Could not open ${fileName}.\``
  gives the translator a name to keep.
- When one English word has two meanings, add a `context`: `t({ context:
  'tool', message: 'Line' })` and `t({ context: 'noun', message: 'line'
  })`. Line the tool, Scale the tool, Model the root, Move the command,
  and Open the verb are the known cases.
- Add a `comment` when the words alone do not say where the string
  appears or what a placeholder holds: `t({ comment: 'Units dropdown
  label', message: 'Architectural' })`.
- Numbers and dates go through `formatNumber` and `formatRelativeTime` in
  `app/src/i18n`, which wrap `Intl.NumberFormat` and
  `Intl.RelativeTimeFormat` for the active locale. Never `toFixed` on a
  number a person reads.
- Strings in `crates/` stay English. Typed error codes cross the WASM
  boundary and `app/src/kernelErrors.ts` holds the translated copy for
  each code; a new kernel error variant without an entry there fails the
  existing exhaustiveness test.
- Undo and redo labels for kernel-derived entries come from the structured
  `kind` on each history entry, not from the English `label` field. Add a
  new kind's message to the table in `app/src/i18n/historyLabels.ts`. A
  transaction the UI itself labels (a tool's draft commit, a batch repair)
  passes its label through `t` at the moment it is created, so the entry
  reads in the language the user was working in; the headless API sees
  that same text.

The lint rule `lingui/no-unlocalized-strings`, run by Oxlint through
`pnpm --dir app lint`, fails `scripts/verify.sh` on any string literal that
bypasses the macros. CSS values, identifiers, storage keys, log calls, and
test files are exempt in `app/.oxlintrc.json`; add an exemption there
rather than disabling the rule inline. The rule checks only `placeholder`,
`alt`, `aria-label`, and `value` among native attributes, so the same lint
script runs `scripts/i18n-lint-gaps.mjs`, which fails on a literal
`title="…"` tooltip. Single lowercase words (`'projected'`) are also below
the rule's radar by configuration, so read what you write.

After adding or changing text, run:

```sh
pnpm --dir app i18n:extract
```

This rewrites `app/src/locales/<locale>/messages.po` for every locale,
adding new entries and marking changed ones. Commit the catalogs with the
code. `scripts/verify.sh` runs extract itself and fails if the committed
catalogs are stale.

Changing the English wording of a message changes its key, so the entry
loses its translations in every language and comes back as untranslated.
That is intended: a wording change is a meaning change until a translator
confirms otherwise. Fixing a typo in English costs the same, so batch
copy edits.

## Translating

Catalogs live at `app/src/locales/<locale>/messages.po`, one file per
language, in gettext format. Each entry has the English source, the
translation, the file and line it came from, and any comment the
developer attached:

```po
#. Units dropdown label
#: src/settings/units.ts:58
msgid "Architectural"
msgstr "建築"
```

Any gettext editor works. Poedit is free and shows source, translation,
and comments side by side. Translation platforms such as Weblate read the
same files; Hew does not run one today.

Working rules:

- Match the terms SketchUp's localized edition uses for the same tool or
  concept. Hew's users come from SketchUp and should recognize every
  word. Keep a glossary in `docs/i18n/glossary-<locale>.md` and add to it
  as decisions are made.
- Keep every placeholder exactly as it appears in the source. `{fileName}`
  stays `{fileName}`. Reorder freely around it.
- Keep ICU plural structure. Languages with one plural form still need the
  `other` branch.
- Leave an entry empty rather than guessing. An empty translation falls
  back to English in the app; a wrong one does not.
- Do not translate the product name, file extensions, shortcut keys, or
  API identifiers quoted in messages.

Submit translations as a pull request that changes only the `.po` file
and the glossary. The pull request title is `i18n(<locale>): …`. The
translator's name goes in the catalog header's `Last-Translator` field
and in the changelog entry for the release that ships the update.

To see the result in the app, run the dev server with the catalog in
place and pick the language in Settings. The dev server compiles catalogs
on the fly; no build step.

## How the app picks a language

The Language tab in the Settings window (a Language card on the Windows
settings page) offers System default plus every shipped language. It is stored in `localStorage` under
`hew.settings.language` and synced across the main and Settings windows
the same way the units and theme settings are.

System default resolves the browser's or OS's preferred languages in
order against the shipped set and falls back to English. An explicit
choice is never overridden by the system locale. Switching languages
re-renders the open windows, updates the document language attribute,
and rebuilds the native menu without reloading or touching the model.

A language ships only when its catalog is complete. `pnpm --dir app
i18n:check` fails the verify gate on any untranslated entry in a shipped
locale, so a user who picks a language never sees a mixed interface. A
partial catalog can still be tested locally by adding its locale to
`SHIPPED_LOCALES` in `app/src/i18n/locales.ts` on a branch.

Missing entries fall back to English at runtime in every case.

### Pseudo-locale

With debug mode on in Settings, the language list also offers
`pseudo-LOCALE`. It is generated from the English catalog at compile time:
every string gets accented letters, bracket markers, and thirty percent
padding, so `Push/Pull` renders as `⟦ Þûšh/Þûĺĺ... ⟧`. Anything still
in plain English was never wrapped. Anything clipped or overflowing is a
layout bug. Run the app in it after any sweep of UI text.

### Numbers, units, and dates

Decimal and grouping separators follow the active locale for display.
Parsing in the Measurements box accepts a period as the decimal separator
in every locale today; locale-specific parsing for comma-decimal
languages is a planned, separate change.

On first launch the length unit defaults from the region: millimeters for
Japan, meters for other metric regions, architectural feet and inches for
the United States, Liberia, and Myanmar. A persisted choice is never
replaced.

Relative times ("2 minutes ago") and absolute dates use the active
locale's `Intl` formats.

### Keyboard and input methods

Tool shortcuts are letters, as in SketchUp: the letter the keystroke
produced wins when it is a plain ASCII letter, so the key cap the tool
rail shows is the one that works on AZERTY and Dvorak too. When the
keystroke produced no ASCII letter (an idle IME yielding a full-width
character, a dead key, a non-Latin layout) the physical key position
stands in. Digits follow the same rule (the produced digit, else the
physical digit key). The keymap (`app/src/keymap/chord.ts`) applies this
to every shortcut, including the ones a person binds in Settings ▸
Controls. Option/Alt combinations are never bare-letter shortcuts. While
an input method editor is composing, the global shortcut handler, the
viewport, the Measurements box, the command palette, and every text
field's Enter/Escape/arrow handling ignore the keystroke, including the
confirming Enter WebKit delivers after composition ends. The Measurements
box reads ASCII digits; with a Japanese IME, toggle to direct input to
type a length, as in SketchUp.

Text that is created at a moment rather than rendered, such as a toast or
a tool's status hint, stays in the language that was active when it was
made until it is next refreshed.

Palette search normalizes the query and the candidate labels with NFKC
and case folding, so full-width and half-width input match. Translated
tool names and English tool names both match at the same rank. Hiragana
and katakana are not treated as equivalent.

## Adding a language

1. Add the BCP 47 tag to `ALL_LOCALES` and its name, written in that
   language, to `LOCALE_NAMES` in `app/src/i18n/locales.ts`. The Settings
   list shows every language in its own script.
2. Run `pnpm --dir app i18n:extract`. A new, untranslated
   `app/src/locales/<locale>/messages.po` appears.
3. Translate, as above.
4. When the catalog is complete, add the tag to `SHIPPED_LOCALES` in the
   same file; `pnpm --dir app i18n:check` now enforces completeness for
   it. Run `pnpm --dir app i18n:menus` to generate
   `shells/tauri/src-tauri/locales/menu.<locale>.json`, commit it, and
   add it to `MENU_LOCALES` in `shells/tauri/src-tauri/src/menu_labels.rs`
   so the shell embeds it. `nativeMenuParity.test.ts` fails if the three
   lists disagree or the JSON is stale.
5. Add a changelog entry naming the translator.

Right-to-left languages are not supported yet. The layout does not mirror,
and the pseudo-locale has no RTL variant enabled.

## Native menus

The macOS menu bar is built in Rust in `shells/tauri/src-tauri/src/main.rs`
from menu IDs. Every label is a lookup into
`shells/tauri/src-tauri/locales/menu.<locale>.json`, which
`pnpm --dir app i18n:menus` generates from the one label table in
`app/src/i18n/nativeMenu.ts` and the gettext catalogs, so a translator
never edits Rust and the HTML menu bar and the native one share entries.
The shell embeds one file per shipped locale at compile time.

When the app activates a locale it tells the shell, which relabels every
item in place (IDs, accelerators, check states, and enabled states
survive) and writes the locale to a one-line `language` file in the app
config directory. The next launch reads that file before building the
menu, so the menu bar is in the right language from its first frame. An
unshipped locale, the pseudo-locale included, shows English menus.

Menu labels are plain text with no placeholders. File names in the
Recent menu and window titles are shown verbatim.

## Checks that run in `scripts/verify.sh`

- `lingui extract` produces no diff against the committed catalogs.
- `i18n:check` finds no untranslated entry in a shipped locale.
- Oxlint with the Lingui rules reports nothing.
- The generated native-menu JSON matches the catalogs.
- Playwright runs pinned to English; one smoke test runs in the
  pseudo-locale and screenshots the main surfaces.
