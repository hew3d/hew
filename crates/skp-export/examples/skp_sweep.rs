//! Every `.skp` under the given paths, through everything Hew does with a
//! SketchUp file, checked by reading each result back:
//!
//! 1. open (import into a fresh document);
//! 2. save back unchanged, building on the file (its own container);
//! 3. edit — a solid moved, one deleted, a box drawn, a material made
//!    translucent, a tag hidden — and save back;
//! 4. open that save and save it again unchanged (a second generation);
//! 5. export whole, without the source, to both containers.
//!
//! Each written file must parse without desyncs (2017 files must pass the
//! whole-file grammar) and import as the document that was written. One
//! line per file, then a summary; nothing is written to disk unless
//! `--keep <dir>` is given, which keeps each file's step-3 save as
//! `<dir>/<stem>-<container>.skp` (and its expectation `.expect.hew`).
//!
//! ```text
//! cargo run --release -p skp-export --example skp_sweep -- [--max-mb N] [--keep dir] [--resaved] <file or dir> …
//! ```
//!
//! `--resaved` sweeps only SketchUp's re-saves (`*-resaved.skp`, which are
//! otherwise skipped): Hew's files after a trip through SketchUp, opened in
//! Hew and edited again.

#[path = "../tests/common/mod.rs"]
mod common;

use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::time::Instant;

use common::{DocD, digest, near};
use kernel::{Document, NodeId};
use skp_export::{SkpSource, Target, WriteEnv, export, save};

const ENV: WriteEnv = WriteEnv {
    unix_time: 1_800_000_000,
    seed: 5,
};

fn main() {
    let mut max_mb = 130u64;
    let mut keep: Option<PathBuf> = None;
    let mut resaved = false;
    let mut roots = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--max-mb" => max_mb = args.next().unwrap().parse().unwrap(),
            "--keep" => keep = Some(PathBuf::from(args.next().unwrap())),
            "--resaved" => resaved = true,
            _ => roots.push(PathBuf::from(a)),
        }
    }
    if let Some(k) = &keep {
        std::fs::create_dir_all(k).unwrap();
    }
    let mut files = Vec::new();
    for r in &roots {
        walk(r, &mut files, resaved);
    }
    files.sort();
    files.dedup();
    let (mut ok, mut no_import, mut skipped) = (0, 0, 0);
    let mut bad: Vec<String> = Vec::new();
    for f in &files {
        let len = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
        if len > max_mb << 20 {
            skipped += 1;
            println!("SKIP {} ({} MB)", f.display(), len >> 20);
            continue;
        }
        let bytes = std::fs::read(f).unwrap();
        let t = Instant::now();
        let r = std::panic::catch_unwind(AssertUnwindSafe(|| sweep(&bytes, f, keep.as_deref())));
        let took = t.elapsed().as_secs_f64();
        match r {
            Ok(Ok(Outcome::Clean(note))) => {
                ok += 1;
                println!("ok   {} {took:.1}s {note}", f.display());
            }
            Ok(Ok(Outcome::NoImport(e))) => {
                no_import += 1;
                println!("NOIM {} {e}", f.display());
            }
            Ok(Err(problems)) => {
                println!("FAIL {} {took:.1}s", f.display());
                for p in &problems {
                    println!("     {p}");
                }
                bad.push(f.display().to_string());
            }
            Err(e) => {
                let msg = e
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                println!("PANIC {} {msg}", f.display());
                bad.push(f.display().to_string());
            }
        }
    }
    println!(
        "\n{} files: {ok} clean, {} failed, {no_import} do not import, {skipped} skipped",
        files.len(),
        bad.len()
    );
    for b in &bad {
        println!("  {b}");
    }
}

/// Every `.skp` under `p`; SketchUp's re-saves (`*-resaved.skp`) only
/// with `resaved` (then only those).
fn walk(p: &Path, out: &mut Vec<PathBuf>, resaved: bool) {
    if p.is_dir() {
        for e in std::fs::read_dir(p).unwrap().flatten() {
            walk(&e.path(), out, resaved);
        }
    } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("skp"))
        && p.file_name()
            .is_some_and(|n| n.to_string_lossy().contains("-resaved") == resaved)
    {
        out.push(p.to_path_buf());
    }
}

enum Outcome {
    Clean(String),
    NoImport(String),
}

fn open(bytes: &[u8]) -> Result<(Document, SkpSource, usize), String> {
    let out = skp_import::import(bytes).map_err(|e| e.to_string())?;
    let warnings = out.warnings.len();
    let provenance = out.provenance.clone();
    let mut doc = Document::new();
    let (report, _) = doc
        .ingest(out.scene, out.textures_missing)
        .map_err(|e| format!("ingest: {e:?}"))?;
    doc.forget_history().map_err(|e| format!("{e:?}"))?;
    let src = SkpSource::after_open(bytes.to_vec(), &doc, &provenance, &report);
    Ok((doc, src, warnings))
}

