//! `hew-cli`: standalone MCP adapter, script runner, and one-shot dispatch
//! (docs/agents/HEW_API.md §12). This binary is a thin argv shell — every
//! subcommand's real logic lives in the `hew_cli` library
//! (`crates/hew-cli/src/lib.rs`) so `tests/cli.rs` can call it directly.
//!
//! Headless is the default. `--live [--launch] [--instance <pid>]`
//! discovers and forwards to a running desktop instance instead
//! (docs/agents/HEW_API.md §11.2, §12) — see [`hew_cli::live`].

use hew_cli::live::LiveOptions;
use std::path::PathBuf;

/// The top-level help page. Structure follows bettercli.org's help-page
/// guidance: what the program is, how to start, what the commands are,
/// what it reads from the environment, and where to go next. Printed to
/// stdout on request (a help page is output, not an error) and to stderr
/// only when it accompanies a usage error.
const USAGE_HEAD: &str = r#" — build and edit Hew models from a script, a shell, or an AI client.

USAGE
    hew-cli <command> [options]

COMMANDS
    run <script>          Run a file of JSON-RPC envelopes in order
    dispatch <method>     Send one command
    documents             List the app's open documents (--live)
    events                Print the app's events as they happen (--live)
    library index <dir>   Write hew.json and thumbnails for a folder of Library
                          items, or --check that they are up to date
    catalog check <ref>   Run the listing checks on one source
    catalog build <dir>   Build the catalog's generated files (--out <dir>)
    mcp                   Serve MCP on stdio, for an AI client
"#;

/// The rest of the top-level help page, after the command list.
const USAGE_TAIL: &str = r#"
EXAMPLES
    # Draw a square, pull it into a solid, write the file.
    hew-cli run box.jsonl --out box.hew

    # Ask a saved model what is in it. Read-only, so the file is untouched.
    hew-cli dispatch hew.query.scene --file box.hew

    # Frame the model in the app the user already has open.
    hew-cli dispatch hew.view.zoom_extents --live

    # See every window the app has open, then work in one that is not in front.
    hew-cli documents --live
    hew-cli dispatch hew.query.scene --live --document doc_2

    # Watch what changes in the app, one JSON line per event.
    hew-cli events --live

    # Serve an AI client. Put this in the client's MCP config.
    hew-cli mcp

WHERE THE DOCUMENT LIVES
    By default hew-cli holds the document itself and never contacts the
    desktop app. Pass --live and it drives the document the user has open
    instead: edits land in their window, in their undo history, labeled
    with the connection that made them.

OPTIONS
    --live                Work on a running app's open document
    --launch              With --live, start the app first and wait for it
    --instance <pid>      Pick one when several apps are running
    --document <ref>      With --live, work in this open document (id, path,
                          label, or title) instead of the front window
    --out <file.hew>      run: write the finished document here
    --file <model.hew>    dispatch: open this file, and save it back
    -h, --help            Print help
    -V, --version         Print version

ENVIRONMENT
    HEW_APP               Executable --launch starts. On macOS it defaults
                          to `open -a Hew`.
    HEW_RUNTIME_DIR       Where to look for a running app. A testing hook;
                          leave it unset.

LEARN MORE
    hew-cli <command> --help    what each command takes, and an example
    docs/API_GUIDE.md           how to drive Hew, with worked examples
    docs/agents/HEW_API.md             the protocol itself
"#;

/// The top-level help page, led by the name and version. The version is
/// formatted at print time because [`hew_cli::VERSION`] is chosen at
/// compile time from an environment variable.
fn usage() -> String {
    format!(
        "hew-cli {}{USAGE_HEAD}{PLUGIN_COMMAND_LINE}{USAGE_TAIL}",
        hew_cli::VERSION
    )
}

/// The `plugin` line of the command list: present in a preview build only.
#[cfg(hew_preview)]
const PLUGIN_COMMAND_LINE: &str =
    "    plugin                Load a plugin from a folder while you build it (--live)\n";
#[cfg(not(hew_preview))]
const PLUGIN_COMMAND_LINE: &str = "";

/// `hew-cli run --help`.
const USAGE_RUN: &str = r#"hew-cli run — run a file of JSON-RPC envelopes in order.

USAGE
    hew-cli run <script.json|script.jsonl> [--out <file.hew>]
                [--live [--launch] [--instance <pid>] [--document <ref>]]

The script speaks the protocol directly: one JSON object per line, or a
single JSON array. It opens with its own hew.meta.hello, then
hew.doc.new or hew.doc.open to get a document (headless), or
hew.doc.attach to bind to the app's (live).

Each reply prints to stdout as one line. The first refusal stops the
script, prints to stderr, and exits 1.

