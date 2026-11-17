//! `hew-cli plugin`: a preview build's subcommand for plugin development
//! (docs/agents/PLUGINS.md "Developer mode and folder plugins"). The module
//! is compiled only under `cfg(hew_preview)`, so a production binary holds
//! none of it.
//!
//! Nothing here loads, installs or changes a plugin. `plugin dev <dir>`
//! sends `hew.plugin.dev`, which only asks the app to show the person a
//! request; the person picks the folder and reads what the plugin asks for
//! in the app. The CLI then watches `hew.plugin.list` and says what it can
//! see. That list holds no folder path and no running state, so the CLI
//! cannot tell which listed plugin came from the folder it was given, and
//! it cannot tell that a plugin is running or that it reloaded. It reports
//! the folder plugins that appear or change after the request was sent,
//! and says so.

use crate::live::{self, LiveError, LiveOptions, LiveSession};
use api::{DispatchOutcome, Request, RequestId, Response};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long `plugin dev` waits between looks at the plugin list.
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// What the command does, besides the folder to ask about.
pub struct DevOptions {
    /// Send the request, print the first answer and stop, for scripts.
    pub once: bool,
    /// How long to wait between looks at the list.
    pub interval: Duration,
    /// Stop after this many looks at the list (a test hook; `None` keeps
    /// watching until the app closes the connection or the user stops it).
    pub max_polls: Option<usize>,
}

impl Default for DevOptions {
    fn default() -> Self {
        DevOptions {
            once: false,
            interval: POLL_INTERVAL,
            max_polls: None,
        }
    }
}

/// Why the command stopped, as the process exit code.
const OK: i32 = 0;
const REFUSED: i32 = 1;

/// The longest a string from the app is printed.
const MAX_PRINTED: usize = 160;

/// A string that came from the app, made safe to print to a terminal:
/// control characters (escape, bell, carriage return, newline), the
/// characters that reorder or hide text, and other invisible ones are
/// removed, whitespace runs become one space, and the result is cut to
/// [`MAX_PRINTED`] characters with an ellipsis.
pub fn clean(text: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    let mut count = 0usize;
    for c in text.chars() {
        if c.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if c.is_control()
            || matches!(c,
                '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{115F}' | '\u{1160}'
                | '\u{17B4}'..='\u{17B5}' | '\u{180B}'..='\u{180F}'
                | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}' | '\u{3164}' | '\u{FE00}'..='\u{FE0F}'
                | '\u{FEFF}' | '\u{FFA0}' | '\u{FFF0}'..='\u{FFFB}'
                | '\u{E0000}'..='\u{E0FFF}')
        {
            continue;
        }
        let needed = if space { 2 } else { 1 };
        if count + needed > MAX_PRINTED {
            out.push('\u{2026}');
            return out;
        }
        if space {
            out.push(' ');
            count += 1;
            space = false;
        }
        out.push(c);
        count += 1;
    }
    out
}

/// The folder to ask about, as a canonical absolute path: symlinks and any
/// `.` or `..` segments resolved, no trailing separator. It must exist and
/// be a directory. What is sent is exactly what is returned.
pub fn absolute_folder(dir: &Path) -> Result<String, String> {
    let shown = clean(&dir.display().to_string());
    let meta = std::fs::metadata(dir)
        .map_err(|e| format!("cannot read \"{shown}\": {}", clean(&e.to_string())))?;
    if !meta.is_dir() {
        return Err(format!("\"{shown}\" is not a folder"));
    }
    let resolved: PathBuf = std::fs::canonicalize(dir)
        .map_err(|e| format!("cannot resolve \"{shown}\": {}", clean(&e.to_string())))?;
    let resolved = plain(resolved);
    resolved
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| format!("\"{shown}\" is not a path Hew can be told about"))
}

/// A canonical Windows path without its `\\?\` prefix, which the app does
/// not expect; any other path as it is.
fn plain(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(rest) = path.to_str().and_then(|p| p.strip_prefix(r"\\?\")) {
            if !rest.starts_with("UNC\\") {
                return PathBuf::from(rest);
            }
        }
    }
    path
}

