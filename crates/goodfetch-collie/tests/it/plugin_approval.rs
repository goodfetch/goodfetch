//! `plugin add`'s approval through Collie's executable (ADR 0033, issue
//! 900): once the Plugin is installed and pinned, the person sees each
//! permission its manifest declares with its limits, printed once, and a
//! yes approves the Plugin and those permissions in the Operator policy;
//! a no leaves the policy as it was. Under `--json` the command is refused
//! with `person_required`, exit 4, before any engine call. The Plugin is in
//! the store and pinned already, so no registry is reached.
#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]

use goodfetch_engine::plugin_oci::{
    INDEX_MEDIA_TYPE, MANIFEST_MEDIA_TYPE, Platform, TAR_GZIP_MEDIA_TYPE,
};
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{TestEnv, cli, pty};
use rstest::rstest;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::COLLIE;
use crate::clean_up_rules::stand_in::StandIn;

/// A visibly fake image index digest.
const DIGEST: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

/// The Plugin's OCI reference, pinned by [`DIGEST`].
fn reference() -> String {
    format!("registry.example.com/goodfetch-plugin-kettle@{DIGEST}")
}

/// Places the Plugin `kettle` in the store of `env` with `manifest`, added
/// at [`DIGEST`], and pins it in the lockfile beside the project
/// configuration of `env`'s folder.
fn place(env: &TestEnv, manifest: &Value) {
    let plugin = env.home().join("plugins").join("kettle");
    let bundle = plugin.join("1.0.0");
    std::fs::create_dir_all(&bundle).unwrap();
    std::fs::write(bundle.join("goodfetch-plugin.json"), manifest.to_string()).unwrap();
    std::fs::write(plugin.join("1.0.0.digest"), DIGEST).unwrap();
    std::fs::write(env.folder().join("goodfetch.toml"), "version = \"0.1\"\n").unwrap();
    std::fs::write(
        env.folder().join("goodfetch-plugins.lock"),
        format!(
            "version = \"0.1\"\n\n[[plugin]]\nname = \"kettle\"\nversion = \"1.0.0\"\ndigest = \"{DIGEST}\"\n"
        ),
    )
    .unwrap();
}

/// Places the Plugin `kettle` in the store of `env`, added at [`DIGEST`],
/// declaring Effects on `shop.test` and the value of `env:TOKEN`, and pins
/// it in the lockfile beside the project configuration of `env`'s folder.
fn placed(env: &TestEnv) {
    place(
        env,
        &json!({
            "manifest_version": "0.1",
            "name": "kettle",
            "version": "1.0.0",
            "kinds": ["notification"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "kettle",
            "network_level": "none",
            "permissions": {
                "domains": ["shop.test"],
                "secrets": ["env:TOKEN"],
                "grants": ["effects", "secret_value"],
            },
        }),
    );
}

/// The person installs the Plugin, sees its three permissions once, each
/// with what it lets the Plugin do and what the engine cannot check, and
/// answers the approval: a yes writes it in the Operator policy, a no
/// writes nothing.
///
/// Covers: story:7-18, story:7-20, command:plugin.add
#[rstest]
#[case::approved("y", true)]
#[case::declined("n", false)]
fn plugin_add_states_the_permissions_once_and_asks_for_the_approval(
    #[case] answer: &str,
    #[case] approved: bool,
) {
    let env = TestEnv::new();
    placed(&env);
    let policy = HomeLayout::of(env.home()).operator_policy();

    let mut added = pty(&env, COLLIE)
        .args(["plugin", "add", &reference()])
        .spawn()
        .unwrap();
    added.expect("[y/N]").unwrap();
    added.send_line("y").unwrap();
    added.expect("in the Operator policy? [y/N]").unwrap();
    added.send_line(answer).unwrap();
    let output = added.finish().unwrap();

    let shown = output.stripped();
    assert_eq!(output.exit, Some(0), "{shown}");
    assert_eq!(
        (
            shown.matches("The engine cannot check").count(),
            shown.contains("receive the value of env:TOKEN itself"),
        ),
        (3, true),
        "{shown}"
    );
    let written = std::fs::read_to_string(&policy).unwrap_or_default();
    assert_eq!(
        (
            written.contains("allow = true"),
            written.contains(DIGEST),
            shown.contains("Approved the Plugin kettle"),
        ),
        (approved, approved, approved),
        "{written}\n{shown}"
    );
}

/// Under `--json` no person answers: the command is refused with
/// `person_required`, exit 4, its fix naming the interactive command, and
/// nothing is installed or approved.
///
/// Covers: story:7-20, command:plugin.add
#[test]
fn plugin_add_under_json_is_refused_for_want_of_a_person() {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE)
        .args(["plugin", "add", &reference(), "--json"])
        .run()
        .unwrap();

    let failed: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(
        (
            output.exit,
            &failed["error"]["code"],
            failed["error"]["fix"]
                .as_str()
                .is_some_and(|fix| fix.contains("plugin add")),
        ),
        (Some(4), &json!("person_required"), true),
        "{output:?}"
    );
    assert!(!HomeLayout::of(env.home()).operator_policy().exists());
}

