//! `hew.plugin.list`, `hew.plugin.dev`, `hew.plugin.activity` and
//! `hew.plugin.reload` (docs/agents/PLUGINS.md "Developer mode"): Preview
//! commands (`preview-features.json`) that the desktop app
//! answers itself, before a request reaches a dispatcher. A request that
//! does reach this one came to a host with no plugins to show, so each
//! refuses `host_capability_missing` (after checking its params), the
//! answer every unsupported host effect gives. None takes a document or
//! changes one.
//!
//! `hew.permissions.request` (PLUGINS.md "Optional permissions") is the
//! same kind of Preview command: the page's plugin host answers it for a
//! running plugin's own connection before the frame reaches a dispatcher.
//! Every other caller (a socket client, a connected app, an agent) gets
//! here and is refused `host_capability_missing` once its params check
//! out.

use super::{CmdError, Ctx, Handler};
use crate::host::unsupported;
use serde_json::Value;

/// This namespace's slice of the handler table.
pub(crate) fn handler(name: &str) -> Option<Handler> {
    match name {
        "hew.plugin.list" => Some(list),
        "hew.plugin.dev" => Some(dev),
        "hew.plugin.activity" => Some(activity),
        "hew.plugin.reload" => Some(reload),
        "hew.permissions.request" => Some(permissions_request),
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

fn activity(_: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    plugin_id_param(params, "hew.plugin.activity")?;
    Err(CmdError::Refusal(unsupported("read a plugin's activity")))
}

fn reload(_: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    plugin_id_param(params, "hew.plugin.reload")?;
    Err(CmdError::Refusal(unsupported("reload a plugin")))
}

/// `{id}` and nothing else: the shape both shell-answered per-plugin
/// commands take. Whether the id names an installed plugin is the app's to
/// say.
fn plugin_id_param(params: &Value, method: &str) -> Result<(), CmdError> {
    let ok = params.as_object().is_some_and(|m| {
        m.len() == 1
            && m.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| (1..=128).contains(&id.len()))
    });
    if ok {
        Ok(())
    } else {
        Err(CmdError::Params(format!(
            "{method} takes {{id}}, an installed plugin's id"
        )))
    }
}

fn permissions_request(_: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let Some(object) = params.as_object() else {
        return Err(CmdError::Params(
            "hew.permissions.request takes {permissions, reason?}".into(),
        ));
    };
    if let Some(extra) = object
        .keys()
        .find(|k| k.as_str() != "permissions" && k.as_str() != "reason")
    {
        return Err(CmdError::Params(format!("unknown field `{extra}`")));
    }
    let names_ok = object
        .get("permissions")
        .and_then(Value::as_array)
        .is_some_and(|list| {
            (1..=64).contains(&list.len())
                && list
                    .iter()
                    .all(|v| v.as_str().is_some_and(|s| (1..=256).contains(&s.len())))
        });
    if !names_ok {
        return Err(CmdError::Params(
            "`permissions` must be 1 to 64 permission names".into(),
        ));
    }
    if let Some(reason) = object.get("reason")
        && !reason.as_str().is_some_and(|r| r.chars().count() <= 200)
    {
        return Err(CmdError::Params(
            "`reason` must be text of at most 200 characters".into(),
        ));
    }
    Err(CmdError::Refusal(unsupported(
        "ask the person for a plugin's permissions",
    )))
}

fn no_params(params: &Value) -> Result<(), CmdError> {
    match params.as_object() {
        Some(map) if map.is_empty() => Ok(()),
        None if params.is_null() => Ok(()),
        _ => Err(CmdError::Params("this command takes no parameters".into())),
    }
}
