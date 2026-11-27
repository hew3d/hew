//! The Preview wall in the native menu (docs/dev/DEVELOPMENT.md "Preview
//! features"). On macOS the native menu is the only menu, so a production
//! build (built without `HEW_PREVIEW=1`, so without `cfg(hew_preview)`)
//! must hold no item of a Preview feature.
//!
//! What is gated comes from `preview-features.json` at the repo root, the
//! same file the app and `crates/api` read: an item is gated when the app
//! action it dispatches ([`crate::menu_action_for`]) is in some entry's
//! `actions`. The menu is built in full (so its source stays one static
//! chain per submenu) and then pruned by [`wall_off`] before it is
//! attached:
//!
//! - every gated item goes;
//! - a submenu left empty goes;
//! - a submenu the wall leaves with a single item goes too, item and all
//!   (Object ▸ Smooth ▸ Off alone: production reaches "turn Smooth off"
//!   from Object Info instead);
//! - in every submenu that lost something, separators that would lead,
//!   trail, or double up go.
//!
//! A gated id that arrives as a menu event anyway is dropped
//! ([`blocks_event`]). A preview build prunes nothing.

use std::collections::BTreeSet;
use std::sync::OnceLock;
use tauri::menu::{Menu, MenuItemKind, Submenu};
use tauri::Runtime;

/// Whether this build prunes: a production build does, a preview build
/// does not.
pub const WALL_ON: bool = !cfg!(hew_preview);

const REGISTRY_JSON: &str = include_str!("../../../../preview-features.json");

#[derive(serde::Deserialize)]
struct RegistryFile {
    features: Vec<Feature>,
}

#[derive(serde::Deserialize)]
struct Feature {
    actions: Vec<String>,
}

/// Every app action some registry entry gates.
fn gated_actions() -> &'static BTreeSet<String> {
    static ACTIONS: OnceLock<BTreeSet<String>> = OnceLock::new();
    ACTIONS.get_or_init(|| {
        serde_json::from_str::<RegistryFile>(REGISTRY_JSON)
            .expect("preview-features.json does not parse")
            .features
            .into_iter()
            .flat_map(|f| f.actions)
            .collect()
    })
}

/// Whether the native item `id` belongs to a Preview feature: the action
/// it dispatches (or the id itself, for an item that dispatches none) is
/// in the registry.
pub fn native_id_gated(id: &str) -> bool {
    let action = crate::menu_action_for(id).unwrap_or(id);
    gated_actions().contains(action)
}

/// Whether a menu event for `id` is dropped: in a production build, every
/// gated id.
pub fn blocks_event(id: &str) -> bool {
    WALL_ON && native_id_gated(id)
}

/// One menu entry, as the planner sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Item(String),
    Separator,
    Submenu { id: String, children: Vec<Node> },
}

impl Node {
    fn id(&self) -> Option<&str> {
        match self {
            Node::Item(id) | Node::Submenu { id, .. } => Some(id),
            Node::Separator => None,
        }
    }
}

/// What to remove from one menu level: positions (into the level as
/// built), and the plans of the submenus kept at this level.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub remove: Vec<usize>,
    pub inner: Vec<(usize, Plan)>,
}

impl Plan {
    fn is_empty(&self) -> bool {
        self.remove.is_empty() && self.inner.is_empty()
    }
}