EXAMPLE
    A square pulled into a solid. The extrude names the rectangle's
    result by the label the draw gave it, so the two are one undo step:

    {"jsonrpc":"2.0","id":1,"method":"hew.meta.hello","params":{"protocol":1}}
    {"jsonrpc":"2.0","id":2,"method":"hew.doc.new","params":{}}
    {"jsonrpc":"2.0","id":3,"method":"hew.doc.transact","params":{"commands":[
      {"method":"hew.sketch.draw_rect","as":"rect","params":{"plane":{"ground":true},"corner_a":[0,0,0],"corner_b":[1,1,0]}},
      {"method":"hew.solid.extrude","params":{"region":{"$ref":"rect#/region_id"},"distance":0.5}}
    ]}}

OPTIONS
    --out <file.hew>    Write the finished document here. Headless only —
                        live, add a hew.doc.save frame with a path.
    --live              Run against the app's open document instead.
    --launch            Start the app first and wait for it.
    --instance <pid>    Pick one when several apps are running.
    --document <ref>    Live: attach to this open document (an id such as
                        doc_2, or its path, label, or title) rather than
                        the front window. A frame may also carry its own
                        "document" member to address one document for just
                        that command.
"#;

/// `hew-cli dispatch --help`.
const USAGE_DISPATCH: &str = r#"hew-cli dispatch — send one command.

USAGE
    hew-cli dispatch <method> [params-json]
                     (--file <model.hew> |
                      --live [--launch] [--instance <pid>] [--document <ref>])

Params are a JSON object, defaulting to {}. The reply prints to stdout.
A refusal prints to stderr and exits 1.

Exactly one of --file or --live: the command has to act on something.

EXAMPLES
    hew-cli dispatch hew.query.scene --file chair.hew
    hew-cli dispatch hew.entity.move '{"ids":["obj_1"],"translation":[0,0,0.5]}' --file chair.hew
    hew-cli dispatch hew.doc.export '{"format":"stl","path":"chair.stl"}' --live
    hew-cli dispatch hew.query.scene --live --document "524 E Tremaine Ave"
    hew-cli dispatch hew.doc.export '{"format":"skp","path":"chair.skp","units":"mm"}' --file chair.hew

OPTIONS
    --file <model.hew>  Open this file. A command that changes it saves
                        it back in place; a read-only one leaves it alone.
    --live              Send to the app's open document instead. Nothing
                        is saved — the edit is already in the user's
                        document, and their undo history.
    --launch            Start the app first and wait for it.
    --instance <pid>    Pick one when several apps are running.
    --document <ref>    Send to this open document (an id such as doc_2, or
                        its path, label, or title) instead of the front
                        window. It is reached where it is: nothing is
                        raised, and the user's window order is untouched.
"#;

/// `hew-cli documents --help`.
const USAGE_DOCUMENTS: &str = r#"hew-cli documents — list the app's open documents.

USAGE
    hew-cli documents --live [--launch] [--instance <pid>]

