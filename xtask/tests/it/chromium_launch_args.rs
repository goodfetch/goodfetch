//! Seam A (`xtask codegen` launch-argument codegen, ADR 0074, ticket #395):
//! `xtask codegen` generates `chromium/launch-args.json` from the engine's
//! launch-argument code, `xtask codegen --check` fails when it drifts,
//! changing the fixed seed or a method's arguments changes the file, the same
//! inputs give identical bytes, and the file is one of the Chromium build
//! gate's test inputs.
//! Declares no Feature (#325, Out of Scope, ambiguity 9).

use crate::workspace::{Outcome, Workspace, finding};

const JSON2TS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../node_modules/.bin/json2ts");
const DATAMODEL_CODEGEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../.venv/bin/datamodel-codegen"
);
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

fn run_codegen(workspace: &Workspace) -> Outcome {
    let outcome = workspace.xtask(&[
        "codegen",
        "--json2ts",
        JSON2TS,
        "--datamodel-codegen",
        DATAMODEL_CODEGEN,
    ]);
    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
    outcome
}

fn fixture_with_page_example() -> Workspace {
    let workspace = Workspace::from_fixture("codegen");
    workspace.write("crates/goodfetch-page/Cargo.toml", PAGE_MANIFEST.as_bytes());
    workspace.write("crates/goodfetch-page/src/lib.rs", b"");
    workspace.write(EXAMPLE_PATH, INITIAL_EXAMPLE.as_bytes());
    run_codegen(&workspace);
    workspace
}

#[test]
fn launch_args_codegen_is_deterministic_catches_drift_and_tracks_seed_and_method_changes() {
    let workspace = fixture_with_page_example();
    let initial_bytes = workspace
        .read(LAUNCH_ARGS)
        .expect("codegen writes chromium/launch-args.json");

    // Good input: `--check` passes on freshly generated output.
    let clean = workspace.xtask(&["codegen", "--check"]);
    assert_eq!(clean.code, 0, "{}{}", clean.stdout, clean.stderr);

    // Determinism: regenerating with identical inputs yields identical bytes.
    run_codegen(&workspace);
    assert_eq!(workspace.read(LAUNCH_ARGS).unwrap(), initial_bytes);

    // Bad input 1: hand-editing the committed file is caught by `--check`.
    workspace.write(LAUNCH_ARGS, b"{\"hand_edited\": true}\n");
    let hand_edited = workspace.xtask(&["codegen", "--check"]);
    assert_eq!(hand_edited.code, 1);
    assert_eq!(hand_edited.findings(), vec![finding("drift", LAUNCH_ARGS)]);

    // Restore clean state before testing source changes.
    run_codegen(&workspace);
    assert_eq!(workspace.xtask(&["codegen", "--check"]).code, 0);

    // Bad input 2: changing the fixed seed drifts until regenerated, and regeneration changes bytes.
    workspace.edit(EXAMPLE_PATH, "const SEED: u64 = 1;", "const SEED: u64 = 2;");
    let seed_drift = workspace.xtask(&["codegen", "--check"]);
    assert_eq!(seed_drift.code, 1);
    assert_eq!(seed_drift.findings(), vec![finding("drift", LAUNCH_ARGS)]);

    run_codegen(&workspace);
    let after_seed_bytes = workspace.read(LAUNCH_ARGS).unwrap();
    assert_ne!(after_seed_bytes, initial_bytes);
    assert_eq!(workspace.xtask(&["codegen", "--check"]).code, 0);

    // Bad input 3: changing a method's launch arguments drifts until regenerated, and regeneration changes bytes.
    workspace.edit(
        EXAMPLE_PATH,
        "\"--disable-blink-features=AutomationControlled\",\n];\nconst JS_INJECTION_ARGS",
        "\"--disable-blink-features=AutomationControlled\",\n    \"--disable-back-forward-cache\",\n];\nconst JS_INJECTION_ARGS",
    );
    let args_drift = workspace.xtask(&["codegen", "--check"]);
    assert_eq!(args_drift.code, 1);
    assert_eq!(args_drift.findings(), vec![finding("drift", LAUNCH_ARGS)]);

    run_codegen(&workspace);
    let after_args_bytes = workspace.read(LAUNCH_ARGS).unwrap();
    assert_ne!(after_args_bytes, after_seed_bytes);
    assert_eq!(workspace.xtask(&["codegen", "--check"]).code, 0);
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

    let parse = |stdout: &str| {
        let build = stdout
            .lines()
            .find_map(|l| l.strip_prefix("build_inputs="))
            .expect("build_inputs=");
        let test = stdout
            .lines()
            .find_map(|l| l.strip_prefix("test_inputs="))
            .expect("test_inputs=");
        (build.to_owned(), test.to_owned())
    };
    let (build_1, test_1) = parse(&first.stdout);
    let (build_2, test_2) = parse(&second.stdout);
    assert_eq!(
        build_1, build_2,
        "launch-args.json must not affect build_inputs"
    );
    assert_ne!(test_1, test_2, "launch-args.json must affect test_inputs");
}
