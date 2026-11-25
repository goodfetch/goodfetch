//! `xtask release assets`: the GitHub Release's files made ready to upload.
//! Each Product's archive and debug symbols per target and its SBOM, each
//! named after its executable, must be there (ADR 0094). A file over the
//! part size is split into parts below it, and the digest table lists each
//! uploaded file, part and whole file, so a download can reassemble and
//! verify the whole (ADR 0074).

use sha2::{Digest as _, Sha256};

use crate::workspace::{Workspace, finding};

/// The targets each Product ships an archive for (ADR 0024).
pub(crate) const TARGETS: [&str; 8] = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
];

/// The folder the built files are gathered in.
const FOLDER: &str = "dist";

/// The files each Product must put on the GitHub Release.
pub(crate) fn product_files() -> Vec<String> {
    let mut files = Vec::new();
    for executable in ["goodfetch-collie", "goodfetch-foxhound"] {
        for target in TARGETS {
            files.push(format!("{executable}-{target}.tar.xz"));
            files.push(format!("{executable}-{target}-debug.tar.xz"));
        }
        files.push(format!("{executable}.cdx.json"));
    }
    files
}

/// `xtask release assets` on the folder with parts of 80 bytes: each
/// Product file, its name's bytes, stays whole.
const SPLIT: [&str; 6] = ["release", "assets", "--folder", FOLDER, "--part-size", "80"];

/// A Chromium bundle of 200 bytes, so 3 parts.
fn bundle() -> Vec<u8> {
    (0..200_u8).collect()
}

/// A workspace whose folder holds every Product file, each its name's
/// bytes, and the Chromium bundle.
pub(crate) fn gathered() -> Workspace {
    let workspace = Workspace::from_fixture("release");
    for file in product_files() {
        workspace.write(&format!("{FOLDER}/{file}"), file.as_bytes());
    }
    workspace.write(&format!("{FOLDER}/chromium-linux-x64.tar.xz"), &bundle());
    workspace
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[test]
fn assets_split_a_file_over_the_part_size_and_list_every_digest() {
    let bundle = bundle();
    let workspace = gathered();

    let outcome = workspace.xtask(&SPLIT);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let bundle_file = |name: &str| workspace.read(&format!("{FOLDER}/{name}"));
    assert_eq!(bundle_file("chromium-linux-x64.tar.xz"), None);
    let parts: Vec<Vec<u8>> = ["001", "002", "003"]
        .iter()
        .map(|number| bundle_file(&format!("chromium-linux-x64.tar.xz.part-{number}")).unwrap())
        .collect();
    assert_eq!(parts.concat(), bundle);
    assert_eq!(bundle_file("chromium-linux-x64.tar.xz.part-004"), None);

    let mut expected: Vec<String> = product_files()
        .iter()
        .map(|file| format!("{}  {file}", sha256(file.as_bytes())))
        .collect();
    expected.push(format!("{}  chromium-linux-x64.tar.xz", sha256(&bundle)));
    for (number, part) in ["001", "002", "003"].iter().zip(&parts) {
        expected.push(format!(
            "{}  chromium-linux-x64.tar.xz.part-{number}",
            sha256(part)
        ));
    }
    expected.sort_by(|left, right| left[66..].cmp(&right[66..]));
    assert_eq!(
        workspace.read_text(&format!("{FOLDER}/SHA256SUMS")),
        expected.join("\n") + "\n"
    );
}

/// A re-run after a split reads the parts back, so the table still lists
/// the whole file's digest.
#[test]
fn assets_rerun_on_split_parts_writes_the_same_table() {
    let workspace = gathered();
    assert_eq!(workspace.xtask(&SPLIT).code, 0);
    let table = workspace.read_text(&format!("{FOLDER}/SHA256SUMS"));

    let outcome = workspace.xtask(&SPLIT);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(workspace.read_text(&format!("{FOLDER}/SHA256SUMS")), table);
}

#[test]
fn assets_refuse_a_folder_missing_a_product_file() {
    let workspace = gathered();
    workspace.remove(&format!(
        "{FOLDER}/goodfetch-foxhound-aarch64-apple-darwin-debug.tar.xz"
    ));
    workspace.remove(&format!("{FOLDER}/goodfetch-collie.cdx.json"));

    let outcome = workspace.xtask(&SPLIT);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(
        outcome.findings(),
        [
            finding("missing", "goodfetch-collie.cdx.json"),
            finding(
                "missing",
                "goodfetch-foxhound-aarch64-apple-darwin-debug.tar.xz"
            ),
        ]
    );
    assert_eq!(workspace.read(&format!("{FOLDER}/SHA256SUMS")), None);
    assert!(
        workspace
            .read(&format!("{FOLDER}/chromium-linux-x64.tar.xz"))
            .is_some()
    );
}
