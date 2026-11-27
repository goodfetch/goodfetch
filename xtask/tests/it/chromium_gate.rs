//! Seam A (`xtask chromium gate`, ADR 0074, ticket #400):
//! two Git tree hashes (build inputs vs test inputs, outputs excluded),
//! build and verdict attestations, HTTP profile check (spec #600), empty
//! manifest fast pass, and merge-queue mismatch refusal.
//! Declares no Feature (#325, Out of Scope, ambiguity 9).

use serde_json::json;

use crate::workspace::{Workspace, finding};

fn init_git(workspace: &Workspace) {
    let run = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .current_dir(workspace.path())
            .args(args)
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?} failed");
    };
    run(&["init", "-q"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
}

fn fixture_chromium_workspace() -> Workspace {
    let workspace = Workspace::from_fixture("empty");
    init_git(&workspace);
    // Build inputs under chromium/
    workspace.write(
        "chromium/version.toml",
        b"version = \"136.0.7103.0\"\nungoogled_tag = \"136.0.7103.0-1\"\n",
    );
    workspace.write("chromium/patches/0001-seed.patch", b"--- a\n+++ b\n");
    workspace.write("chromium/args.gn", b"is_official_build = true\n");
    // Test inputs
    workspace.write("chromium/wpt.toml", b"commit = \"aaaa1111\"\n");
    workspace.write("chromium/khronos.toml", b"commit = \"bbbb2222\"\n");
    workspace.write(
        "chromium/expectations/gtest/unit_tests-linux.filter",
        b"# clean\n",
    );
    workspace.write("chromium/launch-args.json", b"[\"--headless=new\"]\n");
    workspace.write(
        ".github/workflows/chromium-build-gate.yml",
        b"name: Chromium build gate\n",
    );
    // Outputs (must be in neither hash)
    workspace.write("chromium/manifest.json", b"{\"builds\": []}\n");
    workspace.write("chromium/protocol.json", b"{\"domains\": []}\n");
    workspace.write(
        "chromium/http-profiles/linux-x64.json",
        b"{\"ja4\": \"t13d\"}\n",
    );
    workspace
}

fn parse_hashes(stdout: &str) -> (String, String) {
    let mut build = None;
    let mut test = None;
    for line in stdout.lines() {
        if let Some(value) = line.strip_prefix("build_inputs=") {
            build = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("test_inputs=") {
            test = Some(value.to_owned());
        }
    }
    (
        build.expect("build_inputs= line"),
        test.expect("test_inputs= line"),
    )
}

#[test]
fn build_and_test_hashes_track_their_inputs_and_ignore_outputs() {
    let workspace = fixture_chromium_workspace();
    let initial = workspace.xtask(&["chromium", "gate", "hashes"]);
    assert_eq!(initial.code, 0, "{}{}", initial.stdout, initial.stderr);
    let (build_0, test_0) = parse_hashes(&initial.stdout);
    assert_eq!(
        build_0.len(),
        40,
        "Git tree hash is 40 hex chars: {build_0}"
    );
    assert_eq!(test_0.len(), 40, "Git tree hash is 40 hex chars: {test_0}");

    // Changing outputs (manifest, protocol JSON, HTTP profile) changes NEITHER hash.
    workspace.write(
        "chromium/manifest.json",
        b"{\"builds\": [{\"platform\": \"linux-x64\", \"digest\": \"sha256:abc\"}]}\n",
    );
    workspace.write(
        "chromium/protocol.json",
        b"{\"domains\": [{\"domain\": \"Page\"}]}\n",
    );
    workspace.write(
        "chromium/http-profiles/linux-x64.json",
        b"{\"ja4\": \"changed\"}\n",
    );
    let after_outputs = workspace.xtask(&["chromium", "gate", "hashes"]);
    assert_eq!(
        after_outputs.code, 0,
        "{}{}",
        after_outputs.stdout, after_outputs.stderr
    );
    assert_eq!(
        parse_hashes(&after_outputs.stdout),
        (build_0.clone(), test_0.clone())
    );

    // Changing a build input changes ONLY build_inputs.
    workspace.write(
        "chromium/patches/0001-seed.patch",
        b"--- a\n+++ b\n+changed\n",
    );
    let after_build = workspace.xtask(&["chromium", "gate", "hashes"]);
    assert_eq!(after_build.code, 0);
    let (build_1, test_1) = parse_hashes(&after_build.stdout);
    assert_ne!(build_1, build_0);
    assert_eq!(test_1, test_0);

    // Changing a test input (workflow file) changes ONLY test_inputs.
    workspace.write(
        ".github/workflows/chromium-build-gate.yml",
        b"name: Chromium build gate v2\n",
    );
    let after_test = workspace.xtask(&["chromium", "gate", "hashes"]);
    assert_eq!(after_test.code, 0);
    let (build_2, test_2) = parse_hashes(&after_test.stdout);
    assert_eq!(build_2, build_1);
    assert_ne!(test_2, test_1);
}

#[test]
fn an_empty_manifest_passes_and_builds_nothing() {
    let workspace = fixture_chromium_workspace();
    let outcome = workspace.xtask(&[
        "chromium",
        "gate",
        "check",
        "--event",
        "pull_request",
        "--junit",
        "target/junit/chromium-build.xml",
    ]);
    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
    assert!(
        outcome.stdout.contains("verdict=pass"),
        "{}",
        outcome.stdout
    );
    assert!(
        workspace
            .path()
            .join("target/junit/chromium-build.xml")
            .is_file(),
        "JUnit file written"
    );
}

fn setup_attested_workspace() -> (Workspace, String, String, String) {
    let workspace = fixture_chromium_workspace();
    let hashes = workspace.xtask(&["chromium", "gate", "hashes"]);
    let (build_hash, test_hash) = parse_hashes(&hashes.stdout);

    let digest =
        "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned();
    let manifest = json!({
        "builds": [{
            "platform": "linux-x64",
            "digest": digest,
            "http_profile": "chromium/http-profiles/linux-x64.json"
        }]
    });
    workspace.write("chromium/manifest.json", manifest.to_string().as_bytes());
    (workspace, digest, build_hash, test_hash)
}

#[test]
fn fast_path_passes_when_attestations_and_http_profile_match() {
    let (workspace, digest, build_hash, test_hash) = setup_attested_workspace();
    let good_attestations = json!({
        "builds": [{ "digest": digest, "build_inputs": build_hash }],
        "verdicts": [{ "digest": digest, "build_inputs": build_hash, "test_inputs": test_hash }]
    });
    workspace.write(
        "attestations.json",
        good_attestations.to_string().as_bytes(),
    );

    let ok = workspace.xtask(&[
        "chromium",
        "gate",
        "check",
        "--event",
        "merge_group",
        "--attestations",
        "attestations.json",
    ]);
    assert_eq!(ok.code, 0, "{}{}", ok.stdout, ok.stderr);
    assert!(ok.stdout.contains("fast_path=hit"), "{}", ok.stdout);

    // Missing HTTP profile in repository -> fails (spec #600 comment on #400).
    workspace.remove("chromium/http-profiles/linux-x64.json");
    let missing_profile = workspace.xtask(&[
        "chromium",
        "gate",
        "check",
        "--event",
        "pull_request",
        "--attestations",
        "attestations.json",
    ]);
    assert_eq!(
        missing_profile.code, 1,
        "{}{}",
        missing_profile.stdout, missing_profile.stderr
    );
    assert!(
        missing_profile
            .findings()
            .contains(&finding("missing-http-profile", "linux-x64")),
        "{:?}",
        missing_profile.findings()
    );
}

#[test]
fn fast_path_fails_when_verdict_or_build_attestation_mismatches() {
    let (workspace, digest, build_hash, test_hash) = setup_attested_workspace();

    // Verdict naming another head's test hash -> fails fast path (and merge queue).
    let wrong_verdict = json!({
        "builds": [{ "digest": digest, "build_inputs": build_hash }],
        "verdicts": [{ "digest": digest, "build_inputs": build_hash, "test_inputs": "0000000000000000000000000000000000000000" }]
    });
    workspace.write("attestations.json", wrong_verdict.to_string().as_bytes());
    let bad_verdict = workspace.xtask(&[
        "chromium",
        "gate",
        "check",
        "--event",
        "merge_group",
        "--attestations",
        "attestations.json",
    ]);
    assert_eq!(
        bad_verdict.code, 1,
        "{}{}",
        bad_verdict.stdout, bad_verdict.stderr
    );
    assert!(
        bad_verdict
            .findings()
            .contains(&finding("verdict-attestation-mismatch", "linux-x64")),
        "{:?}",
        bad_verdict.findings()
    );

    // Build attestation naming other build inputs -> fails.
    let wrong_build = json!({
        "builds": [{ "digest": digest, "build_inputs": "1111111111111111111111111111111111111111" }],
        "verdicts": [{ "digest": digest, "build_inputs": build_hash, "test_inputs": test_hash }]
    });
    workspace.write("attestations.json", wrong_build.to_string().as_bytes());
    let bad_build = workspace.xtask(&[
        "chromium",
        "gate",
        "check",
        "--event",
        "pull_request",
        "--attestations",
        "attestations.json",
    ]);
    assert_eq!(
        bad_build.code, 1,
        "{}{}",
        bad_build.stdout, bad_build.stderr
    );
    assert!(
        bad_build
            .findings()
            .contains(&finding("build-attestation-mismatch", "linux-x64")),
        "{:?}",
        bad_build.findings()
    );
}
