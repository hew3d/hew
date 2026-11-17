//! `hew.plugin.list` and `hew.plugin.dev` (docs/agents/PLUGINS.md "Developer
//! mode"): Preview commands (`preview-features.json`) that the desktop app
//! answers itself, before a request reaches a dispatcher. A request that
//! does reach this one came to a host with no plugins to show, so both
//! refuse `host_capability_missing`, the answer every unsupported host
//! effect gives. Neither takes a document or changes one.

use super::{CmdError, Ctx, Handler};
use crate::host::unsupported;
use serde_json::Value;

/// This namespace's slice of the handler table.
pub(crate) fn handler(name: &str) -> Option<Handler> {
    match name {
        "hew.plugin.list" => Some(list),
        "hew.plugin.dev" => Some(dev),
        _ => None,
    }
}

fn list(_: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    no_params(params)?;
    Err(CmdError::Refusal(unsupported("list plugins")))
}

fn dev(_: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    match params.get("path").and_then(Value::as_str) {
        Some(path) if !path.is_empty() => {}
        _ => {
            return Err(CmdError::Params(
                "hew.plugin.dev takes {path}, the plugin's folder".into(),
            ));
        }
    }
    Err(CmdError::Refusal(unsupported(
        "show a request to load a plugin",
    )))
}

fn no_params(params: &Value) -> Result<(), CmdError> {
    match params.as_object() {
        Some(map) if map.is_empty() => Ok(()),
        None if params.is_null() => Ok(()),
        _ => Err(CmdError::Params("this command takes no parameters".into())),
    }
}
