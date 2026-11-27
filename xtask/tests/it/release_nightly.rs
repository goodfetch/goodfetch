//! Nightly channel release pipeline and cross-channel release gate (spec #130,
//! issue 143; ADRs 0030, 0032, 0039, 0057, 0061, 0069 and 0094).
//! Covers: story:1-8, story:1-9, story:1-10, story:1-73, story:1-81

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;

use crate::release_assets::TARGETS;
use crate::workspace::{Workspace, finding};

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}

fn read_nightly_workflow() -> Value {
    let text =
        std::fs::read_to_string(root().join(".github/workflows/release-nightly.yml")).unwrap();
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

fn exe_suffix(target: &str) -> &'static str {
    if target.contains("-windows-") {
        ".exe"
    } else {
        ""
    }
}

fn debug_name(executable: &str, target: &str) -> String {
    if target.contains("-windows-") {
        format!("{}.pdb", executable.replace('-', "_"))
    } else if target.contains("-apple-") {
        format!("{executable}.dSYM")
    } else {
        format!("{executable}.dwp")
    }
}

fn packaged_nightly_workspace() -> Workspace {
    let workspace = Workspace::from_fixture("release");
    workspace.write(
        "crates/goodfetch-protocol/generated/goodfetch-catalog.json",
        include_bytes!("../../../crates/goodfetch-protocol/generated/goodfetch-catalog.json"),
    );
    workspace.write(
        "sdks/python/collie-client/src/goodfetch_collie_client/__init__.py",
        b"__all__ = []\n",
    );
    workspace.write(
        "sdks/python/collie/src/goodfetch_collie/__init__.py",
        b"__all__ = []\n",
    );
    workspace.write(
        "sdks/python/foxhound/src/goodfetch_foxhound/__init__.py",
        b"__all__ = []\n",
    );
    for target in TARGETS {
        let suffix = exe_suffix(target);
        for executable in ["goodfetch-collie", "goodfetch-foxhound"] {
            workspace.write(
                &format!("bins/{target}/{executable}{suffix}"),
                format!("nightly-bin:{executable}:{target}").as_bytes(),
            );
            workspace.write(
                &format!("bins/{target}/{}", debug_name(executable, target)),
                b"debug-symbols",
            );
        }
    }
    let pkg = workspace.xtask(&[
        "release",
        "package",
        "--version",
        "0.0.0-dev.1",
        "--binaries",
        "bins",
        "--out",
        "out",
    ]);
    assert_eq!(pkg.code, 0, "{}", pkg.stderr);
    let assets = workspace.xtask(&["release", "assets", "--folder", "out/assets"]);
    assert_eq!(assets.code, 0, "{}", assets.stderr);
    workspace
}

