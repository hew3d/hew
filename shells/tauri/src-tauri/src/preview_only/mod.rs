//! Shell code that exists only in a preview build (`cfg(hew_preview)`, see
//! build.rs). `main.rs` declares this module under that cfg, and every
//! command defined here is registered in `generate_handler!` under its own
//! `#[cfg(hew_preview)]` line, so a production build compiles none of it
//! and its webview can reach none of it (preview_menu.rs's
//! `a_loader_shell_command_is_in_no_production_handler_list` checks the
//! layout against `loader.shellCommands` in preview-features.json).

pub mod plugins;