/// Story 7-3 & 7-19: before any approval is written, `plugin add` prints every
/// field the person grants: kinds, network level, permissions, environment
/// variables, files, processes, browsers, and declared spend units (each with
/// its maximum per call and its price, or `unbounded` when unpriced), both for
/// a toy Plugin with none of the optional ones and for Plugins declaring each.
/// The Operator policy stays absent until the person confirms.
///
/// Covers: story:7-3, story:7-19, command:plugin.add
#[rstest]
#[case::toy(
    json!({
        "manifest_version": "0.1",
        "name": "kettle",
        "version": "1.0.0",
        "kinds": ["extract"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "kettle",
        "network_level": "none",
    }),
    &[
        "Kinds: extract",
        "Network level: none",
        "Environment variables: none",
        "Files: none",
        "Processes: none",
        "Browsers: none",
        "Spend: none",
        "Permissions: none",
    ],
)]
#[case::with_spend(
    json!({
        "manifest_version": "0.1",
        "name": "kettle",
        "version": "1.0.0",
        "kinds": ["extract"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "kettle",
        "network_level": "none",
        "spend": {
            "solves": { "max_per_call": 3, "price": "0.002 USD" },
            "extra": { "max_per_call": 5 },
        },
    }),
    &[
        "Kinds: extract",
        "Spend:",
        "solves: max 3 per call, price: 0.002 USD",
        "extra: max 5 per call, price: unbounded",
    ],
)]
#[case::with_browsers(
    json!({
        "manifest_version": "0.1",
        "name": "kettle",
        "version": "1.0.0",
        "kinds": ["browser"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "kettle",
        "network_level": "proxy_tcp",
        "env": { "KETTLE_ENDPOINT": "endpoint" },
        "files": ["/etc/kettle/ca.pem"],
        "processes": ["/usr/bin/kettle-helper"],
        "browsers": [{
            "name": "kettle-browser",
            "platforms": {
                "linux-x64": {
                    "url": "https://downloads.example.com/kettle-browser-linux-x64.tar.gz",
                    "digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
                },
            },
        }],
        "permissions": {
            "domains": ["api.example.com"],
            "grants": ["effects"],
        },
    }),
    &[
        "Kinds: browser",
        "Network level: proxy_tcp",
        "Environment variables: KETTLE_ENDPOINT <- endpoint",
        "Files: /etc/kettle/ca.pem",
        "Processes: /usr/bin/kettle-helper",
        "kettle-browser",
        "linux-x64",
        "sha256:1111111111111111111111111111111111111111111111111111111111111111",
        "cause Effects on api.example.com",
    ],
)]
fn plugin_add_shows_everything_granted_before_writing_the_approval(
    #[case] manifest: Value,
    #[case] expected: &[&str],
) {
    let env = TestEnv::new();
    place(&env, &manifest);
    let policy = HomeLayout::of(env.home()).operator_policy();

    let mut added = pty(&env, COLLIE)
        .args(["plugin", "add", &reference()])
        .spawn()
        .unwrap();
    added.expect("[y/N]").unwrap();
    added.send_line("y").unwrap();
    added.expect("in the Operator policy? [y/N]").unwrap();
    let written_before = policy.exists();
    added.send_line("y").unwrap();
    let output = added.finish().unwrap();

    let shown = output.stripped();
    let missing: Vec<&&str> = expected
        .iter()
        .filter(|needle| !shown.contains(**needle))
        .collect();
    assert_eq!(
        (output.exit, written_before, policy.exists(), missing),
        (Some(0), false, true, Vec::<&&str>::new()),
        "{shown}"
    );
}