Prints one JSON object per open document, in window creation order:
its id (doc_N — unique for the app's lifetime, never reused), title,
path (null until saved), dirty (unsaved changes), document_id (the
persistent id the file carries, null until first saved), front (the
window the user is looking at), and label if one was given at
hew.doc.new/open.

Pass an id, path, label, or title to --document on run/dispatch/mcp to
work in any of them, front or not. The same list is hew.meta.documents.

EXAMPLE
    hew-cli documents --live

OPTIONS
    --live              Ask the running app. Required: a document you
                        hold yourself has nothing to list.
    --launch            Start the app first and wait for it.
    --instance <pid>    Pick one when several apps are running.
"#;

/// `hew-cli events --help`.
const USAGE_EVENTS: &str = r#"hew-cli events — print the app's events as they happen.

USAGE
    hew-cli events --live [--launch] [--instance <pid>] [--document <ref>]
                   [--filter <name,name,...>]

Attaches to the app's front window (or --document), subscribes to its
events, and prints each one to stdout as it arrives: one compact JSON
line per notification, such as hew.event.document_changed with what
the edit added, removed, or modified. The subscribe reply goes to
stderr. Runs until the app closes the connection (exit 0) or something
fails (exit 1); press Ctrl-C to stop.

EXAMPLES
    hew-cli events --live
    hew-cli events --live --filter document_changed,scenes_changed

OPTIONS
    --live              Listen to the running app. Required.
    --launch            Start the app first and wait for it.
    --instance <pid>    Pick one when several apps are running.
    --document <ref>    Listen to this open document (an id, path, label,
                        or title) instead of the front window.
    --filter <names>    Only these events, comma separated. Default: all
                        of them: document_changed, selection_changed,
                        context_changed, document_will_save,
                        document_saved, document_opened, document_closed,
                        units_changed, scenes_changed, tool_changed,
                        plugin_state, slot_changed, settings_changed,
                        ui_cancel, window_closed, surface_closed.
"#;

/// `hew-cli mcp --help`.
const USAGE_MCP: &str = r#"hew-cli mcp — serve MCP on stdio, for an AI client.

USAGE
    hew-cli mcp [--live [--launch] [--instance <pid>] [--document <ref>]]

Speaks newline-delimited JSON-RPC to whatever spawned the process. The
tools come from the command registry, so they match the protocol
exactly: capabilities, transact, query, describe_scene, snapshot.

With --live, every tool call reaches the app the user has open rather
than a document only the model can see.

EXAMPLE
    Add it to the client's MCP configuration as a stdio server:

    {"mcpServers": {"hew": {"command": "/path/to/hew-cli", "args": ["mcp"]}}}

    Then ask the model to build something. It draws, pushes, and paints
    through the same commands a script would use.

OPTIONS
    --live              Drive the app's open document.
    --launch            Start the app first and wait for it.
    --instance <pid>    Pick one when several apps are running.
    --document <ref>    Start attached to this open document (an id, path,
                        label, or title) instead of the front window. Every
                        tool also takes a per-call "document".
"#;

/// `hew-cli plugin --help`: a preview build only.
#[cfg(hew_preview)]
const USAGE_PLUGIN: &str = r#"hew-cli plugin — work on a plugin from a folder while you build it.

USAGE
    hew-cli plugin dev <folder> --live [--launch] [--instance <pid>] [--once]
    hew-cli plugin list --live [--launch] [--instance <pid>]

`plugin dev` asks the running app to show you a request to load the
plugin in <folder>. Nothing loads from the command line: in Hew you
choose the folder in the app's own folder picker and read what the plugin
asks for, and the plugin is not loaded until you allow it. The folder
here only tells the picker where to start. Developer mode must be on in
Hew (Settings > Plugins), and so must the Plugins switch.

It then checks the app's plugin list about once a second and prints what
it can see: a plugin from a developer folder that appears or changes
after the request ("<name> <version> is installed from a developer
folder and turned on."), one that waits for you to review a change
("Waiting for you to review a change in Hew."), and the other reasons it
cannot go on, in plain words. It stops when you press Ctrl-C
or the app closes after it answered. Exit 0 when it ends on its own, 1
when the app refuses the request, or closes before answering, or the
connection fails, 2 for a mistake in the command.

What it cannot tell: the app's list gives no folder path and does not say
whether a plugin is running. So it cannot know which listed plugin came
from <folder>, and it cannot see a reload that leaves the version alone.
It reports every folder plugin that changes, and Settings > Plugins in
Hew is the place to see what is running.

`plugin list` prints developer mode, then one line per installed plugin:
its id, name, version, where it came from, and its state.

EXAMPLES
    hew-cli plugin dev ./my-plugin --live
    hew-cli plugin dev ./my-plugin --live --once
    hew-cli plugin list --live

OPTIONS
    --live              Talk to the running app. Required.
    --launch            Start the app first and wait for it.
    --instance <pid>    Pick one when several apps are running.
    --once              dev: send the request, print the first answer and
                        stop, for scripts. Nothing is watched.
"#;

fn usage_error(message: &str) -> ! {
    eprintln!("{message}\n");
    eprint!("{}", usage());
    std::process::exit(2);
}

/// A mistyped subcommand names the nearest real one rather than only
/// listing all of them: `hew-cil run` should say "did you mean run?".
/// Distance 2 or less, so a genuinely different word suggests nothing.
fn unknown_subcommand(typed: &str) -> String {
    #[cfg(hew_preview)]
    const COMMANDS: [&str; 8] = [
        "run",
        "dispatch",
        "documents",
        "events",
        "library",
        "catalog",
        "mcp",
        "plugin",
    ];
    #[cfg(not(hew_preview))]
    const COMMANDS: [&str; 7] = [
        "run",
        "dispatch",
        "documents",
        "events",
        "library",
        "catalog",
        "mcp",
    ];
    // An MCP client configured with `"args": ["mcp --live"]` hands over the
    // whole string as one argument; the client then only shows a closed
    // connection, so say what is wrong and show the fix.
    let mut words = typed.split_whitespace();
    if let (Some(first), Some(_)) = (words.next(), words.next())
        && COMMANDS.contains(&first)
    {
        let split: Vec<String> = typed
            .split_whitespace()
            .map(|w| format!("\"{w}\""))
            .collect();
        return format!(
            "hew-cli: unknown command \"{typed}\". It looks like several arguments \
             arrived as one; give each its own entry, for example \"args\": [{}]",
            split.join(", ")
        );
    }
    let nearest = COMMANDS
        .iter()
        .map(|c| (edit_distance(typed, c), *c))
        .filter(|(d, _)| *d <= 2)
        .min_by_key(|(d, _)| *d);
    match nearest {
        Some((_, guess)) => {
            format!("hew-cli: unknown command \"{typed}\". Did you mean \"{guess}\"?")
        }
        None => format!("hew-cli: unknown command \"{typed}\""),
    }
}

/// Levenshtein distance, for [`unknown_subcommand`]'s suggestion.
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut row = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            row[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(row[j] + 1);
        }
        std::mem::swap(&mut prev, &mut row);
    }
    prev[b.len()]
}

