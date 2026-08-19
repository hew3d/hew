//! `hew-cli`'s logic, as a library — `src/main.rs` is a thin argv shell
//! over this crate so `crates/hew-cli/tests/cli.rs` can exercise the real
//! `run`/`dispatch`/`mcp` code paths directly, spawn-free (no subprocess,
//! no piping stdio).
//!
//! Headless is the default (docs/HEW_API.md §12): every subcommand embeds
//! `crates/api` and the kernel directly through [`host::CliHost`]. `--live`
//! (discovering and forwarding to a running desktop instance over the
//! local socket transport, §11.2) is implemented in [`live`]; it never
//! touches a filesystem-backed document at all — every envelope goes to
//! the running app instead.

pub mod host;
pub mod live;
pub mod mcp;
pub mod print;
pub mod run;

pub use host::CliHost;
