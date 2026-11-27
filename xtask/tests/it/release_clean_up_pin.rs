//! Tests for Clean-up rules pinning on release branches and nightlies against a
//! real `registry:2` daily-channel stand-in, seed fallback, zero build-script
//! network downloads, notice inventory coverage, and nightly asset/notes wiring
//! (ADRs 0030, 0032 and 0047; issue 207).

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::workspace::Workspace;

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let mut out = String::with_capacity(64);
    for b in hasher.finalize() {
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[cfg(target_os = "linux")]
struct DailySetFixture<'a> {
    tag: &'a str,
    line: &'a str,
    version: &'a str,
    gate: &'a str,
    full_bytes: &'a [u8],
    embedded_bytes: &'a [u8],
}

#[cfg(target_os = "linux")]
fn push_daily_set(registry: &str, repository: &str, set: &DailySetFixture<'_>) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();

    let upload_blob = |bytes: &[u8]| -> String {
        let digest = format!("sha256:{}", sha256_hex(bytes));
        let start = agent
            .post(&format!("{registry}/v2/{repository}/blobs/uploads/"))
            .send_empty()
            .expect("start blob upload");
        assert_eq!(start.status(), 202);
        let loc = start
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .expect("Location header");
        let put_base = if loc.starts_with("http://") || loc.starts_with("https://") {
            loc.to_owned()
        } else {
            format!("{registry}{loc}")
        };
        let sep = if put_base.contains('?') { '&' } else { '?' };
        let put = agent
            .put(&format!("{put_base}{sep}digest={digest}"))
            .header("Content-Type", "application/octet-stream")
            .send(bytes)
            .expect("put blob");
        assert!(put.status() == 201 || put.status() == 202);
        digest
    };

    let config_bytes = b"{}";
    let config_digest = upload_blob(config_bytes);
    let full_digest = upload_blob(set.full_bytes);
    let embedded_digest = upload_blob(set.embedded_bytes);

    let manifest = json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {
            "mediaType": "application/vnd.oci.empty.v1+json",
            "digest": config_digest,
            "size": config_bytes.len(),
        },
        "layers": [
            {
                "mediaType": "application/vnd.goodfetch.clean-up-rules.full.v1+tar+xz",
                "digest": full_digest,
                "size": set.full_bytes.len(),
                "annotations": { "dev.goodfetch.clean-up-rules.part": "full" },
            },
            {
                "mediaType": "application/vnd.goodfetch.clean-up-rules.embedded.v1+text",
                "digest": embedded_digest,
                "size": set.embedded_bytes.len(),
                "annotations": { "dev.goodfetch.clean-up-rules.part": "embedded" },
            }
        ],
        "annotations": {
            "dev.goodfetch.clean-up-rules.line": set.line,
            "dev.goodfetch.clean-up-rules.version": set.version,
            "dev.goodfetch.clean-up-rules.gate": set.gate,
        }
    });
    let manifest_bytes = serde_json::to_vec(&manifest).expect("serialize manifest");
    let put = agent
        .put(&format!("{registry}/v2/{repository}/manifests/{}", set.tag))
        .header("Content-Type", "application/vnd.oci.image.manifest.v1+json")
        .send(&manifest_bytes)
        .expect("put manifest");
    assert!(
        put.status() == 201 || put.status() == 200,
        "manifest upload failed with {}",
        put.status()
    );
}

#[cfg(target_os = "linux")]
fn seed_registry_daily_sets(url: &str, repo: &str) {
    let fixtures = [
        DailySetFixture {
            tag: "daily-1.0-20261125-1",
            line: "1.0",
            version: "2026.11.25.1",
            gate: "passed",
            full_bytes: b"full-1.0-older",
            embedded_bytes: b"||old.example^\n",
        },
        DailySetFixture {
            tag: "daily-1.0-20261126-1",
            line: "1.0",
            version: "2026.11.26.1",
            gate: "passed",
            full_bytes: b"full-1.0-winner",
            embedded_bytes: b"||winner-1.0.example^\n",
        },
        DailySetFixture {
            tag: "daily-1.0-20261127-1",
            line: "1.0",
            version: "2026.11.27.1",
            gate: "failed",
            full_bytes: b"full-1.0-broken",
            embedded_bytes: b"||broken-1.0.example^\n",
        },
        DailySetFixture {
            tag: "daily-2.0-20261127-9",
            line: "2.0",
            version: "2026.11.27.9",
            gate: "passed",
            full_bytes: b"full-2.0-newest",
            embedded_bytes: b"||line-2.0.example^\n",
        },
        DailySetFixture {
            tag: "daily-main-20261126-3",
            line: "main",
            version: "2026.11.26.3",
            gate: "passed",
            full_bytes: b"full-main-winner",
            embedded_bytes: b"||main-winner.example^\n",
        },
        DailySetFixture {
            tag: "daily-main-20261127-4",
            line: "main",
            version: "2026.11.27.4",
            gate: "failed",
            full_bytes: b"full-main-broken",
            embedded_bytes: b"||main-broken.example^\n",
        },
    ];
    for set in &fixtures {
        push_daily_set(url, repo, set);
    }
}