/// One request, answered: its result, or the refusal as plain words.
fn call(
    session: &mut LiveSession,
    id: &str,
    method: &str,
    params: Value,
) -> Result<Value, Failure> {
    let request = Request {
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Text(id.to_string())),
        method: method.to_string(),
        params: Some(params),
        document: None,
    };
    match session.dispatch(request) {
        Ok(DispatchOutcome::Reply(response) | DispatchOutcome::DryRun(response)) => {
            answer(response)
        }
        Ok(DispatchOutcome::Dropped | DispatchOutcome::Forward(_)) => {
            unreachable!("a live session answers an id-carrying request with a reply")
        }
        Err(LiveError::Closed) => Err(Failure::Closed),
        Err(e) => Err(Failure::Transport(e.to_string())),
    }
}

fn answer(response: Response) -> Result<Value, Failure> {
    match response.error {
        None => Ok(response.result.unwrap_or(Value::Null)),
        Some(err) => {
            let name = err
                .data
                .as_ref()
                .and_then(|d| d.get("refusal"))
                .and_then(Value::as_str)
                .map(clean);
            let explanation = err
                .data
                .as_ref()
                .and_then(|d| d.get("explanation"))
                .and_then(Value::as_str)
                .map(clean)
                .unwrap_or_else(|| clean(&err.message));
            Err(Failure::Refused {
                code: err.code,
                name,
                explanation,
            })
        }
    }
}

enum Failure {
    /// The app answered with an error.
    Refused {
        code: i64,
        name: Option<String>,
        explanation: String,
    },
    /// The app closed the connection.
    Closed,
    /// The connection failed some other way.
    Transport(String),
}

/// A refusal in plain words, for the person at the terminal.
pub fn refusal_words(code: i64, name: Option<&str>, explanation: &str) -> String {
    match name {
        Some("developer_mode_off") => "Turn on developer mode in Hew: Settings > Plugins.".into(),
        Some("confirmation_open") => {
            "Hew is already asking you something. Answer it in Hew, then run this again.".into()
        }
        Some("request_waiting") => {
            "Another request to load a plugin is already waiting in Hew. Answer or dismiss it there, then run this again.".into()
        }
        Some("request_dismissed") => {
            "You dismissed a request in Hew a moment ago. Try again in a minute.".into()
        }
        Some("plugins_off") => {
            "Plugins are turned off in Hew. Turn them on in Settings, then run this again.".into()
        }
        Some("plugins_disabled" | "no_plugin_dir") => {
            "Plugins are not available in this copy of Hew.".into()
        }
        Some("io") => "Hew could not read or write its plugin folder.".into(),
        Some("preview_only") => {
            "This copy of Hew was built without plugin support, so it cannot load a plugin.".into()
        }
        Some(other) => format!("Hew answered: {}", clean(other)),
        None if code == -32602 => {
            format!("Hew did not accept the request: {}", clean(explanation))
        }
        None => clean(explanation),
    }
}

/// Prints a failure and returns the exit code. A closed connection is a
/// normal end only after the app has answered at least once.
fn report(out: &mut impl Write, what: &str, failure: Failure, answered: bool) -> i32 {
    match failure {
        Failure::Refused {
            code,
            name,
            explanation,
        } => {
            let _ = writeln!(
                out,
                "{}",
                refusal_words(code, name.as_deref(), &explanation)
            );
            REFUSED
        }
        Failure::Closed if answered => {
            let _ = writeln!(out, "Hew closed the connection.");
            OK
        }
        Failure::Closed => {
            let _ = writeln!(out, "Hew closed the connection before answering.");
            REFUSED
        }
        Failure::Transport(why) => {
            let _ = writeln!(out, "hew-cli plugin {what}: {}", clean(&why));
            REFUSED
        }
    }
}

/// A listed plugin, as far as the list says.
#[derive(Debug, Clone, PartialEq)]
pub struct Listed {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub source: Option<String>,
    pub enabled: bool,
    pub state: String,
}