/// `plugin search --json` prints kinds, signer identity and conformance
/// results from the cached Plugin index, and `plugin index import --json` is
/// refused with `person_required` (exit 4) naming `plugin index import`.
///
/// Covers: story:7-1, story:7-9, command:plugin.search
// Covers: operation:plugin.index_import
#[test]
fn plugin_search_lists_index_and_index_import_requires_a_person() {
    let env = TestEnv::new();
    let index_dir = env.home().join("plugin-index");
    std::fs::create_dir_all(&index_dir).unwrap();
    let fixture = json!({
        "payload": {
            "version": 3,
            "signed_at": "2026-11-27T00:00:00Z",
            "expires_at": "2099-12-31T00:00:00Z",
            "plugins": [{
                "name": "kettle",
                "reference": "ghcr.io/vendor/goodfetch-plugin-kettle",
                "digest": DIGEST,
                "kinds": ["extract"],
                "signer": {
                    "issuer": "https://token.actions.githubusercontent.com",
                    "workflow": "https://github.com/vendor/kettle/.github/workflows/release.yml@refs/tags/v1.0.0",
                    "repository": "github.com/vendor/kettle",
                },
                "origin": "third_party",
                "conformance": [{
                    "suite": "extract",
                    "passed": true,
                    "engine_version": "0.1.0",
                }],
            }],
            "revocations": [],
        },
        "bundle": {
            "key_id": "test-key",
            "issuer": "https://token.actions.githubusercontent.com",
            "workflow": "https://github.com/goodfetch/goodfetch/.github/workflows/plugin-index.yml@refs/heads/main",
            "signature": "AAAA",
        },
    });
    std::fs::write(index_dir.join("index.json"), fixture.to_string()).unwrap();

    // `plugin search --json` lists kinds, signer and conformance results.
    let search = cli(&env, COLLIE)
        .args(["plugin", "search", "--json"])
        .run()
        .unwrap();
    assert_eq!(search.exit, Some(0), "stderr: {}", search.stderr);
    let body: Value = serde_json::from_str(&search.stdout).unwrap();
    assert_eq!(body["index_version"], json!(3));
    assert_eq!(body["plugins"][0]["name"], json!("kettle"));
    assert_eq!(body["plugins"][0]["kinds"], json!(["extract"]));
    assert_eq!(
        body["plugins"][0]["signer"]["repository"],
        json!("github.com/vendor/kettle")
    );
    assert_eq!(body["plugins"][0]["conformance"][0]["passed"], json!(true));

    // `plugin index import <file> --json` refuses with `person_required`, exit 4.
    let index_file = env.folder().join("signed-index.json");
    std::fs::write(&index_file, fixture.to_string()).unwrap();
    let import = cli(&env, COLLIE)
        .args([
            "plugin",
            "index",
            "import",
            index_file.to_str().unwrap(),
            "--json",
        ])
        .run()
        .unwrap();
    assert_eq!(import.exit, Some(4));
    let failed: Value = serde_json::from_str(&import.stdout).unwrap();
    assert_eq!(failed["error"]["code"], json!("person_required"));
    let fix = failed["error"]["details"]["fix"]
        .as_str()
        .or_else(|| failed["error"]["fix"].as_str());
    assert!(
        fix.is_some_and(|text| text.contains("plugin index import")),
        "{failed}"
    );
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// Minimal POSIX ustar tar wrapped in a stored RFC 1952 gzip stream (no extra
/// dev-dependencies needed).
fn tar_gz_bundle(files: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let mut tar = Vec::new();
    for &(name, data, mode) in files {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..108].copy_from_slice(format!("{mode:07o}\0").as_bytes());
        header[108..116].copy_from_slice(b"0000000\0");
        header[116..124].copy_from_slice(b"0000000\0");
        header[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
        header[136..148].copy_from_slice(b"00000000000\0");
        header[148..156].copy_from_slice(b"        ");
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        let checksum: u32 = header.iter().map(|&b| u32::from(b)).sum();
        header[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
        tar.extend_from_slice(&header);
        tar.extend_from_slice(data);
        let rem = data.len() % 512;
        if rem != 0 {
            tar.resize(tar.len() + (512 - rem), 0);
        }
    }
    tar.resize(tar.len() + 1024, 0);

    // IEEE 802.3 CRC-32 for the RFC 1952 trailer.
    let mut crc = 0xffff_ffffu32;
    for &byte in &tar {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    crc ^= 0xffff_ffff;

    let mut gz = Vec::with_capacity(tar.len() + 32);
    gz.extend_from_slice(&[0x1f, 0x8b, 0x08, 0x00, 0, 0, 0, 0, 0x00, 0x03]);
    let mut chunks = tar.chunks(65_535).peekable();
    while let Some(chunk) = chunks.next() {
        let last = u8::from(chunks.peek().is_none());
        let len = u16::try_from(chunk.len()).unwrap();
        gz.push(last);
        gz.extend_from_slice(&len.to_le_bytes());
        gz.extend_from_slice(&(!len).to_le_bytes());
        gz.extend_from_slice(chunk);
    }
    gz.extend_from_slice(&crc.to_le_bytes());
    let isize_len = u32::try_from(tar.len()).unwrap();
    gz.extend_from_slice(&isize_len.to_le_bytes());
    gz
}

fn publish_plugin_v2(stand_in: &StandIn, manifest: &Value) -> String {
    let manifest_text = manifest.to_string();
    let layer = tar_gz_bundle(&[
        ("goodfetch-plugin.json", manifest_text.as_bytes(), 0o644),
        ("kettle", b"#!/bin/sh\nexit 0\n", 0o755),
    ]);
    let layer_digest = sha256_hex(&layer);
    let oci_manifest = serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": MANIFEST_MEDIA_TYPE,
        "config": {
            "mediaType": "application/vnd.oci.empty.v1+json",
            "digest": sha256_hex(b"{}"),
            "size": 2,
        },
        "layers": [{
            "mediaType": TAR_GZIP_MEDIA_TYPE,
            "digest": layer_digest,
            "size": layer.len(),
        }],
    }))
    .unwrap();
    let manifest_digest = sha256_hex(&oci_manifest);
    let host = Platform::host();
    let index = serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": INDEX_MEDIA_TYPE,
        "manifests": [{
            "mediaType": MANIFEST_MEDIA_TYPE,
            "digest": manifest_digest,
            "size": oci_manifest.len(),
            "platform": { "os": host.os, "architecture": host.architecture },
        }],
    }))
    .unwrap();
    let index_digest = sha256_hex(&index);
    let repo = "goodfetch-plugin-kettle";
    stand_in.serve(&format!("/v2/{repo}/manifests/{index_digest}"), index);
    stand_in.serve(
        &format!("/v2/{repo}/manifests/{manifest_digest}"),
        oci_manifest,
    );
    stand_in.serve(&format!("/v2/{repo}/blobs/{layer_digest}"), layer);
    index_digest
}

