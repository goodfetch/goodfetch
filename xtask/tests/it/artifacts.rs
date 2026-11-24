//! `xtask artifacts fetch` and `verify` (ADR 0066; TESTING.md, Upgrades and
//! version mismatches): the released artifacts `compat/releases.toml` lists,
//! both Products' files of each release (ADR 0094), fetched from a local
//! stand-in for the pinned artifact mirror, each checked by its SHA-256 and
//! by `gh attestation verify` against a stand-in `gh` that records each call
//! and refuses a bundle that says `bad`. The local tooling declares no
//! Feature.
#![cfg(unix)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use rstest::rstest;
use sha2::{Digest as _, Sha256};

use crate::github::GitHub;
use crate::workspace::{Finding, Outcome, Workspace, finding, write_executable};

/// The host `compat/releases.toml`'s URLs name; `--mirror` stands the
/// stand-in in for it.
const HOST: &str = "https://releases.example.invalid";

/// One artifact of the fixture's release `0.1.0`.
struct Entry {
    /// Its table in `compat/releases.toml`.
    table: &'static str,
    /// The target of a per-target artifact.
    target: Option<&'static str>,
    /// Its URL's path on the mirror.
    path: &'static str,
    /// Where it is kept, from the set's folder.
    kept: &'static str,
}

const ENTRIES: [Entry; 9] = [
    Entry {
        table: "[[release.collie.executables]]",
        target: Some("x86_64-unknown-linux-gnu"),
        path: "/v0.1.0/goodfetch-collie-x86_64-unknown-linux-gnu.tar.xz",
        kept: "0.1.0/collie/executables/x86_64-unknown-linux-gnu/goodfetch-collie-x86_64-unknown-linux-gnu.tar.xz",
    },
    Entry {
        table: "[release.collie.helm_chart]",
        target: None,
        path: "/charts/goodfetch-collie-0.1.0.tgz",
        kept: "0.1.0/collie/helm_chart/goodfetch-collie-0.1.0.tgz",
    },
    Entry {
        table: "[release.collie.image]",
        target: None,
        path: "/images/goodfetch-collie/sha256:01",
        kept: "0.1.0/collie/image/sha256_01",
    },
    Entry {
        table: "[[release.foxhound.executables]]",
        target: Some("x86_64-unknown-linux-gnu"),
        path: "/v0.1.0/goodfetch-foxhound-x86_64-unknown-linux-gnu.tar.xz",
        kept: "0.1.0/foxhound/executables/x86_64-unknown-linux-gnu/goodfetch-foxhound-x86_64-unknown-linux-gnu.tar.xz",
    },
    Entry {
        table: "[release.foxhound.helm_chart]",
        target: None,
        path: "/charts/goodfetch-foxhound-0.1.0.tgz",
        kept: "0.1.0/foxhound/helm_chart/goodfetch-foxhound-0.1.0.tgz",
    },
    Entry {
        table: "[release.foxhound.image]",
        target: None,
        path: "/images/goodfetch-foxhound/sha256:02",
        kept: "0.1.0/foxhound/image/sha256_02",
    },
    Entry {
        table: "[release.collie_client.npm]",
        target: None,
        path: "/npm/collie-client-0.1.0.tgz",
        kept: "0.1.0/collie_client/npm/collie-client-0.1.0.tgz",
    },
    Entry {
        table: "[release.collie_client.pypi]",
        target: None,
        path: "/pypi/goodfetch_collie_client-0.1.0-py3-none-any.whl",
        kept: "0.1.0/collie_client/pypi/goodfetch_collie_client-0.1.0-py3-none-any.whl",
    },
    Entry {
        table: "[release.collie_client.crates_io]",
        target: None,
        path: "/crates/goodfetch-collie-client-0.1.0.crate",
        kept: "0.1.0/collie_client/crates_io/goodfetch-collie-client-0.1.0.crate",
    },
];

fn sha256(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .fold(String::new(), |mut hex, byte| {
            write!(hex, "{byte:02x}").unwrap();
            hex
        })
}

/// What the mirror serves at an artifact's path.
fn content(entry: &Entry) -> String {
    format!("the bytes of {}", entry.path)
}

/// The attestation bundle's path beside the artifact's.
fn bundle_path(entry: &Entry) -> String {
    format!("{}.sigstore.jsonl", entry.path)
}

fn bundle(entry: &Entry) -> String {
    format!("{{\"attests\":\"{}\"}}\n", entry.path)
}

/// `compat/releases.toml` listing every entry with the digests the mirror
/// serves.
fn manifest() -> String {
    let mut text = String::from(
        "# The fixture's one release (ADR 0066).\n\n[[release]]\nversion = \"0.1.0\"\nformats = {}\n",
    );
    for entry in &ENTRIES {
        write!(text, "\n{}\n", entry.table).unwrap();
        if let Some(target) = entry.target {
            writeln!(text, "target = \"{target}\"").unwrap();
        }
        writeln!(
            text,
            "name = \"{}\"\nurl = \"{HOST}{}\"\nsha256 = \"{}\"\nattestation = {{ url = \"{HOST}{}\", sha256 = \"{}\" }}",
            entry.path.rsplit('/').next().unwrap(),
            entry.path,
            sha256(&content(entry)),
            bundle_path(entry),
            sha256(&bundle(entry)),
        )
        .unwrap();
    }
    text
}

