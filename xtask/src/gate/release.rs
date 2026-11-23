//! What the Release suite's Linux x64 glibc cell runs again of the Pre-merge
//! suite's units that a hosted run passed on its tree (#3384): the members
//! whose tests the Release suite selects otherwise. The hosted Pre-merge
//! jobs and the cell's Rust step build each crate's tests for the same
//! target, with the same nextest profile and Cargo profile, so a crate's
//! test set differs only where a test reads the running suite.
//!
//! Size: one concern, that list of members and why each is on it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context as _, Result};

use super::plan::Layer;
use crate::affected::WorkspaceMembers;
use crate::git;

/// What a test that reads the running suite names: the test support's
/// accessor, the variable `just test` sets, and the Release suite itself.
/// A crate's tests that read the suite only through a helper of another
/// crate's, which names none of these, are not found.
const READS_THE_SUITE: [&str; 3] = [".suite()", "GOODFETCH_TEST_SUITE", "Suite::Release"];

/// The members whose units the Release cell runs again, each with why:
/// those of `shuffled`, whose tests Vitest, Playwright and pytest run and
/// which the cell runs on each Node line and each CPython, and each crate
/// with a tracked Rust file that reads the running suite. On any layer but
/// `release`, none.
pub(super) fn differing(
    layer: Layer,
    root: &Path,
    members: &WorkspaceMembers,
    shuffled: &BTreeSet<String>,
) -> Result<BTreeMap<String, String>> {
    if layer != Layer::Release {
        return Ok(BTreeMap::new());
    }
    let mut differing: BTreeMap<String, String> = shuffled
        .iter()
        .map(|member| {
            (
                member.clone(),
                "the Release suite runs its tests on each Node line and each CPython".to_owned(),
            )
        })
        .collect();
    let files = git::text(root, &["ls-files", "-z", "--", "*.rs"])?;
    for path in files.split('\0').filter(|path| !path.is_empty()) {
        let Some(owner) = members.owner(path) else {
            continue;
        };
        if differing.contains_key(&owner) {
            continue;
        }
        let file = root.join(path);
        let bytes = match std::fs::read(&file) {
            Ok(bytes) => bytes,
            // A tracked file the checkout lacks, such as one a sparse
            // checkout leaves out, reads nothing.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", file.display()));
            }
        };
        let text = String::from_utf8_lossy(&bytes);
        if READS_THE_SUITE.iter().any(|needle| text.contains(needle)) {
            differing.insert(owner, format!("its tests read the running suite ({path})"));
        }
    }
    Ok(differing)
}