/// Reads a `hew.plugin.list` result: the plugins, and developer mode.
pub fn parse_list(result: &Value) -> (Vec<Listed>, bool) {
    // Everything here came over the socket: cleaned before it can be printed.
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(clean);
    let plugins = result
        .get("plugins")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    Some(Listed {
                        id: text(row, "id")?,
                        name: text(row, "name").unwrap_or_default(),
                        version: text(row, "version"),
                        source: text(row, "source"),
                        enabled: row.get("enabled").and_then(Value::as_bool).unwrap_or(false),
                        state: text(row, "state").unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let developer_mode = result
        .get("developer_mode")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    (plugins, developer_mode)
}

/// Whether the app says a request waits for the person, from a
/// `hew.plugin.list` result; `None` from an app that does not say.
pub fn request_waiting(result: &Value) -> Option<bool> {
    result.get("request_waiting").and_then(Value::as_bool)
}

/// What a plugin looked like at one look: the parts a change shows in.
type Look = (Option<String>, bool, String);

fn look(p: &Listed) -> Look {
    (p.version.clone(), p.enabled, p.state.clone())
}

/// Folder plugins, and any plugin the app could not read (its source is
/// unknown, so it may be a folder), keyed by id.
fn watched(plugins: &[Listed]) -> BTreeMap<String, Listed> {
    plugins
        .iter()
        .filter(|p| p.source.as_deref() == Some("dir") || p.state == "unreadable")
        .map(|p| (p.id.clone(), p.clone()))
        .collect()
}

/// The status line for a watched plugin that appeared or changed.
pub fn status_line(p: &Listed) -> String {
    let version = p.version.as_deref().unwrap_or("");
    let label = if version.is_empty() {
        p.name.clone()
    } else {
        format!("{} {version}", p.name)
    };
    match p.state.as_str() {
        "needs_developer_mode" => "Turn on developer mode in Hew: Settings > Plugins.".to_string(),
        "needs_review" => format!("Waiting for you to review a change in Hew ({label})."),
        "checking" => format!("Hew is checking the folder of {label}."),
        "plugins_off" => format!(
            "{label} is not running: Plugins are turned off in Hew. Turn them on in Settings."
        ),
        "unreadable" => format!(
            "Hew cannot read the record of one plugin ({}). Look at it in Settings > Plugins.",
            p.id
        ),
        _ if !p.enabled => format!("Installed {label}. It is turned off in Hew."),
        _ => format!("{label} is installed from a developer folder and turned on."),
    }
}

/// The lines printed when a watched plugin changes: its status, and, the
/// first time a plugin counts as loaded, what the list cannot say.
fn changes(
    before: &BTreeMap<String, Look>,
    now: &BTreeMap<String, Listed>,
    noted: &mut bool,
) -> Vec<String> {
    let mut lines = Vec::new();
    for (id, p) in now {
        if before.get(id) == Some(&look(p)) {
            continue;
        }
        lines.push(status_line(p));
        if p.state == "installed" && p.enabled && !*noted {
            *noted = true;
            lines.push(
                "Hew's list does not say whether a plugin is running; check Settings > Plugins."
                    .to_string(),
            );
        }
    }
    lines
}

/// `hew-cli plugin dev <dir>` once connected: asks, then watches. Writes
/// its lines to `out`; returns the exit code.
pub fn dev_session(
    session: &mut LiveSession,
    path: &str,
    options: &DevOptions,
    out: &mut impl Write,
) -> i32 {
    // What was already installed, so only what appears or changes after
    // the request is reported. Not needed by a script that stops after the
    // first answer.
    let mut known: BTreeMap<String, Look> = BTreeMap::new();
    if !options.once {
        match call(session, "plugin-list", "hew.plugin.list", json!({})) {
            Ok(result) => {
                let (plugins, _) = parse_list(&result);
                known = watched(&plugins)
                    .into_iter()
                    .map(|(id, p)| (id, look(&p)))
                    .collect();
            }
            Err(failure) => return report(out, "dev", failure, false),
        }
    }
    match call(
        session,
        "plugin-dev",
        "hew.plugin.dev",
        json!({ "path": path }),
    ) {
        Ok(_) => {}
        Err(failure) => return report(out, "dev", failure, false),
    }
    let _ = writeln!(out, "Asked Hew to open this folder:");
    let _ = writeln!(out, "  {}", clean(path));
    let _ = writeln!(
        out,
        "Waiting for you in Hew: choose the folder and read what the plugin asks for."
    );
    let _ = out.flush();
    if options.once {
        return OK;
    }
    let mut noted = false;
    // Whether any folder plugin has appeared or changed since the request.
    let mut seen = false;
    let mut polls = 0usize;
    loop {
        if options.max_polls.is_some_and(|max| polls >= max) {
            return OK;
        }
        std::thread::sleep(options.interval);
        polls += 1;
        match call(session, "plugin-list", "hew.plugin.list", json!({})) {
            Ok(result) => {
                let (plugins, _) = parse_list(&result);
                let now = watched(&plugins);
                let lines = changes(&known, &now, &mut noted);
                seen |= !lines.is_empty();
                for line in lines {
                    let _ = writeln!(out, "{line}");
                }
                let _ = out.flush();
                known = now.iter().map(|(id, p)| (id.clone(), look(p))).collect();
                // The request ended with nothing to show for it: dismissed,
                // expired, or used up by a folder that did not install.
                if request_waiting(&result) == Some(false) && !seen {
                    let _ = writeln!(
                        out,
                        "The request is no longer waiting in Hew: it was dismissed or declined, or it ran out of time."
                    );
                    let _ = out.flush();
                    return REFUSED;
                }
            }
            Err(failure) => return report(out, "dev", failure, true),
        }
    }
}

/// `hew-cli plugin dev <dir> --live ...`.
pub fn dev_live(dir: &Path, opts: &LiveOptions, options: &DevOptions) -> i32 {
    let path = match absolute_folder(dir) {
        Ok(p) => p,
        Err(why) => {
            eprintln!("hew-cli plugin dev: {why}");
            return REFUSED;
        }
    };
    let mut session = match live::connect_live(opts, live::build_hello_request("hew-cli:plugin")) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hew-cli plugin dev --live: {e}");
            return REFUSED;
        }
    };
    dev_session(&mut session, &path, options, &mut std::io::stdout())
}

