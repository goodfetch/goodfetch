//! Tests for the Plugin index publish and pull-request review workflows and
//! `xtask release plugin-index` commands (ADRs 0033 and 0052; issue 1534).
//! Declares no Feature of its own: the validation and client stories are area 7's.

use std::path::Path;

use serde_json::{Value, json};

use crate::workspace::{Workspace, write_executable};

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
}

#[test]
fn publish_raises_version_sets_14_day_expiry_preserves_revocations_and_verifies_bundle() {
    let workspace = Workspace::from_fixture("empty");
    let initial = json!({
        "format_version": "0.1",
        "version": 4,
        "signed_at": "2026-11-01T00:00:00Z",
        "expires_at": "2026-11-15T00:00:00Z",
        "revocations": [
            { "digest": "sha256:0000000000000000000000000000000000000000000000000000000000000001", "reason": "compromised" }
        ],
        "entries": []
    });
    workspace.write(
        "plugin-index/index.json",
        serde_json::to_string_pretty(&initial)
            .expect("serialize index")
            .as_bytes(),
    );

    // Push to default branch (`--min-age-days 0`) raises version to 5 and sets 14-day expiry.
    let prepared = workspace.xtask(&[
        "release",
        "plugin-index",
        "prepare",
        "--index",
        "plugin-index/index.json",
        "--now",
        "2026-11-05T12:00:00Z",
        "--min-age-days",
        "0",
    ]);
    assert_eq!(prepared.code, 0, "{}", prepared.stderr);
    let doc: Value = serde_json::from_str(&workspace.read_text("plugin-index/index.json"))
        .expect("parse updated index");
    assert_eq!(doc["version"], 5);
    assert_eq!(doc["signed_at"], "2026-11-05T12:00:00Z");
    assert_eq!(doc["expires_at"], "2026-11-19T12:00:00Z");
    assert_eq!(doc["revocations"].as_array().expect("revocations").len(), 1);

    // Rehearsal sign and verify bundle against the bundled public key.
    let signed = workspace.xtask(&[
        "release",
        "sign-rehearsal",
        "--file",
        "plugin-index/index.json",
        "--key-dir",
        "keys",
    ]);
    assert_eq!(signed.code, 0, "{}", signed.stderr);
    let verified = workspace.xtask(&[
        "release",
        "verify-rehearsal",
        "--file",
        "plugin-index/index.json",
        "--signature",
        "plugin-index/index.json.sig",
        "--attestation",
        "plugin-index/index.json.sigstore.jsonl",
        "--public-key",
        "keys/rehearsal.pub",
    ]);
    assert_eq!(verified.code, 0, "{}", verified.stderr);

    // Scheduled re-sign skips an index younger than 10 days (e.g. 9 days old).
    let fresh = workspace.xtask(&[
        "release",
        "plugin-index",
        "prepare",
        "--index",
        "plugin-index/index.json",
        "--now",
        "2026-11-14T11:59:59Z",
        "--min-age-days",
        "10",
    ]);
    assert_eq!(fresh.code, 0, "{}", fresh.stderr);
    assert_eq!(fresh.stdout.trim(), "unchanged");

    // At 10 days old, scheduled re-sign bumps version to 6 with a new 14-day expiry.
    let aged = workspace.xtask(&[
        "release",
        "plugin-index",
        "prepare",
        "--index",
        "plugin-index/index.json",
        "--now",
        "2026-11-15T12:00:00Z",
        "--min-age-days",
        "10",
    ]);
    assert_eq!(aged.code, 0, "{}", aged.stderr);
    let doc2: Value = serde_json::from_str(&workspace.read_text("plugin-index/index.json"))
        .expect("parse re-signed index");
    assert_eq!(doc2["version"], 6);
    assert_eq!(doc2["expires_at"], "2026-11-29T12:00:00Z");
}