/// Covers: story:1-26
/// Against a real `registry:2` stand-in holding daily sets for two release
/// lines (`1.0`, `2.0`) and `main`, some marked as failing their gate:
/// - a branch cut or patch on `release/1.0` pins the newest passing set of
///   `1.0`, never `2.0`'s newer set and never `1.0`'s failing set;
/// - pinning to a file on `main` is refused (never writes to `main`);
/// - the nightly pin resolves `main`'s newest passing set at build time
///   without modifying `main`.
#[cfg(target_os = "linux")]
#[test]
fn registry_2_daily_channel_pins_newest_passing_set_of_own_line_and_never_writes_to_main() {
    use crate::container::Container;

    const REGISTRY_PIN: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ci/registry/image.toml"
    ));
    let registry = Container::start(REGISTRY_PIN, 5000, "/v2/", &[]);
    let url = registry.url();
    seed_registry_daily_sets(url, "goodfetch/clean-up-rules");

    let workspace = Workspace::from_fixture("empty");
    workspace.commit();
    assert!(
        Command::new("git")
            .args(["checkout", "-B", "main"])
            .current_dir(workspace.path())
            .status()
            .expect("checkout main")
            .success()
    );

    // 1. Writing a pin file while checked out on `main` is refused.
    let on_main = workspace.xtask(&[
        "release",
        "pin-clean-up-rules",
        "--line",
        "release/1.0",
        "--registry",
        url,
        "--pin-file",
        "clean-up-rules-pin.json",
    ]);
    assert_ne!(
        on_main.code, 0,
        "pinning to a file while on `main` must be refused"
    );

    // 2. Nightly resolution for `main` succeeds without writing to `main`.
    let nightly = workspace.xtask(&[
        "release",
        "pin-clean-up-rules",
        "--line",
        "main",
        "--registry",
        url,
        "--out-assets",
        "nightly-assets",
    ]);
    assert_eq!(nightly.code, 0, "{}", nightly.stderr);
    let nightly_json: Value = serde_json::from_str(nightly.stdout.trim()).expect("parse summary");
    assert_eq!(nightly_json["version"], "2026.11.26.3");
    assert_eq!(
        workspace.read_text("nightly-assets/goodfetch-clean-up-rules-embedded.txt"),
        "||main-winner.example^\n"
    );

    // 3. Switch to `release/1.0`: pins 2026.11.26.1 (skipping failing 2026.11.27.1 and 2.0).
    let checkout = Command::new("git")
        .args(["checkout", "-b", "release/1.0"])
        .current_dir(workspace.path())
        .status()
        .expect("git checkout release/1.0");
    assert!(checkout.success());

    let pinned = workspace.xtask(&[
        "release",
        "pin-clean-up-rules",
        "--line",
        "release/1.0",
        "--registry",
        url,
        "--pin-file",
        "clean-up-rules-pin.json",
        "--out-assets",
        "release-assets",
    ]);
    assert_eq!(pinned.code, 0, "{}", pinned.stderr);
    let pin: Value =
        serde_json::from_str(&workspace.read_text("clean-up-rules-pin.json")).expect("parse pin");
    assert_eq!(pin["line"], "1.0");
    assert_eq!(pin["version"], "2026.11.26.1");
    assert_eq!(
        workspace.read_text("release-assets/goodfetch-clean-up-rules-embedded.txt"),
        "||winner-1.0.example^\n"
    );
    let notes = workspace.read_text("release-assets/clean-up-rules-notes.md");
    assert!(
        notes.contains("2026.11.26.1")
            && notes.contains("goodfetch-clean-up-rules-full.tar.xz")
            && notes.contains("goodfetch-clean-up-rules-embedded.txt"),
        "release notes snippet missing version or asset digests:\n{notes}"
    );
}

/// Covers: story:1-79
/// Until the daily channel (#208) publishes, `pin-clean-up-rules` falls back
/// to the committed seed set, `goodfetch-clean-up-rules/build.rs` performs zero
/// network downloads, and `release-nightly.yml` resolves Clean-up rules before
/// attestation and includes their digests in pre-release notes.
#[test]
fn seed_fallback_and_nightly_attestation_wiring_work_offline() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        "crates/goodfetch-clean-up-rules/rules/manifest.json",
        br#"{"format_version":"0.1","source":"embedded","published_at":"2026-10-22T23:42:37Z","lists":{"ads":"easylist.txt"}}"#,
    );
    workspace.write(
        "crates/goodfetch-clean-up-rules/rules/easylist.txt",
        b"||ads.example^\n",
    );

    let outcome = workspace.xtask(&[
        "release",
        "pin-clean-up-rules",
        "--line",
        "main",
        "--out-assets",
        "seed-assets",
    ]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let summary: Value = serde_json::from_str(outcome.stdout.trim()).expect("parse seed summary");
    assert_eq!(summary["source"], "committed-seed");
    assert!(
        workspace
            .read_text("seed-assets/goodfetch-clean-up-rules-embedded.txt")
            .contains("||ads.example^")
    );

    // Build script never downloads anything over the network.
    let build_rs =
        std::fs::read_to_string(repo_root().join("crates/goodfetch-clean-up-rules/build.rs"))
            .expect("read build.rs");
    for forbidden in ["http://", "https://", "ureq", "reqwest", "curl"] {
        assert!(
            !build_rs.contains(forbidden),
            "build.rs must never download anything ({forbidden})"
        );
    }

    // Nightly workflow resolves Clean-up rules before `attest-build-provenance`
    // and includes `clean-up-rules-notes.md` in `--notes-file`.
    let nightly =
        std::fs::read_to_string(repo_root().join(".github/workflows/release-nightly.yml"))
            .expect("read release-nightly.yml");
    let pin_pos = nightly
        .find("pin-clean-up-rules")
        .expect("pin-clean-up-rules in nightly");
    let attest_pos = nightly
        .find("actions/attest-build-provenance")
        .expect("attest-build-provenance in nightly");
    assert!(
        pin_pos < attest_pos,
        "Clean-up rules assets must be staged before attestation"
    );
    assert!(
        nightly.contains("clean-up-rules-notes.md"),
        "nightly pre-release notes must include Clean-up rules version and digests"
    );
}
