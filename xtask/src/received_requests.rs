//! The received-requests check (ADRs 0064 and 0086): a test reads the
//! requests the Proving ground's page hosts received only through
//! `ReceivedRequests` (`goodfetch-test-support`'s, or the conformance
//! harness's on its `ProvingGround`), whose `of_the_case` leaves out the
//! robots.txt the engine reads of each document's origin and the icon
//! Chromium asks for, both sent whenever they like, and whose `every` keeps
//! them for a claim about all a site saw. A count over the raw list raced
//! with them under load five times (fix-queue 53, 57, 64 and 71, and #1608).
//!
//! It reads every Rust file under `crates/` that names the observations,
//! and refuses each `["requests"]` index or `get("requests")` call on
//! anything but another index: a Run report's `["result"]["requests"]` is
//! another record. The Proving ground's own tests, outside `crates/`, check
//! the record itself.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::Path;

use anyhow::{Context as _, Result};

use crate::check::Finding;
use crate::tree;

/// The files that hold the helpers, the one place the list is read.
const HELPERS: [&str; 2] = [
    "crates/goodfetch-test-support/src/received_requests.rs",
    "crates/goodfetch-conformance/src/proving_ground.rs",
];

/// The word a file that reads the Proving ground's record names.
const OBSERVATIONS: &str = "observations";

/// The reads of the list by its key.
const READS: [&str; 2] = ["[\"requests\"]", "get(\"requests\")"];

pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for path in tree::files(root, "crates")? {
        let rust = Path::new(&path)
            .extension()
            .is_some_and(|extension| extension == "rs");
        if !rust || HELPERS.contains(&path.as_str()) {
            continue;
        }
        let text =
            std::fs::read_to_string(root.join(&path)).with_context(|| format!("reading {path}"))?;
        if !text.contains(OBSERVATIONS) {
            continue;
        }
        for (number, line) in text.lines().enumerate() {
            if raw_read(line) {
                findings.push(Finding::new(
                    "raw-requests-read",
                    format!("{path}:{}", number + 1),
                    "reads the Proving ground's requests itself; count them through `ReceivedRequests`: `of_the_case()` leaves out the engine's robots.txt reads and the browser's icons, `every()` keeps them for a claim about all a site saw, after a named state such as `robots_txt_read`",
                ));
            }
        }
    }
    Ok(findings)
}

/// Whether `line` reads a list by the key `requests` from a value that is
/// not itself an index into another, such as a Run report's result.
fn raw_read(line: &str) -> bool {
    READS.iter().any(|read| {
        line.match_indices(read).any(|(at, _)| {
            let before = line[..at].trim_end();
            let receiver = if read.starts_with('[') {
                before
            } else {
                before.strip_suffix('.').unwrap_or(before)
            };
            !receiver.ends_with(']')
        })
    })
}