#[test]
fn publish_advances_past_previously_published_version_and_checks_published_age() {
    let workspace = Workspace::from_fixture("empty");
    let committed = json!({
        "format_version": "0.1",
        "version": 4,
        "signed_at": "2026-11-01T00:00:00Z",
        "expires_at": "2026-11-15T00:00:00Z",
        "revocations": [],
        "entries": []
    });
    workspace.write(
        "plugin-index/index.json",
        serde_json::to_string_pretty(&committed)
            .expect("serialize committed index")
            .as_bytes(),
    );
    let published = json!({
        "format_version": "0.1",
        "version": 12,
        "signed_at": "2026-11-20T00:00:00Z",
        "expires_at": "2026-12-04T00:00:00Z",
        "revocations": [],
        "entries": []
    });
    workspace.write(
        "target/published.json",
        serde_json::to_string_pretty(&published)
            .expect("serialize published")
            .as_bytes(),
    );
    let still_fresh = workspace.xtask(&[
        "release",
        "plugin-index",
        "prepare",
        "--index",
        "plugin-index/index.json",
        "--published",
        "target/published.json",
        "--now",
        "2026-11-25T00:00:00Z",
        "--min-age-days",
        "10",
    ]);
    assert_eq!(still_fresh.code, 0, "{}", still_fresh.stderr);
    assert_eq!(still_fresh.stdout.trim(), "unchanged");

    let bumped_past_published = workspace.xtask(&[
        "release",
        "plugin-index",
        "prepare",
        "--index",
        "plugin-index/index.json",
        "--published",
        "target/published.json",
        "--now",
        "2026-11-30T00:00:00Z",
        "--min-age-days",
        "10",
    ]);
    assert_eq!(
        bumped_past_published.code, 0,
        "{}",
        bumped_past_published.stderr
    );
    let doc3: Value = serde_json::from_str(&workspace.read_text("plugin-index/index.json"))
        .expect("parse bumped index");
    assert_eq!(doc3["version"], 13);
    assert_eq!(doc3["expires_at"], "2026-12-14T00:00:00Z");
}

fn write_plugin_entry(
    workspace: &Workspace,
    name: &str,
    digest: &str,
    description: &str,
) -> String {
    let entry = json!({
        "name": name,
        "kind": "sink",
        "repository": "ghcr.io/example/plugin-csv",
        "digest": digest,
        "description": description,
    });
    let rel = format!("plugin-index/entries/{name}.json");
    workspace.write(
        &rel,
        serde_json::to_string_pretty(&entry)
            .expect("serialize entry")
            .as_bytes(),
    );
    rel
}

