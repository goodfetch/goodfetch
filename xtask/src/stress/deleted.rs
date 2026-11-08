//! The tests the base lists and the head does not: the Floor's "no skipped
//! or deleted test without a `Quality-change` trailer" (`TESTING.md`), read
//! from what each runner collects rather than from the diff's lines. A test
//! removed from one file and another of its name added elsewhere then no
//! longer cancel out, as they do for the floor guard; a test renamed, moved
//! to another module or file, or no longer compiled in is deleted under the
//! ID it had. Each fails the selection unless a trailer of the change from
//! the base to HEAD names that ID.
//!
//! An rstest case is matched on its description, not its number: rstest
//! names each case `case_<n>_<description>`, `<n>` its position zero-padded
//! to the width of the case count, so a case inserted or reordered renumbers
//! the ones after it. A case with no description, `case_<n>`, has only its
//! position to be matched on. A deleted case's finding, and the trailer that
//! accepts it, name its ID at the base, number and all.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Result;

use crate::check::Finding;
use crate::floor_guard::Trailers;
use crate::runners::{Runner, TestId};

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
    let mut deleted = Vec::new();
    for (runner, head) in at_head {
        let Some(base) = at_base.get(runner) else {
            continue;
        };
        let listed: BTreeSet<Cow<'_, str>> = head
            .iter()
            .map(|test| matched_name(*runner, test))
            .collect();
        deleted.extend(
            base.iter()
                .filter(|test| !listed.contains(&matched_name(*runner, test)))
                .map(|test| {
                    Finding::new(
                        "deleted-test",
                        test.to_string(),
                        format!(
                            "{} lists it at the base and not at the head; a `Quality-change` trailer naming this test ID records why, and names its replacement",
                            runner.name()
                        ),
                    )
                }),
        );
    }
    if deleted.is_empty() {
        return Ok(Judged {
            accepted: Vec::new(),
            refused: Vec::new(),
        });
    }
    let (accepted, refused) = Trailers::read(root, &format!("{base}..HEAD"))?.judge(deleted);
    Ok(Judged { accepted, refused })
}

/// The name `test` is matched on between the base and the head: its ID,
/// with each rstest case of a nextest test that has a description written
/// `case_*_<description>`. No Rust name holds a `*`, so that name is no
/// other test's.
fn matched_name(runner: Runner, test: &TestId) -> Cow<'_, str> {
    if runner != Runner::Nextest {
        return Cow::Borrowed(test.as_str());
    }
    let segments: Vec<Cow<'_, str>> = test
        .as_str()
        .split("::")
        .map(|segment| {
            described_case(segment).map_or(Cow::Borrowed(segment), |description| {
                Cow::Owned(format!("case_*_{description}"))
            })
        })
        .collect();
    Cow::Owned(segments.join("::"))
}

/// The description of an rstest case named `case_<n>_<description>`.
fn described_case(segment: &str) -> Option<&str> {
    let (position, description) = segment.strip_prefix("case_")?.split_once('_')?;
    (!position.is_empty() && position.bytes().all(|byte| byte.is_ascii_digit()))
        .then_some(description)
}