/// Prints `page` and exits 0 when `args` asks for help — every
/// subcommand's first move, so `hew-cli run --help` explains `run`
/// rather than dumping the whole top-level page.
fn help_requested(args: &[String], page: &str) -> bool {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{page}");
        return true;
    }
    false
}

/// The `--live`/`--launch`/`--instance <pid>` flags, shared by all three
/// subcommands (docs/agents/HEW_API.md §12).
#[derive(Default)]
struct LiveFlags {
    live: bool,
    launch: bool,
    instance: Option<u32>,
    document: Option<String>,
}

impl LiveFlags {
    /// `Some(LiveOptions)` when `--live` was given, having already
    /// rejected `--launch`/`--instance` without it.
    fn into_options(self, usage_ctx: &str) -> Option<LiveOptions> {
        if !self.live && (self.launch || self.instance.is_some() || self.document.is_some()) {
            usage_error(&format!(
                "{usage_ctx}: --launch/--instance/--document only make sense with --live"
            ));
        }
        self.live.then_some(LiveOptions {
            launch: self.launch,
            instance: self.instance,
            document: self.document,
        })
    }
}

/// Recognizes and consumes one of `--live`/`--launch`/`--instance <pid>`
/// starting at `args[*i]`; returns whether it did. `usage_ctx` names the
/// subcommand for error messages (e.g. `"hew-cli run"`).
fn try_parse_live_flag(
    args: &[String],
    i: &mut usize,
    flags: &mut LiveFlags,
    usage_ctx: &str,
) -> bool {
    match args[*i].as_str() {
        "--live" => {
            flags.live = true;
            *i += 1;
            true
        }
        "--launch" => {
            flags.launch = true;
            *i += 1;
            true
        }
        "--document" => {
            let Some(reference) = args.get(*i + 1) else {
                usage_error(&format!(
                    "{usage_ctx}: --document needs a document id, path, label, or title"
                ));
            };
            flags.document = Some(reference.clone());
            *i += 2;
            true
        }
        "--instance" => {
            let Some(raw) = args.get(*i + 1) else {
                usage_error(&format!("{usage_ctx}: --instance needs a pid"));
            };
            let Ok(pid) = raw.parse::<u32>() else {
                usage_error(&format!(
                    "{usage_ctx}: --instance value \"{raw}\" is not a pid"
                ));
            };
            flags.instance = Some(pid);
            *i += 2;
            true
        }
        _ => false,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(subcommand) = args.first() else {
        usage_error("hew-cli: missing subcommand");
    };

    let code = match subcommand.as_str() {
        "run" => run_subcommand(&args[1..]),
        "dispatch" => dispatch_subcommand(&args[1..]),
        "mcp" => mcp_subcommand(&args[1..]),
        "documents" => documents_subcommand(&args[1..]),
        "events" => events_subcommand(&args[1..]),
        "library" => library_subcommand(&args[1..]),
        "catalog" => catalog_subcommand(&args[1..]),
        #[cfg(hew_preview)]
        "plugin" => plugin_subcommand(&args[1..]),
        // A help page is what the user asked for, so it goes to stdout
        // where it can be piped and paged. Only a usage ERROR prints it
        // to stderr, alongside what went wrong.
        "-h" | "--help" | "help" => {
            print!("{}", usage());
            0
        }
        "-V" | "--version" => {
            println!("hew-cli {}", hew_cli::VERSION);
            0
        }
        other => usage_error(&unknown_subcommand(other)),
    };
    std::process::exit(code);
}

fn run_subcommand(args: &[String]) -> i32 {
    if help_requested(args, USAGE_RUN) {
        return 0;
    }
    let Some(script) = args.first() else {
        usage_error("hew-cli run: missing <script.json|script.jsonl>");
    };
    if script.starts_with("--") {
        usage_error("hew-cli run: missing <script.json|script.jsonl>");
    }
    let mut out: Option<PathBuf> = None;
    let mut live_flags = LiveFlags::default();
    let mut i = 1;
    while i < args.len() {
        if try_parse_live_flag(args, &mut i, &mut live_flags, "hew-cli run") {
            continue;
        }
        match args[i].as_str() {
            "--out" => {
                let Some(path) = args.get(i + 1) else {
                    usage_error("hew-cli run: --out needs a path");
                };
                out = Some(PathBuf::from(path));
                i += 2;
            }
            other => usage_error(&format!("hew-cli run: unrecognized argument \"{other}\"")),
        }
    }
    let live = live_flags.into_options("hew-cli run");
    if out.is_some() && live.is_some() {
        usage_error(
            "hew-cli run: --out is not available in live mode: add a hew.doc.save frame with a path to your script, or run headless",
        );
    }
    hew_cli::run::run_script(&PathBuf::from(script), out.as_deref(), live.as_ref()).exit_code
}

fn dispatch_subcommand(args: &[String]) -> i32 {
    if help_requested(args, USAGE_DISPATCH) {
        return 0;
    }
    let Some(method) = args.first() else {
        usage_error("hew-cli dispatch: missing <method>");
    };
    let mut params: serde_json::Value = serde_json::json!({});
    let mut file: Option<PathBuf> = None;
    let mut live_flags = LiveFlags::default();
    let mut i = 1;
    // An optional bare JSON-object argument (params) may come right after
    // <method>, before any flags.
    if let Some(next) = args.get(i)
        && !next.starts_with("--")
    {
        params = match serde_json::from_str(next) {
            Ok(v) => v,
            Err(e) => usage_error(&format!("hew-cli dispatch: params is not valid JSON: {e}")),
        };
        i += 1;
    }
    while i < args.len() {
        if try_parse_live_flag(args, &mut i, &mut live_flags, "hew-cli dispatch") {
            continue;
        }
        match args[i].as_str() {
            "--file" => {
                let Some(path) = args.get(i + 1) else {
                    usage_error("hew-cli dispatch: --file needs a path");
                };
                file = Some(PathBuf::from(path));
                i += 2;
            }
            other => usage_error(&format!(
                "hew-cli dispatch: unrecognized argument \"{other}\""
            )),
        }
    }
    let live = live_flags.into_options("hew-cli dispatch");
    match (file, live) {
        (Some(_), Some(_)) => {
            usage_error("hew-cli dispatch: --file and --live are mutually exclusive")
        }
        (None, None) => usage_error("hew-cli dispatch: neither --file nor --live was given"),
        (Some(file), None) => hew_cli::run::dispatch_file(&file, method, params).exit_code,
        (None, Some(opts)) => hew_cli::run::dispatch_live(method, params, &opts).exit_code,
    }
}

fn documents_subcommand(args: &[String]) -> i32 {
    if help_requested(args, USAGE_DOCUMENTS) {
        return 0;
    }
    let mut live_flags = LiveFlags::default();
    let mut i = 0;
    while i < args.len() {
        if try_parse_live_flag(args, &mut i, &mut live_flags, "hew-cli documents") {
            continue;
        }
        usage_error(&format!(
            "hew-cli documents: unrecognized argument \"{}\"",
            args[i]
        ));
    }
    if live_flags.document.is_some() {
        usage_error("hew-cli documents: --document does not apply — it lists every document");
    }
    let Some(opts) = live_flags.into_options("hew-cli documents") else {
        usage_error("hew-cli documents: --live is required — there is nothing to list otherwise");
    };
    hew_cli::run::list_documents_live(&opts).exit_code
}

/// `hew-cli library index --help`.
const USAGE_LIBRARY_INDEX: &str = r#"hew-cli library index — write the index of a folder of Library items.

USAGE
    hew-cli library index <folder> [--check] [--name <text>] [--description <text>]
                          [--author <text>] [--homepage <url>] [--license <spdx>]
                          [--license-url <url>]

Writes <folder>/hew.json with one entry per .hew file under the folder (any
layout; names starting with a dot are skipped), and a thumbnail for each in
<folder>/thumbnails/. Every file must be a Library item: one saved from Hew,
carrying a Library id and a category. A file that is not is named and nothing
is written. A symbolic link at hew.json, thumbnails/ or a thumbnail is refused.

An existing hew.json keeps its name, description, author, homepage and license;
a flag replaces just its own field. With no hew.json yet, --name is required.

OPTIONS
    --check             Write nothing. Exit non-zero and name every file that is
                        missing, added or changed since hew.json was written.
    --name <text>       The source's name.
    --description <text>
    --author <text>
    --homepage <url>    An https URL.
    --license <spdx>    An SPDX identifier, or LicenseRef-<name> with --license-url.
    --license-url <url>

EXAMPLE
    hew-cli library index ./parts --name "Shop Hardware" --license CC-BY-4.0
    hew-cli library index ./parts --check
"#;

fn library_subcommand(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("index") => {
            let rest = &args[1..];
            if help_requested(rest, USAGE_LIBRARY_INDEX) {
                return 0;
            }
            match hew_cli::library_index::run(rest) {
                Ok(()) => 0,
                Err(message) => {
                    eprintln!("hew-cli library index: {message}");
                    1
                }
            }
        }
        Some("-h" | "--help") => {
            print!("{USAGE_LIBRARY_INDEX}");
            0
        }
        Some(other) => usage_error(&format!(
            "hew-cli library: unknown subcommand \"{other}\" (the one there is: index)"
        )),
        None => usage_error("hew-cli library: missing subcommand (library index <folder>)"),
    }
}

