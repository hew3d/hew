//! `hew-cli library index <folder> [--check] [flags]`
//! (docs/design/shared-library-sources.md 6.1, 6.4, 7.6): writes the
//! `hew.json` index and the `thumbnails/<item sha256>.png` files for a
//! folder of Library items, or with `--check` compares the folder against
//! the index it already has and writes nothing.
//!
//! Every failure is a returned message, never a panic: the folder is
//! whatever a publisher pointed us at, so a file may be anything.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use sources::{IndexHeader, LocalItem, LocalMeta, SourceIndex};

/// Pixel size of the PNG written for each item. Must stay equal to the size
/// the app renders Library thumbnails at: the `renderItemThumbnail(bytes, 256)`
/// call sites in `app/src/App.tsx` and `app/src/panels/LibraryDialog.tsx`.
pub const THUMBNAIL_SIZE: u32 = 256;

const INDEX_FILE: &str = "hew.json";
const THUMBNAIL_DIR: &str = "thumbnails";

/// Parsed command line of `library index`.
#[derive(Debug, Default, PartialEq)]
struct Options {
    folder: Option<PathBuf>,
    check: bool,
    name: Option<String>,
    description: Option<String>,
    author: Option<String>,
    homepage: Option<String>,
    license: Option<String>,
    license_url: Option<String>,
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut o = Options::default();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (arg, None),
        };
        let slot = match flag {
            "--check" => {
                if inline.is_some() {
                    return Err("--check takes no value".into());
                }
                o.check = true;
                i += 1;
                continue;
            }
            "--name" => &mut o.name,
            "--description" => &mut o.description,
            "--author" => &mut o.author,
            "--homepage" => &mut o.homepage,
            "--license" => &mut o.license,
            "--license-url" => &mut o.license_url,
            f if f.starts_with("--") => return Err(format!("unrecognized option \"{f}\"")),
            _ => {
                if o.folder.is_some() {
                    return Err(format!("unexpected extra argument \"{arg}\""));
                }
                o.folder = Some(PathBuf::from(arg));
                i += 1;
                continue;
            }
        };
        i += 1;
        let value = match inline {
            Some(v) => v,
            None => {
                let v = args
                    .get(i)
                    .ok_or_else(|| format!("{flag} needs a value"))?
                    .clone();
                i += 1;
                v
            }
        };
        *slot = Some(value);
    }
    Ok(o)
}

/// Runs `library index` over `args` (everything after `library index`).
/// `Ok` carries the warnings printed to stderr already; `Err` is the
/// message for stderr and means a non-zero exit.
pub fn run(args: &[String]) -> Result<(), String> {
    let opts = parse_args(args)?;
    let folder = opts
        .folder
        .clone()
        .ok_or_else(|| "missing <folder>".to_string())?;
    if !folder.is_dir() {
        return Err(format!("{}: not a folder", folder.display()));
    }
    let existing = read_existing(&folder)?;
    if opts.check {
        let Some(existing) = existing else {
            return Err(format!(
                "--check needs an existing {INDEX_FILE} in {}",
                folder.display()
            ));
        };
        return check(&folder, &existing);
    }
    let header = header_for(&opts, existing.as_ref())?;
    write_index(&folder, header, existing.as_ref())
}