/// What the mirror serves: every artifact and bundle, but with `changes`
/// applied, each the path and its body, `None` for a path it lacks.
fn mirror(changes: &[(String, Option<String>)]) -> GitHub {
    let mut routes: Vec<(String, String)> = ENTRIES
        .iter()
        .flat_map(|entry| {
            [
                (entry.path.to_owned(), content(entry)),
                (bundle_path(entry), bundle(entry)),
            ]
        })
        .collect();
    for (path, body) in changes {
        routes.retain(|(served, _)| served != path);
        if let Some(body) = body {
            routes.push((path.clone(), body.clone()));
        }
    }
    let routes: Vec<(&str, &str)> = routes
        .iter()
        .map(|(path, body)| (path.as_str(), body.as_str()))
        .collect();
    GitHub::serving(&routes)
}

/// The workspace with `manifest` as `compat/releases.toml` and a stand-in
/// `gh` under `shims/`, which appends each call's arguments to `gh-calls`,
/// prints a trusted root for `attestation trusted-root` and refuses an
/// `attestation verify` whose bundle says `bad`.
fn workspace(manifest: &str) -> Workspace {
    let workspace = Workspace::from_fixture("empty");
    workspace.write("compat/releases.toml", manifest.as_bytes());
    std::fs::create_dir_all(workspace.path().join("shims")).unwrap();
    write_executable(
        &workspace.path().join("shims/gh"),
        r#"#!/bin/sh
echo "$*" >> "$GH_CALLS"
case "$1 $2" in
"attestation trusted-root") echo '{"mediaType":"trusted-root"}' ;;
"attestation verify")
  while [ $# -gt 0 ]; do
    if [ "$1" = --bundle ]; then bundle="$2"; fi
    shift
  done
  if grep -q bad "$bundle"; then echo "bundle refused" >&2; exit 1; fi ;;
*) echo "unexpected gh call" >&2; exit 2 ;;
esac
"#,
    );
    workspace
}

