//! `xtask release proving-ground-digests` and `xtask release fetch-chromium`:
//! fixing the Proving ground's digest before the executables build, and
//! fetching the Goodfetch Chromium by manifest digest after verifying its
//! build and verdict attestations against the commit's input tree hashes
//! (spec #130, issue 141; ADRs 0033, 0039 and 0074).

use std::io::{Read as _, Write as _};
use std::thread;

use serde_json::Value;

use crate::release_assets::{TARGETS, sha256};
use crate::workspace::{Workspace, finding};

fn binary_name(target: &str) -> &'static str {
    if target.contains("-windows-") {
        "goodfetch-proving-ground.exe"
    } else {
        "goodfetch-proving-ground"
    }
}

/// Serves fixed `(path, bytes)` responses over HTTP on a free loopback port,
/// stopping after `expected` requests.
#[expect(
    clippy::disallowed_types,
    reason = "local HTTP stand-in for testing artifact fetch in xtask tests (ADR 0060)"
)]
fn serve_http(files: Vec<(String, Vec<u8>)>, expected: usize) -> (String, thread::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        for _ in 0..expected {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let mut buffer = [0_u8; 4096];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]);
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/");
            if let Some((_, body)) = files.iter().find(|(candidate, _)| candidate == path) {
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(body);
            } else {
                let _ = stream.write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        }
    });
    (url, handle)
}

