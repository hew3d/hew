//! A guard for the live app's WASM kernel: certain `std` calls PANIC on
//! `wasm32-unknown-unknown` (there is no clock, pid, or OS to ask), and a
//! panic inside the live webview wedges the whole window. Native tests
//! cannot see this — the call works fine on the machine running them —
//! which is exactly how `hew.library.save` shipped a trap that took the app
//! down the first time a live client used it. Every crate compiled into the
//! WASM module is scanned, outside its `#[cfg(test)]` code, for the calls
//! below; a legitimate use (a host's NATIVE default, never reached from
//! WASM) carries a `// wasm-guard: <why>` marker on the same line.

use std::path::{Path, PathBuf};

const FORBIDDEN: &[&str] = &[
    "SystemTime::now",
    "Instant::now",
    "process::id",
    "std::env::var",
    "thread::sleep",
    "thread::spawn",
    "thread::Builder",
    "std::fs::",
    "std::net::",
    ".elapsed()",
];

/// Crates linked into `wasm-api` (its Cargo.toml dependencies) plus itself.
const CRATES: &[&str] = &[
    "api",
    "kernel",
    "inference",
    "tessellate",
    "mesh-export",
    "softrender",
    "hlr",
    "pdfwrite",
    "dae-import",
    "gltf-import",
    "skp-import",
    "stl-import",
    "image-import",
    "mesh-heal",
    "sources",
    "wasm-api",
];

/// `text` without its test-only code: every `#[cfg(test)]` item — a whole
/// `mod tests { … }`, or a single `use`/`fn`/`thread_local!` mid-file — is
/// cut at the item's end (its balanced closing brace, or its `;`). Cutting
/// from the FIRST `#[cfg(test)]` to the end of the file would skip
/// production code that follows a one-line test-only helper.
fn without_test_code(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("#[cfg(test)]") {
        out.push_str(&rest[..at]);
        let after = &rest[at + "#[cfg(test)]".len()..];
        let mut depth = 0usize;
        let mut end = after.len();
        for (i, c) in after.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        end = i + 1;
                        break;
                    }
                }
                ';' if depth == 0 => {
                    end = i + 1;
                    break;
                }
                _ => {}
            }
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn wasm_reachable_code_makes_no_calls_that_panic_on_wasm() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut offenders = Vec::new();
    for krate in CRATES {
        let mut files = Vec::new();
        rust_files(&root.join(krate).join("src"), &mut files);
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            let live = without_test_code(&text);
            for (n, line) in live.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                if FORBIDDEN.iter().any(|f| code.contains(f)) && !line.contains("wasm-guard:") {
                    offenders.push(format!("{}:{}: {}", file.display(), n + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these panic (or do nothing useful) on wasm32-unknown-unknown; route them through a host \
         method, or mark a native-only default with `// wasm-guard: <why>`:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn test_only_items_are_cut_without_hiding_the_code_after_them() {
    let src = "fn live() {}\n#[cfg(test)]\nuse std::time::Instant;\nfn after() { Instant::now(); }\n#[cfg(test)]\nmod tests {\n fn t() { SystemTime::now(); }\n}\nfn tail() {}\n";
    let kept = without_test_code(src);
    assert!(
        kept.contains("fn after()"),
        "production code after a test-only use is kept"
    );
    assert!(
        kept.contains("fn tail()"),
        "code after a test module is kept"
    );
    assert!(!kept.contains("SystemTime"), "the test module is cut");
}