#[test]
fn review_accepts_verified_entry_and_refuses_bad_provenance_failed_suite_or_wall_bypass() {
    let workspace = Workspace::from_fixture("empty");
    let ok_exec = workspace.path().join("ok.sh");
    let fail_exec = workspace.path().join("fail.sh");
    write_executable(&ok_exec, "#!/usr/bin/env bash\nexit 0\n");
    write_executable(&fail_exec, "#!/usr/bin/env bash\nexit 1\n");

    let valid_digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    // 1. Valid entry: provenance passes, plugins test passes -> accepted and results recorded.
    let good = write_plugin_entry(
        &workspace,
        "csv-sink",
        valid_digest,
        "Exports crawled rows to CSV files.",
    );
    let accepted = workspace.xtask(&[
        "release",
        "plugin-index",
        "validate",
        "--entry",
        &good,
        "--attestation-verify",
        &ok_exec.to_string_lossy(),
        "--plugins-test",
        &ok_exec.to_string_lossy(),
        "--now",
        "2026-11-27T12:00:00Z",
    ]);
    assert_eq!(accepted.code, 0, "{}", accepted.stderr);
    let recorded: Value = serde_json::from_str(&workspace.read_text(&good)).expect("parse entry");
    assert_eq!(recorded["verification"]["status"], "passed");
    assert_eq!(recorded["verification"]["digest"], valid_digest);

    // 2. Failing provenance attestation -> refused.
    let bad_prov = workspace.xtask(&[
        "release",
        "plugin-index",
        "validate",
        "--entry",
        &good,
        "--attestation-verify",
        &fail_exec.to_string_lossy(),
        "--plugins-test",
        &ok_exec.to_string_lossy(),
    ]);
    assert_ne!(bad_prov.code, 0, "failed provenance must be refused");

    // 3. Failing plugins test suite -> refused.
    let bad_suite = workspace.xtask(&[
        "release",
        "plugin-index",
        "validate",
        "--entry",
        &good,
        "--attestation-verify",
        &ok_exec.to_string_lossy(),
        "--plugins-test",
        &fail_exec.to_string_lossy(),
    ]);
    assert_ne!(bad_suite.code, 0, "failed plugins test must be refused");

    // 4. Entry presenting itself as a way past Walls / paywalls / crawler verification -> refused (ADR 0052).
    let wall_entry = write_plugin_entry(
        &workspace,
        "wall-breaker",
        valid_digest,
        "Helps scripts bypass paywalls and solve captcha verification automatically.",
    );
    let refused_wall = workspace.xtask(&[
        "release",
        "plugin-index",
        "validate",
        "--entry",
        &wall_entry,
        "--attestation-verify",
        &ok_exec.to_string_lossy(),
        "--plugins-test",
        &ok_exec.to_string_lossy(),
    ]);
    assert_ne!(
        refused_wall.code, 0,
        "Wall/paywall bypass plugin must be refused under ADR 0052"
    );

    // 5. Unpinned mutable tag instead of sha256 digest -> refused.
    let unpinned = write_plugin_entry(&workspace, "mutable-tag", "latest", "Clean CSV sink.");
    let refused_tag = workspace.xtask(&[
        "release",
        "plugin-index",
        "validate",
        "--entry",
        &unpinned,
        "--attestation-verify",
        &ok_exec.to_string_lossy(),
        "--plugins-test",
        &ok_exec.to_string_lossy(),
    ]);
    assert_ne!(refused_tag.code, 0, "unpinned digest must be refused");
}

#[test]
fn workflows_are_keyless_outside_deny_open_issue_on_failure_and_review_has_no_secrets() {
    let publish =
        std::fs::read_to_string(repo_root().join(".github/workflows/plugin-index-publish.yml"))
            .expect("read plugin-index-publish.yml");
    assert!(
        publish.contains("cosign sign-blob --yes")
            && publish.contains("gh issue create")
            && publish.contains("ci/host/gitleaks.sh")
            && publish.contains("ci/leaks.sh")
            && publish.contains("--published target/plugin-index/published.json")
            && !publish.contains("secrets."),
        "publish workflow must be keyless Sigstore with no stored signing key, install gitleaks, track published version, and open an issue on failure"
    );

    let review =
        std::fs::read_to_string(repo_root().join(".github/workflows/plugin-index-review.yml"))
            .expect("read plugin-index-review.yml");
    assert!(
        review.contains("contents: read")
            && !review.contains("secrets.")
            && !review.contains(": write")
            && !review.contains("command -v true")
            && review.contains("ci/host/gitleaks.sh")
            && review.contains("ci/leaks.sh"),
        "review workflow must hold read-only permissions, zero secrets, install gitleaks, and never silently bypass plugins test"
    );

    let codeowners =
        std::fs::read_to_string(repo_root().join(".github/CODEOWNERS")).expect("read CODEOWNERS");
    assert!(
        codeowners.contains("/plugin-index/") && codeowners.contains("@Jbmanllr"),
        "CODEOWNERS must require owner approval for plugin-index changes"
    );

    let endpoints = std::fs::read_to_string(repo_root().join("ci/egress/endpoints.toml"))
        .expect("read endpoints.toml");
    assert!(
        endpoints.contains("plugin-index-publish.yml#publish")
            && endpoints.contains("plugin-index-review.yml#validate"),
        "both plugin-index workflows must be declared outside the deny in endpoints.toml"
    );
}
