//! Platform signing jobs, the Platform signature gate, and signed-artifact
//! reuse (spec #130, issue 142; ADRs 0039, 0051, 0074, 0076, 0088 and 0094).
//! Covers: story:1-6, story:1-7, story:1-72, story:1-86

use std::collections::BTreeSet;
use std::path::Path;

use rstest::rstest;
use serde_json::{Value, json};

use crate::workspace::{Workspace, finding};

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}

fn read_workflow() -> Value {
    let text = std::fs::read_to_string(root().join(".github/workflows/release-build.yml")).unwrap();
    serde_saphyr::from_str(&text).unwrap()
}

fn needs_of(job: &Value) -> BTreeSet<String> {
    match job.get("needs") {
        Some(Value::String(one)) => BTreeSet::from([one.clone()]),
        Some(Value::Array(many)) => many
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => BTreeSet::new(),
    }
}

fn valid_report() -> Value {
    json!({
        "macos": [
            { "target": "aarch64-apple-darwin", "binary": "goodfetch-collie", "codesign_strict": true, "notarized": true, "allow_jit": false },
            { "target": "aarch64-apple-darwin", "binary": "goodfetch-foxhound", "codesign_strict": true, "notarized": true, "allow_jit": false },
            { "target": "aarch64-apple-darwin", "binary": "goodfetch-proving-ground", "codesign_strict": true, "notarized": true, "allow_jit": false },
            { "target": "x86_64-apple-darwin", "binary": "goodfetch-collie", "codesign_strict": true, "notarized": true, "allow_jit": false },
            { "target": "x86_64-apple-darwin", "binary": "goodfetch-foxhound", "codesign_strict": true, "notarized": true, "allow_jit": false },
            { "target": "x86_64-apple-darwin", "binary": "goodfetch-proving-ground", "codesign_strict": true, "notarized": true, "allow_jit": false }
        ],
        "macos_apps": [
            { "target": "aarch64-apple-darwin", "bundle": "Goodfetch Chromium.app", "codesign_deep": true, "spctl_ok": true, "stapler_valid": true, "allow_jit_outside_helpers": false },
            { "target": "x86_64-apple-darwin", "bundle": "Goodfetch Chromium.app", "codesign_deep": true, "spctl_ok": true, "stapler_valid": true, "allow_jit_outside_helpers": false }
        ],
        "windows": [
            { "target": "x86_64-pc-windows-msvc", "binary": "goodfetch-collie.exe", "signtool_pa": true, "rfc3161_timestamp": true },
            { "target": "x86_64-pc-windows-msvc", "binary": "goodfetch-foxhound.exe", "signtool_pa": true, "rfc3161_timestamp": true },
            { "target": "x86_64-pc-windows-msvc", "binary": "goodfetch-proving-ground.exe", "signtool_pa": true, "rfc3161_timestamp": true },
            { "target": "x86_64-pc-windows-msvc", "binary": "goodfetch-filter-broker.exe", "signtool_pa": true, "rfc3161_timestamp": true },
            { "target": "aarch64-pc-windows-msvc", "binary": "goodfetch-collie.exe", "signtool_pa": true, "rfc3161_timestamp": true },
            { "target": "aarch64-pc-windows-msvc", "binary": "goodfetch-foxhound.exe", "signtool_pa": true, "rfc3161_timestamp": true },
            { "target": "aarch64-pc-windows-msvc", "binary": "goodfetch-proving-ground.exe", "signtool_pa": true, "rfc3161_timestamp": true },
            { "target": "aarch64-pc-windows-msvc", "binary": "goodfetch-filter-broker.exe", "signtool_pa": true, "rfc3161_timestamp": true }
        ]
    })
}