/// `hew-cli catalog --help`.
const USAGE_CATALOG: &str = r#"hew-cli catalog — check a source for listing, or build the catalog files.

USAGE
    hew-cli catalog check [--] <source reference>
    hew-cli catalog build <catalog checkout> --out <dir> [--previous <dir>]

check runs every listing check on one source and prints one line per failed
check, "<code>: <message>". It exits non-zero when any check fails. The codes:
reference, index_invalid, license_missing, license_format, file_integrity,
item_mismatch, ids_unique, public_https, cors, rate_limited, unreachable.
A reference is never a flag: after `--`, or without it, one that begins with
"-" or holds whitespace is a `reference` failure. Only a lone --help prints
this page.

build reads library.json, plugins.json and removed.json in the checkout and
writes catalog.json and sources/<slug>.json into --out. A source that fails
its checks keeps its snapshot from --previous (default: --out, when it
exists) and is marked stale; with none to keep it is left out and named. A
removed source is never requested.

Requests carry an Origin header as the web app's do, wait between requests to
one host, and use https only. GITHUB_TOKEN, when set, is sent to
api.github.com and nowhere else.

ENVIRONMENT
    GITHUB_TOKEN          Raises GitHub's API rate limit.

EXAMPLE
    hew-cli catalog check github:owner/repo
    hew-cli catalog build ./catalog --out ./data