/// The lines `plugin list` prints.
pub fn list_lines(result: &Value) -> Vec<String> {
    let (plugins, developer_mode) = parse_list(result);
    let mut lines = vec![format!(
        "Developer mode: {}",
        if developer_mode { "on" } else { "off" }
    )];
    if plugins.is_empty() {
        lines.push("No plugins are installed.".to_string());
    }
    for p in &plugins {
        lines.push(format!(
            "{}  {} {}  from {}  {}{}",
            p.id,
            p.name,
            p.version.as_deref().unwrap_or("(unknown version)"),
            p.source.as_deref().unwrap_or("an unreadable record"),
            state_words(&p.state),
            if p.enabled { "" } else { ", turned off" },
        ));
    }
    lines
}

fn state_words(state: &str) -> &str {
    match state {
        "needs_developer_mode" => "needs developer mode",
        "needs_review" => "waiting for a review",
        "checking" => "being checked",
        "plugins_off" => "not running: Plugins are turned off in Hew",
        "unreadable" => "unreadable",
        "installed" => "installed",
        other => other,
    }
}

/// `hew-cli plugin list --live`.
pub fn list_live(opts: &LiveOptions) -> i32 {
    let mut session = match live::connect_live(opts, live::build_hello_request("hew-cli:plugin")) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hew-cli plugin list --live: {e}");
            return REFUSED;
        }
    };
    list_session(&mut session, &mut std::io::stdout())
}

