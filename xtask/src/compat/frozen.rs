//! A frozen set's `SHA256SUMS`: one `<digest>  <path>` line per file, as
//! `sha256sum` writes and `sha256sum --check` reads them, so a changed byte
//! is found. Only the committed snapshots of what the current reader makes
//! of the set, under its `snapshots/` folder, may change (ADR 0066).

use std::fmt::Write as _;

use sha2::{Digest as _, Sha256};

use super::tree::Tree;

/// The digest file's name in each frozen set.
pub(super) const SUMS: &str = "SHA256SUMS";

/// The folder of a frozen set whose files are snapshots, never frozen.
const SNAPSHOTS: &str = "snapshots/";

/// The `SHA256SUMS` of a set's files.
pub(super) fn sums(set: &Tree) -> Vec<u8> {
    let mut text = String::new();
    for (path, bytes) in set {
        // Writing to a String cannot fail.
        let _ = writeln!(text, "{}  {path}", hex(&Sha256::digest(bytes)));
    }
    text.into_bytes()
}

/// The paths of a frozen set that break its `SHA256SUMS`: a listed file
/// changed or gone, an unlisted file that is no snapshot, or the digest file
/// itself when it is missing or unreadable.
pub(super) fn broken(set: &Tree) -> Vec<String> {
    let Some(listing) = set
        .get(SUMS)
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
    else {
        return vec![SUMS.to_owned()];
    };
    let mut broken = Vec::new();
    let mut listed = Vec::new();
    for line in listing.lines() {
        let Some((digest, path)) = line.split_once("  ").or_else(|| line.split_once(" *")) else {
            return vec![SUMS.to_owned()];
        };
        listed.push(path);
        let matches = set
            .get(path)
            .is_some_and(|bytes| hex(&Sha256::digest(bytes)) == digest);
        if !matches {
            broken.push(path.to_owned());
        }
    }
    for path in set.keys() {
        if path != SUMS && !path.starts_with(SNAPSHOTS) && !listed.contains(&path.as_str()) {
            broken.push(path.clone());
        }
    }
    broken.sort();
    broken
}

/// The digest of a whole set: of its `SHA256SUMS`, so one line of a log
/// tells two runs' sets apart.
pub(super) fn digest(set: &Tree) -> String {
    hex(&Sha256::digest(sums(set)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}