"#;

/// Spacing between two requests to one host, and the time one request may take.
const CATALOG_REQUEST_GAP: std::time::Duration = std::time::Duration::from_millis(250);
const CATALOG_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// The current UTC time as `YYYY-MM-DDTHH:MM:SSZ`.
fn iso_utc_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

fn catalog_subcommand(args: &[String]) -> i32 {
    use hew_cli::catalog;
    let policy = catalog::FetchPolicy::new(CATALOG_REQUEST_GAP, CATALOG_REQUEST_TIMEOUT);
    match args.first().map(String::as_str) {
        Some("check") => {
            let mut rest = &args[1..];
            // Only a lone --help asks for help. Anything else after `check`
            // is a reference, so a list line such as `-h` is refused by the
            // check rather than read as a flag; `--` ends flag parsing.
            if rest.len() == 1 && rest[0] == "--help" {
                print!("{USAGE_CATALOG}");
                return 0;
            }
            if rest.first().map(String::as_str) == Some("--") {
                rest = &rest[1..];
            }
            let token = std::env::var("GITHUB_TOKEN").ok();
            let fetcher = catalog::HttpFetcher::new(&policy, token);
            let code = catalog::run_check(rest, &fetcher, &policy, &mut std::io::stdout());
            if code == 2 {
                usage_error("hew-cli catalog check: give exactly one source reference");
            }
            code
        }
        Some("build") => {
            let rest = &args[1..];
            if help_requested(rest, USAGE_CATALOG) {
                return 0;
            }
            let mut checkout: Option<PathBuf> = None;
            let mut out: Option<PathBuf> = None;
            let mut previous: Option<PathBuf> = None;
            let mut i = 0;
            while i < rest.len() {
                match rest[i].as_str() {
                    flag @ ("--out" | "--previous") => {
                        let Some(value) = rest.get(i + 1) else {
                            usage_error(&format!("hew-cli catalog build: {flag} needs a folder"));
                        };
                        let slot = if flag == "--out" {
                            &mut out
                        } else {
                            &mut previous
                        };
                        *slot = Some(PathBuf::from(value));
                        i += 2;
                    }
                    other if other.starts_with("--") => usage_error(&format!(
                        "hew-cli catalog build: unrecognized argument \"{other}\""
                    )),
                    other if checkout.is_none() => {
                        checkout = Some(PathBuf::from(other));
                        i += 1;
                    }
                    other => usage_error(&format!(
                        "hew-cli catalog build: unexpected extra argument \"{other}\""
                    )),
                }
            }
            let (Some(checkout), Some(out)) = (checkout, out) else {
                usage_error("hew-cli catalog build: needs <catalog checkout> and --out <dir>");
            };
            let previous = catalog::resolve_previous(previous.as_deref(), &out);
            let token = std::env::var("GITHUB_TOKEN").ok();
            let fetcher = catalog::HttpFetcher::new(&policy, token);
            match catalog::build_catalog(
                &checkout,
                &out,
                &fetcher,
                &policy,
                previous.as_deref(),
                &iso_utc_now(),
            ) {
                Ok(report) => {
                    println!(
                        "built {} fresh, {} stale, {} removed, {} failed",
                        report.fresh.len(),
                        report.stale.len(),
                        report.removed.len(),
                        report.failed.len()
                    );
                    for s in &report.stale {
                        eprintln!("stale (kept the last snapshot): {s}");
                    }
                    for s in &report.failed {
                        eprintln!("failed (left out, no earlier snapshot): {s}");
                    }
                    0
                }
                Err(e) => {
                    eprintln!("hew-cli catalog build: {e:?}");
                    1
                }
            }
        }
        Some("-h" | "--help") => {
            print!("{USAGE_CATALOG}");
            0
        }
        Some(other) => usage_error(&format!(
            "hew-cli catalog: unknown subcommand \"{other}\" (check or build)"
        )),
        None => usage_error("hew-cli catalog: missing subcommand (check or build)"),
    }
}