/// `plugin update` in a PTY prints the new digest, signer and permission diff
/// once with a single `[y/N]` prompt (no confirmation loop); answering `y`
/// updates the store, lockfile and Operator policy while answering `n` leaves
/// them unchanged.
///
/// Covers: story:7-11, story:7-12, operation:plugin.update
#[rstest]
#[case::approved("y", true)]
#[case::declined("n", false)]
fn plugin_update_prints_digest_signer_and_diff_once_in_pty(
    #[case] answer: &str,
    #[case] updated: bool,
) {
    let env = TestEnv::new();
    let host = "registry.test";
    let stand_in = StandIn::start(&[host]);
    let ca_file = env.folder().join("registry-ca.pem");
    stand_in.write_authority(&ca_file);

    // Installed v1.0.0 with `effects` on `api.shop.test` and `old.example`.
    place(
        &env,
        &json!({
            "manifest_version": "0.1",
            "name": "kettle",
            "version": "1.0.0",
            "kinds": ["notification"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "kettle",
            "network_level": "none",
            "permissions": {
                "domains": ["api.shop.test", "old.example"],
                "grants": ["effects"],
            },
        }),
    );

    // Target v2.0.0 widens `network_level` and `api.shop.test -> *.shop.test`,
    // adds `new.example`, and removes `old.example`.
    let v2_digest = publish_plugin_v2(
        &stand_in,
        &json!({
            "manifest_version": "0.1",
            "name": "kettle",
            "version": "2.0.0",
            "kinds": ["notification"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "kettle",
            "network_level": "proxy_tcp",
            "permissions": {
                "domains": ["*.shop.test", "new.example"],
                "grants": ["effects"],
            },
        }),
    );

    // Trust the local stand-in registry and allow the exact test digest in
    // Operator policy (the CLI binary uses the empty bundled attestation root).
    let layout = HomeLayout::of(env.home());
    let policy_path = layout.operator_policy();
    std::fs::create_dir_all(policy_path.parent().unwrap()).unwrap();
    std::fs::write(
        &policy_path,
        format!(
            "version = \"0.1\"\n\n[settings.network.resolve]\n\"{host}\" = [\"127.0.0.1\"]\n\n[settings.tls.trusted_cas]\n\"{host}\" = ['{}']\n\n[settings.plugins.kettle]\nallow = true\ndigest = \"{v2_digest}\"\n",
            ca_file.display()
        ),
    )
    .unwrap();

    let target_ref = format!(
        "{}/goodfetch-plugin-kettle@{v2_digest}",
        stand_in.origin(host)
    );
    let mut proc = pty(&env, COLLIE)
        .args(["plugin", "update", &target_ref])
        .spawn()
        .unwrap();
    if let Err(err) = proc.expect("[y/N]") {
        let out = proc.finish().map(|o| (o.exit, o.stripped()));
        panic!("prompt not shown: {err:?}; output: {out:?}");
    }
    proc.send_line(answer).unwrap();
    let output = proc.finish().unwrap();

    let shown = output.stripped();
    assert_eq!(output.exit, Some(0), "{shown}");
    // Prints digest, signer, and diff once with a single prompt (no confirmation loop).
    assert_eq!(shown.matches("[y/N]").count(), 1, "{shown}");
    assert_eq!(shown.matches("New digest:").count(), 1, "{shown}");
    assert_eq!(shown.matches("Signer:").count(), 1, "{shown}");
    assert!(shown.contains(&v2_digest), "{shown}");
    assert!(shown.contains("Added (1):"), "{shown}");
    assert!(shown.contains("Widened (2):"), "{shown}");
    assert!(shown.contains("Removed (1):"), "{shown}");

    let lockfile = std::fs::read_to_string(env.folder().join("goodfetch-plugins.lock")).unwrap();
    assert_eq!(
        lockfile.contains(&v2_digest),
        updated,
        "{lockfile}\n{shown}"
    );
    assert_eq!(
        env.home()
            .join("plugins")
            .join("kettle")
            .join("2.0.0")
            .exists(),
        updated,
        "{shown}"
    );
}

/// Under `--json` and in a non-terminal pipe/script, `plugin update` is
/// refused with `person_required` and exit `4`, its `fix` naming `plugin
/// update`, before any engine call.
///
/// Covers: story:7-11, story:7-12, operation:plugin.update
#[test]
fn plugin_update_refuses_json_and_piped_stdin_with_person_required() {
    let env = TestEnv::new();
    let target = reference();

    // 1. Under `--json`.
    let json_out = cli(&env, COLLIE)
        .args(["plugin", "update", &target, "--json"])
        .run()
        .unwrap();
    let failed: Value = serde_json::from_str(&json_out.stdout).unwrap();
    assert_eq!(json_out.exit, Some(4), "{json_out:?}");
    assert_eq!(failed["error"]["code"], json!("person_required"));
    assert!(
        failed["error"]["fix"]
            .as_str()
            .is_some_and(|fix| fix.contains("plugin update")),
        "{failed}"
    );

    // 2. From a pipe/script (no TTY, no `--json`).
    let pipe_out = cli(&env, COLLIE)
        .args(["plugin", "update", &target])
        .stdin("y\n")
        .run()
        .unwrap();
    assert_eq!(pipe_out.exit, Some(4), "{pipe_out:?}");
    assert!(
        pipe_out.stderr.contains("person_required") || pipe_out.stderr.contains("plugin update"),
        "{pipe_out:?}"
    );
}

/// Approving a Plugin declaring `network_level = "direct"` prints what the
/// engine cannot check at `direct` (destinations beyond configured ones, DNS
/// rebinding, loopback, local network, cloud-metadata address, and writes) and
/// writes `direct = true` under `approved` in the Operator policy.
///
/// Covers: story:7-56, command:plugin.add
#[test]
fn plugin_add_for_direct_prints_what_is_not_checked_and_approves_direct() {
    let env = TestEnv::new();
    place(
        &env,
        &json!({
            "manifest_version": "0.1",
            "name": "kettle",
            "version": "1.0.0",
            "kinds": ["notification"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "kettle",
            "network_level": "direct",
        }),
    );
    let policy = HomeLayout::of(env.home()).operator_policy();

    let mut added = pty(&env, COLLIE)
        .args(["plugin", "add", &reference()])
        .spawn()
        .unwrap();
    added.expect("[y/N]").unwrap();
    added.send_line("y").unwrap();
    added.expect("in the Operator policy? [y/N]").unwrap();
    added.send_line("y").unwrap();
    let output = added.finish().unwrap();

    let shown = output.stripped();
    assert_eq!(output.exit, Some(0), "{shown}");
    for phrase in [
        "reach the network directly",
        "The engine cannot check",
        "configured destinations",
        "DNS rebinding",
        "cloud-metadata",
        "Effects rule",
    ] {
        assert!(shown.contains(phrase), "missing `{phrase}` in:\n{shown}");
    }
    let written = std::fs::read_to_string(&policy).unwrap_or_default();
    assert!(
        written.contains("direct = true"),
        "expected `direct = true` in policy:\n{written}"
    );
}