#[test]
fn channel_gate_verifies_same_digest_across_channels_and_blocks_divergence_or_forbidden_channel() {
    let workspace = packaged_nightly_workspace();

    // Sign and attest every asset listed in SHA256SUMS for offline verification.
    let sums = workspace.read_text("out/assets/SHA256SUMS");
    for line in sums.lines() {
        let Some((_, file)) = line.split_once("  ") else {
            continue;
        };
        let signed = workspace.xtask(&[
            "release",
            "sign-rehearsal",
            "--file",
            &format!("out/assets/{file}"),
            "--key-dir",
            "keys",
        ]);
        assert_eq!(signed.code, 0, "signing {file}: {}", signed.stderr);
    }

    // 1. Clean nightly with matching bytes across archive, npm, and wheels passes.
    let ok = workspace.xtask(&[
        "release",
        "channel-gate",
        "--version",
        "0.0.0-dev.1",
        "--binaries",
        "bins",
        "--packaged",
        "out",
        "--channels",
        "npm:nightly,ghcr:nightly,plugins-oci,github-prerelease",
        "--verify-attestations",
        "--public-key",
        "keys/rehearsal.pub",
    ]);
    assert_eq!(ok.code, 0, "{}\n{}", ok.stdout, ok.stderr);

    // 2. Forbidden nightly channel (PyPI, crates.io, Homebrew) is refused.
    let forbidden = workspace.xtask(&[
        "release",
        "channel-gate",
        "--version",
        "0.0.0-dev.1",
        "--binaries",
        "bins",
        "--packaged",
        "out",
        "--channels",
        "npm:nightly,pypi,crates.io,homebrew",
    ]);
    assert_ne!(forbidden.code, 0);
    let findings = forbidden.findings();
    assert!(findings.contains(&finding("forbidden-nightly-channel", "pypi")));
    assert!(findings.contains(&finding("forbidden-nightly-channel", "crates.io")));
    assert!(findings.contains(&finding("forbidden-nightly-channel", "homebrew")));

    // 3. Altering one byte in one channel (npm platform binary) trips the gate.
    workspace.write(
        "out/npm/collie-linux-x64/bin/goodfetch-collie",
        b"tampered-bytes",
    );
    let mismatched = workspace.xtask(&[
        "release",
        "channel-gate",
        "--version",
        "0.0.0-dev.1",
        "--binaries",
        "bins",
        "--packaged",
        "out",
        "--channels",
        "npm:nightly,ghcr:nightly,plugins-oci,github-prerelease",
    ]);
    assert_ne!(mismatched.code, 0);
    assert!(mismatched.findings().contains(&finding(
        "channel-digest-mismatch",
        "npm/collie-linux-x64/bin/goodfetch-collie"
    )));
}

#[test]
fn release_nightly_workflow_enforces_fixed_order_oidc_and_channel_exclusions() {
    let workflow = read_nightly_workflow();
    let jobs = workflow["jobs"].as_object().unwrap();

    // 1. Release suite runs on main's head strictly before build.
    assert!(jobs.contains_key("release-suite"));
    assert!(needs_of(&jobs["build"]).contains("release-suite"));
    assert!(needs_of(&jobs["attest-and-sign"]).contains("build"));

    // 2. Per-byte verification runs after attest-and-sign and before release-gate.
    for job in [
        "verify-bytes-linux",
        "verify-bytes-windows",
        "verify-bytes-macos",
    ] {
        assert!(needs_of(&jobs[job]).contains("attest-and-sign"));
        assert!(needs_of(&jobs["release-gate"]).contains(job));
    }

    // 3. Publish jobs depend on release-gate; smoke installs run after publish.
    for pub_job in ["publish-npm", "publish-oci", "publish-github-prerelease"] {
        assert!(needs_of(&jobs[pub_job]).contains("release-gate"));
        for smoke in [
            "smoke-channels-linux",
            "smoke-channels-windows",
            "smoke-channels-macos",
        ] {
            assert!(needs_of(&jobs[smoke]).contains(pub_job));
        }
    }

    // 4. Nightly publishes with OIDC provenance to npm nightly, GHCR nightly,
    //    Plugin OCI and GitHub pre-release (with .devN wheels); NEVER PyPI,
    //    crates.io or Homebrew.
    let raw =
        std::fs::read_to_string(root().join(".github/workflows/release-nightly.yml")).unwrap();
    assert!(raw.contains("--provenance"));
    assert!(raw.contains("--tag nightly"));
    assert!(raw.contains("--prerelease"));
    assert!(raw.contains("OCI_REGISTRY: ghcr.io"));
    assert!(raw.contains("/goodfetch/${product}:nightly"));
    assert!(raw.contains("actions/attest-build-provenance@"));
    assert!(!raw.contains("cargo publish"));
    assert!(!raw.contains("twine upload"));
    assert!(!raw.contains("uv publish"));
    assert!(!raw.contains("homebrew-tap"));

    // 5. verify-bytes.sh checks --version, hello handshake, wheels and npm.
    let verify_script = std::fs::read_to_string(root().join("ci/release/verify-bytes.sh")).unwrap();
    assert!(verify_script.contains("\"method\":\"hello\""));
    assert!(verify_script.contains("goodfetch-collie"));
    assert!(verify_script.contains("goodfetch-foxhound"));
}