fn events_subcommand(args: &[String]) -> i32 {
    if help_requested(args, USAGE_EVENTS) {
        return 0;
    }
    let mut live_flags = LiveFlags::default();
    let mut filter: Option<Vec<String>> = None;
    let mut i = 0;
    while i < args.len() {
        if try_parse_live_flag(args, &mut i, &mut live_flags, "hew-cli events") {
            continue;
        }
        match args[i].as_str() {
            "--filter" => {
                let Some(raw) = args.get(i + 1) else {
                    usage_error("hew-cli events: --filter needs event names, comma separated");
                };
                filter = Some(
                    raw.split(',')
                        .map(str::trim)
                        .filter(|n| !n.is_empty())
                        .map(str::to_string)
                        .collect(),
                );
                i += 2;
            }
            other => usage_error(&format!(
                "hew-cli events: unrecognized argument \"{other}\""
            )),
        }
    }
    let Some(opts) = live_flags.into_options("hew-cli events") else {
        usage_error("hew-cli events: --live is required — events come from the running app");
    };
    hew_cli::run::events_live(&opts, filter)
}

/// `hew-cli plugin dev|list`: a preview build only.
#[cfg(hew_preview)]
fn plugin_subcommand(args: &[String]) -> i32 {
    if help_requested(args, USAGE_PLUGIN) {
        return 0;
    }
    let Some(action) = args.first() else {
        usage_error("hew-cli plugin: missing dev or list");
    };
    let mut live_flags = LiveFlags::default();
    let mut once = false;
    let mut folder: Option<PathBuf> = None;
    let mut i = 1;
    while i < args.len() {
        if try_parse_live_flag(args, &mut i, &mut live_flags, "hew-cli plugin") {
            continue;
        }
        match args[i].as_str() {
            "--once" if action == "dev" => {
                once = true;
                i += 1;
            }
            other if action == "dev" && folder.is_none() && !other.starts_with("--") => {
                folder = Some(PathBuf::from(other));
                i += 1;
            }
            other => usage_error(&format!(
                "hew-cli plugin: unrecognized argument \"{other}\""
            )),
        }
    }
    if live_flags.document.is_some() {
        usage_error(
            "hew-cli plugin: --document does not apply: plugins belong to the app, not to a document",
        );
    }
    match action.as_str() {
        "dev" => {
            let Some(folder) = folder else {
                usage_error("hew-cli plugin dev: missing <folder>");
            };
            let Some(opts) = live_flags.into_options("hew-cli plugin dev") else {
                usage_error(
                    "hew-cli plugin dev: --live is required: the request goes to the running app",
                );
            };
            hew_cli::plugin::dev_live(
                &folder,
                &opts,
                &hew_cli::plugin::DevOptions {
                    once,
                    ..Default::default()
                },
            )
        }
        "list" => {
            let Some(opts) = live_flags.into_options("hew-cli plugin list") else {
                usage_error(
                    "hew-cli plugin list: --live is required: the list is the running app's",
                );
            };
            hew_cli::plugin::list_live(&opts)
        }
        other => usage_error(&format!(
            "hew-cli plugin: unknown action \"{other}\"; use dev or list"
        )),
    }
}