fn sweep(bytes: &[u8], path: &Path, keep: Option<&Path>) -> Result<Outcome, Vec<String>> {
    let (doc, src, warnings) = match open(bytes) {
        Ok(x) => x,
        Err(e) => return Ok(Outcome::NoImport(e)),
    };
    let target = skp_export::target_of(bytes);
    let want = Want::of(&doc);
    let mut problems = Vec::new();
    let mut note = format!("{target:?} warnings={warnings}");

    // 2. Unchanged save.
    let t = Instant::now();
    match save(&doc, target, ENV, Some(&src)) {
        Err(e) => problems.push(format!("unchanged save: {e}")),
        Ok(s) => {
            note += &format!(" save={:.1}s", t.elapsed().as_secs_f64());
            if !s.export.simplified.is_empty() {
                note += &format!(" simplified={:?}", s.export.simplified);
            }
            check(
                "unchanged save",
                &s.export.bytes,
                target,
                &want,
                &mut problems,
            );
        }
    }

    // 3. Edit and save.
    let mut edited = doc.clone();
    edit(&mut edited);
    let want_edited = Want::of(&edited);
    match save(&edited, target, ENV, Some(&src)) {
        Err(e) => problems.push(format!("edited save: {e}")),
        Ok(s) => {
            check(
                "edited save",
                &s.export.bytes,
                target,
                &want_edited,
                &mut problems,
            );
            if let Some(dir) = keep {
                let stem: String = path
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() {
                            c.to_ascii_lowercase()
                        } else {
                            '-'
                        }
                    })
                    .collect();
                let c = match target {
                    Target::Stream(r) => r.number().to_string(),
                    Target::Skp2026 => "26".to_string(),
                    Target::Skp2026Compat => "26c".to_string(),
                };
                std::fs::write(dir.join(format!("{stem}-{c}.skp")), &s.export.bytes).unwrap();
                std::fs::write(
                    dir.join(format!("{stem}-{c}.expect.hew")),
                    edited.save_for_persistence(),
                )
                .unwrap();
            }
            // 4. Second generation: open the edited save, save it unchanged.
            match open(&s.export.bytes) {
                Err(e) => problems.push(format!("edited save does not reopen: {e}")),
                Ok((d2, src2, _)) => match save(&d2, target, ENV, Some(&src2)) {
                    Err(e) => problems.push(format!("second save: {e}")),
                    Ok(s2) => {
                        check(
                            "second save",
                            &s2.export.bytes,
                            target,
                            &want_edited,
                            &mut problems,
                        );
                        // Further generations must not drift or grow.
                        let mut sizes = vec![s.export.bytes.len(), s2.export.bytes.len()];
                        let mut last = s2.export.bytes;
                        for _ in 0..3 {
                            let Ok((d, src, _)) = open(&last) else { break };
                            let Ok(n) = save(&d, target, ENV, Some(&src)) else {
                                break;
                            };
                            sizes.push(n.export.bytes.len());
                            last = n.export.bytes;
                        }
                        check("fifth save", &last, target, &want_edited, &mut problems);
                        // (A byte or two is the 2026 container's compression.)
                        if sizes[4] > sizes[1] + sizes[1] / 1000 + 64 {
                            problems.push(format!("re-saves grow: {sizes:?}"));
                        } else if sizes.windows(2).any(|w| w[1] != w[0]) {
                            note += &format!(" sizes={sizes:?}");
                        }
                    }
                },
            }
        }
    }

    // 5. Whole exports.
    for t in [Target::Stream(skpwright::Release::R2017), Target::Skp2026] {
        match export(&doc, t, ENV) {
            Err(e) => problems.push(format!("export {t:?}: {e}")),
            Ok(x) => check(&format!("export {t:?}"), &x.bytes, t, &want, &mut problems),
        }
    }
    if problems.is_empty() {
        Ok(Outcome::Clean(note))
    } else {
        Err(problems)
    }
}

/// A document's digest at a micron, and at about 10 µm: siblings sort by their
/// digest, so a vertex a micron off (a pose rewritten from the document)
/// can reorder two of them in the fine one; the coarse one tells that
/// apart from a real difference, every field still compared.
struct Want {
    fine: DocD,
    coarse: DocD,
}

impl Want {
    fn of(doc: &Document) -> Want {
        let fine = digest(doc);
        // (Not a power of ten: drawn sizes sit on decimal rounding
        // boundaries, 9.302745 m at every decimal resolution alike.)
        common::set_quantum(99_991);
        let coarse = digest(doc);
        common::set_quantum(1_000_000);
        Want { fine, coarse }
    }
}