/// Plans one level. Returns the plan, how many entries (not separators)
/// survive, and whether the level lost any. `lone` collects the single
/// survivors of submenus the wall reduced to one entry.
fn plan_level(
    nodes: &[Node],
    gated: &dyn Fn(&str) -> bool,
    lone: &mut Vec<String>,
) -> (Plan, usize, bool) {
    let mut remove: BTreeSet<usize> = BTreeSet::new();
    let mut inner = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        match node {
            Node::Item(id) if gated(id) => {
                remove.insert(i);
            }
            Node::Submenu { id, .. } if gated(id) => {
                remove.insert(i);
            }
            Node::Submenu { children, .. } => {
                let (plan, kept, reduced) = plan_level(children, gated, lone);
                if reduced && kept <= 1 {
                    if kept == 1 {
                        let survivor = children
                            .iter()
                            .enumerate()
                            .find(|(j, c)| c.id().is_some() && !plan.remove.contains(j))
                            .and_then(|(_, c)| c.id())
                            .expect("one entry survives");
                        lone.push(survivor.to_string());
                    }
                    remove.insert(i);
                } else if !plan.is_empty() {
                    inner.push((i, plan));
                }
            }
            _ => {}
        }
    }
    let reduced = !remove.is_empty();
    if reduced {
        // Separators among the survivors: none first, none last, never
        // two together.
        let mut last_kept_was_separator = true; // a leading one goes
        let mut pending: Option<usize> = None;
        for (i, node) in nodes.iter().enumerate() {
            if remove.contains(&i) {
                continue;
            }
            match node {
                Node::Separator if last_kept_was_separator => {
                    remove.insert(i);
                }
                Node::Separator => {
                    last_kept_was_separator = true;
                    pending = Some(i);
                }
                _ => {
                    last_kept_was_separator = false;
                    pending = None;
                }
            }
        }
        if let Some(trailing) = pending {
            remove.insert(trailing);
        }
    }
    let kept = nodes
        .iter()
        .enumerate()
        .filter(|(i, n)| !remove.contains(i) && n.id().is_some())
        .count();
    (
        Plan {
            remove: remove.into_iter().collect(),
            inner,
        },
        kept,
        reduced,
    )
}

/// Plans a whole menu bar: what to remove, and the ids the single-survivor
/// rule removed with their submenu.
pub fn plan(nodes: &[Node], gated: &dyn Fn(&str) -> bool) -> (Plan, Vec<String>) {
    let mut lone = Vec::new();
    let (plan, _, _) = plan_level(nodes, gated, &mut lone);
    (plan, lone)
}

/// `nodes` with `plan` applied: what [`wall_off`] does to a real menu.
#[cfg(test)]
fn apply_to_nodes(nodes: &[Node], plan: &Plan) -> Vec<Node> {
    nodes
        .iter()
        .enumerate()
        .filter(|(i, _)| !plan.remove.contains(i))
        .map(
            |(i, n)| match (n, plan.inner.iter().find(|(j, _)| *j == i)) {
                (Node::Submenu { id, children }, Some((_, p))) => Node::Submenu {
                    id: id.clone(),
                    children: apply_to_nodes(children, p),
                },
                _ => n.clone(),
            },
        )
        .collect()
}

fn node_of<R: Runtime>(item: &MenuItemKind<R>) -> tauri::Result<Node> {
    Ok(match item {
        MenuItemKind::Submenu(s) => Node::Submenu {
            id: s.id().0.clone(),
            children: s
                .items()?
                .iter()
                .map(node_of)
                .collect::<tauri::Result<_>>()?,
        },
        // A separator is the predefined item with no text.
        MenuItemKind::Predefined(p) if p.text()?.is_empty() => Node::Separator,
        MenuItemKind::Predefined(p) => Node::Item(p.id().0.clone()),
        MenuItemKind::MenuItem(m) => Node::Item(m.id().0.clone()),
        MenuItemKind::Check(c) => Node::Item(c.id().0.clone()),
        MenuItemKind::Icon(i) => Node::Item(i.id().0.clone()),
    })
}

fn apply_to_submenu<R: Runtime>(submenu: &Submenu<R>, plan: &Plan) -> tauri::Result<()> {
    let items = submenu.items()?;
    for (i, inner) in &plan.inner {
        if let Some(MenuItemKind::Submenu(s)) = items.get(*i) {
            apply_to_submenu(s, inner)?;
        }
    }
    for i in plan.remove.iter().rev() {
        submenu.remove_at(*i)?;
    }
    Ok(())
}