#[test]
fn proving_ground_digests_records_every_target_before_executables_build() {
    let workspace = Workspace::from_fixture("release");
    for target in TARGETS {
        let content = format!("proving-ground-bytes-{target}");
        workspace.write(
            &format!("pg-bins/{target}/{}", binary_name(target)),
            content.as_bytes(),
        );
    }

    let outcome = workspace.xtask(&[
        "release",
        "proving-ground-digests",
        "--binaries",
        "pg-bins",
        "--out",
        "out/proving-ground-digests.json",
    ]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let document: Value =
        serde_json::from_str(&workspace.read_text("out/proving-ground-digests.json")).unwrap();
    let targets = document["targets"].as_object().unwrap();
    assert_eq!(targets.len(), TARGETS.len());
    for target in TARGETS {
        let expected = sha256(format!("proving-ground-bytes-{target}").as_bytes());
        assert_eq!(
            targets[target]["binary"].as_str(),
            Some(binary_name(target))
        );
        assert_eq!(targets[target]["sha256"].as_str(), Some(expected.as_str()));
    }
}

#[test]
fn proving_ground_digests_refuses_when_a_target_binary_is_missing() {
    let workspace = Workspace::from_fixture("release");
    for target in &TARGETS[..TARGETS.len() - 1] {
        workspace.write(
            &format!("pg-bins/{target}/{}", binary_name(target)),
            b"binary",
        );
    }

    let outcome = workspace.xtask(&[
        "release",
        "proving-ground-digests",
        "--binaries",
        "pg-bins",
        "--out",
        "out/proving-ground-digests.json",
    ]);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(
        outcome.findings(),
        [finding("missing", "aarch64-pc-windows-msvc")]
    );
    assert_eq!(workspace.read("out/proving-ground-digests.json"), None);
}

#[test]
fn fetch_chromium_passes_without_building_when_no_manifest_is_committed() {
    let workspace = Workspace::from_fixture("release");

    let outcome = workspace.xtask(&["release", "fetch-chromium", "--out", "dist"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(
        outcome.stdout.contains("no Goodfetch Chromium"),
        "stdout: {}",
        outcome.stdout
    );
}

#[test]
fn fetch_chromium_verifies_tree_hashes_attestations_and_digests_before_saving() {
    let workspace = Workspace::from_fixture("release");
    workspace.write("chromium/version.txt", b"156.0.8078.4\n");
    workspace.write("chromium/patches/001-stealth.patch", b"patch-bytes\n");
    workspace.write("chromium/expectations/linux.toml", b"pass = true\n");
    workspace.write(
        ".github/workflows/chromium-gate.yml",
        b"name: chromium-gate\n",
    );

    let hashes = workspace.xtask(&["release", "chromium-tree-hashes"]);
    assert_eq!(hashes.code, 0, "{}", hashes.stderr);
    let tree: Value = serde_json::from_str(&hashes.stdout).unwrap();
    let build_tree = tree["build_input_tree"].as_str().unwrap().to_owned();
    let test_tree = tree["test_input_tree"].as_str().unwrap().to_owned();

    let bundle_bytes = b"goodfetch-chromium-bundle-bytes".to_vec();
    let driver_bytes = b"goodfetch-chromedriver-bytes".to_vec();
    let bundle_sha = sha256(&bundle_bytes);
    let driver_sha = sha256(&driver_bytes);
    let (base_url, server) = serve_http(
        vec![
            ("/bundle.tar.xz".to_owned(), bundle_bytes.clone()),
            ("/chromedriver.tar.xz".to_owned(), driver_bytes.clone()),
        ],
        2,
    );

    let manifest = format!(
        r#"workflow = ".github/workflows/chromium-gate.yml"
build_input_tree = "{build_tree}"
test_input_tree = "{test_tree}"

[[artifact]]
target = "x86_64-unknown-linux-gnu"
kind = "bundle"
filename = "goodfetch-chromium-x86_64-unknown-linux-gnu.tar.xz"
url = "{base_url}/bundle.tar.xz"
sha256 = "{bundle_sha}"

[[artifact]]
target = "x86_64-unknown-linux-gnu"
kind = "chromedriver"
filename = "goodfetch-chromedriver-x86_64-unknown-linux-gnu.tar.xz"
url = "{base_url}/chromedriver.tar.xz"
sha256 = "{driver_sha}"
"#
    );
    workspace.write("chromium/manifest.toml", manifest.as_bytes());

    for digest in [&bundle_sha, &driver_sha] {
        let build_att = serde_json::json!({
            "subject_sha256": digest,
            "workflow": ".github/workflows/chromium-gate.yml",
            "build_input_tree": build_tree,
        });
        let verdict_att = serde_json::json!({
            "subject_sha256": digest,
            "workflow": ".github/workflows/chromium-gate.yml",
            "build_input_tree": build_tree,
            "test_input_tree": test_tree,
            "verdict": "pass",
        });
        workspace.write(
            &format!("attestations/{digest}.build.json"),
            build_att.to_string().as_bytes(),
        );
        workspace.write(
            &format!("attestations/{digest}.verdict.json"),
            verdict_att.to_string().as_bytes(),
        );
    }

    let outcome = workspace.xtask(&[
        "release",
        "fetch-chromium",
        "--attestations",
        "attestations",
        "--out",
        "dist",
    ]);
    server.join().unwrap();

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        workspace.read("dist/goodfetch-chromium-x86_64-unknown-linux-gnu.tar.xz"),
        Some(bundle_bytes)
    );
    assert_eq!(
        workspace.read("dist/goodfetch-chromedriver-x86_64-unknown-linux-gnu.tar.xz"),
        Some(driver_bytes)
    );
}

#[test]
fn fetch_chromium_refuses_input_tree_mismatch() {
    let workspace = Workspace::from_fixture("release");
    workspace.write("chromium/version.txt", b"156.0.8078.4\n");
    let hashes = workspace.xtask(&["release", "chromium-tree-hashes"]);
    assert_eq!(hashes.code, 0, "{}", hashes.stderr);
    let tree: Value = serde_json::from_str(&hashes.stdout).unwrap();
    let test_tree = tree["test_input_tree"].as_str().unwrap().to_owned();

    let stale_manifest = format!(
        r#"workflow = ".github/workflows/chromium-gate.yml"
build_input_tree = "0000000000000000000000000000000000000000000000000000000000000000"
test_input_tree = "{test_tree}"

[[artifact]]
target = "x86_64-unknown-linux-gnu"
kind = "bundle"
filename = "goodfetch-chromium-x86_64-unknown-linux-gnu.tar.xz"
url = "http://127.0.0.1:1/unused"
sha256 = "1111111111111111111111111111111111111111111111111111111111111111"
"#
    );
    workspace.write("chromium/manifest.toml", stale_manifest.as_bytes());
    let mismatch = workspace.xtask(&[
        "release",
        "fetch-chromium",
        "--attestations",
        "attestations",
        "--out",
        "dist",
    ]);
    assert_eq!(mismatch.code, 1);
    assert_eq!(
        mismatch.findings(),
        [finding("input-tree-mismatch", "chromium/manifest.toml")]
    );
}

#[test]
fn fetch_chromium_refuses_bad_attestation_and_tampered_bytes() {
    let workspace = Workspace::from_fixture("release");
    workspace.write("chromium/version.txt", b"156.0.8078.4\n");
    let hashes = workspace.xtask(&["release", "chromium-tree-hashes"]);
    assert_eq!(hashes.code, 0, "{}", hashes.stderr);
    let tree: Value = serde_json::from_str(&hashes.stdout).unwrap();
    let build_tree = tree["build_input_tree"].as_str().unwrap().to_owned();
    let test_tree = tree["test_input_tree"].as_str().unwrap().to_owned();

    let good_bytes = b"expected-bytes".to_vec();
    let good_sha = sha256(&good_bytes);
    let (base_url, server) =
        serve_http(vec![("/bundle.tar.xz".to_owned(), b"tampered".to_vec())], 1);
    let manifest = format!(
        r#"workflow = ".github/workflows/chromium-gate.yml"
build_input_tree = "{build_tree}"
test_input_tree = "{test_tree}"

[[artifact]]
target = "x86_64-unknown-linux-gnu"
kind = "bundle"
filename = "goodfetch-chromium-x86_64-unknown-linux-gnu.tar.xz"
url = "{base_url}/bundle.tar.xz"
sha256 = "{good_sha}"
"#
    );
    workspace.write("chromium/manifest.toml", manifest.as_bytes());
    workspace.write(
        &format!("attestations/{good_sha}.build.json"),
        serde_json::json!({
            "subject_sha256": good_sha,
            "workflow": ".github/workflows/chromium-gate.yml",
            "build_input_tree": build_tree,
        })
        .to_string()
        .as_bytes(),
    );
    workspace.write(
        &format!("attestations/{good_sha}.verdict.json"),
        serde_json::json!({
            "subject_sha256": good_sha,
            "workflow": ".github/workflows/chromium-gate.yml",
            "build_input_tree": build_tree,
            "test_input_tree": test_tree,
            "verdict": "fail",
        })
        .to_string()
        .as_bytes(),
    );
    let bad_verdict = workspace.xtask(&[
        "release",
        "fetch-chromium",
        "--attestations",
        "attestations",
        "--out",
        "dist",
    ]);
    assert_eq!(bad_verdict.code, 1);
    assert_eq!(
        bad_verdict.findings(),
        [finding(
            "attestation-invalid",
            "goodfetch-chromium-x86_64-unknown-linux-gnu.tar.xz"
        )]
    );

    workspace.write(
        &format!("attestations/{good_sha}.verdict.json"),
        serde_json::json!({
            "subject_sha256": good_sha,
            "workflow": ".github/workflows/chromium-gate.yml",
            "build_input_tree": build_tree,
            "test_input_tree": test_tree,
            "verdict": "pass",
        })
        .to_string()
        .as_bytes(),
    );
    let tampered = workspace.xtask(&[
        "release",
        "fetch-chromium",
        "--attestations",
        "attestations",
        "--out",
        "dist",
    ]);
    server.join().unwrap();
    assert_eq!(tampered.code, 1);
    assert_eq!(
        tampered.findings(),
        [finding(
            "digest-mismatch",
            "goodfetch-chromium-x86_64-unknown-linux-gnu.tar.xz"
        )]
    );
    assert_eq!(
        workspace.read("dist/goodfetch-chromium-x86_64-unknown-linux-gnu.tar.xz"),
        None
    );
}