/// [`list_live`] once connected.
pub fn list_session(session: &mut LiveSession, out: &mut impl Write) -> i32 {
    match call(session, "plugin-list", "hew.plugin.list", json!({})) {
        Ok(result) => {
            for line in list_lines(&result) {
                let _ = writeln!(out, "{line}");
            }
            OK
        }
        Err(failure) => report(out, "list", failure, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, source: &str, state: &str, enabled: bool) -> Value {
        json!({ "id": id, "name": id, "version": "1.0.0", "source": source,
                "enabled": enabled, "state": state })
    }

    #[test]
    fn only_folder_plugins_and_unreadable_records_are_watched() {
        let (plugins, _) = parse_list(&json!({
            "plugins": [
                row("a", "file", "installed", true),
                row("b", "dir", "installed", true),
                { "id": "c", "name": "c", "version": null, "source": null,
                  "enabled": false, "state": "unreadable" },
            ],
            "developer_mode": true,
        }));
        let ids: Vec<String> = watched(&plugins).into_keys().collect();
        assert_eq!(ids, ["b", "c"]);
    }

    #[test]
    fn each_state_gets_its_plain_line() {
        let (plugins, _) = parse_list(&json!({ "plugins": [
            row("loaded", "dir", "installed", true),
            row("off", "dir", "installed", false),
            row("review", "dir", "needs_review", true),
            row("mode", "dir", "needs_developer_mode", true),
        ] }));
        let lines: Vec<String> = plugins.iter().map(status_line).collect();
        assert_eq!(
            lines[0],
            "loaded 1.0.0 is installed from a developer folder and turned on."
        );
        assert_eq!(lines[1], "Installed off 1.0.0. It is turned off in Hew.");
        assert_eq!(
            lines[2],
            "Waiting for you to review a change in Hew (review 1.0.0)."
        );
        assert_eq!(
            lines[3],
            "Turn on developer mode in Hew: Settings > Plugins."
        );
    }

    #[test]
    fn a_change_is_reported_once_and_what_the_list_cannot_say_is_said_once() {
        let (first, _) = parse_list(&json!({ "plugins": [row("p", "dir", "installed", true)] }));
        let now = watched(&first);
        let mut noted = false;
        let lines = changes(&BTreeMap::new(), &now, &mut noted);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines[0].starts_with("p 1.0.0 is installed from a developer folder and turned on.")
        );
        assert!(lines[1].contains("does not say whether a plugin is running"));
        // Seen again, unchanged: nothing.
        let before: BTreeMap<String, Look> =
            now.iter().map(|(id, p)| (id.clone(), look(p))).collect();
        assert!(changes(&before, &now, &mut noted).is_empty());
        // Changed (a new version): the status line only, the note is spent.
        let (second, _) = parse_list(&json!({ "plugins": [
            { "id": "p", "name": "p", "version": "1.1.0", "source": "dir",
              "enabled": true, "state": "installed" }
        ] }));
        let lines = changes(&before, &watched(&second), &mut noted);
        assert_eq!(
            lines,
            ["p 1.1.0 is installed from a developer folder and turned on."]
        );
    }

    #[test]
    fn refusals_are_said_in_plain_words() {
        assert_eq!(
            refusal_words(-32000, Some("developer_mode_off"), "x"),
            "Turn on developer mode in Hew: Settings > Plugins."
        );
        assert!(
            refusal_words(-32000, Some("preview_only"), "x").contains("without plugin support")
        );
        assert!(refusal_words(-32602, None, "path missing").contains("path missing"));
        assert_eq!(
            refusal_words(-32000, Some("something_new"), "Because."),
            "Hew answered: something_new"
        );
        assert_eq!(refusal_words(-32000, None, "Because."), "Because.");
        assert!(refusal_words(-32000, Some("request_waiting"), "x").contains("already waiting"));
        assert!(refusal_words(-32000, Some("request_dismissed"), "x").contains("in a minute"));
        assert_eq!(
            refusal_words(-32000, Some("io"), "raw"),
            "Hew could not read or write its plugin folder."
        );
        assert_eq!(
            refusal_words(-32000, Some("new\u{1b}[2J\u{7}name"), "x"),
            "Hew answered: new[2Jname"
        );
    }

    #[test]
    fn the_folder_must_exist_and_be_a_folder() {
        let dir = std::env::temp_dir().join(format!("hew-cli-plugin-dir-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("plugin.json");
        std::fs::write(&file, "{}").unwrap();
        let ok = absolute_folder(&dir).expect("a folder");
        assert!(Path::new(&ok).is_absolute());
        assert!(absolute_folder(&file).unwrap_err().contains("not a folder"));
        assert!(absolute_folder(&dir.join("missing")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_list_prints_developer_mode_and_one_line_per_plugin() {
        let lines = list_lines(&json!({ "plugins": [
            row("com.example.a", "dir", "needs_review", false)
        ], "developer_mode": true }));
        assert_eq!(lines[0], "Developer mode: on");
        assert_eq!(
            lines[1],
            "com.example.a  com.example.a 1.0.0  from dir  waiting for a review, turned off"
        );
        assert_eq!(
            list_lines(&json!({ "plugins": [], "developer_mode": false })),
            ["Developer mode: off", "No plugins are installed."]
        );
    }

    #[test]
    fn text_from_the_app_is_printed_without_control_or_reordering_characters() {
        let hostile = "evil\u{1b}[2J\u{7}na\rme\u{202E}gnp\u{2066}x\n\nnext";
        assert_eq!(clean(hostile), "evil[2Jna megnpx next");
        let long = "a".repeat(5000);
        let cut = clean(&long);
        assert_eq!(cut.chars().count(), MAX_PRINTED + 1);
        assert!(cut.ends_with('\u{2026}'));
        let (plugins, _) = parse_list(&json!({ "plugins": [
            { "id": "i\u{1b}d", "name": "n\u{7}ame\u{1b}]0;title", "version": "1\r.0",
              "source": "dir", "enabled": true, "state": "ins\u{1b}talled" },
            { "id": "bad\u{1b}[31m", "name": "bad\u{1b}[31m", "version": null,
              "source": null, "enabled": false, "state": "unreadable" },
        ] }));
        let mut printed = list_lines(&json!({ "plugins": [
            { "id": "bad\u{1b}[31m", "name": "x\u{202E}y", "version": null,
              "source": null, "enabled": false, "state": "unreadable" }
        ] }));
        printed.extend(plugins.iter().map(status_line));
        for line in printed {
            assert!(
                !line
                    .chars()
                    .any(|c| c.is_control() || ('\u{202A}'..='\u{202E}').contains(&c)),
                "{line:?}"
            );
        }
    }

    #[test]
    fn a_closed_connection_is_a_failure_until_the_app_has_answered() {
        let mut out = Vec::new();
        assert_eq!(report(&mut out, "dev", Failure::Closed, false), REFUSED);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Hew closed the connection before answering.\n"
        );
        let mut out = Vec::new();
        assert_eq!(report(&mut out, "dev", Failure::Closed, true), OK);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Hew closed the connection.\n"
        );
    }

    #[test]
    fn the_path_sent_is_canonical_with_no_dots_or_trailing_slash() {
        let base =
            std::env::temp_dir().join(format!("hew-cli-plugin-canon-{}", std::process::id()));
        std::fs::create_dir_all(base.join("a")).unwrap();
        std::fs::create_dir_all(base.join("b")).unwrap();
        let tricky = base.join("a").join("..").join("b").join(".");
        let sent = absolute_folder(&tricky).expect("a folder");
        let want = std::fs::canonicalize(base.join("b")).unwrap();
        assert_eq!(Path::new(&sent), plain(want).as_path());
        assert!(!sent.ends_with('/') || sent == "/");
        assert!(Path::new(&sent).components().all(|c| !matches!(
            c,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn each_refusal_the_app_gives_for_the_request_is_plain_words_and_exit_one() {
        let cases: [(&str, &str); 6] = [
            (
                "developer_mode_off",
                "Turn on developer mode in Hew: Settings > Plugins.",
            ),
            (
                "plugins_off",
                "Plugins are turned off in Hew. Turn them on in Settings, then run this again.",
            ),
            (
                "confirmation_open",
                "Hew is already asking you something. Answer it in Hew, then run this again.",
            ),
            (
                "request_waiting",
                "Another request to load a plugin is already waiting in Hew. Answer or dismiss it there, then run this again.",
            ),
            (
                "request_dismissed",
                "You dismissed a request in Hew a moment ago. Try again in a minute.",
            ),
            ("dry_run_host_command", "Hew answered: dry_run_host_command"),
        ];
        for (name, words) in cases {
            let mut out = Vec::new();
            let failure = Failure::Refused {
                code: -32000,
                name: Some(name.to_string()),
                explanation: "ignored".to_string(),
            };
            assert_eq!(report(&mut out, "dev", failure, false), REFUSED, "{name}");
            assert_eq!(
                String::from_utf8(out).unwrap(),
                format!("{words}\n"),
                "{name}"
            );
        }
    }

    #[test]
    fn a_folder_plugin_while_plugins_are_off_is_not_running() {
        let (plugins, _) = parse_list(&json!({ "plugins": [
            row("p", "dir", "plugins_off", true)
        ] }));
        assert_eq!(
            status_line(&plugins[0]),
            "p 1.0.0 is not running: Plugins are turned off in Hew. Turn them on in Settings."
        );
        let lines = list_lines(&json!({ "plugins": [row("p", "dir", "plugins_off", true)] }));
        assert_eq!(
            lines[1],
            "p  p 1.0.0  from dir  not running: Plugins are turned off in Hew"
        );
    }

    #[test]
    fn the_list_says_whether_a_request_waits_when_the_app_does() {
        assert_eq!(
            request_waiting(&json!({ "request_waiting": true })),
            Some(true)
        );
        assert_eq!(
            request_waiting(&json!({ "request_waiting": false })),
            Some(false)
        );
        assert_eq!(request_waiting(&json!({ "plugins": [] })), None);
    }
}
