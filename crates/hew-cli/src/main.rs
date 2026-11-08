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
const USAGE_BODY: &str = r#" — build and edit Hew models from a script, a shell, or an AI client.

USAGE
    hew-cli <command> [options]

COMMANDS
    run <script>          Run a file of JSON-RPC envelopes in order
    dispatch <method>     Send one command
    documents             List the app's open documents (--live)
    events                Print the app's events as they happen (--live)
    mcp                   Serve MCP on stdio, for an AI client

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
    format!("hew-cli {}{USAGE_BODY}", hew_cli::VERSION)
}

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

fn usage_error(message: &str) -> ! {
    eprintln!("{message}\n");
    eprint!("{}", usage());
    std::process::exit(2);
}

/// A mistyped subcommand names the nearest real one rather than only
/// listing all of them: `hew-cil run` should say "did you mean run?".
/// Distance 2 or less, so a genuinely different word suggests nothing.
fn unknown_subcommand(typed: &str) -> String {
    const COMMANDS: [&str; 5] = ["run", "dispatch", "documents", "events", "mcp"];
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
        for page in [
            USAGE_RUN,
            USAGE_DISPATCH,
            USAGE_DOCUMENTS,
            USAGE_EVENTS,
            USAGE_MCP,
        ] {
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