/// While accounts are closed under ADR 0076, `0.y` previews report
/// `not run: accounts not open` without blocking (grilling G-18 Q11).
#[rstest]
#[case::preview("0.1.0")]
#[case::preview_rc("0.2.0-rc.1")]
#[case::nightly("0.0.0-dev.1")]
fn signature_gate_reports_not_run_for_zero_y_previews_while_accounts_closed(#[case] version: &str) {
    let workspace = Workspace::from_fixture("release");

    let outcome = workspace.xtask(&[
        "release",
        "signature-gate",
        "--version",
        version,
        "--pr-comment",
        "gate-status.txt",
    ]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(outcome.stdout.trim(), "not run: accounts not open");
    assert_eq!(
        workspace.read_text("gate-status.txt").trim(),
        "not run: accounts not open"
    );
}

/// For `1.0.0` candidates and any `>= 1.0.0` version, closed accounts block
/// the release (ADR 0039, grilling G-18 Q11).
#[rstest]
#[case::one_zero_rc("1.0.0-rc.1")]
#[case::one_zero("1.0.0")]
#[case::minor("1.4.0")]
fn signature_gate_blocks_one_zero_and_candidates_while_accounts_closed(#[case] version: &str) {
    let workspace = Workspace::from_fixture("release");

    let outcome = workspace.xtask(&["release", "signature-gate", "--version", version]);

    assert_ne!(outcome.code, 0, "expected gate refusal: {}", outcome.stdout);
    assert_eq!(
        outcome.findings(),
        [finding("platform-signature-required", version)]
    );
}

#[test]
fn signature_gate_passes_valid_report_and_rejects_unsigned_missing_timestamp_or_allow_jit() {
    let workspace = Workspace::from_fixture("release");
    workspace.write(
        "report-ok.json",
        serde_json::to_string_pretty(&valid_report())
            .unwrap()
            .as_bytes(),
    );

    let ok = workspace.xtask(&[
        "release",
        "signature-gate",
        "--version",
        "1.0.0",
        "--accounts-open",
        "--report",
        "report-ok.json",
    ]);
    assert_eq!(ok.code, 0, "{}\n{}", ok.stdout, ok.stderr);

    // Mutate report: fail timestamp, allow_jit, and `.app` stapler validation.
    let mut bad = valid_report();
    bad["windows"][0]["rfc3161_timestamp"] = json!(false);
    bad["macos"][0]["allow_jit"] = json!(true);
    bad["macos_apps"][0]["stapler_valid"] = json!(false);
    workspace.write(
        "report-bad.json",
        serde_json::to_string_pretty(&bad).unwrap().as_bytes(),
    );

    let failed = workspace.xtask(&[
        "release",
        "signature-gate",
        "--version",
        "1.0.0",
        "--accounts-open",
        "--report",
        "report-bad.json",
    ]);
    assert_ne!(failed.code, 0, "mutated report must fail");
    let findings = failed.findings();
    assert!(findings.contains(&finding(
        "missing-timestamp",
        "x86_64-pc-windows-msvc/goodfetch-collie.exe"
    )));
    assert!(findings.contains(&finding(
        "forbidden-entitlement",
        "aarch64-apple-darwin/goodfetch-collie"
    )));
    assert!(findings.contains(&finding(
        "notarization-failed",
        "aarch64-apple-darwin/Goodfetch Chromium.app"
    )));
}

#[test]
fn signing_reuse_key_reuses_identical_unsigned_bytes_and_invalidates_on_change() {
    let workspace = Workspace::from_fixture("release");
    workspace.write("bins/aarch64-apple-darwin/goodfetch-collie", b"v1-collie");
    workspace.write(
        "bins/aarch64-apple-darwin/goodfetch-foxhound",
        b"v1-foxhound",
    );

    let first = workspace.xtask(&[
        "release",
        "signing-reuse-key",
        "--version",
        "1.4.0",
        "--binaries",
        "bins",
        "--out",
        "reuse-1.json",
    ]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    assert!(first.stdout.starts_with("sign "), "{}", first.stdout);

    let second = workspace.xtask(&[
        "release",
        "signing-reuse-key",
        "--version",
        "1.4.0",
        "--binaries",
        "bins",
        "--out",
        "reuse-2.json",
        "--existing",
        "reuse-1.json",
    ]);
    assert_eq!(second.code, 0, "{}", second.stderr);
    assert!(second.stdout.starts_with("reuse "), "{}", second.stdout);

    // Changing one binary invalidates reuse.
    workspace.write("bins/aarch64-apple-darwin/goodfetch-collie", b"v2-collie");
    let changed = workspace.xtask(&[
        "release",
        "signing-reuse-key",
        "--version",
        "1.4.0",
        "--binaries",
        "bins",
        "--out",
        "reuse-3.json",
        "--existing",
        "reuse-1.json",
    ]);
    assert_eq!(changed.code, 0, "{}", changed.stderr);
    assert!(changed.stdout.starts_with("sign "), "{}", changed.stdout);
}

#[test]
fn only_signing_jobs_use_release_environment_and_run_in_fixed_order() {
    let workflow = read_workflow();
    let jobs = workflow["jobs"].as_object().unwrap();

    let expected_signing_jobs: BTreeSet<&str> = [
        "sign-proving-ground-macos",
        "sign-proving-ground-windows",
        "sign-macos",
        "sign-windows",
    ]
    .into_iter()
    .collect();

    let actual_release_env_jobs: BTreeSet<&str> = jobs
        .iter()
        .filter(|(_, job)| job.get("environment").and_then(Value::as_str) == Some("release"))
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(
        actual_release_env_jobs, expected_signing_jobs,
        "ONLY signing jobs may name the `release` environment (ADR 0039)"
    );

    // Build jobs never reference any signing secret.
    for build_job in ["proving-ground", "build-targets", "fetch-chromium"] {
        let serialized = serde_json::to_string(&jobs[build_job]).unwrap();
        assert!(
            !serialized.contains("APPLE_") && !serialized.contains("AZURE_"),
            "build job `{build_job}` must never reference signing credentials"
        );
    }

    // Proving ground signing runs strictly before `build-targets`.
    let pg_needs = needs_of(&jobs["sign-proving-ground"]);
    assert!(pg_needs.contains("sign-proving-ground-macos"));
    assert!(pg_needs.contains("sign-proving-ground-windows"));
    assert!(needs_of(&jobs["build-targets"]).contains("sign-proving-ground"));

    // Signature gate and signed-bundle smoke run before `package-and-digests`.
    assert!(needs_of(&jobs["platform-signature-gate"]).contains("sign-artifacts"));
    for smoke in [
        "signed-bundle-smoke-linux",
        "signed-bundle-smoke-windows",
        "signed-bundle-smoke-macos",
    ] {
        assert!(needs_of(&jobs[smoke]).contains("platform-signature-gate"));
        assert!(needs_of(&jobs["package-and-digests"]).contains(smoke));
    }

    // Scripts enforce hardened runtime without allow-jit on Product executables,
    // Chrome's helper entitlement scheme on Chromium `.app` bundles (ADR 0039),
    // single-ZIP notarization, RFC 3161 timestamps, and signed-bundle smoke
    // without `--no-sandbox`.
    let macos_script = std::fs::read_to_string(root().join("ci/release/sign-macos.sh")).unwrap();
    assert!(macos_script.contains("--options runtime"));
    assert!(macos_script.contains("notarytool submit"));
    assert!(macos_script.contains("create-keychain"));
    assert!(macos_script.contains("delete-keychain"));
    let sign_binary_fn = macos_script
        .split("sign_binary()")
        .nth(1)
        .and_then(|rest| rest.split("\n}\n").next())
        .expect("sign_binary function in sign-macos.sh");
    assert!(
        !sign_binary_fn.contains("--entitlements"),
        "Product executables must never receive --entitlements"
    );

    let win_script = std::fs::read_to_string(root().join("ci/release/sign-windows.ps1")).unwrap();
    assert!(win_script.contains("timestamp.acs.microsoft.com"));
    assert!(win_script.contains("x86_64-pc-windows-msvc"));
    assert!(win_script.contains("aarch64-pc-windows-msvc"));
    assert!(win_script.contains("goodfetch-collie.exe"));
    assert!(win_script.contains("goodfetch-foxhound.exe"));

    let smoke_script =
        std::fs::read_to_string(root().join("ci/release/smoke-signed-chromium.sh")).unwrap();
    assert!(smoke_script.contains("/json/version"));
    assert!(!smoke_script.contains(" --no-sandbox"));
}