fn artifacts(workspace: &Workspace, args: &[&str]) -> Outcome {
    let shims = workspace.path().join("shims");
    let path = std::env::join_paths(
        std::iter::once(shims).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let calls = workspace.path().join("gh-calls");
    let mut arguments = vec!["artifacts"];
    arguments.extend_from_slice(args);
    workspace.xtask_with_env(
        &arguments,
        &[("PATH", path.as_os_str()), ("GH_CALLS", calls.as_os_str())],
    )
}

fn fetch(workspace: &Workspace, mirror: &GitHub) -> Outcome {
    artifacts(workspace, &["fetch", "--mirror", mirror.url()])
}

/// The `gh` calls so far, one line each.
fn gh_calls(workspace: &Workspace) -> Vec<String> {
    workspace
        .read("gh-calls")
        .map(|calls| {
            String::from_utf8(calls)
                .unwrap()
                .lines()
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The versions file a passing command printed, and its content.
fn versions(outcome: &Outcome) -> (PathBuf, serde_json::Value) {
    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
    let path = PathBuf::from(outcome.stdout.trim_end());
    let text = std::fs::read_to_string(&path).unwrap();
    (path, serde_json::from_str(&text).unwrap())
}

fn failed(outcome: &Outcome) -> Vec<Finding> {
    assert_eq!(outcome.code, 1, "{}{}", outcome.stdout, outcome.stderr);
    outcome.findings()
}

/// The set's folder: where the versions file sits.
fn set(versions_file: &Path) -> PathBuf {
    versions_file.parent().unwrap().to_owned()
}

#[test]
fn fetch_keeps_both_products_files_verified_and_lists_the_release() {
    let workspace = workspace(&manifest());
    let mirror = mirror(&[]);

    let (path, listed) = versions(&fetch(&workspace, &mirror));

    assert_eq!(
        listed,
        serde_json::json!({"versions": [{"kind": "release", "version": "0.1.0", "folder": "0.1.0"}]})
    );
    let set = set(&path);
    for entry in &ENTRIES {
        assert_eq!(
            std::fs::read_to_string(set.join(entry.kept)).unwrap(),
            content(entry)
        );
    }
    let calls = gh_calls(&workspace);
    assert_eq!(calls[0], "attestation trusted-root");
    let root = set.join("trusted_root.jsonl");
    let verified: Vec<&String> = calls[1..].iter().collect();
    assert_eq!(verified.len(), ENTRIES.len());
    for (entry, call) in ENTRIES.iter().zip(verified) {
        let kept = set.join(entry.kept);
        assert_eq!(
            *call,
            format!(
                "attestation verify {} --bundle {}.sigstore.jsonl --custom-trusted-root {} --repo goodfetch/goodfetch",
                kept.display(),
                kept.display(),
                root.display()
            )
        );
    }
}

#[test]
fn fetch_fails_on_a_mismatched_digest() {
    let workspace = workspace(&manifest());
    let image = &ENTRIES[5];
    let mirror = mirror(&[(image.path.to_owned(), Some("other bytes".to_owned()))]);

    let findings = failed(&fetch(&workspace, &mirror));

    assert_eq!(findings, vec![finding("digest-mismatch", image.kept)]);
    let set = workspace
        .path()
        .join("target/artifacts")
        .join(sha256(&manifest()));
    assert!(!set.join(image.kept).exists());
    assert!(!set.join("versions.json").exists());
}

#[rstest]
#[case::collies_executable(0)]
#[case::foxhounds_chart(4)]
fn fetch_fails_on_a_missing_artifact_of_either_product(#[case] index: usize) {
    let workspace = workspace(&manifest());
    let entry = &ENTRIES[index];
    let mirror = mirror(&[(entry.path.to_owned(), None)]);

    let findings = failed(&fetch(&workspace, &mirror));

    assert_eq!(findings, vec![finding("missing-artifact", entry.kept)]);
}

#[test]
fn fetch_fails_on_a_bad_attestation() {
    let image = &ENTRIES[2];
    let bad = "{\"attests\":\"nothing, bad\"}\n";
    let workspace = workspace(&manifest().replace(&sha256(&bundle(image)), &sha256(bad)));
    let mirror = mirror(&[(bundle_path(image), Some(bad.to_owned()))]);

    let findings = failed(&fetch(&workspace, &mirror));

    assert_eq!(findings, vec![finding("attestation-refused", image.kept)]);
}

#[test]
fn an_empty_list_gives_the_current_build_as_the_only_version() {
    let workspace = workspace("# Nothing is released yet (ADR 0066).\n");
    let mirror = mirror(&[]);

    let (_, listed) = versions(&fetch(&workspace, &mirror));

    assert_eq!(
        listed,
        serde_json::json!({"versions": [{"kind": "current"}]})
    );
    assert_eq!(mirror.requests(), vec![]);
    assert_eq!(gh_calls(&workspace), Vec::<String>::new());
}

#[test]
fn a_second_fetch_of_the_same_manifest_uses_the_cache() {
    let workspace = workspace(&manifest());
    let mirror = mirror(&[]);
    let (first, _) = versions(&fetch(&workspace, &mirror));
    let requests = mirror.requests().len();
    let calls = gh_calls(&workspace).len();

    let (second, _) = versions(&fetch(&workspace, &mirror));

    assert_eq!(second, first);
    assert_eq!(mirror.requests().len(), requests);
    assert_eq!(gh_calls(&workspace).len(), calls);
}

#[test]
fn a_changed_manifest_is_another_set() {
    let workspace = workspace(&manifest());
    let mirror = mirror(&[]);
    let (first, _) = versions(&fetch(&workspace, &mirror));
    workspace.append("compat/releases.toml", "# A comment changes the set.\n");

    let (second, _) = versions(&fetch(&workspace, &mirror));

    assert_ne!(set(&second), set(&first));
}

#[test]
fn verify_checks_the_kept_set_offline() {
    let workspace = workspace(&manifest());
    let mirror = mirror(&[]);
    let (path, _) = versions(&fetch(&workspace, &mirror));
    let requests = mirror.requests().len();
    let calls = gh_calls(&workspace).len();

    let (printed, _) = versions(&artifacts(&workspace, &["verify"]));

    assert_eq!(printed, path);
    assert_eq!(mirror.requests().len(), requests);
    let verifies = gh_calls(&workspace)[calls..].to_vec();
    assert_eq!(verifies.len(), ENTRIES.len());
    assert!(
        verifies
            .iter()
            .all(|call| call.starts_with("attestation verify "))
    );
}

#[test]
fn verify_fails_on_a_changed_file() {
    let workspace = workspace(&manifest());
    let (path, _) = versions(&fetch(&workspace, &mirror(&[])));
    let chart = &ENTRIES[1];
    std::fs::write(set(&path).join(chart.kept), "changed").unwrap();

    let findings = failed(&artifacts(&workspace, &["verify"]));

    assert_eq!(findings, vec![finding("digest-mismatch", chart.kept)]);
}

#[test]
fn verify_fails_before_a_fetch() {
    let workspace = workspace(&manifest());

    let findings = failed(&artifacts(&workspace, &["verify"]));

    let mut expected = vec![finding("not-fetched", "trusted_root.jsonl")];
    expected.extend(
        ENTRIES
            .iter()
            .map(|entry| finding("not-fetched", entry.kept)),
    );
    assert_eq!(findings, expected);
    assert_eq!(gh_calls(&workspace), Vec::<String>::new());
}
