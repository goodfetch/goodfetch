//! The tests the base lists and the head does not: the Floor's "no skipped
//! or deleted test without a `Quality-change` trailer" (`TESTING.md`), read
//! from what each runner collects rather than from the diff's lines. A test
//! removed from one file and another of its name added elsewhere then no
//! longer cancel out, as they do for the floor guard; a test renamed, moved
//! to another module or file, or no longer compiled in is deleted under the
//! ID it had. Each fails the selection unless a trailer of the change from
//! the base to HEAD names that ID.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Result;

use crate::check::Finding;
use crate::floor_guard::Trailers;
use crate::quarantine::{Runner, TestId};

/// What the trailers make of the deleted tests.
pub(super) struct Judged {
    /// A `quality-change` note for each deleted test a trailer names.
    pub(super) accepted: Vec<Finding>,
    /// A `deleted-test` finding for each one no trailer names.
    pub(super) refused: Vec<Finding>,
}

/// Each test of a runner's listing at `base` that its listing at the head
/// lacks, judged by the trailers of the commits from `base` to HEAD. A
/// runner absent from `head` listed nothing, so it deletes nothing.
pub(super) fn judge(
    root: &Path,
    base: &str,
    at_base: &BTreeMap<Runner, BTreeSet<TestId>>,
    at_head: &BTreeMap<Runner, BTreeSet<TestId>>,
) -> Result<Judged> {
    let deleted: Vec<Finding> = at_head
        .iter()
        .filter_map(|(runner, head)| Some((*runner, at_base.get(runner)?.difference(head))))
        .flat_map(|(runner, gone)| {
            gone.map(move |test| {
                Finding::new(
                    "deleted-test",
                    test.to_string(),
                    format!(
                        "{} lists it at the base and not at the head; a `Quality-change` trailer naming this test ID records why, and names its replacement",
                        runner.name()
                    ),
                )
            })
        })
        .collect();
    if deleted.is_empty() {
        return Ok(Judged {
            accepted: Vec::new(),
            refused: Vec::new(),
        });
    }
    let (accepted, refused) = Trailers::read(root, &format!("{base}..HEAD"))?.judge(deleted);
    Ok(Judged { accepted, refused })
}