fn read_existing(folder: &Path) -> Result<Option<SourceIndex>, String> {
    let path = folder.join(INDEX_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    sources::parse_index(&bytes)
        .map(Some)
        .map_err(|e| format!("{}: {} ({e:?})", path.display(), e.code()))
}

fn header_for(opts: &Options, existing: Option<&SourceIndex>) -> Result<IndexHeader, String> {
    let mut h = match existing {
        Some(x) => IndexHeader {
            name: x.name.clone(),
            description: x.description.clone(),
            author: x.author.clone(),
            homepage: x.homepage.clone(),
            license: x.license.clone(),
            license_url: x.license_url.clone(),
        },
        None => {
            if opts.name.is_none() {
                return Err(format!(
                    "there is no {INDEX_FILE} here yet, so --name is required"
                ));
            }
            IndexHeader::default()
        }
    };
    if let Some(v) = &opts.name {
        h.name = v.clone();
    }
    if opts.description.is_some() {
        h.description = opts.description.clone();
    }
    if opts.author.is_some() {
        h.author = opts.author.clone();
    }
    if opts.homepage.is_some() {
        h.homepage = opts.homepage.clone();
    }
    if opts.license.is_some() {
        h.license = opts.license.clone();
    }
    if opts.license_url.is_some() {
        h.license_url = opts.license_url.clone();
    }
    Ok(h)
}

/// Every `.hew` under `folder`, as `/`-separated relative paths. Skips any
/// path segment that starts with `.`, the root `thumbnails/` folder, and
/// symlinks. Sorted by byte order so the walk never decides the output.
fn find_items(folder: &Path, warnings: &mut Vec<String>) -> Result<Vec<String>, String> {
    fn walk(
        dir: &Path,
        prefix: &str,
        out: &mut Vec<String>,
        warnings: &mut Vec<String>,
    ) -> Result<(), String> {
        let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                warnings.push(format!(
                    "skipped {}: the name is not valid UTF-8",
                    entry.path().display()
                ));
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            let rel = format!("{prefix}{name}");
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if prefix.is_empty() && name == THUMBNAIL_DIR {
                    continue;
                }
                walk(&entry.path(), &format!("{rel}/"), out, warnings)?;
            } else if kind.is_file() && name.ends_with(".hew") {
                out.push(rel);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(folder, "", &mut out, warnings)?;
    out.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    Ok(out)
}

/// Reads one item file and its `hew.library` metadata. A file with no
/// `hew.library.id` or no category (or no readable container) aborts the run
/// by name.
fn read_item(folder: &Path, rel: &str) -> Result<LocalItem, String> {
    let path = folder.join(rel);
    let bytes = std::fs::read(&path).map_err(|e| format!("{rel}: {e}"))?;
    let summary = kernel::read_item_summary(&bytes)
        .map_err(|e| format!("{rel}: not a readable Hew file ({e:?})"))?;
    let raw = summary
        .doc_attrs
        .get("hew.library")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let mut meta = library::parse_item_meta(&raw);
    if meta.id.is_none() {
        return Err(format!(
            "{rel}: not a Library item (it has no hew.library id). Save it to the Library, \
             or move it out of this folder"
        ));
    }
    if meta.category.is_none() {
        return Err(format!(
            "{rel}: it has no hew.library category. Save it to the Library again, \
             or move it out of this folder"
        ));
    }
    if meta.name.is_none() {
        meta.name = Some(library::display_name(&meta, &summary, rel));
    }
    Ok(LocalItem {
        path: rel.to_string(),
        bytes,
        meta: local_meta(meta),
    })
}

/// Maps the Library's item metadata onto what the index builder reads. A
/// missing category falls to Components.
fn local_meta(m: library::ItemMeta) -> LocalMeta {
    LocalMeta {
        id: m.id,
        name: m.name,
        category: Some(match m.category {
            Some(library::Category::Material) => sources::Category::Materials,
            Some(library::Category::Model) => sources::Category::Models,
            Some(library::Category::Profile) => sources::Category::Profiles,
            Some(library::Category::Component) | None => sources::Category::Components,
        }),
        keywords: m.keywords,
        collection: m.collection,
    }
}

fn read_items(folder: &Path, warnings: &mut Vec<String>) -> Result<Vec<LocalItem>, String> {
    find_items(folder, warnings)?
        .iter()
        .map(|rel| read_item(folder, rel))
        .collect()
}

fn render_thumbnail(bytes: &[u8]) -> Option<Vec<u8>> {
    // A hostile file must not take the CLI down: a panic in the renderer
    // reads as "cannot render".
    std::panic::catch_unwind(|| {
        let doc = kernel::Document::load(bytes).ok()?;
        softrender::render_document_thumbnail(&doc, THUMBNAIL_SIZE)
    })
    .ok()
    .flatten()
}

fn write_index(
    folder: &Path,
    header: IndexHeader,
    existing: Option<&SourceIndex>,
) -> Result<(), String> {
    let mut warnings = Vec::new();
    let items = read_items(folder, &mut warnings)?;
    let mut index = sources::build_index(&items, header);
    let mut thumbnails: Vec<(String, Vec<u8>)> = Vec::new();
    for (entry, item) in index.items.iter_mut().zip(sorted_by_path(&items)) {
        if let Some(old) = existing.and_then(|x| x.items.iter().find(|o| o.path == entry.path)) {
            entry.license = old.license.clone();
        }
        match render_thumbnail(&item.bytes) {
            Some(png) => {
                let rel = format!("{THUMBNAIL_DIR}/{}.png", entry.sha256);
                entry.thumbnail = Some(rel.clone());
                thumbnails.push((rel, png));
            }
            None => warnings.push(format!(
                "warning: {}: no thumbnail could be rendered; indexed without one",
                entry.path
            )),
        }
    }
    if let Some(x) = existing {
        index.plugins = x.plugins.clone();
    }
    let mut json = serde_json::to_string_pretty(&index).map_err(|e| e.to_string())?;
    json.push('\n');
    // Validate before touching the disk, so a bad folder writes nothing.
    if let Err(e) = sources::parse_index(json.as_bytes()) {
        let which = match &e {
            sources::IndexError::Invalid { item: Some(n), .. } => index
                .items
                .get(*n)
                .map(|i| format!("{}: ", i.path))
                .unwrap_or_default(),
            _ => String::new(),
        };
        return Err(format!(
            "{which}the index is not valid: {} ({e:?})",
            e.code()
        ));
    }
    let thumb_dir = folder.join(THUMBNAIL_DIR);
    // Never write through a link: the folder may be someone else's checkout.
    refuse_symlink(&thumb_dir)?;
    for (rel, _) in &thumbnails {
        refuse_symlink(&folder.join(rel))?;
    }
    refuse_symlink(&folder.join(INDEX_FILE))?;
    if !thumbnails.is_empty() {
        std::fs::create_dir_all(&thumb_dir).map_err(|e| format!("{}: {e}", thumb_dir.display()))?;
    }
    for (rel, png) in &thumbnails {
        let path = folder.join(rel);
        std::fs::write(&path, png).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    remove_stray_thumbnails(&thumb_dir, &thumbnails);
    let path = folder.join(INDEX_FILE);
    std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))?;
    for w in warnings {
        eprintln!("{w}");
    }
    println!(
        "wrote {INDEX_FILE}: {} item(s), {} thumbnail(s)",
        index.items.len(),
        thumbnails.len()
    );
    Ok(())
}

/// Errors, naming the path, when it exists and is a symlink.
fn refuse_symlink(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => Err(format!(
            "{}: is a symbolic link; refusing to write through it (nothing was written)",
            path.display()
        )),
        _ => Ok(()),
    }
}

