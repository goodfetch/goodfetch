//! `xtask release digests`: each Product's crate carries the digest of its
//! executable's archive per target, at its URL on the GitHub Release of the
//! same version, written before `cargo package` (ADRs 0051 and 0094).

use std::collections::BTreeMap;

use crate::release_assets::{TARGETS, gathered, sha256};
use crate::workspace::{Workspace, finding};

/// Each Product's manifest, and its executable.
const PRODUCTS: [(&str, &str); 2] = [
    ("crates/goodfetch-collie/Cargo.toml", "goodfetch-collie"),
    (
        "crates/foxhound/goodfetch-foxhound/Cargo.toml",
        "goodfetch-foxhound",
    ),
];

const DIGESTS: [&str; 6] = [
    "release",
    "digests",
    "--assets",
    "dist",
    "--version",
    "1.4.0",
];

/// The manifest's digest table: each target's URL and SHA-256.
fn digest_table(workspace: &Workspace, manifest: &str) -> BTreeMap<String, (String, String)> {
    let manifest: toml::Table = toml::from_str(&workspace.read_text(manifest)).unwrap();
    manifest["package"]["metadata"]["goodfetch-executables"]
        .as_table()
        .unwrap()
        .iter()
        .map(|(target, entry)| {
            let field = |key: &str| entry[key].as_str().unwrap().to_owned();
            (target.clone(), (field("url"), field("sha256")))
        })
        .collect()
}

#[test]
fn digests_write_each_product_crate_table_of_its_archives() {
    let workspace = gathered();
    assert_eq!(
        workspace
            .xtask(&["release", "assets", "--folder", "dist"])
            .code,
        0
    );

    let outcome = workspace.xtask(&DIGESTS);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    for (manifest, executable) in PRODUCTS {
        let expected: BTreeMap<String, (String, String)> = TARGETS
            .iter()
            .map(|target| {
                let archive = format!("{executable}-{target}.tar.xz");
                (
                    (*target).to_owned(),
                    (
                        format!("https://github.com/goodfetch/goodfetch/releases/download/v1.4.0/{archive}"),
                        sha256(archive.as_bytes()),
                    ),
                )
            })
            .collect();
        assert_eq!(digest_table(&workspace, manifest), expected, "{manifest}");
    }
}

#[test]
fn digests_refuse_a_digest_table_without_an_archive() {
    let workspace = gathered();
    assert_eq!(
        workspace
            .xtask(&["release", "assets", "--folder", "dist"])
            .code,
        0
    );
    let archive = "goodfetch-foxhound-x86_64-pc-windows-msvc.tar.xz";
    workspace.edit(
        "dist/SHA256SUMS",
        &format!("{}  {archive}\n", sha256(archive.as_bytes())),
        "",
    );
    let before: Vec<String> = PRODUCTS
        .iter()
        .map(|(manifest, _)| workspace.read_text(manifest))
        .collect();

    let outcome = workspace.xtask(&DIGESTS);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(outcome.findings(), [finding("missing", archive)]);
    let after: Vec<String> = PRODUCTS
        .iter()
        .map(|(manifest, _)| workspace.read_text(manifest))
        .collect();
    assert_eq!(after, before);
}