/// Prunes a production build's menu bar (see the module doc); a no-op in a
/// preview build. Run once, after the menu is built and before it is
/// attached.
pub fn wall_off<R: Runtime>(menu: &Menu<R>) -> tauri::Result<()> {
    if !WALL_ON {
        return Ok(());
    }
    let items = menu.items()?;
    let nodes = items
        .iter()
        .map(node_of)
        .collect::<tauri::Result<Vec<_>>>()?;
    let (plan, _) = plan(&nodes, &native_id_gated);
    for (i, inner) in &plan.inner {
        if let Some(MenuItemKind::Submenu(s)) = items.get(*i) {
            apply_to_submenu(s, inner)?;
        }
    }
    for i in plan.remove.iter().rev() {
        menu.remove_at(*i)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const MAIN_RS: &str = include_str!("main.rs");

    /// The `.item(&x)` / `.separator()` chain of every
    /// `let x = SubmenuBuilder::with_id(handle, "<id>", ...)...build()?` in
    /// main.rs, by variable: `(submenu id, entries)` with `None` for a
    /// separator.
    fn submenu_chains() -> BTreeMap<String, (String, Vec<Option<String>>)> {
        let mut out = BTreeMap::new();
        let mut rest = MAIN_RS;
        while let Some(at) = rest.find("SubmenuBuilder::with_id(") {
            let head = &rest[..at];
            let var = head
                .rfind("let ")
                .map(|l| head[l + 4..].trim_end())
                .and_then(|s| s.strip_suffix('='))
                .map(|s| s.trim().to_string());
            let body = &rest[at..];
            let end = body.find(".build()?").unwrap_or(body.len());
            let chain = &body[..end];
            rest = &body[end..];
            let (Some(var), Some(id)) = (var, chain.split('"').nth(1)) else {
                continue;
            };
            if var.contains(char::is_whitespace) {
                continue;
            }
            let mut entries = Vec::new();
            let mut s = chain;
            loop {
                let item = s.find(".item(&");
                let sep = s.find(".separator()");
                match (item, sep) {
                    (Some(i), sep) if sep.is_none_or(|p| i < p) => {
                        let tail = &s[i + 7..];
                        let close = tail.find(')').expect("closing paren");
                        let var = tail[..close].trim();
                        // `.item(&PredefinedMenuItem::separator(handle)?)`
                        // is a separator written out.
                        entries.push(if var.starts_with("PredefinedMenuItem::separator") {
                            None
                        } else {
                            Some(var.to_string())
                        });
                        s = &tail[close..];
                    }
                    (_, Some(p)) => {
                        entries.push(None);
                        s = &s[p + 12..];
                    }
                    _ => break,
                }
            }
            out.insert(var, (id.to_string(), entries));
        }
        out
    }

    /// The native id an item variable is bound to in main.rs.
    fn item_id(var: &str) -> Option<String> {
        let at = MAIN_RS.find(&format!("let {var} ="))?;
        let tail = &MAIN_RS[at..];
        let stmt = &tail[..tail.find(";")?];
        for ctor in [
            "check_item(",
            "gated_item(",
            "edit_item(",
            "MenuItemBuilder::with_id(",
        ] {
            if let Some(c) = stmt.find(ctor) {
                return stmt[c..].split('"').nth(1).map(str::to_string);
            }
        }
        None
    }

    fn tree(var: &str, chains: &BTreeMap<String, (String, Vec<Option<String>>)>) -> Node {
        match chains.get(var) {
            Some((id, entries)) => Node::Submenu {
                id: id.clone(),
                children: entries
                    .iter()
                    .map(|e| match e {
                        None => Node::Separator,
                        Some(v) => tree(v, chains),
                    })
                    .collect(),
            },
            None => Node::Item(item_id(var).unwrap_or_else(|| format!("?{var}"))),
        }
    }

    /// The menu bar main.rs assembles, as the planner sees it.
    fn menu_bar() -> Vec<Node> {
        let chains = submenu_chains();
        let at = MAIN_RS
            .find("let menu = MenuBuilder::new(handle)")
            .expect("the menu bar is assembled");
        let tail = &MAIN_RS[at..];
        let chain = &tail[..tail.find(".build()?").expect("built")];
        chain
            .split(".item(&")
            .skip(1)
            .map(|s| tree(s[..s.find(')').expect("paren")].trim(), &chains))
            .collect()
    }

    fn ids(nodes: &[Node], out: &mut Vec<String>) {
        for n in nodes {
            if let Some(id) = n.id() {
                out.push(id.to_string());
            }
            if let Node::Submenu { children, .. } = n {
                ids(children, out);
            }
        }
    }

    /// Whether submenu `id` is built with no entries in main.rs.
    fn filled_at_run_time(id: &str) -> bool {
        submenu_chains()
            .values()
            .any(|(sid, entries)| sid == id && entries.is_empty())
    }

    fn check_shape(nodes: &[Node], path: &str) {
        for n in nodes {
            if let Node::Submenu { id, children } = n {
                let here = format!("{path} > {id}");
                // Open Recent and the like are filled at run time; only a
                // submenu built with entries must keep one.
                if children.is_empty() && filled_at_run_time(id) {
                    continue;
                }
                assert!(!children.is_empty(), "{here} is empty");
                assert_ne!(
                    children.first(),
                    Some(&Node::Separator),
                    "{here} leads with a separator"
                );
                assert_ne!(
                    children.last(),
                    Some(&Node::Separator),
                    "{here} ends with a separator"
                );
                for w in children.windows(2) {
                    assert!(
                        !(w[0] == Node::Separator && w[1] == Node::Separator),
                        "{here} has two separators together"
                    );
                }
                check_shape(children, &here);
            }
        }
    }

    fn render(nodes: &[Node], depth: usize, out: &mut String) {
        for n in nodes {
            match n {
                Node::Separator => out.push_str(&format!("{}-\n", "  ".repeat(depth))),
                Node::Item(id) => out.push_str(&format!("{}{id}\n", "  ".repeat(depth))),
                Node::Submenu { id, children } => {
                    out.push_str(&format!("{}{id}\n", "  ".repeat(depth)));
                    render(children, depth + 1, out);
                }
            }
        }
    }

    /// Desktop commands of a code-loading Preview feature
    /// (`loader.shellCommands`). A desktop command answers any window's
    /// webview, so one of these must not exist in a production build at
    /// all: it is defined in a module compiled only under
    /// `cfg(hew_preview)`, and its name is not in the handler list a
    /// production build registers.
    #[test]
    fn a_loader_shell_command_is_in_no_production_handler_list() {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Loader {
            shell_commands: Vec<String>,
        }
        #[derive(serde::Deserialize)]
        struct Entry {
            loader: Option<Loader>,
        }
        #[derive(serde::Deserialize)]
        struct File {
            features: Vec<Entry>,
        }
        let names: Vec<String> = serde_json::from_str::<File>(REGISTRY_JSON)
            .expect("preview-features.json does not parse")
            .features
            .into_iter()
            .filter_map(|f| f.loader)
            .flat_map(|l| l.shell_commands)
            .collect();

        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let main = std::fs::read_to_string(src.join("main.rs")).expect("main.rs");
        // The list every build registers: the first `generate_handler!` in
        // main.rs, up to its closing bracket.
        let start = main
            .find("tauri::generate_handler![")
            .expect("main.rs registers its commands with generate_handler!");
        let list = &main[start..start + main[start..].find("])").expect("the list closes")];
        assert!(
            list.contains("read_file") && list.lines().count() > 10,
            "the handler list was not read"
        );
        // What a production build registers: the list without any entry
        // that sits under its own `#[cfg(hew_preview)]` line.
        let mut production_list = String::new();
        let mut gated_next = false;
        for line in list.lines() {
            if line.trim() == "#[cfg(hew_preview)]" {
                gated_next = true;
            } else if gated_next {
                gated_next = false;
            } else {
                production_list.push_str(line);
                production_list.push('\n');
            }
        }
        let is_ident = |c: char| c.is_alphanumeric() || c == '_';
        let named = |text: &str, name: &str| {
            text.match_indices(name).any(|(i, _)| {
                !text[..i].chars().next_back().is_some_and(is_ident)
                    && !text[i + name.len()..].chars().next().is_some_and(is_ident)
            })
        };
        // The gated module tree: `src/preview_only/`, declared in main.rs
        // only as `#[cfg(hew_preview)] mod preview_only;`.
        fn sources(dir: &std::path::Path, out: &mut String) {
            for entry in std::fs::read_dir(dir).expect("a source folder") {
                let path = entry.expect("entry").path();
                if path.is_dir() {
                    sources(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push_str(&std::fs::read_to_string(&path).expect("source"));
                }
            }
        }
        let gated_dir = src.join("preview_only");
        let mut gated_source = String::new();
        if gated_dir.is_dir() {
            let declared = main.lines().collect::<Vec<_>>().windows(2).any(|w| {
                w[0].trim() == "#[cfg(hew_preview)]" && w[1].trim() == "mod preview_only;"
            });
            assert!(
                declared,
                "src/preview_only exists but main.rs does not declare it under #[cfg(hew_preview)]"
            );
            assert_eq!(
                main.matches("mod preview_only").count(),
                1,
                "main.rs declares preview_only more than once"
            );
            sources(&gated_dir, &mut gated_source);
        }
        // The whole of main.rs must register every name somewhere (under the
        // cfg), or the command is written but answers nobody.
        for name in &names {
            assert!(
                !named(&production_list, name),
                "{name} is in the handler list a production build registers"
            );
            let defined = gated_source
                .match_indices(&format!("fn {name}"))
                .any(|(i, m)| {
                    gated_source[i + m.len()..]
                        .chars()
                        .next()
                        .is_some_and(|c| c == '(' || c == '<')
                });
            assert!(
                defined,
                "{name} is in loader.shellCommands but is not defined under src/preview_only/"
            );
            assert!(
                named(&main, name),
                "{name} is defined but main.rs never registers it"
            );
        }
    }

    #[test]
    fn the_wall_is_on_only_in_a_production_build() {
        assert_eq!(WALL_ON, !cfg!(hew_preview));
        assert!(!gated_actions().is_empty());
    }

    /// The production menu bar, computed from main.rs and the registry in
    /// either build: no gated item anywhere, no empty submenu, no stray
    /// separator, and the single-survivor rule removes only what the
    /// maintainer agreed to (a tripwire: a new case is a decision, not a
    /// silent side effect). `--nocapture` prints the chains.
    #[test]
    fn the_production_menu_holds_no_preview_item_and_reads_cleanly() {
        let bar = menu_bar();
        let mut before = Vec::new();
        ids(&bar, &mut before);
        let gated: Vec<&String> = before.iter().filter(|id| native_id_gated(id)).collect();
        assert!(!gated.is_empty(), "the registry gates no native item");

        let (plan, lone) = plan(&bar, &native_id_gated);
        assert_eq!(lone, vec!["object-smooth-off".to_string()]);
        let after = apply_to_nodes(&bar, &plan);
        let mut kept = Vec::new();
        ids(&after, &mut kept);
        for id in &kept {
            assert!(!native_id_gated(id), "{id} survives");
        }
        check_shape(&after, "menu");
        // Nothing outside the wall moved: every surviving id was in the
        // built menu, and every removed one is gated, a lone survivor, or a
        // submenu the wall emptied.
        let removed: Vec<&String> = before.iter().filter(|id| !kept.contains(id)).collect();
        for id in removed {
            assert!(
                native_id_gated(id) || lone.contains(id) || id.starts_with("submenu:"),
                "{id} was removed but is not a Preview item"
            );
        }
        let mut printed = String::new();
        render(&after, 0, &mut printed);
        println!("{printed}");
    }

    #[test]
    fn the_planner_cleans_separators_and_drops_reduced_submenus() {
        let item = |s: &str| Node::Item(s.to_string());
        let sub = |id: &str, children: Vec<Node>| Node::Submenu {
            id: id.to_string(),
            children,
        };
        let gated = |id: &str| id.starts_with('g');
        let bar = vec![sub(
            "m",
            vec![
                item("g1"),
                Node::Separator,
                item("a"),
                Node::Separator,
                sub("s1", vec![item("g2"), item("g3")]),
                sub("s2", vec![item("keep"), Node::Separator, item("g4")]),
                Node::Separator,
                item("b"),
                Node::Separator,
                item("g5"),
            ],
        )];
        let (plan, lone) = plan(&bar, &gated);
        assert_eq!(lone, vec!["keep".to_string()]);
        assert_eq!(
            apply_to_nodes(&bar, &plan),
            vec![sub("m", vec![item("a"), Node::Separator, item("b")])]
        );
        // A menu with nothing gated is left exactly as it is, stray
        // separators included.
        let untouched = vec![sub("m", vec![Node::Separator, item("a"), Node::Separator])];
        let (kept, lone) = super::plan(&untouched, &gated);
        assert!(kept.is_empty() && lone.is_empty());
    }
}