fn sorted_by_path(items: &[LocalItem]) -> Vec<&LocalItem> {
    let mut v: Vec<&LocalItem> = items.iter().collect();
    v.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    v
}

/// Removes `<64 hex>.png` files in `thumbnails/` that no item names any
/// more. Anything else in the folder is left alone.
fn remove_stray_thumbnails(dir: &Path, kept: &[(String, Vec<u8>)]) {
    let keep: BTreeSet<&str> = kept
        .iter()
        .filter_map(|(rel, _)| rel.strip_prefix("thumbnails/"))
        .collect();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let stem = name.strip_suffix(".png").unwrap_or("");
        let hashed =
            stem.len() == 64 && stem.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if hashed && !keep.contains(name.as_str()) && entry.file_type().is_ok_and(|t| t.is_file()) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// `--check`: compares the folder with the index, writes nothing.
fn check(folder: &Path, index: &SourceIndex) -> Result<(), String> {
    let mut warnings = Vec::new();
    let items = read_items(folder, &mut warnings)?;
    let mut problems = Vec::new();
    let on_disk: BTreeSet<&str> = items.iter().map(|i| i.path.as_str()).collect();
    for entry in &index.items {
        if !on_disk.contains(entry.path.as_str()) {
            problems.push(format!(
                "missing: {} is in {INDEX_FILE} but not on disk",
                entry.path
            ));
        }
    }
    for item in &items {
        match index.items.iter().find(|e| e.path == item.path) {
            None => problems.push(format!("added: {} is not in {INDEX_FILE}", item.path)),
            Some(entry) => {
                if entry.sha256 != library::sha256_hex(&item.bytes)
                    || entry.bytes != item.bytes.len() as u64
                {
                    problems.push(format!("changed: {} differs from {INDEX_FILE}", item.path));
                } else if entry.id != item.meta.id.clone().unwrap_or_default() {
                    problems.push(format!("changed: {} has a different id", item.path));
                }
                if let Some(t) = &entry.thumbnail
                    && !folder.join(t).is_file()
                {
                    problems.push(format!("missing: {t} (thumbnail of {})", item.path));
                }
            }
        }
    }
    if problems.is_empty() {
        println!("{INDEX_FILE} is up to date ({} item(s))", index.items.len());
        Ok(())
    } else {
        problems.push(format!(
            "{INDEX_FILE} is out of date; run `hew-cli library index` again"
        ));
        Err(problems.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn library_categories_map_to_index_categories() {
        use sources::Category as C;
        for (l, c) in [
            (Some(library::Category::Component), C::Components),
            (Some(library::Category::Material), C::Materials),
            (Some(library::Category::Model), C::Models),
            (Some(library::Category::Profile), C::Profiles),
            (None, C::Components),
        ] {
            let m = library::ItemMeta {
                category: l,
                ..Default::default()
            };
            assert_eq!(local_meta(m).category, Some(c));
        }
    }

    #[test]
    fn flags_parse_with_space_or_equals() {
        let o = parse_args(&s(&["dir", "--name", "A", "--license=MIT", "--check"])).unwrap();
        assert_eq!(o.folder, Some(PathBuf::from("dir")));
        assert_eq!(o.name.as_deref(), Some("A"));
        assert_eq!(o.license.as_deref(), Some("MIT"));
        assert!(o.check);
    }

    #[test]
    fn bad_arguments_are_errors() {
        assert!(parse_args(&s(&["--name"])).is_err());
        assert!(parse_args(&s(&["a", "b"])).is_err());
        assert!(parse_args(&s(&["--bogus"])).is_err());
    }
}
