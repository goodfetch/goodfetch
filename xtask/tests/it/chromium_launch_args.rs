//! Seam A (`xtask codegen` launch-argument codegen, ADR 0074, ticket #395):
//! `xtask codegen` generates `chromium/launch-args.json` from the engine's
//! launch-argument code, `xtask codegen --check` fails when it drifts,
//! changing the fixed seed or a method's arguments changes the file, the same
//! inputs give identical bytes, missing example sources fail loudly, and the
//! file is one of the Chromium build gate's test inputs.
//! Declares no Feature (#325, Out of Scope, ambiguity 9).

use crate::chromium_gate::parse_hashes;
use crate::codegen::codegen;
use crate::workspace::{Workspace, finding};

const LAUNCH_ARGS: &str = "chromium/launch-args.json";
const EXAMPLE_PATH: &str = "crates/goodfetch-page/examples/launch-arguments.rs";

const PAGE_MANIFEST: &str = r#"[package]
name = "goodfetch-page"
version.workspace = true
edition.workspace = true
publish = false
"#;

const INITIAL_EXAMPLE: &str = r#"use std::path::PathBuf;

const SEED: u64 = 1;
const NONE_ARGS: &[&str] = &[
    "--no-first-run",
    "--disable-blink-features=AutomationControlled",
];
const JS_INJECTION_ARGS: &[&str] = &[
    "--no-first-run",
    "--disable-blink-features=AutomationControlled",
];

fn quote_list(items: &[&str]) -> String {
    items
        .iter()
        .map(|item| format!("\"{item}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

fn main() {
    let target = PathBuf::from(std::env::args_os().nth(1).expect("output path"));
    let json = format!(
        "{{\n  \"generated_by\": \"xtask codegen\",\n  \"identity_seed\": {SEED},\n  \"methods\": {{\n    \"js_injection\": {{ \"args\": [{}], \"init_script\": \"/* seed={SEED} */\" }},\n    \"none\": {{ \"args\": [{}] }}\n  }}\n}}\n",
        quote_list(JS_INJECTION_ARGS),
        quote_list(NONE_ARGS),
    );
    std::fs::write(&target, json).expect("write launch args");
}
"#;

fn generated_with_page_example() -> Workspace {
    Workspace::made_once_per_run("chromium-launch-args", || {
        let workspace = Workspace::from_fixture("codegen");
        workspace.write("crates/goodfetch-page/Cargo.toml", PAGE_MANIFEST.as_bytes());
        workspace.write("crates/goodfetch-page/src/lib.rs", b"");
        workspace.write(EXAMPLE_PATH, INITIAL_EXAMPLE.as_bytes());
        codegen(&workspace);
        workspace
    })
}

#[test]
fn codegen_writes_launch_args_and_check_passes() {
    let workspace = generated_with_page_example();
    assert!(workspace.read(LAUNCH_ARGS).is_some());

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
}

#[test]
fn regenerating_launch_args_with_same_inputs_gives_identical_bytes() {
    let workspace = generated_with_page_example();
    let first = workspace.read(LAUNCH_ARGS).unwrap();

    codegen(&workspace);

    assert_eq!(workspace.read(LAUNCH_ARGS).unwrap(), first);
}

#[test]
fn check_fails_when_launch_args_is_edited_by_hand() {
    let workspace = generated_with_page_example();
    workspace.write(LAUNCH_ARGS, b"{\"hand_edited\": true}\n");

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(outcome.code, 1);
    assert_eq!(outcome.findings(), vec![finding("edited", LAUNCH_ARGS)]);
}

#[test]
fn changing_the_fixed_seed_drifts_until_regenerated_and_changes_bytes() {
    let workspace = generated_with_page_example();
    let before = workspace.read(LAUNCH_ARGS).unwrap();
    workspace.edit(EXAMPLE_PATH, "const SEED: u64 = 1;", "const SEED: u64 = 2;");

    let drift = workspace.xtask(&["codegen", "--check"]);
    assert_eq!(drift.code, 1);
    assert_eq!(drift.findings(), vec![finding("stale", LAUNCH_ARGS)]);

    codegen(&workspace);
    assert_ne!(workspace.read(LAUNCH_ARGS).unwrap(), before);
    assert_eq!(workspace.xtask(&["codegen", "--check"]).code, 0);
}

#[test]
fn changing_a_methods_arguments_drifts_until_regenerated_and_changes_bytes() {
    let workspace = generated_with_page_example();
    let before = workspace.read(LAUNCH_ARGS).unwrap();
    workspace.edit(
        EXAMPLE_PATH,
        "\"--disable-blink-features=AutomationControlled\",\n];\nconst JS_INJECTION_ARGS",
        "\"--disable-blink-features=AutomationControlled\",\n    \"--disable-back-forward-cache\",\n];\nconst JS_INJECTION_ARGS",
    );

    let drift = workspace.xtask(&["codegen", "--check"]);
    assert_eq!(drift.code, 1);
    assert_eq!(drift.findings(), vec![finding("stale", LAUNCH_ARGS)]);

    codegen(&workspace);
    assert_ne!(workspace.read(LAUNCH_ARGS).unwrap(), before);
    assert_eq!(workspace.xtask(&["codegen", "--check"]).code, 0);
}

#[test]
fn check_fails_when_page_crate_lacks_launch_arguments_example() {
    let workspace = generated_with_page_example();
    workspace.remove(EXAMPLE_PATH);

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_ne!(outcome.code, 0);
    assert!(
        outcome.stderr.contains(EXAMPLE_PATH),
        "expected missing example error naming {EXAMPLE_PATH}: {}",
        outcome.stderr
    );
}

#[test]
fn launch_args_file_is_hashed_as_a_chromium_gate_test_input() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write("chromium/version.toml", b"version = \"136.0.0.0\"\n");
    workspace.write(LAUNCH_ARGS, b"{\"identity_seed\": 1}\n");

    let first = workspace.xtask(&["chromium", "gate", "hashes"]);
    assert_eq!(first.code, 0, "{}{}", first.stdout, first.stderr);

    workspace.write(LAUNCH_ARGS, b"{\"identity_seed\": 2}\n");
    let second = workspace.xtask(&["chromium", "gate", "hashes"]);
    assert_eq!(second.code, 0, "{}{}", second.stdout, second.stderr);

    let (build_1, test_1) = parse_hashes(&first.stdout);
    let (build_2, test_2) = parse_hashes(&second.stdout);
    assert_eq!(
        build_1, build_2,
        "launch-args.json must not affect build_inputs"
    );
    assert_ne!(test_1, test_2, "launch-args.json must affect test_inputs");
}
