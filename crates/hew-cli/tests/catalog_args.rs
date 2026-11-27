//! `hew-cli catalog check` argument handling: a reference is data, never a
//! flag. The check workflow passes references from a pull request's JSON
//! files, so a line such as `-h` must not be read as a request for help and
//! pass the check.

use std::process::Command;

fn check(args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hew-cli"))
        .arg("catalog")
        .arg("check")
        .args(args)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code().unwrap(), text)
}

fn assert_reference_refusal(args: &[&str]) {
    let (code, text) = check(args);
    assert_eq!(code, 1, "{args:?}: {text}");
    assert!(
        text.lines().any(|l| l.starts_with("reference: ")),
        "{args:?}: {text}"
    );
    assert!(!text.contains("USAGE"), "{args:?} printed help: {text}");
}

#[test]
fn a_dash_h_reference_after_the_separator_is_refused() {
    assert_reference_refusal(&["--", "-h"]);
}

#[test]
fn a_dash_dash_help_reference_after_the_separator_is_refused() {
    assert_reference_refusal(&["--", "--help"]);
}

#[test]
fn a_bare_dash_h_is_a_refused_reference_not_help() {
    assert_reference_refusal(&["-h"]);
}

#[test]
fn any_reference_starting_with_a_dash_is_refused() {
    assert_reference_refusal(&["--", "-x"]);
    assert_reference_refusal(&["--", "--verbose"]);
}

#[test]
fn a_reference_with_a_leading_space_is_refused() {
    assert_reference_refusal(&["--", " github:a/b"]);
}

#[test]
fn a_reference_with_an_embedded_newline_is_refused() {
    assert_reference_refusal(&["--", "github:a/b\nx"]);
}

#[test]
fn a_reference_with_a_control_character_is_refused() {
    assert_reference_refusal(&["--", "github:a/b\u{7}"]);
}

#[test]
fn the_sole_help_flag_still_prints_help() {
    let (code, text) = check(&["--help"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("catalog"), "{text}");
}
