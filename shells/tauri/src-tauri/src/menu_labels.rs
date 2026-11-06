//! Native menu labels per locale (docs/LOCALIZATION.md "Native menus").
//!
//! The menu bar is built from ids; every label comes from a per-locale JSON
//! file that `pnpm --dir app i18n:menus` generates from the app's one label
//! table (`app/src/i18n/nativeMenu.ts`) and its gettext catalogs. The files
//! are embedded at compile time, one per shipped locale, so the shell never
//! reads them from disk and a missing or malformed file is a build-time
//! problem, not a blank menu.
//!
//! The frontend tells the shell which locale is active (`set_menu_locale`)
//! and the shell remembers it in a one-line file in the app config dir, so
//! the next launch builds the menu in that language from its first frame
//! rather than flashing English until the webview comes up.

use std::collections::HashMap;
use std::path::Path;

/// Embedded label files. Add a line when a locale ships
/// (`SHIPPED_LOCALES` in `app/src/i18n/locales.ts`); the app's
/// `nativeMenuParity.test.ts` checks the two lists agree.
const MENU_LOCALES: &[(&str, &str)] = &[
    ("en", include_str!("../locales/menu.en.json")),
    ("ar", include_str!("../locales/menu.ar.json")),
    // Debug aid: the pseudo-locale's menu, so the native menu bar can be
    // checked like every other surface. Never offered to users.
    (
        "pseudo-LOCALE",
        include_str!("../locales/menu.pseudo-LOCALE.json"),
    ),
];

const FALLBACK_LOCALE: &str = "en";
const LANGUAGE_FILE: &str = "language";

/// The label table for one locale, with English filled in for any key the
/// locale's file lacks, and the key itself as a last resort (a visible
/// `submenu:file` beats a crash or an empty title).
#[derive(Debug, Clone)]
pub struct MenuLabels {
    locale: String,
    labels: HashMap<String, String>,
}

fn parse(json: &str) -> HashMap<String, String> {
    serde_json::from_str::<HashMap<String, String>>(json).unwrap_or_default()
}

fn file_for(locale: &str) -> Option<&'static str> {
    MENU_LOCALES
        .iter()
        .find(|(l, _)| l.eq_ignore_ascii_case(locale))
        .map(|(_, json)| *json)
}

impl MenuLabels {
    /// Labels for `locale`, falling back to English for an unshipped locale
    /// (the pseudo-locale, a catalog still being translated).
    pub fn for_locale(locale: &str) -> Self {
        let mut labels = parse(file_for(FALLBACK_LOCALE).unwrap_or("{}"));
        let resolved = match file_for(locale) {
            Some(json) if !locale.eq_ignore_ascii_case(FALLBACK_LOCALE) => {
                labels.extend(parse(json));
                locale.to_string()
            }
            _ => FALLBACK_LOCALE.to_string(),
        };
        MenuLabels {
            locale: resolved,
            labels,
        }
    }

    /// The locale these labels resolved to.
    pub fn locale(&self) -> &str {
        &self.locale
    }

    /// The label for `key`, or the key itself when no file names it.
    pub fn get<'a>(&'a self, key: &'a str) -> &'a str {
        self.labels.get(key).map(String::as_str).unwrap_or(key)
    }

    /// The label for `key` only if a file names it — used when walking an
    /// existing menu, where ids that are not labels (`recent:<path>`) must
    /// be left alone.
    pub fn lookup(&self, key: &str) -> Option<&str> {
        self.labels.get(key).map(String::as_str)
    }
}

/// The locale the last session ran its menu in, if the frontend recorded
/// one. A missing or unreadable file means English.
pub fn read_persisted_locale(config_dir: Option<&Path>) -> Option<String> {
    let path = config_dir?.join(LANGUAGE_FILE);
    // A locale tag is a few bytes; never read an oversized file into memory.
    if std::fs::metadata(&path).ok()?.len() > 64 {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    let locale = text.trim();
    if locale.is_empty()
        || locale.len() > 32
        || !locale
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return None;
    }
    Some(locale.to_string())
}

/// Remember `locale` for the next launch. Best-effort: a failure here only
/// costs an English first frame next time.
pub fn write_persisted_locale(config_dir: &Path, locale: &str) {
    if std::fs::create_dir_all(config_dir).is_err() {
        return;
    }
    let _ = std::fs::write(config_dir.join(LANGUAGE_FILE), format!("{locale}\n"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_file_has_every_section_title() {
        let l = MenuLabels::for_locale("en");
        assert_eq!(l.locale(), "en");
        assert_eq!(l.get("submenu:file"), "File");
        assert_eq!(l.get("file-new"), "New");
        assert_eq!(l.get("predefined:quit"), "Quit Hew");
    }

    #[test]
    fn unknown_locale_falls_back_to_english_and_unknown_key_to_itself() {
        let l = MenuLabels::for_locale("xx-unknown");
        assert_eq!(l.locale(), "en");
        assert_eq!(l.get("file-new"), "New");
        assert_eq!(l.get("no-such-key"), "no-such-key");
        assert_eq!(l.lookup("no-such-key"), None);
    }

    #[test]
    fn persisted_locale_round_trips_and_rejects_garbage() {
        let dir = std::env::temp_dir().join(format!("hew-menu-labels-{}", std::process::id()));
        write_persisted_locale(&dir, "ja");
        assert_eq!(read_persisted_locale(Some(&dir)).as_deref(), Some("ja"));
        std::fs::write(dir.join(LANGUAGE_FILE), "../etc\n").unwrap();
        assert_eq!(read_persisted_locale(Some(&dir)), None);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(read_persisted_locale(None), None);
    }
}