fn mcp_subcommand(args: &[String]) -> i32 {
    if help_requested(args, USAGE_MCP) {
        return 0;
    }
    let mut live_flags = LiveFlags::default();
    let mut i = 0;
    while i < args.len() {
        if try_parse_live_flag(args, &mut i, &mut live_flags, "hew-cli mcp") {
            continue;
        }
        usage_error(&format!(
            "hew-cli mcp: unrecognized argument \"{}\"",
            args[i]
        ));
    }
    let live = live_flags.into_options("hew-cli mcp");
    hew_cli::mcp::run_stdio(live.as_ref())
}

#[cfg(test)]
mod tests {
    use super::{
        USAGE_DISPATCH, USAGE_DOCUMENTS, USAGE_EVENTS, USAGE_MCP, USAGE_RUN, edit_distance,
        unknown_subcommand, usage,
    };

    /// A mistyped command names the nearest real one. Silently accepting
    /// a typo, or listing every command and leaving the reader to spot
    /// the difference, both make the CLI harder to use than it needs to
    /// be (bettercli.org's help-page guidance, following npm).
    #[test]
    fn a_mistyped_command_suggests_the_real_one() {
        assert!(unknown_subcommand("runn").contains(r#"Did you mean "run""#));
        assert!(unknown_subcommand("dispatchh").contains(r#"Did you mean "dispatch""#));
        assert!(unknown_subcommand("mpc").contains(r#"Did you mean "mcp""#));
        assert!(unknown_subcommand("event").contains(r#"Did you mean "events""#));
        // Nothing close enough: say so rather than guess wildly.
        let far = unknown_subcommand("frobnicate");
        assert!(far.contains("frobnicate"));
        assert!(!far.contains("Did you mean"));
    }

    /// The plugin subcommand is a preview build's alone: it is a suggestion
    /// there, and no build suggests it by accident otherwise.
    #[test]
    fn plugin_is_suggested_only_in_a_preview_build() {
        let hint = unknown_subcommand("plugn");
        assert_eq!(
            hint.contains(r#"Did you mean "plugin""#),
            cfg!(hew_preview),
            "{hint}"
        );
        assert_eq!(usage().contains("plugin"), cfg!(hew_preview));
    }

    /// CLI-2: several arguments joined into one string get a hint that shows
    /// the split `args` array, not just "unknown command".
    #[test]
    fn joined_arguments_get_a_split_args_hint() {
        let msg = unknown_subcommand("mcp --live --launch");
        assert!(msg.contains("arrived as one"), "{msg}");
        assert!(
            msg.contains(r#""args": ["mcp", "--live", "--launch"]"#),
            "{msg}"
        );
        // A multi-word string whose first word is not a command gets no hint.
        assert!(!unknown_subcommand("frobnicate now").contains("arrived as one"));
    }

    #[test]
    fn edit_distance_counts_single_character_edits() {
        assert_eq!(edit_distance("run", "run"), 0);
        assert_eq!(edit_distance("runn", "run"), 1);
        assert_eq!(edit_distance("mpc", "mcp"), 2);
        assert_eq!(edit_distance("", "mcp"), 3);
    }

    /// Every help page answers the three questions a reader arrives with:
    /// what this is, how to start, and where to look next.
    #[test]
    fn the_help_page_says_what_it_is_how_to_start_and_where_to_go_next() {
        let usage = usage();
        assert!(usage.starts_with("hew-cli "), "leads with name and version");
        for section in [
            "USAGE",
            "COMMANDS",
            "EXAMPLES",
            "OPTIONS",
            "ENVIRONMENT",
            "LEARN MORE",
        ] {
            assert!(usage.contains(section), "top-level help needs {section}");
        }
        // The environment it reads is documented, not folded into prose.
        assert!(usage.contains("HEW_APP") && usage.contains("HEW_RUNTIME_DIR"));
        // And it points at per-command help rather than explaining
        // everything at the top level.
        assert!(usage.contains("hew-cli <command> --help"));
    }

    /// The banner line and `--version` print the same stamp.
    #[test]
    fn usage_banner_and_version_flag_agree() {
        let banner = usage();
        let first = banner.lines().next().unwrap();
        assert!(
            first.starts_with(&format!("hew-cli {} ", hew_cli::VERSION)),
            "banner leads with the stamped version: {first}"
        );
    }

    #[test]
    fn every_command_has_its_own_help_with_an_example() {
        #[allow(unused_mut)]
        let mut pages = vec![
            USAGE_RUN,
            USAGE_DISPATCH,
            USAGE_DOCUMENTS,
            USAGE_EVENTS,
            USAGE_MCP,
        ];
        #[cfg(hew_preview)]
        pages.push(super::USAGE_PLUGIN);
        for page in pages {
            assert!(page.starts_with("hew-cli "));
            assert!(page.contains("USAGE"));
            assert!(
                page.contains("EXAMPLE"),
                "a command's help shows the command being used"
            );
            assert!(page.contains("OPTIONS"));
        }
    }
}