fn check(what: &str, bytes: &[u8], target: Target, want: &Want, problems: &mut Vec<String>) {
    match skpwright::Model::parse(bytes) {
        Err(e) => {
            problems.push(format!("{what}: does not parse: {e:?}"));
            return;
        }
        Ok(m) => {
            let d: Vec<_> = m
                .diagnostics
                .iter()
                .filter(|d| d.is_desync())
                .take(3)
                .collect();
            if !d.is_empty() {
                problems.push(format!("{what}: desyncs {d:?}"));
            }
        }
    }
    if matches!(target, Target::Stream(_))
        && let Err(e) = skpwright::verify(bytes)
    {
        problems.push(format!("{what}: stream grammar: {e:?}"));
    }
    let back = match open(bytes) {
        Ok((d, _, _)) => d,
        Err(e) => {
            problems.push(format!("{what}: does not reimport: {e}"));
            return;
        }
    };
    let got = Want::of(&back);
    for d in differs(want, &got) {
        problems.push(format!("{what}: {d}"));
    }
}

fn differs(want: &Want, got: &Want) -> Vec<String> {
    let (a, b) = (&want.fine, &got.fine);
    let mut out = Vec::new();
    macro_rules! f {
        ($f:ident) => {
            let (p, q) = (format!("{:?}", a.$f), format!("{:?}", b.$f));
            // (Equal at 10 µm, every field: siblings a micron apart that
            // sorted the other way.)
            let (pc, qc) = (format!("{:?}", want.coarse.$f), format!("{:?}", got.coarse.$f));
            if p != q && !near(&p, &q) && pc != qc && !near(&pc, &qc) {
                let at = first_far(&p, &q);
                let win = |s: &str| s.chars().skip(at.saturating_sub(120)).take(300).collect::<String>();
                out.push(format!(
                    "{} differs at {at} (len {} vs {}):\n       want: …{}…\n       got:  …{}…",
                    stringify!($f),
                    p.len(),
                    q.len(),
                    win(&p),
                    win(&q)
                ));
            }
        };
    }
    f!(roots);
    f!(tags);
    f!(guides);
    f!(section);
    f!(camera);
    f!(scenes);
    f!(sketches);
    f!(attrs);
    f!(annotations);
    out
}

/// Where two digests first differ by more than `near` forgives (a number
/// off by more than one, or any other token), as a char offset into `a`.
fn first_far(a: &str, b: &str) -> usize {
    let tokens = |s: &str| -> Vec<(usize, String)> {
        let mut out = Vec::new();
        let mut cur = String::new();
        let mut start = 0;
        for (i, c) in s.char_indices() {
            if c.is_ascii_digit() || c == '-' {
                if cur.is_empty() {
                    start = i;
                }
                cur.push(c);
            } else {
                if !cur.is_empty() {
                    out.push((start, std::mem::take(&mut cur)));
                }
                out.push((i, c.to_string()));
            }
        }
        if !cur.is_empty() {
            out.push((start, cur));
        }
        out
    };
    let (ta, tb) = (tokens(a), tokens(b));
    for ((i, x), (_, y)) in ta.iter().zip(&tb) {
        let close =
            matches!((x.parse::<i64>(), y.parse::<i64>()), (Ok(p), Ok(q)) if (p - q).abs() <= 1);
        if x != y && !close {
            return a[..*i].chars().count();
        }
    }
    ta.len().min(tb.len())
}

/// The edits a person makes: move a solid up, delete another, draw a new
/// box, make a material translucent, hide a tag.
fn edit(doc: &mut Document) {
    use kernel::{Plane, Point3, Transform, Vec3};
    let solids: Vec<kernel::ObjectId> = doc
        .top_level_nodes()
        .into_iter()
        .filter_map(|n| match n {
            // (Loose geometry, "Model", has no object identity in the file:
            // which Objects it reads back as follows from its faces, so a
            // deletion can join what was split. The acceptance set's
            // `ld-layers` case edits loose geometry.)
            NodeId::Object(o) if doc.object_name(o) != Some("Model") => Some(o),
            _ => None,
        })
        .collect();
    if let Some(&o) = solids.first() {
        doc.transform_object(o, &Transform::translation(Vec3::new(0.0, 0.0, 0.5)))
            .unwrap();
    }
    if let Some(&o) = solids.get(1) {
        doc.delete_node(NodeId::Object(o)).unwrap();
    }
    let s =
        doc.add_sketch(Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0.0, 0.0, 1.0)).unwrap());
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        let p = |x: f64, y: f64| Point3::new(x, y, 0.0);
        let ring = [
            p(-300.0, -300.0),
            p(-299.0, -300.0),
            p(-299.0, -299.0),
            p(-300.0, -299.0),
        ];
        for k in 0..4 {
            sk.add_segment(ring[k], ring[(k + 1) % 4]).unwrap();
        }
    }
    doc.end_sketch_gesture(s).unwrap();
    let region = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, region, 1.0).unwrap();
    let solid = doc
        .materials()
        .iter()
        .find(|(_, m)| m.texture.is_none() && m.color.a == 255)
        .map(|(id, _)| id);
    if let Some(mid) = solid {
        doc.set_material_alpha(mid, 128).unwrap();
    }
    let first_tag = doc.tag_meta().map(|(p, _)| p.to_vec()).next();
    if let Some(path) = first_tag {
        doc.set_tag_hidden(path, true);
    }
}
